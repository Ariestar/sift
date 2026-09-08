use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use anyhow::{Context, Result};
use regex::Regex;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

use crate::commands::memory::filter::Filter;
use crate::commands::memory::search::{SearchIndex, SearchMatch};
use crate::commands::memory::workset::{self, QuerySourceResult};
use crate::tui::workspace::{WorkspaceSession, WorkspaceSource};

use super::load::{query_source, sessions_from_records};

#[derive(Default)]
pub(super) struct WorkspaceSearchOutput {
    pub(super) sessions: Vec<WorkspaceSession>,
    pub(super) matches: Vec<WorkspaceSearchMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceSearchMatch {
    pub(super) session_index: usize,
    pub(super) dialogue_index: usize,
    pub(super) at: WorkAt,
    pub(super) matched_line: usize,
}

struct SearchJobEvent {
    generation: u64,
    result: std::result::Result<(Vec<WorkspaceSession>, SearchIndex), String>,
}

struct SearchPump {
    tx: Sender<SearchJobEvent>,
    rx: Receiver<SearchJobEvent>,
    generation: u64,
    inflight: bool,
}

impl SearchPump {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            generation: 0,
            inflight: false,
        }
    }

    fn start(&mut self, sources: &[WorkspaceSource], source_scope: &[bool], cwd: &Path) {
        assert_eq!(sources.len(), source_scope.len());
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.inflight = true;
        let sources = sources.to_vec();
        let source_scope = source_scope.to_vec();
        let cwd = cwd.to_path_buf();
        let tx = self.tx.clone();
        let spawned = thread::Builder::new()
            .name("sivtr-search".into())
            .spawn(move || {
                let result = load_search_corpus(&sources, &source_scope, &cwd)
                    .map_err(|error| format!("{error:#}"));
                let _ = tx.send(SearchJobEvent { generation, result });
            });
        if let Err(error) = spawned {
            let _ = self.tx.send(SearchJobEvent {
                generation,
                result: Err(format!("failed to spawn search loader thread: {error}")),
            });
        }
    }

    fn poll(
        &mut self,
    ) -> Option<std::result::Result<(Vec<WorkspaceSession>, SearchIndex), String>> {
        loop {
            match self.rx.try_recv() {
                Ok(event) if event.generation == self.generation => {
                    self.inflight = false;
                    return Some(event.result);
                }
                Ok(_) => continue,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }

    fn cancel(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.inflight = false;
    }

    fn is_fetching(&self) -> bool {
        self.inflight
    }
}

pub(super) struct WorkspaceSearch {
    input_open: bool,
    query: String,
    output: WorkspaceSearchOutput,
    corpus: Vec<WorkspaceSession>,
    index: Option<SearchIndex>,
    regex: Option<Regex>,
    error: Option<String>,
    cursor: usize,
    dirty: bool,
    apply_pending: bool,
    pump: SearchPump,
}

impl WorkspaceSearch {
    pub(super) fn new() -> Self {
        Self {
            input_open: false,
            query: String::new(),
            output: WorkspaceSearchOutput::default(),
            corpus: Vec::new(),
            index: None,
            regex: None,
            error: None,
            cursor: 0,
            dirty: true,
            apply_pending: false,
            pump: SearchPump::new(),
        }
    }

    pub(super) fn open(&mut self, sources: &[WorkspaceSource], source_scope: &[bool], cwd: &Path) {
        self.input_open = true;
        self.query.clear();
        self.start_load(sources, source_scope, cwd);
    }

    pub(super) fn restart(
        &mut self,
        sources: &[WorkspaceSource],
        source_scope: &[bool],
        cwd: &Path,
    ) {
        self.start_load(sources, source_scope, cwd);
    }

    fn start_load(&mut self, sources: &[WorkspaceSource], source_scope: &[bool], cwd: &Path) {
        self.clear_results();
        self.pump.start(sources, source_scope, cwd);
        self.dirty = true;
    }

    pub(super) fn accept(&mut self) {
        self.input_open = false;
    }

    pub(super) fn clear(&mut self) {
        self.input_open = false;
        self.query.clear();
        self.clear_results();
        self.dirty = true;
    }

    fn clear_results(&mut self) {
        self.pump.cancel();
        self.output = WorkspaceSearchOutput::default();
        self.corpus.clear();
        self.index = None;
        self.regex = None;
        self.error = None;
        self.cursor = 0;
        self.apply_pending = false;
    }

    pub(super) fn edit(&mut self, edit: impl FnOnce(&mut String)) {
        edit(&mut self.query);
        if self.index.is_some() {
            self.error = None;
        }
        self.cursor = 0;
        self.apply_pending = true;
        self.dirty = true;
    }

    pub(super) fn next(&mut self) -> bool {
        if self.output.matches.is_empty() {
            return false;
        }
        self.cursor = (self.cursor + 1) % self.output.matches.len();
        self.apply_pending = true;
        true
    }

    pub(super) fn previous(&mut self) -> bool {
        if self.output.matches.is_empty() {
            return false;
        }
        self.cursor = self
            .cursor
            .checked_sub(1)
            .unwrap_or_else(|| self.output.matches.len().saturating_sub(1));
        self.apply_pending = true;
        true
    }

    pub(super) fn poll(&mut self, cwd: &Path) -> bool {
        let mut changed = false;
        if let Some(result) = self.pump.poll() {
            match result {
                Ok((corpus, index)) => {
                    self.corpus = corpus;
                    self.index = Some(index);
                    self.error = None;
                    self.dirty = true;
                }
                Err(error) => {
                    self.output = WorkspaceSearchOutput::default();
                    self.corpus.clear();
                    self.index = None;
                    self.regex = None;
                    self.error = Some(error);
                    self.apply_pending = false;
                    self.dirty = false;
                }
            }
            changed = true;
        }
        if self.dirty {
            if self.query.trim().is_empty() {
                self.output = WorkspaceSearchOutput::default();
                self.regex = None;
                self.error = None;
                self.apply_pending = false;
                self.dirty = false;
                changed = true;
            } else if let Some(result) = self
                .index
                .as_ref()
                .map(|index| index.search(&self.query, cwd))
            {
                match result {
                    Ok(result) => {
                        self.output = project_search_output(&self.corpus, &result.matches);
                        self.regex = Some(result.regex);
                        self.error = None;
                        self.apply_pending = !self.output.matches.is_empty();
                    }
                    Err(error) => {
                        self.output = WorkspaceSearchOutput::default();
                        self.regex = None;
                        self.error = Some(format!("{error:#}"));
                        self.apply_pending = false;
                    }
                }
                self.dirty = false;
                changed = true;
            } else if self.error.is_some() {
                self.dirty = false;
            }
        }
        if self.cursor >= self.output.matches.len() {
            self.cursor = 0;
        }
        changed
    }

    pub(super) fn query_active(&self) -> bool {
        !self.query.trim().is_empty()
    }

    pub(super) fn input_open(&self) -> bool {
        self.input_open
    }

    pub(super) fn query(&self) -> &str {
        &self.query
    }

    pub(super) fn output(&self) -> &WorkspaceSearchOutput {
        &self.output
    }

    pub(super) fn records_for<'a>(
        &'a self,
        session: &WorkspaceSession,
    ) -> Option<&'a [WorkRecord]> {
        self.corpus
            .iter()
            .find(|candidate| {
                candidate.source == session.source && candidate.session_id == session.session_id
            })
            .map(|candidate| candidate.records.as_slice())
    }

    pub(super) fn regex(&self) -> Option<&Regex> {
        self.regex.as_ref()
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn is_fetching(&self) -> bool {
        self.pump.is_fetching()
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn pending_match(&self) -> Option<WorkspaceSearchMatch> {
        if !self.query_active() || !self.apply_pending {
            return None;
        }
        self.output.matches.get(self.cursor).cloned()
    }

    pub(super) fn finish_pending(&mut self) {
        self.apply_pending = false;
    }
}

fn load_search_corpus(
    sources: &[WorkspaceSource],
    source_scope: &[bool],
    cwd: &Path,
) -> Result<(Vec<WorkspaceSession>, SearchIndex)> {
    assert_eq!(sources.len(), source_scope.len());
    let selected: Vec<WorkspaceSource> = sources
        .iter()
        .zip(source_scope)
        .filter(|(_, selected)| **selected)
        .map(|(source, _)| source.clone())
        .collect();
    let query_sources: Vec<_> = selected
        .iter()
        .map(|source| query_source(source, source.selector()))
        .collect();
    let results = workset::query_sources(&query_sources, Filter::none(), Some(cwd))?;
    let mut sessions = Vec::new();
    for (source, result) in selected.iter().zip(results) {
        let mut set = match result {
            QuerySourceResult::Ok(set) => set,
            QuerySourceResult::Err(message) => {
                anyhow::bail!("search source `{}` failed: {message}", source.label())
            }
        };
        set.materialize_parts()
            .with_context(|| format!("load search source `{}`", source.label()))?;
        sessions.extend(sessions_from_records(source, set.into_records()));
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
    let records = sessions
        .iter()
        .flat_map(|session| session.records.iter().cloned())
        .collect();
    Ok((sessions, SearchIndex::new(records)))
}

fn project_search_output(
    corpus: &[WorkspaceSession],
    matches: &[SearchMatch],
) -> WorkspaceSearchOutput {
    let locations = corpus
        .iter()
        .enumerate()
        .flat_map(|(session_index, session)| {
            session
                .records
                .iter()
                .enumerate()
                .map(move |(dialogue_index, record)| {
                    (record.work_ref.whole(), (session_index, dialogue_index))
                })
        })
        .collect::<HashMap<_, _>>();
    let mut sessions = Vec::new();
    let mut session_indices = HashMap::new();
    let mut projected = Vec::new();
    for matched in matches {
        let Some(&(corpus_session_index, dialogue_index)) = locations.get(&matched.anchor.whole())
        else {
            continue;
        };
        let output_session_index =
            *session_indices
                .entry(corpus_session_index)
                .or_insert_with(|| {
                    let output_index = sessions.len();
                    sessions.push(session_meta_shell(&corpus[corpus_session_index]));
                    output_index
                });
        projected.push(WorkspaceSearchMatch {
            session_index: output_session_index,
            dialogue_index,
            at: matched.at,
            matched_line: matched.matched_line,
        });
    }
    WorkspaceSearchOutput {
        sessions,
        matches: projected,
    }
}

fn session_meta_shell(session: &WorkspaceSession) -> WorkspaceSession {
    let mut shell = session.clone();
    shell.records.clear();
    shell
}

pub(super) fn workspace_search_target_ref(
    search: &WorkspaceSearch,
    sessions: &[WorkspaceSession],
    matched: &WorkspaceSearchMatch,
) -> Option<WorkRef> {
    let session = sessions.get(matched.session_index)?;
    search
        .records_for(session)?
        .get(matched.dialogue_index)
        .map(|record| record.work_ref.with_at(matched.at))
}

pub(super) fn active_workspace_content_at(
    search: &WorkspaceSearch,
    session_idx: usize,
    selected_dialogues: &[bool],
    dialogue_idx: usize,
) -> Option<WorkAt> {
    if !search.query_active() || selected_dialogues.iter().any(|selected| *selected) {
        return None;
    }
    let matched = search.output.matches.get(search.cursor())?;
    (matched.session_index == session_idx && matched.dialogue_index == dialogue_idx)
        .then_some(matched.at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkTime, RECORD_SCHEMA_VERSION,
    };
    use std::time::SystemTime;

    fn session() -> WorkspaceSession {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        WorkspaceSession {
            source,
            session_id: "session".into(),
            modified: SystemTime::UNIX_EPOCH,
            title: "dialogue".into(),
            search_title: "dialogue".into(),
            records: vec![record()],
            body_loaded: true,
        }
    }

    fn record() -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: WorkSessionRef {
                id: "session".into(),
                canonical_id: Some("session".into()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "dialogue".into(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::User {
                    content: "first\nneedle".into(),
                },
            }],
        }
    }

    #[test]
    fn projection_keeps_search_bodies_outside_the_list_shell() {
        let corpus = vec![session()];
        let matched = SearchMatch {
            anchor: corpus[0].records[0].work_ref.whole(),
            at: WorkAt::Part(1),
            matched_line: 2,
        };
        let output = project_search_output(&corpus, &[matched]);

        assert_eq!(output.sessions.len(), 1);
        assert!(output.sessions[0].records.is_empty());
        assert_eq!(output.matches[0].dialogue_index, 0);

        let mut search = WorkspaceSearch::new();
        search.corpus = corpus;
        let target = workspace_search_target_ref(&search, &output.sessions, &output.matches[0])
            .expect("search target");
        assert_eq!(target.to_string(), "codex/session/1/p1");
    }
}
