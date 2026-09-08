//! Hierarchical refresh and the source-preset selections.
//!
//! Active rows = marked rows if any, otherwise the cursor row (see
//! [`crate::tui::workspace::active_rows`]). `R` reloads the next hierarchy
//! level under those rows.

use crate::tui::content::block::BlockText;
use crate::tui::workspace::{
    Rows, WorkspaceDialogue, WorkspaceFocus, WorkspaceSession, WorkspaceSource,
};
use sivtr_core::workset::{WorkSelectionAction, WorkSelectionTarget};

use super::load::SessionColumn;
use crate::pane::Viewport;

/// Refresh the next level under active rows of the focused pane.
///
/// | Focus     | Active rows        | Reloads                         |
/// |-----------|--------------------|---------------------------------|
/// | Source    | sources            | those sources (sessions update) |
/// | Sessions  | sessions           | parent sources (dialogues/records update) |
/// | Dialogues | sessions (parents) | parent sources (dialogue list update) |
/// | Content   | —                  | no-op (content is in-memory)    |
///
/// Dialogue content is derived from session records, so session/dialogue refresh
/// re-queries parent sources. There is no separate dialogue transport.
pub(super) fn refresh_next_level(
    focus: WorkspaceFocus,
    rows: &Rows,
    sessions: &[WorkspaceSession],
    sessions_pane: &mut SessionColumn,
    all_sessions: &mut Vec<WorkspaceSession>,
    viewport: Viewport,
) {
    let sources = sessions_pane.sources();
    let sources_to_reload = match focus {
        WorkspaceFocus::Source => rows.source.active_scope_mask(),
        WorkspaceFocus::Sessions | WorkspaceFocus::Dialogues => {
            parent_source_mask(sources, sessions, &rows.sessions.active_scope_rows())
        }
        WorkspaceFocus::Content => return,
    };

    if !sources_to_reload.iter().any(|selected| *selected) {
        return;
    }

    sessions_pane.refresh(&sources_to_reload, viewport);
    *all_sessions = sessions_pane.collect(rows.source.scope_mask());
}

fn parent_source_mask(
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    active_sessions: &[usize],
) -> Vec<bool> {
    let mut parent = vec![false; sources.len()];
    for session in active_sessions.iter().filter_map(|row| sessions.get(*row)) {
        if let Some(source_idx) = sources.iter().position(|source| source == &session.source) {
            parent[source_idx] = true;
        }
    }
    parent
}

#[derive(Clone, Copy)]
pub(super) enum WorkspaceSourceSelection {
    Agents,
    Terminal,
}

pub(super) fn select_sources(
    sources: &[WorkspaceSource],
    source_scope: &mut [bool],
    selection: WorkspaceSourceSelection,
) {
    assert_eq!(sources.len(), source_scope.len());
    for (flag, source) in source_scope.iter_mut().zip(sources) {
        *flag = match selection {
            WorkspaceSourceSelection::Agents => source.is_agent(),
            WorkspaceSourceSelection::Terminal => source.is_terminal(),
        };
    }
}

pub(super) fn toggle_row_selection(
    focus: WorkspaceFocus,
    idx: usize,
    rows: &mut Rows,
    sources: &[WorkspaceSource],
    sessions: &[WorkspaceSession],
    session_records: &[Vec<sivtr_core::record::WorkRecord>],
    dialogues: &[WorkspaceDialogue],
) {
    let target = match focus {
        WorkspaceFocus::Source => sources.get(idx).map(|source| source.selection_target(None)),
        WorkspaceFocus::Sessions => sessions
            .get(idx)
            .map(|session| session.source.selection_target(Some(&session.session_id))),
        WorkspaceFocus::Dialogues => dialogues
            .get(idx)
            .and_then(|dialogue| dialogue.record.clone())
            .map(WorkSelectionTarget::Whole),
        WorkspaceFocus::Content => None,
    };
    if let Some(target) = target {
        rows.selection.apply_target(
            WorkSelectionAction::Toggle,
            target,
            session_records.iter().flatten().cloned(),
        );
    }
}

pub(super) fn toggle_block_selection(
    rows: &mut Rows,
    dialogues: &[WorkspaceDialogue],
    block_id: usize,
) {
    if let Some((_, record, parts)) =
        super::content::workspace_block_parts(dialogues, rows.dialogues.cursor(), block_id)
    {
        rows.selection.apply_target(
            WorkSelectionAction::Toggle,
            WorkSelectionTarget::Parts { record, parts },
            [],
        );
    }
}

#[derive(Clone, Copy)]
pub(super) enum SelectionAction {
    Toggle,
    ToggleAll,
    Range,
}

pub(super) struct SelectionContext<'a> {
    pub(super) rows: &'a mut Rows,
    pub(super) sources: &'a [WorkspaceSource],
    pub(super) sessions: &'a [WorkspaceSession],
    pub(super) session_records: &'a [Vec<sivtr_core::record::WorkRecord>],
    pub(super) dialogues: &'a [WorkspaceDialogue],
    pub(super) content_blocks: (&'a [BlockText], &'a [BlockText]),
    pub(super) content_cursor: &'a mut super::nav::ContentBlockCursor,
}

