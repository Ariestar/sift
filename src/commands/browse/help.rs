//! Help-panel + table-driven action dispatch.
//!
//! Key bindings live in `workspace_help_entries()`. This module only runs actions.

use anyhow::{Context, Result};

use crate::tui::content::block::BlockText;
use crate::tui::content::view::ContentViewMode;
use crate::tui::terminal::suspend;
use crate::tui::workspace::{
    can_open_dialogue_vim, workspace_content_text, ContentIoFocus, ContentScrolls, ExpandedBlocks,
    Rows, WorkspaceDialogue, WorkspaceFocus, WorkspaceHelpAction, WorkspaceSource,
};
use sivtr_core::record::WorkAt;

use super::content::{
    dialogue_text_vim_view, workspace_picked_content_for_copy,
    workspace_picked_content_for_cursor_block, PickedContent, WorkspaceCopyShortcut,
};
use super::nav::{invalidate_panes_below, move_workspace_cursor, ContentBlockCursor};
use super::panes::ContentPane;
use super::selection::{
    apply_selection_action, select_sources, SelectionAction, SelectionContext,
    WorkspaceSourceSelection,
};
use super::vim::open_vim_view;
use super::PICK_CANCELLED_MESSAGE;

/// Result of dispatching a help-table action.
pub(super) enum HelpDispatch {
    Continue,
    Picked(PickedContent),
    /// Caller starts the full workspace search loader.
    OpenSearch,
    /// Caller must refresh session/dialogue load (needs SessionColumn).
    Refresh,
    /// Caller opens the publication lifetime overlay.
    Publish,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_workspace_help_action(
    action: WorkspaceHelpAction,
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    sources: &[WorkspaceSource],
    // Cursor, marks, and the live `v` anchor of all three list panes.
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
    content_io_focus: &mut ContentIoFocus,
    content_mode: &mut ContentViewMode,
    expanded: &mut ExpandedBlocks,
    content_cursor: &mut ContentBlockCursor,
    content_pane: &mut ContentPane,
    content_blocks: (&[BlockText], &[BlockText]),
    show_help: &mut bool,
    content_at: Option<WorkAt>,
    line_filter: Option<&str>,
    sessions: &[crate::tui::workspace::WorkspaceSession],
    session_records: &[Vec<sivtr_core::record::WorkRecord>],
    dialogues: &[WorkspaceDialogue],
    selected_dialogues: &[usize],
    terminal: &mut crate::tui::terminal::Tui,
) -> Result<HelpDispatch> {
    let dialogue_count = rows.dialogues.len();
    match action {
        WorkspaceHelpAction::FocusSource => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Source)
        }
        WorkspaceHelpAction::FocusSessions => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Sessions)
        }
        WorkspaceHelpAction::FocusDialogues if dialogue_count > 0 => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Dialogues)
        }
        WorkspaceHelpAction::FocusContent if dialogue_count > 0 => {
            set_focus(focus, fullscreen, rows, WorkspaceFocus::Content)
        }
        WorkspaceHelpAction::MoveUp | WorkspaceHelpAction::MoveDown => move_workspace_cursor(
            action == WorkspaceHelpAction::MoveUp,
            *focus,
            rows,
            content_scrolls,
            content_cursor,
            content_blocks,
        ),
        WorkspaceHelpAction::PreviousPane => {
            if let Some(next_focus) = focus.previous(dialogue_count) {
                set_focus(focus, fullscreen, rows, next_focus);
            }
        }
        WorkspaceHelpAction::NextPane => {
            if let Some(next_focus) = focus.next(dialogue_count) {
                set_focus(focus, fullscreen, rows, next_focus);
            }
        }
        WorkspaceHelpAction::ToggleSelection => {
            if apply_selection_action(
                SelectionAction::Toggle,
                *focus,
                SelectionContext {
                    rows,
                    sources,
                    sessions,
                    session_records,
                    dialogues,
                    content_blocks: (content_blocks.0, content_blocks.1),
                    content_cursor,
                },
            ) {
                let idx = rows.scope_pane(*focus).map_or(0, |pane| pane.cursor());
                if toggle_list_row(*focus, idx, rows, content_scrolls) {
                    return Ok(HelpDispatch::Refresh);
                }
            }
        }
        WorkspaceHelpAction::SelectAgentSources | WorkspaceHelpAction::SelectTerminalSource => {
            select_sources(
                sources,
                rows.source.scope_mask_mut(),
                if action == WorkspaceHelpAction::SelectAgentSources {
                    WorkspaceSourceSelection::Agents
                } else {
                    WorkspaceSourceSelection::Terminal
                },
            );
            invalidate_panes_below(WorkspaceFocus::Source, rows, content_scrolls);
            return Ok(HelpDispatch::Refresh);
        }
        WorkspaceHelpAction::ToggleAll => {
            if apply_selection_action(
                SelectionAction::ToggleAll,
                *focus,
                SelectionContext {
                    rows,
                    sources,
                    sessions,
                    session_records,
                    dialogues,
                    content_blocks: (content_blocks.0, content_blocks.1),
                    content_cursor,
                },
            ) {
                return Ok(HelpDispatch::Refresh);
            }
        }
        WorkspaceHelpAction::RangeSelect => {
            apply_selection_action(
                SelectionAction::Range,
                *focus,
                SelectionContext {
                    rows,
                    sources,
                    sessions,
                    session_records,
                    dialogues,
                    content_blocks: (content_blocks.0, content_blocks.1),
                    content_cursor,
                },
            );
        }
        WorkspaceHelpAction::OpenVim if can_open_dialogue_vim(*focus, dialogue_count) => {
            let view = dialogue_text_vim_view(workspace_content_text(
                dialogues,
                rows.dialogues.cursor(),
                *content_mode,
                content_at,
            ));
            // A failed editor launch must not kill the picker: report it and keep running.
            suspend(terminal, || {
                if let Err(error) = open_vim_view(&view) {
                    eprintln!("sivtr: editor error: {error}");
                }
                Ok(())
            })??;
        }
        WorkspaceHelpAction::ScrollDown if *focus == WorkspaceFocus::Content => {
            content_scrolls.set(
                *content_io_focus,
                content_scrolls.get(*content_io_focus).saturating_add(10),
            );
        }
        WorkspaceHelpAction::ScrollUp if *focus == WorkspaceFocus::Content => {
            content_scrolls.set(
                *content_io_focus,
                content_scrolls.get(*content_io_focus).saturating_sub(10),
            );
        }
        WorkspaceHelpAction::ScrollContentTop if *focus == WorkspaceFocus::Content => {
            content_scrolls.set(*content_io_focus, 0);
        }
        WorkspaceHelpAction::ScrollContentBottom if *focus == WorkspaceFocus::Content => {
            let lines = content_pane.line_count(*content_io_focus);
            content_scrolls.set(*content_io_focus, lines.saturating_sub(1));
        }
        WorkspaceHelpAction::ToggleContentMode if *focus == WorkspaceFocus::Content => {
            *content_mode = content_mode.toggle();
        }
        WorkspaceHelpAction::ToggleContentIo if *focus == WorkspaceFocus::Content => {
            // Only which half scrolls changes; block ids span the dialogue, so
            // an open block range stays valid across the flip.
            *content_io_focus = match *content_io_focus {
                ContentIoFocus::Input => ContentIoFocus::Output,
                ContentIoFocus::Output => ContentIoFocus::Input,
            };
        }
        WorkspaceHelpAction::ToggleBlockFold if *focus == WorkspaceFocus::Content => {
            if *content_mode == ContentViewMode::Reading {
                if let Some(block) = content_cursor.get() {
                    expanded.toggle(block);
                    content_cursor.follow = true;
                }
            }
        }
        WorkspaceHelpAction::Copy => match *focus {
            WorkspaceFocus::Source => set_focus(focus, fullscreen, rows, WorkspaceFocus::Sessions),
            WorkspaceFocus::Sessions if dialogue_count > 0 => {
                set_focus(focus, fullscreen, rows, WorkspaceFocus::Dialogues)
            }
            WorkspaceFocus::Dialogues | WorkspaceFocus::Content => {
                return Ok(HelpDispatch::Picked(
                    workspace_picked_content_for_copy(
                        dialogues,
                        selected_dialogues,
                        WorkspaceCopyShortcut::Displayed,
                        line_filter,
                        content_at,
                        *content_mode,
                    )
                    .context("Failed to prepare displayed copy")?,
                ));
            }
            WorkspaceFocus::Sessions => {}
        },
        WorkspaceHelpAction::CopyInput if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy(
                    dialogues,
                    selected_dialogues,
                    WorkspaceCopyShortcut::Input,
                    line_filter,
                    None,
                    *content_mode,
                )
                .context("Failed to prepare input copy")?,
            ));
        }
        WorkspaceHelpAction::CopyOutput if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy(
                    dialogues,
                    selected_dialogues,
                    WorkspaceCopyShortcut::Output,
                    line_filter,
                    None,
                    *content_mode,
                )
                .context("Failed to prepare output copy")?,
            ));
        }
        WorkspaceHelpAction::CopyBlock if dialogue_count > 0 => {
            // y copies the block under the content cursor (call + result
            // bodies); marked blocks take over in the picker beforehand.
            // The block id belongs to the *displayed* dialogue — the one under
            // the dialogue cursor.
            let block_id = content_cursor.get().unwrap_or(0);
            if let Some(picked) = workspace_picked_content_for_cursor_block(
                dialogues,
                rows.dialogues.cursor(),
                block_id,
            )? {
                return Ok(HelpDispatch::Picked(picked));
            }
        }
        WorkspaceHelpAction::CopyCommand if dialogue_count > 0 => {
            return Ok(HelpDispatch::Picked(
                workspace_picked_content_for_copy(
                    dialogues,
                    selected_dialogues,
                    WorkspaceCopyShortcut::Command,
                    line_filter,
                    None,
                    *content_mode,
                )
                .context("Failed to prepare command copy")?,
            ));
        }
        WorkspaceHelpAction::ToggleFullscreen => {
            *fullscreen = toggle_fullscreen(*fullscreen, *focus);
        }
        WorkspaceHelpAction::ToggleHelp => {
            *show_help = !*show_help;
        }
        WorkspaceHelpAction::OpenSearch => {
            *show_help = false;
            // Search replaces the session list wholesale; the picker starts
            // the full-corpus loader after this dispatch returns.
            invalidate_panes_below(WorkspaceFocus::Source, rows, content_scrolls);
            return Ok(HelpDispatch::OpenSearch);
        }
        WorkspaceHelpAction::BackOrCancel => match *focus {
            WorkspaceFocus::Source | WorkspaceFocus::Sessions => {
                anyhow::bail!(PICK_CANCELLED_MESSAGE)
            }
            WorkspaceFocus::Dialogues => {
                set_focus(focus, fullscreen, rows, WorkspaceFocus::Sessions);
            }
            WorkspaceFocus::Content => {
                set_focus(focus, fullscreen, rows, WorkspaceFocus::Dialogues);
            }
        },
        WorkspaceHelpAction::Cancel => anyhow::bail!(PICK_CANCELLED_MESSAGE),
        WorkspaceHelpAction::Refresh => return Ok(HelpDispatch::Refresh),
        WorkspaceHelpAction::Publish => return Ok(HelpDispatch::Publish),
        // Focus-gated arms that did not match: ignore.
        WorkspaceHelpAction::FocusDialogues
        | WorkspaceHelpAction::FocusContent
        | WorkspaceHelpAction::OpenVim
        | WorkspaceHelpAction::ScrollDown
        | WorkspaceHelpAction::ScrollUp
        | WorkspaceHelpAction::ScrollContentTop
        | WorkspaceHelpAction::ScrollContentBottom
        | WorkspaceHelpAction::ToggleContentMode
        | WorkspaceHelpAction::ToggleContentIo
        | WorkspaceHelpAction::ToggleBlockFold
        | WorkspaceHelpAction::CopyInput
        | WorkspaceHelpAction::CopyOutput
        | WorkspaceHelpAction::CopyBlock
        | WorkspaceHelpAction::CopyCommand => {}
    }

    Ok(HelpDispatch::Continue)
}

pub(super) fn toggle_fullscreen(
    fullscreen: Option<WorkspaceFocus>,
    focus: WorkspaceFocus,
) -> Option<WorkspaceFocus> {
    if fullscreen == Some(focus) {
        None
    } else {
        Some(focus)
    }
}

/// Toggle the focused list row's selection mark — the single path shared by
/// the Space key and a dot-gutter click. `true` when panes below need a
/// refresh (a Source toggle reshapes the session/dialogue trees).
pub(super) fn toggle_list_row(
    focus: WorkspaceFocus,
    idx: usize,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
) -> bool {
    let Some(pane) = rows.scope_pane_mut(focus) else {
        return false;
    };
    pane.toggle_scope(idx);
    rows.close_ranges();
    invalidate_panes_below(focus, rows, content_scrolls)
}

pub(super) fn set_focus(
    focus: &mut WorkspaceFocus,
    fullscreen: &mut Option<WorkspaceFocus>,
    rows: &mut Rows,
    next: WorkspaceFocus,
) {
    *focus = next;
    // Range selection is per-pane: leaving a pane discards its anchor.
    rows.close_ranges();
    if fullscreen.is_some() {
        *fullscreen = Some(next);
    }
}
