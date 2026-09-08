//! Browse product panes.
//!
//! A pane whose rows are a window over a growing list implements
//! [`crate::pane::Pane`]:
//! 1. `struct MyPane { engine: SlidingPane<K,M,B>, … }`
//! 2. `impl Pane for MyPane` — only map data + call SlidingPane ensure_*
//! 3. Register in picker: `my_pane.poll(); my_pane.ensure(ctx, &input);`
//!
//! Do **not** reimplement viewport growth, keep/evict, or blanking rules.

use crate::pane::{Pane, PaneInput, SlidingPane, WindowRow};
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::{
    active_rows, workspace_content_io_texts, ContentIoFocus, ContentIoFrame, ExpandedBlocks,
    WorkspaceDialogue, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};
use sivtr_core::workset::WorkSet;

// ── Dialogues ───────────────────────────────────────────────────────────

pub type DialogueKey = String;

#[derive(Clone, Debug)]
pub struct DialogueMeta {
    pub source: WorkspaceSource,
    pub work_ref: Option<WorkRef>,
    pub title: String,
}

pub type DialogueEngine = SlidingPane<DialogueKey, DialogueMeta, WorkspaceDialogue>;

/// Active session set + body readiness. Any change force-rebuilds meta.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DialogueFingerprint {
    sessions: Vec<(String, String, bool, usize)>,
}

#[derive(Default)]
pub struct DialoguePane {
    engine: DialogueEngine,
    fingerprint: DialogueFingerprint,
    /// Bumped whenever the engine's rows or bodies change. Callers use it as
    /// the cheap half of a materialization key so they can skip rebuilding
    /// the owned projection (and its body clones) across unchanged redraws.
    generation: u64,
}

impl DialoguePane {
    /// List paint: titles only (no body clone).
    pub fn titles(&self) -> impl Iterator<Item = &str> + '_ {
        self.engine.rows().iter().map(|r| r.meta.title.as_str())
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// WorkSet-derived selection for the dialogue rows currently in view.
    pub fn selection_mask(&self, selection: &WorkSet) -> Vec<bool> {
        self.engine
            .rows()
            .iter()
            .map(|row| {
                row.meta
                    .work_ref
                    .as_ref()
                    .is_some_and(|work_ref| selection.contains(work_ref))
            })
            .collect()
    }

    /// Index-stable rows for content/copy/vim.
    /// Clones **body only** for focus ∪ multi-select; other rows are title shells.
    #[cfg(test)]
    pub fn materialize(&self, selected: &[bool], focus: usize) -> Vec<WorkspaceDialogue> {
        let mut out = Vec::new();
        self.materialize_into(selected, focus, &mut out);
        out
    }

    /// Rebuild `out` from the engine rows. Callers cache `out` between calls
    /// and only invoke this when `generation()`, the dialogue selection, or the
    /// focused index changed, so scrolling content does not re-clone bodies.
    pub fn materialize_into(
        &self,
        selected: &[bool],
        focus: usize,
        out: &mut Vec<WorkspaceDialogue>,
    ) {
        out.clear();
        let rows = self.engine.rows();
        if rows.is_empty() {
            return;
        }
        assert_eq!(selected.len(), rows.len());
        let focus = focus.min(rows.len() - 1);
        for (i, row) in rows.iter().enumerate() {
            // The cursor dialogue's body is always needed: the content pane
            // shows it even when the selection marks other dialogues.
            let need_body = i == focus || selected[i];
            let item = if need_body {
                if let Some(body) = row.body.clone() {
                    body
                } else {
                    shell_from_row(row)
                }
            } else {
                shell_from_row(row)
            };
            out.push(item);
        }
    }

    #[cfg(test)]
    pub fn exhausted(&self) -> bool {
        self.engine.exhausted()
    }

    #[cfg(test)]
    pub fn dialogues(&self) -> Vec<WorkspaceDialogue> {
        // Tests want full bodies when present.
        let n = self.engine.len();
        let selected = vec![true; n];
        self.materialize(&selected, 0)
    }