pub(super) fn apply_selection_action(
    action: SelectionAction,
    focus: WorkspaceFocus,
    context: SelectionContext<'_>,
) -> bool {
    let SelectionContext {
        rows,
        sources,
        sessions,
        session_records,
        dialogues,
        content_blocks,
        content_cursor,
    } = context;
    match action {
        SelectionAction::Toggle => {
            if focus == WorkspaceFocus::Content {
                if let Some(block) = content_cursor.get() {
                    toggle_block_selection(rows, dialogues, block);
                }
                return false;
            }
            let idx = match focus {
                WorkspaceFocus::Dialogues => rows.dialogues.cursor(),
                WorkspaceFocus::Source | WorkspaceFocus::Sessions => {
                    rows.scope_pane(focus).map_or(0, |pane| pane.cursor())
                }
                WorkspaceFocus::Content => 0,
            };
            toggle_row_selection(
                focus,
                idx,
                rows,
                sources,
                sessions,
                session_records,
                dialogues,
            );
            matches!(focus, WorkspaceFocus::Source | WorkspaceFocus::Sessions)
        }
        SelectionAction::ToggleAll => {
            match focus {
                WorkspaceFocus::Content => {
                    if let Some(record) = dialogues
                        .get(rows.dialogues.cursor())
                        .and_then(|dialogue| dialogue.record.as_ref())
                    {
                        rows.selection.apply_target(
                            WorkSelectionAction::Toggle,
                            WorkSelectionTarget::Whole(record.clone()),
                            [],
                        );
                    }
                }
                WorkspaceFocus::Dialogues => rows.selection.apply_target(
                    WorkSelectionAction::Toggle,
                    WorkSelectionTarget::Many(
                        dialogues
                            .iter()
                            .filter_map(|dialogue| dialogue.record.clone())
                            .map(WorkSelectionTarget::Whole)
                            .collect(),
                    ),
                    session_records.iter().flatten().cloned(),
                ),
                WorkspaceFocus::Sessions => rows.selection.apply_target(
                    WorkSelectionAction::Toggle,
                    WorkSelectionTarget::Many(
                        sessions
                            .iter()
                            .map(|session| {
                                session.source.selection_target(Some(&session.session_id))
                            })
                            .collect(),
                    ),
                    session_records.iter().flatten().cloned(),
                ),
                WorkspaceFocus::Source => rows.selection.apply_target(
                    WorkSelectionAction::Toggle,
                    WorkSelectionTarget::Many(
                        sources
                            .iter()
                            .map(|source| source.selection_target(None))
                            .collect(),
                    ),
                    session_records.iter().flatten().cloned(),
                ),
            }
            if matches!(focus, WorkspaceFocus::Source | WorkspaceFocus::Sessions) {
                if let Some(list) = rows.scope_pane_mut(focus) {
                    list.toggle_scope_all();
                    rows.close_ranges();
                    return true;
                }
            }
            false
        }
        SelectionAction::Range => {
            match focus {
                WorkspaceFocus::Content => {
                    let Some(cursor_block) = content_cursor.get() else {
                        return false;
                    };
                    let Some(span) = rows.range(focus, cursor_block) else {
                        return false;
                    };
                    let mut record = None;
                    let mut parts = Vec::new();
                    for id in content_blocks
                        .0
                        .iter()
                        .chain(content_blocks.1)
                        .map(|block| block.id)
                        .filter(|id| span.contains(id))
                    {
                        if let Some((_, selected, block_parts)) =
                            super::content::workspace_block_parts(
                                dialogues,
                                rows.dialogues.cursor(),
                                id,
                            )
                        {
                            record = Some(selected);
                            parts.extend(block_parts);
                        }
                    }
                    if let Some(record) = record {
                        rows.selection.apply_target(
                            WorkSelectionAction::Toggle,
                            WorkSelectionTarget::Parts { record, parts },
                            [],
                        );
                    }
                }
                WorkspaceFocus::Dialogues => {
                    if let Some(span) = rows.range(focus, rows.dialogues.cursor()) {
                        rows.selection.apply_target(
                            WorkSelectionAction::Toggle,
                            WorkSelectionTarget::Many(
                                dialogues
                                    .iter()
                                    .enumerate()
                                    .filter(|(idx, _)| span.contains(idx))
                                    .filter_map(|(_, dialogue)| dialogue.record.clone())
                                    .map(WorkSelectionTarget::Whole)
                                    .collect(),
                            ),
                            session_records.iter().flatten().cloned(),
                        );
                    }
                }
                WorkspaceFocus::Sessions => {
                    if let Some(span) = rows.range(focus, rows.sessions.cursor()) {
                        let targets: Vec<_> = sessions
                            .iter()
                            .enumerate()
                            .filter(|(idx, _)| span.contains(idx))
                            .map(|(_, session)| {
                                session.source.selection_target(Some(&session.session_id))
                            })
                            .collect();
                        rows.selection.apply_target(
                            WorkSelectionAction::Toggle,
                            WorkSelectionTarget::Many(targets),
                            session_records.iter().flatten().cloned(),
                        );
                    }
                }
                WorkspaceFocus::Source => {
                    if let Some(span) = rows.range(focus, rows.source.cursor()) {
                        let targets: Vec<_> = sources
                            .iter()
                            .enumerate()
                            .filter(|(idx, _)| span.contains(idx))
                            .map(|(_, source)| source.selection_target(None))
                            .collect();
                        rows.selection.apply_target(
                            WorkSelectionAction::Toggle,
                            WorkSelectionTarget::Many(targets),
                            session_records.iter().flatten().cloned(),
                        );
                    }
                }
            }
            false
        }
    }
}
