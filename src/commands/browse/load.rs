//! Session pane loaders: catalog + workset I/O on [`crate::pane::SlidingPane`].
//!
//! Per selected source owns one [`SessionPane`]. Meta/body growth is driven by
//! [`SlidingPane::ensure_meta`] / [`SlidingPane::ensure_bodies`]; this module
//! only fulfills those requests over workset.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::commands::memory::filter::Filter;
use crate::commands::memory::workset::{self, QuerySource, QuerySourceResult};
use crate::pane::{
    keep_keys, MetaNeed, Pane, PaneInput, SlidingPane, StorePhase, Viewport, WindowRow,
    FETCH_CEILING, FETCH_FLOOR,
};
use crate::tui::workspace::{
    SourceLoadMarker, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
};
use sivtr_core::ai::AgentProvider;
use sivtr_core::origin::Reach;
use sivtr_core::record::WorkRecord;

/// Session meta without dialogue bodies.
#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub source: WorkspaceSource,
    pub session_id: String,
    pub modified: SystemTime,
    pub title: String,
    pub search_title: String,
}

pub type SessionKey = String;
pub type SessionBody = Vec<WorkRecord>;
pub type SessionPane = SlidingPane<SessionKey, SessionMeta, SessionBody>;

pub(crate) fn query_source(source: &WorkspaceSource, selector: impl Into<String>) -> QuerySource {
    let selector = selector.into();
    if source.is_remote() {
        QuerySource::remote(selector)
    } else {
        QuerySource::local(selector)
    }
}

/// Cap concurrent session-body parse threads.
const BODY_FETCH_CAP: usize = 2;

/// UI-facing per-source session pane.
#[derive(Clone, Debug, Default)]
pub struct SourceLoadState {
    pub pane: SessionPane,
    /// In-memory picker catalogs must not be replaced by a provider reload.
    pinned: bool,
}

impl SourceLoadState {
    pub fn ready_from_sessions(sessions: Vec<WorkspaceSession>, budget: usize) -> Self {
        let rows = sessions
            .into_iter()
            .map(|s| {
                let key = s.session_id.clone();
                let meta = SessionMeta {
                    source: s.source,
                    session_id: s.session_id,
                    modified: s.modified,
                    title: s.title,
                    search_title: s.search_title,
                };
                if s.body_loaded && !s.records.is_empty() {
                    WindowRow::with_body(key, meta, s.records)
                } else {
                    WindowRow::meta_only(key, meta)
                }
            })
            .collect();
        Self {
            pane: SessionPane::ready(rows, budget, true),
            pinned: true,
        }
    }

    fn force_catalog_meta(&mut self, viewport: Viewport) -> Option<MetaNeed> {
        (!self.pinned)
            .then(|| self.pane.force_meta(viewport))
            .flatten()
    }

    pub fn marker(&self) -> SourceLoadMarker {
        let store = self.pane.store();
        match store.phase {
            StorePhase::Idle => SourceLoadMarker::Idle,
            StorePhase::Booting => SourceLoadMarker::Loading,
            StorePhase::Ready if store.list_inflight => SourceLoadMarker::Loading,
            // A refresh or pagination failure with rows still on screen:
            // keep the stale list visible but flag the failed load.
            StorePhase::Ready if store.fail_message.is_some() => SourceLoadMarker::Failed,
            StorePhase::Ready => SourceLoadMarker::Ready,
            StorePhase::Failed => SourceLoadMarker::Failed,
        }
    }

    pub fn is_fetching(&self) -> bool {
        self.pane.is_fetching()
    }

    /// List projection: meta only — never clones dialogue bodies.
    pub fn visible_session_metas(&self) -> Vec<WorkspaceSession> {
        self.pane
            .rows()
            .iter()
            .map(row_to_workspace_session_meta)
            .collect()
    }

    /// Borrow records for a session id if the body is loaded in this pane.
    pub fn body(&self, session_id: &str) -> Option<&[WorkRecord]> {
        self.pane
            .rows()
            .iter()
            .find(|r| r.key == session_id && r.body_loaded)
            .and_then(|r| r.body.as_deref())
    }
}

fn row_to_workspace_session_meta(
    row: &WindowRow<SessionKey, SessionMeta, SessionBody>,
) -> WorkspaceSession {
    WorkspaceSession {
        source: row.meta.source.clone(),
        session_id: row.meta.session_id.clone(),
        modified: row.meta.modified,
        title: row.meta.title.clone(),
        search_title: row.meta.search_title.clone(),
        records: Vec::new(),
        body_loaded: row.body_loaded,
    }
}