    /// Bench-only: inspect engine rows without cloning the pane.
    #[cfg(feature = "perf-benches")]
    pub(crate) fn engine_rows_for_perf(
        &self,
    ) -> &[crate::pane::WindowRow<DialogueKey, DialogueMeta, WorkspaceDialogue>] {
        self.engine.rows()
    }
}

fn shell_from_row(
    row: &crate::pane::WindowRow<DialogueKey, DialogueMeta, WorkspaceDialogue>,
) -> WorkspaceDialogue {
    WorkspaceDialogue {
        source: row.meta.source.clone(),
        work_ref: row.meta.work_ref.clone(),
        record: None,
    }
}

/// Domain context for dialogue ensure (one frame).
///
/// `sessions` is the **meta** list (titles/ids/body_loaded). Turn bodies are
/// read through `records` (the browse loader or a search corpus).
#[derive(Clone, Copy)]
pub struct DialogueCtx<'a> {
    pub sessions: &'a [WorkspaceSession],
    pub session_idx: usize,
    pub session_scope: &'a [bool],
    /// Body lookup; returned slice lives as long as the storage behind the
    /// callback (`SessionColumn` / fixture table), not the `&session` arg.
    pub records: &'a dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
}

impl Pane for DialoguePane {
    type Ctx<'a> = DialogueCtx<'a>;

    fn ensure(&mut self, ctx: DialogueCtx<'_>, input: &PaneInput<'_>) {
        let next = fingerprint(
            ctx.sessions,
            ctx.session_idx,
            ctx.session_scope,
            ctx.records,
        );
        let force = if next != self.fingerprint {
            self.engine.clear();
            self.fingerprint = next;
            true
        } else {
            input.force
        };

        let before = self.engine.len();
        let grown = self
            .engine
            .ensure_meta_sync(input.viewport, force, |budget| {
                meta_prefix(
                    ctx.sessions,
                    ctx.session_idx,
                    ctx.session_scope,
                    ctx.records,
                    budget,
                )
            });

        let keep = self
            .engine
            .keep_for_focus(input.focus, input.selected, input.neighbor_radius);
        // Bodies hydrate asynchronously; a fill changes what materialize
        // projects, so bump the generation even when the row count is stable.
        let mut bodies_filled = false;
        self.engine.ensure_bodies_sync(keep, |key| {
            let body = body_for_key(
                ctx.sessions,
                ctx.session_idx,
                ctx.session_scope,
                ctx.records,
                key,
            );
            if body.is_some() {
                bodies_filled = true;
            }
            body
        });
        let changed = grown || self.engine.len() != before || bodies_filled;
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn len(&self) -> usize {
        self.engine.len()
    }

    fn is_fetching(&self) -> bool {
        self.engine.is_fetching()
    }
}

fn dialogue_key(source: &WorkspaceSource, session_id: &str, record: &WorkRecord) -> DialogueKey {
    format!(
        "{}/{}/{}",
        source.selector(),
        session_id,
        record.work_ref.path.index()
    )
}

fn dialogue_from_record(session: &WorkspaceSession, record: &WorkRecord) -> WorkspaceDialogue {
    WorkspaceDialogue {
        source: session.source.clone(),
        work_ref: Some(record.work_ref.clone()),
        record: Some(record.clone()),
    }
}

fn fingerprint<'a>(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    session_scope: &[bool],
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
) -> DialogueFingerprint {
    DialogueFingerprint {
        sessions: active_rows(session_scope, session_idx, sessions.len())
            .into_iter()
            .filter_map(|i| {
                let s = sessions.get(i)?;
                let n = records(s).map(|r| r.len()).unwrap_or(0);
                Some((s.source.selector(), s.session_id.clone(), s.body_loaded, n))
            })
            .collect(),
    }
}

