//! Fixtures and opaque hot-path kernels for criterion / dhat (`perf-benches`).
//!
//! Benches must not name `pub(crate)` TUI types at the crate boundary.

use crate::pane::{Pane, PaneInput, Viewport};
use crate::tui::workspace::{WorkspaceDialogue, WorkspaceSession, WorkspaceSource};
use sivtr_core::agents::AgentProvider;
use sivtr_core::record::{
    WorkChannel, WorkPart, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource,
    WorkTime, RECORD_SCHEMA_VERSION,
};
use std::cell::RefCell;
use std::time::UNIX_EPOCH;

use super::panes::{DialogueCtx, DialoguePane};

fn fat_record(session: &str, index: usize, title: &str) -> WorkRecord {
    let blob = "x".repeat(4096);
    WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
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
        time: WorkTime::from_components(None, Some("2026-07-17T10:00:00Z".into()), None),
        status: None,
        title: title.to_string(),
        parts: vec![WorkPart {
            seq: 0,
            occurred_at: None,
            data: sivtr_core::record::WorkPartData::Assistant { content: blob },
        }],
    }
}

fn session_with_n(n: usize) -> WorkspaceSession {
    let source = WorkspaceSource::agent(AgentProvider::Codex);
    let records: Vec<_> = (0..n)
        .map(|i| fat_record("s", i + 1, &format!("turn-{i}")))
        .collect();
    WorkspaceSession {
        source,
        session_id: "s".into(),
        modified: UNIX_EPOCH,
        title: "s".into(),
        search_title: "s".into(),
        records,
        body_loaded: true,
    }
}

fn primed_dialogue_pane(n: usize) -> DialoguePane {
    let sessions = vec![session_with_n(n)];
    let mut pane = DialoguePane::default();
    let session_scope = [true];
    let selected = vec![true; n];
    let vp = Viewport {
        first: 0,
        visible: n.max(40),
    };
    let records = |s: &WorkspaceSession| {
        sessions
            .iter()
            .find(|x| x.session_id == s.session_id && x.source == s.source)
            .filter(|x| x.body_loaded)
            .map(|x| x.records.as_slice())
    };
    pane.ensure(
        DialogueCtx {
            sessions: &sessions,
            session_idx: 0,
            session_scope: &session_scope,
            records: &records,
        },
        &PaneInput::new(vp, 0)
            .with_selected(&selected)
            .with_neighbors(n),
    );
    pane
}

/// Opaque prepared pane for repeated hot-path calls (setup outside the timer).
pub struct HotPane {
    inner: DialoguePane,
    n: usize,
    materialized: RefCell<Vec<WorkspaceDialogue>>,
}

impl HotPane {
    pub fn prepare(n: usize) -> Self {
        Self {
            inner: primed_dialogue_pane(n),
            n,
            materialized: RefCell::new(Vec::new()),
        }
    }

    pub fn n(&self) -> usize {
        self.n
    }

    /// Old path: clone every hydrated body.
    pub fn naive_full_clone(&self) -> usize {
        let mut bodies = 0usize;
        for row in self.inner.engine_rows_for_perf() {
            if let Some(body) = row.body.clone() {
                bodies = bodies.saturating_add(1);
                std::hint::black_box(body);
            }
        }
        bodies
    }

    /// New path: titles borrow + materialize focus only.
    pub fn sparse_focus_materialize(&self) -> (usize, usize) {
        let titles: Vec<&str> = self.inner.titles().collect();
        let selected = vec![false; self.inner.len()];
        let focus = self.n / 2;
        let mut buf = self.materialized.borrow_mut();
        self.inner.materialize_into(&selected, focus, &mut buf);
        let bodies = buf.iter().filter(|d| d.record.is_some()).count();
        std::hint::black_box(titles.len());
        (titles.len(), bodies)
    }

    pub fn titles_count(&self) -> usize {
        self.inner.titles().count()
    }
}

