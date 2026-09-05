//! Canonical record and part selection shared by the TUI, CLI, and MCP.

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::agents::AgentProvider;
use crate::query::{load_session_records, LoadMode};
use crate::record::{WorkAt, WorkPath, WorkRecord, WorkRef, WorkScope};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const WORKSET_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkSelectionKind {
    Terminal,
    Agent(AgentProvider),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkSelectionTarget {
    Scope {
        scope: WorkScope,
        kind: WorkSelectionKind,
        session: Option<String>,
    },
    Whole(WorkRecord),
    Parts {
        record: WorkRecord,
        parts: Vec<usize>,
    },
    Many(Vec<WorkSelectionTarget>),
}

impl WorkSelectionTarget {
    fn matches(&self, record: &WorkRecord) -> bool {
        match self {
            Self::Many(targets) => targets.iter().any(|target| target.matches(record)),
            Self::Scope {
                scope,
                kind,
                session,
            } => {
                if &record.work_ref.scope != scope {
                    return false;
                }
                let path_matches = match (kind, &record.work_ref.path) {
                    (WorkSelectionKind::Terminal, WorkPath::Terminal { .. }) => true,
                    (WorkSelectionKind::Agent(expected), WorkPath::Agent { provider, .. }) => {
                        expected == provider
                    }
                    _ => false,
                };
                path_matches
                    && session
                        .as_deref()
                        .is_none_or(|expected| record.work_ref.session() == expected)
            }
            Self::Whole(selected) => selected.work_ref.whole() == record.work_ref.whole(),
            Self::Parts {
                record: selected, ..
            } => selected.work_ref.whole() == record.work_ref.whole(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkSelectionAction {
    Include,
    Toggle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkSet {
    pub schema_version: u32,
    pub created_at: String,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Materialized records backing every anchor.
    records: Vec<WorkRecord>,
    /// Active selection positions; Whole covers every Part of its record.
    anchors: Vec<WorkRef>,
}

impl WorkSet {
    pub fn new(cwd: impl Into<String>, records: Vec<WorkRecord>) -> Self {
        let anchors = records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect();
        Self::from_parts(cwd, records, anchors)
    }

    pub fn from_parts(
        cwd: impl Into<String>,
        records: Vec<WorkRecord>,
        anchors: Vec<WorkRef>,
    ) -> Self {
        let mut set = Self {
            schema_version: WORKSET_SCHEMA_VERSION,
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            cwd: cwd.into(),
            name: None,
            records,
            anchors,
        };
        set.normalize_anchors();
        set
    }

    fn normalize_anchors(&mut self) {
        let mut canonical = Vec::with_capacity(self.anchors.len());
        for anchor in self.anchors.drain(..) {
            let whole = anchor.whole();
            if anchor.at == WorkAt::Whole {
                canonical.retain(|existing: &WorkRef| existing.whole() != whole);
                canonical.push(anchor);
            } else if !canonical.iter().any(|existing| {
                existing == &anchor || (existing.at == WorkAt::Whole && existing.whole() == whole)
            }) {
                canonical.push(anchor);
            }
        }
        for record in &self.records {
            if record.parts.is_empty() {
                continue;
            }
            let whole = record.work_ref.whole();
            let complete = record.parts.iter().all(|part| {
                canonical
                    .iter()
                    .any(|anchor| anchor.at == WorkAt::Part(part.seq) && anchor.whole() == whole)
            });
            if !complete || canonical.iter().any(|anchor| anchor == &whole) {
                continue;
            }
            let first = canonical
                .iter()
                .position(|anchor| anchor.whole() == whole)
                .expect("complete parts must have a matching anchor");
            canonical.retain(|anchor| anchor.whole() != whole);
            canonical.insert(first, whole);
        }
        self.anchors = canonical;
    }

    pub fn anchors(&self) -> &[WorkRef] {
        &self.anchors
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn records(&self) -> &[WorkRecord] {
        &self.records
    }

    pub fn records_mut(&mut self) -> &mut [WorkRecord] {
        &mut self.records
    }

    pub fn into_records(self) -> Vec<WorkRecord> {
        self.records
    }

    pub fn into_parts(self) -> (Vec<WorkRecord>, Vec<WorkRef>) {
        (self.records, self.anchors)
    }

    pub fn select_anchors(&mut self, anchors: Vec<WorkRef>) {
        self.records = records_for_anchors(&self.records, &anchors);
        self.anchors = anchors;
        self.normalize_anchors();
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != WORKSET_SCHEMA_VERSION {
            bail!(
                "unsupported WorkSet schema version {}; expected {}",
                self.schema_version,
                WORKSET_SCHEMA_VERSION
            );
        }
        for (index, anchor) in self.anchors.iter().enumerate() {
            if self.anchors[..index].contains(anchor) {
                bail!("WorkSet contains duplicate anchor");
            }
            if anchor.at == WorkAt::Whole
                && self
                    .anchors
                    .iter()
                    .any(|other| other.at != WorkAt::Whole && other.whole() == anchor.whole())
            {
                bail!("WorkSet contains Part anchors shadowed by Whole");
            }
            let Some(record) = find_record([self.records.as_slice()], anchor) else {
                bail!("WorkSet anchor has no backing record");
            };
            if let WorkAt::Part(seq) = anchor.at {
                if !record.parts.iter().any(|part| part.seq == seq) {
                    bail!("WorkSet Part anchor has no matching part");
                }
            }
        }
        Ok(())
    }

    /// Whether this selection covers an address. A Whole anchor covers every
    /// Part of the same record.
    pub fn contains(&self, anchor: &WorkRef) -> bool {
        self.anchors.iter().any(|selected| {
            selected == anchor
                || (selected.at == WorkAt::Whole && selected.whole() == anchor.whole())
        })
    }

    /// Include a complete record. Whole is the canonical representation for a
    /// record-wide selection, so any narrower anchors for it are removed.
    fn include_whole(&mut self, record: WorkRecord) {
        let whole = record.work_ref.whole();
        self.records.retain(|item| item.work_ref.whole() != whole);
        self.records.push(record);
        self.anchors.retain(|anchor| anchor.whole() != whole);
        self.anchors.push(whole);
    }

    fn include_records(&mut self, records: impl IntoIterator<Item = WorkRecord>) {
        for record in records {
            let whole = record.work_ref.whole();
            if !self
                .anchors
                .iter()
                .any(|anchor| anchor.at == WorkAt::Whole && anchor.whole() == whole)
            {
                self.include_whole(record);
            }
        }
    }

    pub fn apply_target(
        &mut self,
        action: WorkSelectionAction,
        target: WorkSelectionTarget,
        records: impl IntoIterator<Item = WorkRecord>,
    ) {
        match target {
            WorkSelectionTarget::Scope { .. } | WorkSelectionTarget::Many(_) => {
                let selected = records
                    .into_iter()
                    .filter(|record| target.matches(record))
                    .collect();
                match action {
                    WorkSelectionAction::Include => self.include_records(selected),
                    WorkSelectionAction::Toggle => self.toggle_records(selected),
                }
            }
            WorkSelectionTarget::Whole(record) => match action {
                WorkSelectionAction::Include => self.include_whole(record),
                WorkSelectionAction::Toggle => self.toggle_whole(record),
            },
            WorkSelectionTarget::Parts { record, parts } => match action {
                WorkSelectionAction::Include => self.include_parts(record, parts),
                WorkSelectionAction::Toggle => self.toggle_parts(record, parts),
            },
        }
    }

    /// Include selected parts of a record. A Whole anchor already covers them
    /// and remains the only stored representation.
    fn include_parts(&mut self, record: WorkRecord, parts: impl IntoIterator<Item = usize>) {
        let parts: Vec<_> = parts.into_iter().collect();
        if parts.is_empty() {
            return;
        }
        let whole = record.work_ref.whole();
        if !self
            .records
            .iter()
            .any(|item| item.work_ref.whole() == whole)
        {
            self.records.push(record.clone());
        }
        for seq in parts {
            self.anchors.push(whole.with_part(seq));
        }
        self.normalize_anchors();
    }

    /// Toggle a complete record selection.
    fn toggle_whole(&mut self, record: WorkRecord) {
        if self
            .anchors
            .iter()
            .any(|anchor| anchor.at == WorkAt::Whole && anchor.whole() == record.work_ref.whole())
        {
            self.exclude(&record.work_ref);
        } else {
            self.include_whole(record);
        }
    }

    /// Toggle a set of parts. A fully selected set is removed; otherwise the
    /// missing parts are added and a Whole selection remains authoritative.
    fn toggle_parts(&mut self, record: WorkRecord, parts: impl IntoIterator<Item = usize>) {
        let parts: Vec<_> = parts.into_iter().collect();
        if parts.is_empty() {
            return;
        }
        if parts
            .iter()
            .all(|seq| self.contains(&record.work_ref.with_part(*seq)))
        {
            let whole = record.work_ref.whole();
            let remove: HashSet<_> = parts.into_iter().map(|seq| whole.with_part(seq)).collect();
            let had_whole = self
                .anchors
                .iter()
                .any(|anchor| anchor.at == WorkAt::Whole && anchor.whole() == whole);
            let insert_at = self
                .anchors
                .iter()
                .position(|anchor| anchor.whole() == whole);
            self.anchors.retain(|anchor| {
                anchor.whole() != whole || (!had_whole && !remove.contains(anchor))
            });
            if had_whole {
                let remaining = record
                    .parts
                    .iter()
                    .map(|part| whole.with_part(part.seq))
                    .filter(|anchor| !remove.contains(anchor));
                if let Some(index) = insert_at {
                    self.anchors.splice(index..index, remaining);
                }
            }
            self.normalize_anchors();
            if !self.anchors.iter().any(|anchor| anchor.whole() == whole) {
                self.records.retain(|item| item.work_ref.whole() != whole);
            }
        } else {
            self.include_parts(record, parts);
        }
    }

    /// Toggle every record in a scope using the same Whole rule.
    fn toggle_records(&mut self, records: Vec<WorkRecord>) {
        if records.is_empty() {
            return;
        }
        let all_selected = records.iter().all(|record| {
            self.anchors.iter().any(|anchor| {
                anchor.at == WorkAt::Whole && anchor.whole() == record.work_ref.whole()
            })
        });
        if all_selected {
            for record in records {
                self.exclude(&record.work_ref);
            }
        } else {
            for record in records {
                self.include_whole(record);
            }
        }
    }

    /// The part-only subset, preserving anchor order. Whole anchors do not
    /// participate in a block-copy operation.
    pub fn parts_only(&self) -> Option<Self> {
        let anchors: Vec<_> = self
            .anchors()
            .iter()
            .filter(|anchor| anchor.at != WorkAt::Whole)
            .cloned()
            .collect();
        if anchors.is_empty() {
            return None;
        }
        let records = self
            .records
            .iter()
            .filter(|record| {
                anchors
                    .iter()
                    .any(|anchor| anchor.whole() == record.work_ref.whole())
            })
            .cloned()
            .collect();
        Some(Self::from_parts(self.cwd.clone(), records, anchors))
    }

    /// Remove a record and every Part anchor belonging to it.
    pub fn exclude(&mut self, record_ref: &WorkRef) {
        let whole = record_ref.whole();
        self.records
            .retain(|record| record.work_ref.whole() != whole);
        self.anchors.retain(|anchor| anchor.whole() != whole);
    }

    /// Fill in `parts` for any light-loaded record (empty `parts`) whose
    /// session file path is known. Each session file is loaded once (full
    /// view), then matching records are patched in place. Records without a
    /// session path (stdin sets) are already complete and stay untouched.
    pub fn materialize_parts(&mut self) -> Result<()> {
        // Group light records by their session file path, then load each
        // session's full records once and patch matching parts back.
        let mut needed: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, record) in self.records().iter().enumerate() {
            if !record.parts.is_empty() {
                continue;
            }
            let Some(path) = record.session.path.as_deref() else {
                continue;
            };
            needed.entry(path.to_string()).or_default().push(index);
        }

        for (path, indices) in &needed {
            // Any record in the group gives us the namespace; pick the first.
            let namespace = session_namespace(&self.records()[indices[0]].work_ref.path);
            let Some(namespace) = namespace else {
                continue;
            };
            let full = load_session_records(namespace, Path::new(path), LoadMode::Full)
                .with_context(|| format!("Failed to load full session {path} for {namespace}"))?;
            for index in indices {
                if let Some(record) = self.records_mut().get_mut(*index) {
                    if let Some(full_record) = full
                        .iter()
                        .find(|r| r.work_ref.path.index() == record.work_ref.path.index())
                    {
                        record.parts = full_record.parts.clone();
                    }
                }
            }
        }
        Ok(())
    }
}

/// Cache namespace for a record's session file, used by
/// [`WorkSet::materialize_parts`] to pick the right cache view when
/// re-loading full records.
fn session_namespace(path: &WorkPath) -> Option<&'static str> {
    match path {
        WorkPath::Agent { provider, .. } => Some(provider.command_name()),
        WorkPath::Terminal { .. } => Some("terminal"),
    }
}

pub fn records_for_anchors(records: &[WorkRecord], anchors: &[WorkRef]) -> Vec<WorkRecord> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for anchor in anchors {
        let record_ref = anchor.whole();
        if !seen.insert(record_ref.clone()) {
            continue;
        }
        if let Some(record) = find_record([records], anchor) {
            selected.push(record.clone());
        }
    }
    selected
}

pub fn find_record<'a, I>(record_sets: I, anchor: &WorkRef) -> Option<&'a WorkRecord>
where
    I: IntoIterator<Item = &'a [WorkRecord]>,
{
    let record_ref = anchor.whole();
    record_sets
        .into_iter()
        .flat_map(|records| records.iter())
        .find(|record| record.work_ref.whole() == record_ref)
}

pub fn require_record<'a, I>(record_sets: I, anchor: &WorkRef) -> Result<&'a WorkRecord>
where
    I: IntoIterator<Item = &'a [WorkRecord]>,
{
    find_record(record_sets, anchor)
        .with_context(|| format!("No record found for ref `{}`", anchor.whole()))
}