fn meta_prefix<'a>(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    session_scope: &[bool],
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
    budget: usize,
) -> (
    Vec<WindowRow<DialogueKey, DialogueMeta, WorkspaceDialogue>>,
    bool,
) {
    let indices = active_rows(session_scope, session_idx, sessions.len());
    if indices.is_empty() {
        return (Vec::new(), true);
    }
    let mut all_ready = true;
    let mut total = 0usize;
    let mut bodies: Vec<Option<&'a [WorkRecord]>> = Vec::with_capacity(indices.len());
    for &i in &indices {
        let Some(session) = sessions.get(i) else {
            all_ready = false;
            bodies.push(None);
            continue;
        };
        if session.body_loaded {
            match records(session) {
                Some(recs) => {
                    total += recs.len();
                    bodies.push(Some(recs));
                }
                None => {
                    // Flagged loaded but body not yet in pane (async gap).
                    all_ready = false;
                    bodies.push(None);
                }
            }
        } else {
            all_ready = false;
            bodies.push(None);
        }
    }

    let end = budget.min(total);
    let mut rows = Vec::with_capacity(end);
    let mut taken = 0usize;
    'outer: for (pos, &i) in indices.iter().enumerate() {
        let Some(session) = sessions.get(i) else {
            continue;
        };
        let Some(recs) = bodies[pos] else {
            continue;
        };
        for record in recs {
            if taken >= end {
                break 'outer;
            }
            rows.push(WindowRow::meta_only(
                dialogue_key(&session.source, &session.session_id, record),
                DialogueMeta {
                    source: session.source.clone(),
                    work_ref: Some(record.work_ref.clone()),
                    title: record.title.clone(),
                },
            ));
            taken += 1;
        }
    }
    (rows, all_ready && end >= total)
}

fn body_for_key<'a>(
    sessions: &[WorkspaceSession],
    session_idx: usize,
    session_scope: &[bool],
    records: &dyn Fn(&WorkspaceSession) -> Option<&'a [WorkRecord]>,
    key: &DialogueKey,
) -> Option<WorkspaceDialogue> {
    for i in active_rows(session_scope, session_idx, sessions.len()) {
        let Some(session) = sessions.get(i) else {
            continue;
        };
        if !session.body_loaded {
            continue;
        }
        let Some(recs) = records(session) else {
            continue;
        };
        for record in recs {
            if dialogue_key(&session.source, &session.session_id, record) == *key {
                return Some(dialogue_from_record(session, record));
            }
        }
    }
    None
}

// ── Content ─────────────────────────────────────────────────────────────

/// Domain context for dual IO content line-count catalogs.
pub struct ContentCtx<'a> {
    pub dialogues: &'a [WorkspaceDialogue],
    pub highlighted_idx: usize,
    pub mode: ContentViewMode,
    pub target: Option<WorkAt>,
    pub area: ratatui::layout::Rect,
    pub io_focus: ContentIoFocus,
    pub expanded: &'a ExpandedBlocks,
}

/// Tracks layout line counts for Input / Output halves separately.
///
/// Not a [`crate::pane::Pane`]: its rows are the shown dialogue's rendered
/// lines, not a window over a growing list, so there is nothing to grow,
/// keep, or hydrate — `frame` hands the caller the rendered layout.
#[derive(Default)]
pub struct ContentPane {
    input_lines: usize,
    output_lines: usize,
}

impl ContentPane {
    pub fn line_count(&self, half: ContentIoFocus) -> usize {
        match half {
            ContentIoFocus::Input => self.input_lines.max(1),
            ContentIoFocus::Output => self.output_lines.max(1),
        }
    }