#[derive(Debug)]
enum JobKind {
    Meta { budget: usize },
    Body { session_id: String },
}

#[derive(Debug)]
struct JobEvent {
    index: usize,
    gen: u64,
    kind: JobKind,
    result: std::result::Result<Vec<WorkspaceSession>, String>,
    exhausted: bool,
}

/// Background workset pump that fulfills session [`SlidingPane`] needs.
pub struct SourceLoadPump {
    tx: Sender<JobEvent>,
    rx: Receiver<JobEvent>,
    cwd: PathBuf,
    body_inflight: HashSet<String>,
    /// Sessions the current viewport still wants hydrated. A body job that
    /// finishes after the user has scrolled away is dropped, not applied.
    body_wanted: HashSet<(usize, String)>,
    /// `{source_idx}\0{session_id}` → error message for body loads that failed
    /// (thread spawn refused or the body query errored). Failed keys are not
    /// retried by [`SourceLoadPump::sync_bodies`] until an explicit refresh or
    /// the source is reselected.
    body_failed: HashMap<String, String>,
    /// Per-source generation for body jobs. Bumped when a source is dropped so
    /// events from a canceled job cannot apply bodies or record failures
    /// against a newer selection of the same source.
    body_generation: Vec<u64>,
}

impl SourceLoadPump {
    pub fn new(source_count: usize, cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            cwd,
            body_inflight: HashSet::new(),
            body_wanted: HashSet::new(),
            body_failed: HashMap::new(),
            body_generation: vec![0; source_count],
        }
    }

    pub fn kick(
        &mut self,
        sources: &[WorkspaceSource],
        source_scope: &[bool],
        states: &mut [SourceLoadState],
        viewport: Viewport,
        force: bool,
    ) {
        assert_eq!(sources.len(), source_scope.len());
        assert_eq!(sources.len(), states.len());
        for (idx, source) in sources.iter().enumerate() {
            if !source_scope[idx] {
                continue;
            }
            let need: Option<MetaNeed> = if force {
                states[idx].force_catalog_meta(viewport)
            } else {
                states[idx].pane.ensure_meta(viewport)
            };
            if let Some(MetaNeed { gen, budget }) = need {
                self.spawn_meta(idx, source, gen, budget);
            }
        }
    }

    pub fn refresh_selected(
        &mut self,
        sources: &[WorkspaceSource],
        source_scope: &[bool],
        states: &mut [SourceLoadState],
        viewport: Viewport,
    ) {
        assert_eq!(sources.len(), source_scope.len());
        assert_eq!(sources.len(), states.len());
        for (idx, source) in sources.iter().enumerate() {
            if !source_scope[idx] {
                continue;
            }
            // An explicit refresh is a retry: drop recorded body failures so
            // transient errors (remote timeouts, temporary transport issues)
            // get another chance once connectivity recovers.
            self.body_failed
                .retain(|k, _| !k.starts_with(&format!("{idx}\0")));
            if states[idx].pinned {
                continue;
            }
            if let Some(need) = states[idx].force_catalog_meta(viewport) {
                self.spawn_meta(idx, source, need.gen, need.budget);
            }
        }
    }

    pub fn sync_bodies(
        &mut self,
        sources: &[WorkspaceSource],
        states: &mut [SourceLoadState],
        keep: &HashSet<(usize, String)>,
    ) {
        assert_eq!(sources.len(), states.len());
        self.body_wanted.clone_from(keep);
        for (source_idx, state) in states.iter_mut().enumerate() {
            let keep_local: HashSet<String> = keep
                .iter()
                .filter(|(si, _)| *si == source_idx)
                .map(|(_, id)| id.clone())
                .collect();
            let missing = state.pane.ensure_bodies(keep_local);
            let source = &sources[source_idx];
            for session_id in missing {
                if self.body_inflight.len() >= BODY_FETCH_CAP {
                    return;
                }
                let ik = format!("{source_idx}\0{session_id}");
                if self.body_inflight.contains(&ik) || self.body_failed.contains_key(&ik) {
                    continue;
                }
                self.body_inflight.insert(ik);
                self.spawn_body(source_idx, source, &session_id);
            }
        }
    }

    pub fn drop_unselected(&mut self, source_scope: &[bool], states: &mut [SourceLoadState]) {
        assert_eq!(source_scope.len(), states.len());
        for (idx, sel) in source_scope.iter().enumerate() {
            if *sel {
                continue;
            }
            states[idx].pane.clear();
            self.body_inflight
                .retain(|k| !k.starts_with(&format!("{idx}\0")));
            self.body_failed
                .retain(|k, _| !k.starts_with(&format!("{idx}\0")));
            // Invalidate body jobs spawned for this source: their results
            // must not touch the state of a later re-selection.
            self.body_generation[idx] = self.body_generation[idx].saturating_add(1);
        }
    }

    /// True while at least one body-hydration job is still running.
    ///
    /// `SessionColumn::is_fetching` uses this to keep the event loop on its short poll while
    /// bodies stream in: metadata may already be `Ready`, but a copy action against a not-yet
    /// hydrated body would return stale or empty content.
    pub fn has_inflight_bodies(&self) -> bool {
        !self.body_inflight.is_empty()
    }

    fn spawn_meta(&mut self, idx: usize, source: &WorkspaceSource, gen: u64, budget: usize) {
        let budget = budget.clamp(FETCH_FLOOR, FETCH_CEILING);
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        let spawned = thread::Builder::new()
            .name(format!("sivtr-meta-{idx}"))
            .spawn(move || {
                let qs = query_source(&source, source.selector());
                let (result, exhausted) = match workset::query_sources(
                    &[qs],
                    Filter::browse_session_page(budget),
                    Some(&cwd),
                ) {
                    Ok(mut results) => match results.pop() {
                        Some(QuerySourceResult::Ok(set)) => {
                            let n = set.records().len();
                            let mut sessions = sessions_from_records(&source, set.into_records());
                            for s in &mut sessions {
                                s.records.clear();
                                s.body_loaded = false;
                            }
                            sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
                            (Ok(sessions), n < budget)
                        }
                        Some(QuerySourceResult::Err(m)) => (Err(m), false),
                        None => (Err("empty query".into()), false),
                    },
                    Err(e) => (Err(format!("{e:#}")), false),
                };
                let _ = tx.send(JobEvent {
                    index: idx,
                    gen,
                    kind: JobKind::Meta { budget },
                    result,
                    exhausted,
                });
            });
        if let Err(error) = spawned {
            // Without this event the pane would wait forever for a page that
            // will never arrive; surface the spawn failure like any other
            // query error.
            let _ = self.tx.send(JobEvent {
                index: idx,
                gen,
                kind: JobKind::Meta { budget },
                result: Err(format!("failed to spawn meta loader thread: {error}")),
                exhausted: false,
            });
        }
    }

    fn spawn_body(&mut self, idx: usize, source: &WorkspaceSource, session_id: &str) {
        let gen = self.body_generation.get(idx).copied().unwrap_or(0);
        let selector = source.selector();
        let cwd = self.cwd.clone();
        let tx = self.tx.clone();
        let source = source.clone();
        let session_id_owned = session_id.to_string();
        let spawned = thread::Builder::new()
            .name(format!("sivtr-body-{idx}"))
            .spawn(move || {
                let sel = format!("{selector}/{session_id_owned}");
                let qs = query_source(&source, sel);
                let result = match workset::query_sources(&[qs], Filter::none(), Some(&cwd)) {
                    Ok(mut results) => match results.pop() {
                        Some(QuerySourceResult::Ok(mut set)) => match set.materialize_parts() {
                            Ok(()) => {
                                let mut sessions =
                                    sessions_from_records(&source, set.into_records());
                                for s in &mut sessions {
                                    s.body_loaded = !s.records.is_empty();
                                }
                                Ok(sessions)
                            }
                            Err(e) => Err(format!("{e:#}")),
                        },
                        Some(QuerySourceResult::Err(m)) => Err(m),
                        None => Err("empty body".into()),
                    },
                    Err(e) => Err(format!("{e:#}")),
                };
                let _ = tx.send(JobEvent {
                    index: idx,
                    gen,
                    kind: JobKind::Body {
                        session_id: session_id_owned.clone(),
                    },
                    result,
                    exhausted: true,
                });
            });
        if let Err(error) = spawned {
            let _ = self.tx.send(JobEvent {
                index: idx,
                gen,
                kind: JobKind::Body {
                    session_id: session_id.to_string(),
                },
                result: Err(format!("failed to spawn body loader thread: {error}")),
                exhausted: true,
            });
        }
    }

    pub fn drain(&mut self, states: &mut [SourceLoadState]) -> bool {
        let mut changed = false;
        loop {
            match self.rx.try_recv() {
                Ok(ev) => {
                    if self.apply(ev, states) {
                        changed = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    fn apply(&mut self, ev: JobEvent, states: &mut [SourceLoadState]) -> bool {
        let Some(state) = states.get_mut(ev.index) else {
            return false;
        };
        match ev.kind {
            JobKind::Body { session_id } => {
                // Ignore events from a canceled body job: after the source was
                // dropped and reselected, an old thread's result must neither
                // apply bodies nor record failures against the new selection.
                if self.body_generation.get(ev.index).copied().unwrap_or(0) != ev.gen {
                    return false;
                }
                let ik = format!("{}\0{session_id}", ev.index);
                self.body_inflight.remove(&ik);
                if !self.body_wanted.contains(&(ev.index, session_id.clone())) {
                    // Scrolled away while the parse was in flight: drop the
                    // payload so a large session does not land in the store.
                    return false;
                }
                match ev.result {
                    Ok(sessions) => {
                        if sessions.is_empty() {
                            // A successful query that returned no records is
                            // still a terminal outcome: leave the key out of
                            // the retry set instead of respawning it on every
                            // sync_bodies pass. `true` wakes the picker so the
                            // `[!]` marker is noticed.
                            self.body_failed.insert(ik, "no body content".into());
                            return true;
                        }
                        let mut requested_applied = false;
                        for s in sessions {
                            let requested = s.session_id == session_id;
                            if state.pane.apply_body(&s.session_id, s.records) && requested {
                                requested_applied = true;
                            }
                        }
                        if !requested_applied {
                            self.body_failed.insert(ik, "session not found".into());
                        }
                        true
                    }
                    Err(message) => {
                        // The body query itself failed (timeout, remote error,
                        // ...). Keep the message instead of discarding it and
                        // stop the pump from retrying a key that cannot load.
                        // `true` wakes the picker so the change is noticed.
                        self.body_failed.insert(ik, message);
                        true
                    }
                }
            }
            JobKind::Meta { budget } => match ev.result {
                Ok(sessions) => {
                    let rows = sessions
                        .into_iter()
                        .map(|s| {
                            WindowRow::meta_only(
                                s.session_id.clone(),
                                SessionMeta {
                                    source: s.source,
                                    session_id: s.session_id,
                                    modified: s.modified,
                                    title: s.title,
                                    search_title: s.search_title,
                                },
                            )
                        })
                        .collect();
                    state.pane.apply_meta_ok(ev.gen, budget, ev.exhausted, rows)
                }
                Err(message) => state.pane.apply_meta_err(ev.gen, message),
            },
        }
    }
}

// ── Session column as unified [`Pane`] ──────────────────────────────────

/// Multi-source session column. Implements [`Pane`]; picker only calls
/// `poll` / `ensure` / `sessions`.
pub struct SessionColumn {
    sources: Vec<WorkspaceSource>,
    states: Vec<SourceLoadState>,
    pump: SourceLoadPump,
    /// Last merged list length (for multi-source budget expansion).
    merged_len: usize,
}

/// One-frame context for session ensure.
pub struct SessionCtx<'a> {
    pub source_scope: &'a [bool],
    /// Merged sessions currently shown (for body keep mapping).
    pub sessions: &'a [WorkspaceSession],
    pub session_scope: &'a [bool],
    /// When true, the search result owns the full session corpus and its bodies.
    pub search_active: bool,
}

impl SessionColumn {
    pub fn new(sources: Vec<WorkspaceSource>, states: Vec<SourceLoadState>, cwd: PathBuf) -> Self {
        let n = sources.len();
        Self {
            sources,
            states,
            pump: SourceLoadPump::new(n, cwd),
            merged_len: 0,
        }
    }

    pub fn sources(&self) -> &[WorkspaceSource] {
        &self.sources
    }

    pub fn markers(&self) -> Vec<SourceLoadMarker> {
        self.states.iter().map(SourceLoadState::marker).collect()
    }

    pub fn collect(&self, source_scope: &[bool]) -> Vec<WorkspaceSession> {
        // Meta-only list projection — bodies stay in SlidingPane.
        collect_ready_sessions(&self.sources, source_scope, &self.states)
    }

    /// Records for a session if loaded (source resolved via session.source).
    pub fn body_for(&self, session: &WorkspaceSession) -> Option<&[WorkRecord]> {
        let idx = source_index_for_session(&self.sources, session)?;
        self.states.get(idx)?.body(&session.session_id)
    }

    /// Error message for a session whose body hydration failed, if any.
    pub fn body_failure(&self, session: &WorkspaceSession) -> Option<&str> {
        let idx = source_index_for_session(&self.sources, session)?;
        let ik = format!("{}\0{}", idx, session.session_id);
        self.pump.body_failed.get(&ik).map(String::as_str)
    }

    /// Bootstrap / force-load selected sources.
    pub fn kick(&mut self, source_scope: &[bool], viewport: Viewport, force: bool) {
        self.pump.kick(
            &self.sources,
            source_scope,
            &mut self.states,
            viewport,
            force,
        );
    }

    /// `R` reload of given source mask.
    pub fn refresh(&mut self, source_scope: &[bool], viewport: Viewport) {
        self.pump
            .refresh_selected(&self.sources, source_scope, &mut self.states, viewport);
    }
}

impl Pane for SessionColumn {
    type Ctx<'a> = SessionCtx<'a>;

    fn poll(&mut self) -> bool {
        self.pump.drain(&mut self.states)
    }

    fn ensure(&mut self, ctx: SessionCtx<'_>, input: &PaneInput<'_>) {
        if ctx.search_active {
            self.merged_len = ctx.sessions.len();
            return;
        }
        self.pump
            .drop_unselected(ctx.source_scope, &mut self.states);
        if input.force {
            self.pump.refresh_selected(
                &self.sources,
                ctx.source_scope,
                &mut self.states,
                input.viewport,
            );
        } else {
            self.pump.kick(
                &self.sources,
                ctx.source_scope,
                &mut self.states,
                input.viewport,
                false,
            );
        }
        let keep = body_keep_set(
            &self.sources,
            ctx.sessions,
            input.focus,
            ctx.session_scope,
            input.neighbor_radius,
        );
        self.pump
            .sync_bodies(&self.sources, &mut self.states, &keep);
    }

    fn len(&self) -> usize {
        self.merged_len
    }

    fn is_fetching(&self) -> bool {
        // Body hydration runs past metadata loading; keep the poll cycle alive
        // until every in-flight body job reports, so completion and failure
        // events do not sit undrained behind a one-hour idle timeout.
        self.states.iter().any(SourceLoadState::is_fetching) || self.pump.has_inflight_bodies()
    }
}

pub fn body_keep_set(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    focus_idx: usize,
    session_scope: &[bool],
    neighbor_radius: usize,
) -> HashSet<(usize, String)> {
    let index_keys: Vec<usize> = (0..sessions.len()).collect();
    let keep_idx = keep_keys(&index_keys, focus_idx, session_scope, neighbor_radius);
    keep_idx
        .into_iter()
        .filter_map(|i| {
            let session = sessions.get(i)?;
            let si = source_index_for_session(sources, session)?;
            Some((si, session.session_id.clone()))
        })
        .collect()
}

pub fn workspace_source_catalog(
    providers: &[AgentProvider],
    cwd: &Path,
) -> Result<Vec<WorkspaceSource>> {
    let mut sources = Vec::new();
    sources.push(WorkspaceSource::terminal());
    for provider in providers {
        sources.push(WorkspaceSource::agent(*provider));
    }
    // Mount aliases come from the same origin registry every query path uses;
    // it lists mounts only while the daemon is already running.
    let registry = crate::origins::collect(cwd).context("Failed to collect the origin registry")?;
    for entry in registry.entries() {
        if !matches!(entry.reach, Reach::Remote { .. }) {
            continue;
        }
        let origin = &entry.origin;
        sources.push(WorkspaceSource::remote(
            &origin.name,
            WorkspaceSourceKind::Terminal,
        ));
        for provider in providers {
            sources.push(WorkspaceSource::remote(
                &origin.name,
                WorkspaceSourceKind::Agent(*provider),
            ));
        }
    }
    Ok(sources)
}

/// Meta-only merge of ready sources (bodies remain in each source pane).
pub fn collect_ready_sessions(
    sources: &[WorkspaceSource],
    source_scope: &[bool],
    states: &[SourceLoadState],
) -> Vec<WorkspaceSession> {
    assert_eq!(sources.len(), source_scope.len());
    assert_eq!(sources.len(), states.len());
    let mut sessions = Vec::new();
    for (idx, _) in sources.iter().enumerate() {
        if !source_scope[idx] {
            continue;
        }
        sessions.extend(states[idx].visible_session_metas());
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    sessions
}

pub fn source_index_for_session(
    sources: &[WorkspaceSource],
    session: &WorkspaceSession,
) -> Option<usize> {
    sources.iter().position(|source| source == &session.source)
}

pub fn sessions_from_records(
    source: &WorkspaceSource,
    records: Vec<WorkRecord>,
) -> Vec<WorkspaceSession> {
    let mut groups: BTreeMap<String, Vec<WorkRecord>> = BTreeMap::new();
    for record in records {
        let key = record.work_ref.session().to_string();
        groups.entry(key).or_default().push(record);
    }
    let mut sessions = Vec::with_capacity(groups.len());
    for (session_id, mut records) in groups {
        // Newest dialogue first: the record index grows over time, so a
        // reversed index order puts the latest turn at the top.
        records.sort_by_key(|record| std::cmp::Reverse(record.work_ref.path.index()));
        let modified = records
            .iter()
            .filter_map(record_modified)
            .max()
            .unwrap_or(UNIX_EPOCH);
        let search_title = session_search_title(&session_id, &records);
        let short_id = session_id.chars().take(8).collect::<String>();
        let title = if short_id.is_empty() {
            search_title.clone()
        } else {
            format!("{search_title}  [{short_id}]")
        };
        let body_loaded = !records.is_empty();
        sessions.push(WorkspaceSession {
            source: source.clone(),
            session_id,
            modified,
            title,
            search_title,
            records,
            body_loaded,
        });
    }
    sessions
}

fn session_search_title(session_id: &str, records: &[WorkRecord]) -> String {
    records
        .iter()
        .find_map(|record| {
            let title = record.title.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        })
        .unwrap_or_else(|| session_id.to_string())
}

fn record_modified(record: &WorkRecord) -> Option<SystemTime> {
    let stamp = record.time.primary_at()?;
    let dt = sivtr_core::time::parse_timestamp(stamp)?;
    let secs = dt.timestamp().max(0) as u64;
    let nanos = dt.timestamp_subsec_nanos();
    Some(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_nanos(nanos.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTime,
    };

    #[test]
    fn sessions_from_records_groups_by_session() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let records = vec![
            test_record("s1", 1, "first", "2026-07-17T10:00:00Z"),
            test_record("s1", 2, "second", "2026-07-17T11:00:00Z"),
            test_record("s2", 1, "other", "2026-07-17T12:00:00Z"),
        ];
        let sessions = sessions_from_records(&source, records);
        assert_eq!(sessions.len(), 2);
        let s1 = sessions
            .iter()
            .find(|s| s.session_id == "s1")
            .expect("s1 session present");
        // Newest dialogue first: index 2 precedes index 1.
        assert_eq!(
            s1.records
                .iter()
                .map(|r| r.work_ref.index())
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn body_keep_set_uses_shared_keep_keys() {
        let sources = vec![WorkspaceSource::agent(AgentProvider::Codex)];
        let sessions = vec![
            WorkspaceSession {
                source: sources[0].clone(),
                session_id: "a".into(),
                modified: UNIX_EPOCH,
                title: "a".into(),
                search_title: "a".into(),
                records: vec![],
                body_loaded: false,
            },
            WorkspaceSession {
                source: sources[0].clone(),
                session_id: "b".into(),
                modified: UNIX_EPOCH,
                title: "b".into(),
                search_title: "b".into(),
                records: vec![],
                body_loaded: false,
            },
            WorkspaceSession {
                source: sources[0].clone(),
                session_id: "c".into(),
                modified: UNIX_EPOCH,
                title: "c".into(),
                search_title: "c".into(),
                records: vec![],
                body_loaded: false,
            },
        ];
        let session_scope = [false, false, false];
        let keep = body_keep_set(&sources, &sessions, 1, &session_scope, 1);
        assert!(keep.contains(&(0, "a".into())));
        assert!(keep.contains(&(0, "b".into())));
        assert!(keep.contains(&(0, "c".into())));
    }

    #[test]
    fn preloaded_sessions_stay_pinned_on_refresh() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let session = WorkspaceSession {
            source: source.clone(),
            session_id: "s1".into(),
            modified: UNIX_EPOCH,
            title: "s1".into(),
            search_title: "s1".into(),
            records: vec![test_record("s1", 1, "payload", "2026-07-17T10:00:00Z")],
            body_loaded: true,
        };
        let state = SourceLoadState::ready_from_sessions(vec![session], 1);
        let mut column = SessionColumn::new(vec![source], vec![state], PathBuf::from("."));
        let viewport = Viewport {
            first: 0,
            visible: 10,
        };

        column.kick(&[true], viewport, true);
        column
            .pump
            .body_failed
            .insert("0\0s1".into(), "transient".into());
        column.refresh(&[true], viewport);

        assert!(!column.states[0].pane.store().list_inflight);
        assert_eq!(column.states[0].body("s1").unwrap().len(), 1);
        assert!(column.pump.body_failed.is_empty());
    }

    #[test]
    fn body_load_failure_is_preserved_and_not_retried() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sources = vec![source.clone()];
        let meta = SessionMeta {
            source: source.clone(),
            session_id: "s1".into(),
            modified: UNIX_EPOCH,
            title: "s1".into(),
            search_title: "s1".into(),
        };
        let state = SourceLoadState {
            pane: SessionPane::ready(vec![WindowRow::meta_only("s1".into(), meta)], 10, true),
            ..Default::default()
        };
        let mut column = SessionColumn::new(sources.clone(), vec![state], PathBuf::from("."));

        // Simulate a refused spawn / failed query for session "s1".
        column
            .pump
            .body_failed
            .insert("0\0s1".into(), "boom".into());

        // The body is still missing from the pane, but a sync pass must not
        // re-spawn the failed key or clear its recorded error.
        let keep: HashSet<(usize, String)> = [(0, "s1".into())].into();
        column.pump.sync_bodies(&sources, &mut column.states, &keep);
        assert!(column.pump.body_inflight.is_empty());
        assert_eq!(
            column.pump.body_failed.get("0\0s1").map(String::as_str),
            Some("boom")
        );
    }

    #[test]
    fn empty_body_result_is_terminal_and_not_retried() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sources = vec![source.clone()];
        let meta = SessionMeta {
            source: source.clone(),
            session_id: "s1".into(),
            modified: UNIX_EPOCH,
            title: "s1".into(),
            search_title: "s1".into(),
        };
        let state = SourceLoadState {
            pane: SessionPane::ready(vec![WindowRow::meta_only("s1".into(), meta)], 10, true),
            ..Default::default()
        };
        let mut column = SessionColumn::new(sources.clone(), vec![state], PathBuf::from("."));

        // A successful body query that returns no records must still settle
        // the key instead of respawning it on every sync_bodies pass.
        column.pump.body_inflight.insert("0\0s1".into());
        column.pump.body_wanted.insert((0, "s1".into()));
        let empty = JobEvent {
            index: 0,
            gen: 0,
            kind: JobKind::Body {
                session_id: "s1".into(),
            },
            result: Ok(vec![]),
            exhausted: true,
        };
        assert!(column.pump.apply(empty, &mut column.states));
        assert!(column.pump.body_inflight.is_empty());
        assert!(column.pump.body_failed.contains_key("0\0s1"));

        // And a sync pass must not re-spawn the settled key.
        let keep: HashSet<(usize, String)> = [(0, "s1".into())].into();
        column.pump.sync_bodies(&sources, &mut column.states, &keep);
        assert!(column.pump.body_inflight.is_empty());
    }

    #[test]
    fn unmatched_body_result_is_terminal_and_not_retried() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sources = vec![source.clone()];
        let meta = SessionMeta {
            source: source.clone(),
            session_id: "s1".into(),
            modified: UNIX_EPOCH,
            title: "s1".into(),
            search_title: "s1".into(),
        };
        let other_meta = SessionMeta {
            source: source.clone(),
            session_id: "other".into(),
            modified: UNIX_EPOCH,
            title: "other".into(),
            search_title: "other".into(),
        };
        let state = SourceLoadState {
            pane: SessionPane::ready(
                vec![
                    WindowRow::meta_only("s1".into(), meta),
                    WindowRow::meta_only("other".into(), other_meta),
                ],
                10,
                true,
            ),
            ..Default::default()
        };
        let mut column = SessionColumn::new(sources.clone(), vec![state], PathBuf::from("."));

        column.pump.body_inflight.insert("0\0s1".into());
        column.pump.body_wanted.insert((0, "s1".into()));
        let unmatched = JobEvent {
            index: 0,
            gen: 0,
            kind: JobKind::Body {
                session_id: "s1".into(),
            },
            result: Ok(vec![WorkspaceSession {
                source,
                session_id: "other".into(),
                modified: UNIX_EPOCH,
                title: "other".into(),
                search_title: "other".into(),
                records: vec![],
                body_loaded: true,
            }]),
            exhausted: true,
        };

        assert!(column.pump.apply(unmatched, &mut column.states));
        assert!(column.pump.body_inflight.is_empty());
        assert_eq!(
            column.pump.body_failed.get("0\0s1").map(String::as_str),
            Some("session not found")
        );

        let keep: HashSet<(usize, String)> = [(0, "s1".into())].into();
        column.pump.sync_bodies(&sources, &mut column.states, &keep);
        assert!(column.pump.body_inflight.is_empty());
    }

    #[test]
    fn refresh_clears_failed_bodies_and_stale_jobs_are_ignored() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sources = vec![source.clone()];
        let meta = SessionMeta {
            source: source.clone(),
            session_id: "s1".into(),
            modified: UNIX_EPOCH,
            title: "s1".into(),
            search_title: "s1".into(),
        };
        let state = SourceLoadState {
            pane: SessionPane::ready(vec![WindowRow::meta_only("s1".into(), meta)], 10, true),
            ..Default::default()
        };
        let mut column = SessionColumn::new(sources.clone(), vec![state], PathBuf::from("."));

        // A recorded failure is retried by an explicit refresh ...
        column
            .pump
            .body_failed
            .insert("0\0s1".into(), "boom".into());
        column.pump.refresh_selected(
            &sources,
            &[true],
            &mut column.states,
            Viewport {
                first: 0,
                visible: 10,
            },
        );
        assert!(column.pump.body_failed.is_empty());

        // ... and events from a body job spawned before a source was dropped
        // must not record failures against the newer selection.
        column.pump.drop_unselected(&[false], &mut column.states); // gen 0 -> 1
        column.pump.body_inflight.insert("0\0s1".into()); // new selection re-spawned
        let stale = JobEvent {
            index: 0,
            gen: 0,
            kind: JobKind::Body {
                session_id: "s1".into(),
            },
            result: Err("stale job failed".into()),
            exhausted: true,
        };
        assert!(!column.pump.apply(stale, &mut column.states));
        assert!(!column.pump.body_failed.contains_key("0\0s1"));
        // The fresh in-flight marker for the new job is untouched.
        assert!(column.pump.body_inflight.contains("0\0s1"));
    }

    #[test]
    fn stale_body_result_is_dropped_not_applied() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sources = vec![source.clone()];
        let meta = SessionMeta {
            source: source.clone(),
            session_id: "s1".into(),
            modified: UNIX_EPOCH,
            title: "s1".into(),
            search_title: "s1".into(),
        };
        let state = SourceLoadState {
            pane: SessionPane::ready(vec![WindowRow::meta_only("s1".into(), meta)], 10, true),
            ..Default::default()
        };
        let mut column = SessionColumn::new(sources.clone(), vec![state], PathBuf::from("."));

        // s1 is wanted, then the viewport moves on so sync_bodies clears it.
        column.pump.body_wanted.insert((0, "s1".into()));
        let keep = HashSet::new();
        column.pump.sync_bodies(&sources, &mut column.states, &keep);
        assert!(!column.pump.body_wanted.contains(&(0, "s1".into())));

        column.pump.body_inflight.insert("0\0s1".into());
        let arrived = JobEvent {
            index: 0,
            gen: 0,
            kind: JobKind::Body {
                session_id: "s1".into(),
            },
            result: Ok(vec![WorkspaceSession {
                source,
                session_id: "s1".into(),
                modified: UNIX_EPOCH,
                title: "s1".into(),
                search_title: "s1".into(),
                records: vec![test_record("s1", 1, "payload", "2026-07-17T10:00:00Z")],
                body_loaded: true,
            }]),
            exhausted: true,
        };

        assert!(!column.pump.apply(arrived, &mut column.states));
        assert!(column.pump.body_inflight.is_empty());
        assert!(column.pump.body_failed.is_empty());
        assert!(column.states[0].body("s1").is_none());
    }

    #[test]
    fn body_spawn_stops_at_fetch_cap() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sources = vec![source.clone()];
        let rows: Vec<_> = ["s1", "s2", "s3"]
            .into_iter()
            .map(|id| {
                WindowRow::meta_only(
                    id.to_string(),
                    SessionMeta {
                        source: source.clone(),
                        session_id: id.into(),
                        modified: UNIX_EPOCH,
                        title: id.into(),
                        search_title: id.into(),
                    },
                )
            })
            .collect();
        let state = SourceLoadState {
            pane: SessionPane::ready(rows, 10, true),
            ..Default::default()
        };
        let mut column = SessionColumn::new(sources.clone(), vec![state], PathBuf::from("."));

        // Saturate the cap with unrelated keys so the missing keep set cannot
        // spawn more parse threads.
        column.pump.body_inflight.insert("0\0busy1".into());
        column.pump.body_inflight.insert("0\0busy2".into());
        let keep: HashSet<(usize, String)> =
            [(0, "s1".into()), (0, "s2".into()), (0, "s3".into())].into();
        column.pump.sync_bodies(&sources, &mut column.states, &keep);

        assert_eq!(column.pump.body_inflight.len(), BODY_FETCH_CAP);
        assert!(!column.pump.body_inflight.contains("0\0s1"));
        assert!(!column.pump.body_inflight.contains("0\0s2"));
        assert!(!column.pump.body_inflight.contains("0\0s3"));
    }

    fn test_record(session: &str, index: usize, title: &str, ended: &str) -> WorkRecord {
        WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, session, index),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: Some(session.to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::from_components(None, Some(ended.to_string()), None),
            status: None,
            title: title.to_string(),
            parts: vec![],
        }
    }
}
