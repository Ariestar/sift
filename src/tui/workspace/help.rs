//! Help registry: actions, key table, key parse, footer hotkeys.

use crate::tui::workspace::model::WorkspaceFocus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceHelpAction {
    FocusSource,
    FocusSessions,
    FocusDialogues,
    FocusContent,
    MoveUp,
    MoveDown,
    PreviousPane,
    NextPane,
    ToggleSelection,
    ToggleAll,
    SelectAgentSources,
    SelectTerminalSource,
    RangeSelect,
    OpenVim,
    ScrollDown,
    ScrollUp,
    ToggleContentMode,
    /// Switch focused content half (Input ↔ Output).
    ToggleContentIo,
    /// Content half: expand/collapse the cursor block.
    ToggleBlockFold,
    Copy,
    CopyInput,
    CopyOutput,
    CopyBlock,
    CopyCommand,
    ToggleFullscreen,
    /// Toggle the help overlay.
    ToggleHelp,
    /// Toggle the diagnostics overlay (full warning history).
    ToggleDiagnostics,
    OpenSearch,
    /// Esc in the main UI: cancel from Source/Sessions, else step focus left.
    BackOrCancel,
    Cancel,
    /// Refresh next level under active rows (source→sessions, session→dialogues).
    Refresh,
    /// Open the publication lifetime picker for the canonical selection.
    Publish,
    /// Content half: jump scroll to top / bottom.
    ScrollContentTop,
    ScrollContentBottom,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceHelpEntry {
    pub(crate) key: &'static str,
    pub(crate) description: &'static str,
    pub(crate) action: WorkspaceHelpAction,
    /// Short label for the footer (`refresh`). `None` = help-only, not in footer.
    pub(crate) footer_label: Option<&'static str>,
    /// Footer panes; empty = every focus pane.
    pub(crate) footer_panes: &'static [WorkspaceFocus],
}

pub(crate) fn workspace_help_entries() -> &'static [WorkspaceHelpEntry] {
    use WorkspaceFocus::*;
    const ALL: &[WorkspaceFocus] = &[];
    const SRC: &[WorkspaceFocus] = &[Source];
    const DIA: &[WorkspaceFocus] = &[Dialogues];
    const CNT: &[WorkspaceFocus] = &[Content];
    const NAV: &[WorkspaceFocus] = &[Source, Sessions, Dialogues, Content];
    const SD: &[WorkspaceFocus] = &[Sessions, Dialogues];
    const DC: &[WorkspaceFocus] = &[Dialogues, Content];

    &[
        WorkspaceHelpEntry {
            key: "0",
            description: "focus Source pane",
            action: WorkspaceHelpAction::FocusSource,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "1",
            description: "focus Sessions pane",
            action: WorkspaceHelpAction::FocusSessions,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "2",
            description: "focus Dialogues pane",
            action: WorkspaceHelpAction::FocusDialogues,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "3",
            description: "focus Content pane",
            action: WorkspaceHelpAction::FocusContent,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "j",
            description: "move down",
            action: WorkspaceHelpAction::MoveDown,
            footer_label: Some("move"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "k",
            description: "move up",
            action: WorkspaceHelpAction::MoveUp,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "h",
            description: "previous pane",
            action: WorkspaceHelpAction::PreviousPane,
            footer_label: Some("pane"),
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "l",
            description: "next pane",
            action: WorkspaceHelpAction::NextPane,
            footer_label: Some("pane"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "Space",
            description: "toggle selection",
            action: WorkspaceHelpAction::ToggleSelection,
            footer_label: Some("toggle"),
            footer_panes: &[Source, Sessions, Dialogues, Content],
        },
        WorkspaceHelpEntry {
            key: "g",
            description: "select agent sources",
            action: WorkspaceHelpAction::SelectAgentSources,
            footer_label: Some("agents"),
            footer_panes: SRC,
        },
        WorkspaceHelpEntry {
            key: "t",
            description: "select terminal source",
            action: WorkspaceHelpAction::SelectTerminalSource,
            footer_label: Some("terminal"),
            footer_panes: SRC,
        },
        WorkspaceHelpEntry {
            key: "R",
            description: "refresh next level under active rows",
            action: WorkspaceHelpAction::Refresh,
            footer_label: Some("refresh"),
            footer_panes: &[Source, Sessions, Dialogues],
        },
        WorkspaceHelpEntry {
            key: "v",
            description: "range select items",
            action: WorkspaceHelpAction::RangeSelect,
            footer_label: Some("range"),
            // All four panes share the range semantic: the list panes select
            // spans of rows, Content selects a span of blocks in one half.
            footer_panes: &[Source, Sessions, Dialogues, Content],
        },
        WorkspaceHelpEntry {
            key: "a",
            description: "切换全部",
            action: WorkspaceHelpAction::ToggleAll,
            footer_label: Some("全部"),
            footer_panes: &[Source, Sessions, Dialogues, Content],
        },
        WorkspaceHelpEntry {
            key: "t",
            description: "open in Vim",
            action: WorkspaceHelpAction::OpenVim,
            footer_label: Some("vim"),
            footer_panes: SD,
        },
        WorkspaceHelpEntry {
            key: "Ctrl-d",
            description: "scroll content down",
            action: WorkspaceHelpAction::ScrollDown,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "Ctrl-u",
            description: "scroll content up",
            action: WorkspaceHelpAction::ScrollUp,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "r",
            description: "toggle read/raw content",
            action: WorkspaceHelpAction::ToggleContentMode,
            footer_label: Some("read/raw"),
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "Tab",
            description: "switch Input/Output half",
            action: WorkspaceHelpAction::ToggleContentIo,
            footer_label: Some("io"),
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "g",
            description: "scroll content to top",
            action: WorkspaceHelpAction::ScrollContentTop,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "G",
            description: "scroll content to bottom",
            action: WorkspaceHelpAction::ScrollContentBottom,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "PgDn",
            description: "scroll content page down",
            action: WorkspaceHelpAction::ScrollDown,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "PgUp",
            description: "scroll content page up",
            action: WorkspaceHelpAction::ScrollUp,
            footer_label: None,
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "Esc",
            description: "back / cancel",
            action: WorkspaceHelpAction::BackOrCancel,
            footer_label: None,
            footer_panes: ALL,
        },
        WorkspaceHelpEntry {
            key: "i",
            description: "copy input",
            action: WorkspaceHelpAction::CopyInput,
            footer_label: Some("in"),
            footer_panes: DC,
        },
        WorkspaceHelpEntry {
            key: "o",
            description: "copy output",
            action: WorkspaceHelpAction::CopyOutput,
            footer_label: Some("out"),
            footer_panes: DC,
        },
        WorkspaceHelpEntry {
            key: "y",
            description: "copy block",
            action: WorkspaceHelpAction::CopyBlock,
            footer_label: Some("copy"),
            footer_panes: DC,
        },
        WorkspaceHelpEntry {
            key: "c",
            description: "copy command",
            action: WorkspaceHelpAction::CopyCommand,
            footer_label: Some("cmd"),
            footer_panes: DIA,
        },
        WorkspaceHelpEntry {
            key: "Enter",
            description: "expand/collapse block",
            action: WorkspaceHelpAction::ToggleBlockFold,
            footer_label: Some("fold"),
            footer_panes: CNT,
        },
        WorkspaceHelpEntry {
            key: "Enter",
            description: "confirm / open next / copy",
            action: WorkspaceHelpAction::Copy,
            footer_label: Some("enter"),
            // Content's Enter folds blocks (see ToggleBlockFold above).
            footer_panes: &[Source, Sessions, Dialogues],
        },
        WorkspaceHelpEntry {
            key: "z",
            description: "toggle fullscreen",
            action: WorkspaceHelpAction::ToggleFullscreen,
            footer_label: Some("full"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "?",
            description: "toggle help",
            action: WorkspaceHelpAction::ToggleHelp,
            footer_label: Some("help"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "!",
            description: "diagnostics log",
            action: WorkspaceHelpAction::ToggleDiagnostics,
            footer_label: Some("diag"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "/",
            description: "search",
            action: WorkspaceHelpAction::OpenSearch,
            footer_label: Some("search"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "p",
            description: "发布选择",
            action: WorkspaceHelpAction::Publish,
            footer_label: Some("发布"),
            footer_panes: NAV,
        },
        WorkspaceHelpEntry {
            key: "q",
            description: "cancel",
            action: WorkspaceHelpAction::Cancel,
            footer_label: Some("quit"),
            footer_panes: NAV,
        },
    ]
}

/// Parse a help-table key spec into crossterm key identity.
pub(crate) fn parse_help_key(
    spec: &str,
) -> Option<(crossterm::event::KeyCode, crossterm::event::KeyModifiers)> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    if let Some(rest) = spec.strip_prefix("Ctrl-") {
        let (code, _) = parse_help_key(rest)?;
        return Some((code, KeyModifiers::CONTROL));
    }
    let code = match spec {
        "Tab" => KeyCode::Tab,
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Space" => KeyCode::Char(' '),
        "PgDn" | "PageDown" => KeyCode::PageDown,
        "PgUp" | "PageUp" => KeyCode::PageUp,
        s if s.chars().count() == 1 => KeyCode::Char(s.chars().next()?),
        _ => return None,
    };
    Some((code, KeyModifiers::NONE))
}

fn key_matches(
    spec: &str,
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some((want_code, want_mods)) = parse_help_key(spec) else {
        return false;
    };
    // Ctrl bindings require CONTROL; bare keys ignore extra shift on letters via equality.
    if want_mods.contains(KeyModifiers::CONTROL) {
        return code == want_code && modifiers.contains(KeyModifiers::CONTROL);
    }
    // PageDown/Up may arrive without modifiers.
    if matches!(want_code, KeyCode::PageDown | KeyCode::PageUp) {
        return code == want_code;
    }
    // Bare char: no CONTROL (so Ctrl-d doesn't also fire MoveDown on 'd').
    if modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    want_code == code
}

/// Resolve a pressed key through the help registry for the current focus.
///
/// First matching entry whose `footer_panes` allows `focus` wins (empty panes = all).
pub(crate) fn help_action_for_key(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
    focus: WorkspaceFocus,
) -> Option<WorkspaceHelpAction> {
    for entry in workspace_help_entries() {
        if !entry.footer_panes.is_empty() && !entry.footer_panes.contains(&focus) {
            continue;
        }
        if key_matches(entry.key, code, modifiers) {
            return Some(entry.action);
        }
    }
    None
}

/// Footer hotkeys for the focused pane, built from the help registry.
pub(crate) fn workspace_footer_hotkeys(focus: WorkspaceFocus) -> String {
    let mut parts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in workspace_help_entries() {
        let Some(label) = entry.footer_label else {
            continue;
        };
        if !entry.footer_panes.is_empty() && !entry.footer_panes.contains(&focus) {
            continue;
        }
        if !seen.insert(label) {
            continue;
        }
        parts.push(format!("{} {label}", entry.key));
    }
    parts.join("  ")
}