    /// Build the frame for this context, resizing the block selection mask
    /// of the shown dialogue's block ids. Rebuilds the cached layouts; call
    /// it only when the content actually changed.
    pub fn frame(&mut self, ctx: ContentCtx<'_>) -> ContentIoFrame {
        let texts = workspace_content_io_texts(
            ctx.dialogues,
            ctx.highlighted_idx,
            ctx.mode,
            ctx.target,
            ctx.expanded,
        );
        let frame = ContentIoFrame::build(ctx.area, texts, ctx.mode, ctx.io_focus);
        self.input_lines = frame.line_count(ContentIoFocus::Input);
        self.output_lines = frame.line_count(ContentIoFocus::Output);
        frame
    }

    /// WorkSet-derived block highlights for one dialogue. A Whole selection
    /// covers every block; a run highlights only when every part it owns is
    /// selected, while its children remain independently derived.
    pub fn block_selection_mask(
        dialogues: &[WorkspaceDialogue],
        dialogue_idx: usize,
        selection: &WorkSet,
    ) -> Vec<bool> {
        let Some(record) = dialogues
            .get(dialogue_idx)
            .and_then(|dialogue| dialogue.record.as_ref())
        else {
            return Vec::new();
        };
        let (input, output) = crate::tui::content::block::dialogue_blocks(record);
        let mut marked =
            vec![false; crate::tui::content::block::dialogue_block_count(&input, &output)];
        for block in input.iter().chain(&output) {
            mark_selected_blocks(block, record, selection, &mut marked);
        }
        marked
    }
}