/// Grow meta twice (includes setup; measures ensure path).
pub fn run_ensure_growth() -> (usize, usize) {
    let sessions = vec![session_with_n(500)];
    let session_scope = [true];
    let mut pane = DialoguePane::default();
    let empty = [];
    let records = |s: &WorkspaceSession| {
        sessions
            .iter()
            .find(|x| x.session_id == s.session_id && x.source == s.source)
            .filter(|x| x.body_loaded)
            .map(|x| x.records.as_slice())
    };
    pane.ensure(
        DialogueCtx {
            sessions: &sessions,
            session_idx: 0,
            session_scope: &session_scope,
            records: &records,
        },
        &PaneInput::new(
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
        )
        .with_selected(&empty),
    );
    let len1 = pane.len();
    pane.ensure(
        DialogueCtx {
            sessions: &sessions,
            session_idx: 0,
            session_scope: &session_scope,
            records: &records,
        },
        &PaneInput::new(
            Viewport {
                first: 40,
                visible: 10,
            },
            45,
        )
        .with_selected(&empty),
    );
    (len1, pane.len())
}

/// Opaque store of hydrated sessions (avoids leaking private TUI types at the crate boundary).
pub struct HydratedStore {
    sessions: Vec<WorkspaceSession>,
}

impl HydratedStore {
    /// `n_sessions` × `records_each` fat turns (~4KiB text each).
    pub fn new(n_sessions: usize, records_each: usize) -> Self {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sessions = (0..n_sessions)
            .map(|i| {
                let id = format!("s{i}");
                let records: Vec<_> = (0..records_each)
                    .map(|j| fat_record(&id, j + 1, &format!("turn-{j}")))
                    .collect();
                WorkspaceSession {
                    source: source.clone(),
                    session_id: id.clone(),
                    modified: UNIX_EPOCH,
                    title: id.clone(),
                    search_title: id,
                    records,
                    body_loaded: true,
                }
            })
            .collect();
        Self { sessions }
    }

    /// New list path: meta only (what `collect()` returns).
    pub fn project_meta(&self) -> usize {
        let out: Vec<_> = self
            .sessions
            .iter()
            .map(|s| WorkspaceSession {
                source: s.source.clone(),
                session_id: s.session_id.clone(),
                modified: s.modified,
                title: s.title.clone(),
                search_title: s.search_title.clone(),
                records: Vec::new(),
                body_loaded: s.body_loaded,
            })
            .collect();
        std::hint::black_box(out.len())
    }

    /// Old list path: clone every hydrated body into the list vec.
    pub fn project_full(&self) -> usize {
        let out = self.sessions.clone();
        let bodies: usize = out.iter().map(|s| s.records.len()).sum();
        std::hint::black_box((out.len(), bodies)).0
    }

    /// Land every session body in a sliding pane (stale-apply cost).
    pub fn apply_all_bodies(&self) -> usize {
        use crate::pane::{SlidingPane, WindowRow};

        let rows: Vec<_> = self
            .sessions
            .iter()
            .map(|s| WindowRow::meta_only(s.session_id.clone(), ()))
            .collect();
        let mut pane: SlidingPane<String, (), Vec<sivtr_core::record::WorkRecord>> =
            SlidingPane::ready(rows, self.sessions.len(), true);
        let mut applied = 0usize;
        for s in &self.sessions {
            if pane.apply_body(&s.session_id, s.records.clone()) {
                applied += 1;
            }
        }
        std::hint::black_box((pane.len(), applied)).1
    }
}

/// Fat content half: `n_blocks` markdown segments, `lines_per` each.
///
/// Mirrors one large agent session in the content pane (the path that
/// double-laid-out the joined document and every block).
pub struct FatLayout {
    area: ratatui::layout::Rect,
    blocks: Vec<crate::tui::content::block::BlockText>,
}

impl FatLayout {
    pub fn new(n_blocks: usize, lines_per: usize) -> Self {
        use crate::tui::content::block::{BlockRole, BlockText};

        let blocks: Vec<BlockText> = (0..n_blocks)
            .map(|i| {
                let mut text = String::new();
                for j in 0..lines_per {
                    text.push_str(&format!(
                        "## h{i}.{j}\n\nparagraph {j} with `code` and **bold** text that wraps at eighty columns easily.\n\n"
                    ));
                }
                BlockText {
                    id: i,
                    text,
                    tight: false,
                    role: BlockRole::Assistant,
                }
            })
            .collect();
        Self {
            area: ratatui::layout::Rect::new(0, 0, 84, 40),
            blocks,
        }
    }

    pub fn layout_lines(&self) -> usize {
        crate::tui::content::view::layout_content(
            self.area,
            "",
            &self.blocks,
            crate::tui::content::view::ContentViewMode::Reading,
        )
        .lines
        .len()
    }
}