fn mark_selected_blocks(
    block: &crate::tui::content::block::Block,
    record: &WorkRecord,
    selection: &WorkSet,
    marked: &mut [bool],
) {
    if let Some(selected) = marked.get_mut(block.id) {
        *selected = block
            .parts
            .iter()
            .all(|&idx| selection.contains(&record.work_ref.with_part(record.parts[idx].seq)));
    }
    for child in &block.children {
        mark_selected_blocks(child, record, selection, marked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::Viewport;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecord, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
    };
    use std::time::UNIX_EPOCH;

    fn test_record(session: &str, index: usize, title: &str) -> WorkRecord {
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
            time: WorkTime::from_components(None, Some("2026-07-17T10:00:00Z".into()), None),
            status: None,
            title: title.to_string(),
            parts: vec![],
        }
    }

    fn session_with_n(n: usize, body_loaded: bool) -> WorkspaceSession {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let records: Vec<_> = (0..n)
            .map(|i| test_record("s", i + 1, &format!("t{i}")))
            .collect();
        WorkspaceSession {
            source,
            session_id: "s".into(),
            modified: UNIX_EPOCH,
            title: "s".into(),
            search_title: "s".into(),
            records: if body_loaded { records } else { Vec::new() },
            body_loaded,
        }
    }

    fn tick(
        pane: &mut DialoguePane,
        sessions: &[WorkspaceSession],
        viewport: Viewport,
        focus: usize,
        selected: &[bool],
    ) {
        // Fixture table owns bodies; lookup by key (not reborrow of arg).
        let records = |s: &WorkspaceSession| {
            sessions
                .iter()
                .find(|x| x.session_id == s.session_id && x.source == s.source)
                .filter(|x| x.body_loaded)
                .map(|x| x.records.as_slice())
        };
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx: 0,
                session_scope: &[true],
                records: &records,
            },
            &PaneInput::new(viewport, focus).with_selected(selected),
        );
    }

    #[test]
    fn dialogue_waits_for_session_body_then_fills() {
        let pending = vec![session_with_n(0, false)];
        let mut pane = DialoguePane::default();
        tick(
            &mut pane,
            &pending,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert_eq!(pane.len(), 0);
        assert!(!pane.exhausted());

        let ready = vec![session_with_n(30, true)];
        tick(
            &mut pane,
            &ready,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert!(pane.len() >= 20, "len={}", pane.len());
    }

    #[test]
    fn dialogue_meta_fetches_full_catalog_at_ceiling() {
        let sessions = vec![session_with_n(100, true)];
        let mut pane = DialoguePane::default();
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert_eq!(pane.len(), 100);
        assert!(pane.exhausted());
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 40,
                visible: 10,
            },
            45,
            &[],
        );
        assert_eq!(pane.len(), 100);
    }

    #[test]
    fn dialogue_bodies_only_for_keep_set() {
        let sessions = vec![session_with_n(40, true)];
        let mut pane = DialoguePane::default();
        let empty = vec![false; 40];
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 20,
            },
            5,
            &empty,
        );
        let sel = vec![false; pane.len()];
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 20,
            },
            5,
            &sel,
        );
        let loaded: Vec<_> = pane
            .dialogues()
            .iter()
            .enumerate()
            .filter(|(_, d)| d.record.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(loaded, vec![4, 5, 6]);
    }

    #[test]
    fn dialogue_context_change_clears_pane() {
        let sessions = vec![session_with_n(30, true)];
        let mut pane = DialoguePane::default();
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 10,
            },
            0,
            &[],
        );
        assert!(pane.len() > 0);
        let empty: &[WorkspaceSession] = &[];
        let records = |s: &WorkspaceSession| {
            empty
                .iter()
                .find(|x| x.session_id == s.session_id)
                .map(|x| x.records.as_slice())
        };
        pane.ensure(
            DialogueCtx {
                sessions: empty,
                session_idx: 0,
                session_scope: &[],
                records: &records,
            },
            &PaneInput::new(
                Viewport {
                    first: 0,
                    visible: 10,
                },
                0,
            ),
        );
        assert_eq!(pane.len(), 0);
    }

    #[test]
    fn materialize_clones_body_only_for_focus() {
        let sessions = vec![session_with_n(40, true)];
        let mut pane = DialoguePane::default();
        let empty = vec![false; 40];
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 20,
            },
            5,
            &empty,
        );
        let sel = vec![false; pane.len()];
        tick(
            &mut pane,
            &sessions,
            Viewport {
                first: 0,
                visible: 20,
            },
            5,
            &sel,
        );
        assert_eq!(pane.titles().count(), pane.len());
        let rows = pane.materialize(&sel, 5);
        let with_body = rows.iter().filter(|d| d.record.is_some()).count();
        assert_eq!(with_body, 1);
        assert!(rows[5].record.is_some());
    }

    #[test]
    fn generation_tracks_engine_changes_and_materialize_is_stable() {
        let sessions = vec![session_with_n(30, true)];
        let mut pane = DialoguePane::default();
        let records = |s: &WorkspaceSession| {
            sessions
                .iter()
                .find(|x| x.session_id == s.session_id && x.source == s.source)
                .filter(|x| x.body_loaded)
                .map(|x| x.records.as_slice())
        };
        let vp = Viewport {
            first: 0,
            visible: 20,
        };
        let ensure = |pane: &mut DialoguePane| {
            pane.ensure(
                DialogueCtx {
                    sessions: &sessions,
                    session_idx: 0,
                    session_scope: &[false],
                    records: &records,
                },
                &PaneInput::new(vp, 0).with_selected(&[false]),
            )
        };

        let g0 = pane.generation();
        ensure(&mut pane);
        let g1 = pane.generation();
        assert!(g1 > g0, "row build must bump the generation");

        ensure(&mut pane);
        assert_eq!(
            pane.generation(),
            g1,
            "a no-op ensure must not bump the generation"
        );

        // Same inputs → identical projection, focused body included.
        let sel = vec![false; pane.len()];
        let mut out = Vec::new();
        pane.materialize_into(&sel, 0, &mut out);
        let snapshot: Vec<String> = out
            .iter()
            .map(|d| {
                d.work_ref
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_default()
            })
            .collect();
        assert!(
            out.first().is_some_and(|d| d.record.is_some()),
            "focused body present"
        );
        pane.materialize_into(&sel, 0, &mut out);
        let again: Vec<String> = out
            .iter()
            .map(|d| {
                d.work_ref
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(snapshot, again);
    }
}
