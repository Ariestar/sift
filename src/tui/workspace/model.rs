//! Workspace browser domain types (sources, sessions, dialogues, view state).

use ratatui::prelude::Color;
use ratatui::widgets::ListState;
use regex::Regex;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef, WorkScope};
use std::collections::HashSet;
use std::time::SystemTime;

use crate::tui::content::block::{dialogue_block_id, fold_label_for_part, BlockText};
use crate::tui::content::io::{
    ContentIoFocus, ContentIoFrame, ContentIoTexts, ContentScrolls, ExpandedBlocks,
};
use crate::tui::content::text::content_io_from_record;
use crate::tui::content::view::{ContentSelection, ContentViewMode};
use crate::tui::theme;
use crate::tui::workspace::rows::Rows;
use sivtr_core::workset::{WorkSelectionKind, WorkSelectionTarget};

/// Indices of true entries in a selection mask, in order.
pub(crate) fn selected_indices(mask: &[bool]) -> Vec<usize> {
    mask.iter()
        .enumerate()
        .filter_map(|(idx, selected)| selected.then_some(idx))
        .collect()
}

/// Kind of memory source (local path body before any `scope:` prefix).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum WorkspaceSourceKind {
    Terminal,
    Agent(AgentProvider),
}

impl WorkspaceSourceKind {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Agent(provider) => provider.command_name(),
        }
    }

    pub(crate) fn badge(self) -> &'static str {
        match self {
            Self::Terminal => "term",
            Self::Agent(AgentProvider::Codex) => "cdx",
            Self::Agent(AgentProvider::Claude) => "cld",
            Self::Agent(AgentProvider::Cursor) => "cur",
            Self::Agent(AgentProvider::Dsh) => "dsh",
            Self::Agent(AgentProvider::OpenCode) => "opc",
            Self::Agent(AgentProvider::OpenClaw) => "ocw",
            Self::Agent(AgentProvider::Hermes) => "hrm",
            Self::Agent(AgentProvider::Grok) => "grk",
            Self::Agent(AgentProvider::Pi) => "pi",
            Self::Agent(AgentProvider::Qoder) => "qdr",
            Self::Agent(AgentProvider::QoderCn) => "qcn",
            Self::Agent(AgentProvider::Gemini) => "gmi",
            Self::Agent(AgentProvider::Goose) => "gse",
            Self::Agent(AgentProvider::Qwen) => "qwn",
            Self::Agent(AgentProvider::Zcode) => "zcd",
        }
    }

    pub(crate) fn color(self) -> Color {
        match self {
            Self::Terminal => theme::terminal_color(),
            Self::Agent(provider) => theme::provider_color(provider),
        }
    }

    pub(crate) fn is_agent(self) -> bool {
        matches!(self, Self::Agent(_))
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal)
    }
}

/// One selectable Source pane entry. Local and remote share the same shape —
/// remote is only a named scope that `workset::query` already understands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct WorkspaceSource {
    /// Named scope (`desk`, `docs`); `None` = current local workspace.
    pub(crate) scope: Option<String>,
    pub(crate) kind: WorkspaceSourceKind,
}

impl WorkspaceSource {
    pub(crate) fn local(kind: WorkspaceSourceKind) -> Self {
        Self { scope: None, kind }
    }

    pub(crate) fn terminal() -> Self {
        Self::local(WorkspaceSourceKind::Terminal)
    }

    pub(crate) fn agent(provider: AgentProvider) -> Self {
        Self::local(WorkspaceSourceKind::Agent(provider))
    }

    /// A source on another device, addressed by its mount alias.
    pub(crate) fn remote(scope: impl Into<String>, kind: WorkspaceSourceKind) -> Self {
        Self {
            scope: Some(scope.into()),
            kind,
        }
    }

    /// Selector passed to `workset::query` (`codex`, `desk:terminal`, …).
    pub(crate) fn selector(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{scope}:{}", self.kind.path()),
            None => self.kind.path().to_string(),
        }
    }

    /// Compact Source-pane label (`codex`, `desk/codex`).
    pub(crate) fn label(&self) -> String {
        match &self.scope {
            Some(scope) => format!("{scope}/{}", self.kind.path()),
            None => self.kind.path().to_string(),
        }
    }

    pub(crate) fn badge(&self) -> &'static str {
        self.kind.badge()
    }

    pub(crate) fn color(&self) -> Color {
        self.kind.color()
    }

    /// Whether this source needs remote transport (mount on another device).
    pub(crate) fn is_remote(&self) -> bool {
        self.scope.is_some()
    }

    pub(crate) fn is_agent(&self) -> bool {
        self.kind.is_agent()
    }

    pub(crate) fn is_terminal(&self) -> bool {
        self.kind.is_terminal()
    }

    pub(crate) fn selection_target(&self, session: Option<&str>) -> WorkSelectionTarget {
        WorkSelectionTarget::Scope {
            scope: self.scope.as_deref().map_or(WorkScope::Local, |scope| {
                WorkScope::Named(scope.to_string())
            }),
            kind: match self.kind {
                WorkspaceSourceKind::Terminal => WorkSelectionKind::Terminal,
                WorkspaceSourceKind::Agent(provider) => WorkSelectionKind::Agent(provider),
            },
            session: session.map(str::to_string),
        }
    }
}

/// Compact load indicator for the Source pane selection/status column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceLoadMarker {
    Idle,
    Loading,
    Ready,
    Failed,
}

impl SourceLoadMarker {
    /// Leading status/selection glyph. `tick` animates Loading as a circle.
    pub(crate) fn status_glyph(self, selected: bool, tick: u8) -> &'static str {
        match self {
            Self::Loading => {
                const FRAMES: [&str; 4] = ["◐", "◓", "◑", "◒"];
                FRAMES[(tick as usize) % FRAMES.len()]
            }
            Self::Failed => "!",
            Self::Idle | Self::Ready => crate::tui::pane::selection_dot(selected),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextPair {
    pub(crate) plain: String,
    pub(crate) ansi: String,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSession {
    pub(crate) source: WorkspaceSource,
    /// Stable session identity for hydrate / selection (not display title).
    pub(crate) session_id: String,
    pub(crate) modified: SystemTime,
    pub(crate) title: String,
    pub(crate) search_title: String,
    /// Dialogue bodies. Empty until the session is focused/selected and hydrated.
    pub(crate) records: Vec<WorkRecord>,
    /// True when `records` holds full dialogue bodies for this session.
    pub(crate) body_loaded: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceDialogue {
    pub(crate) source: WorkspaceSource,
    pub(crate) work_ref: Option<WorkRef>,
    pub(crate) record: Option<WorkRecord>,
}

impl WorkspaceDialogue {
    /// Input / Output bodies for the dual content panes with per-block fold
    /// state (every workpart is a block; structure blocks fold by default in
    /// read mode).
    pub(crate) fn content_io_texts(
        &self,
        mode: ContentViewMode,
        target: Option<WorkAt>,
        expanded: &ExpandedBlocks,
    ) -> ContentIoTexts {
        if let Some(target @ WorkAt::Part(_)) = target {
            // A targeted part lives alone in its own IO half as one block.
            let Some(record) = self.record.as_ref() else {
                return ContentIoTexts::new(Vec::new(), Vec::new());
            };
            let Some(part) = record.part_for_at(target) else {
                return ContentIoTexts::new(Vec::new(), Vec::new());
            };
            let input = part.kind().is_input();
            let block_id = dialogue_block_id(record, part.seq).expect("target part has a block");
            let shown = match mode {
                ContentViewMode::Raw => true,
                ContentViewMode::Reading => expanded.expanded(block_id, part.kind().is_structure()),
            };
            let segment = BlockText {
                id: block_id,
                text: if shown {
                    crate::tui::content::tool::part_body_text(part)
                } else {
                    fold_label_for_part(part)
                },
                tight: false,
                kind: part.kind(),
            };
            return if input {
                ContentIoTexts::new(vec![segment], Vec::new())
            } else {
                ContentIoTexts::new(Vec::new(), vec![segment])
            };
        }

        let Some(record) = self.record.as_ref() else {
            return ContentIoTexts::new(Vec::new(), Vec::new());
        };
        if record.parts.is_empty() {
            return ContentIoTexts::new(Vec::new(), Vec::new());
        }
        let reading = matches!(mode, ContentViewMode::Reading);
        content_io_from_record(record, reading, expanded)
    }

    pub(crate) fn content_ref(&self, target: Option<WorkAt>) -> Option<WorkRef> {
        let work_ref = self.work_ref.as_ref()?;
        let target = match target {
            Some(target @ WorkAt::Part(_)) => target,
            _ => return Some(work_ref.clone()),
        };
        Some(work_ref.with_at(target))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceFocus {
    Source,
    Sessions,
    Dialogues,
    Content,
}

impl WorkspaceFocus {
    pub(crate) const ORDER: [Self; 4] =
        [Self::Source, Self::Sessions, Self::Dialogues, Self::Content];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Source => "0",
            Self::Sessions => "1",
            Self::Dialogues => "2",
            Self::Content => "3",
        }
    }

    pub(crate) fn from_number_key(key: char, dialogue_count: usize) -> Option<Self> {
        let idx = key.to_digit(10)? as usize;
        Self::ORDER
            .get(idx)
            .copied()
            .filter(|focus| focus.is_available(dialogue_count))
    }

    pub(crate) fn previous(self, dialogue_count: usize) -> Option<Self> {
        let idx = self.order_index()?;
        Self::ORDER[..idx]
            .iter()
            .rev()
            .copied()
            .find(|focus| focus.is_available(dialogue_count))
    }

    pub(crate) fn next(self, dialogue_count: usize) -> Option<Self> {
        let idx = self.order_index()?;
        Self::ORDER[idx.saturating_add(1)..]
            .iter()
            .copied()
            .find(|focus| focus.is_available(dialogue_count))
    }

    fn is_available(self, dialogue_count: usize) -> bool {
        dialogue_count > 0 || !matches!(self, Self::Dialogues | Self::Content)
    }

    fn order_index(self) -> Option<usize> {
        Self::ORDER.iter().position(|focus| *focus == self)
    }
}

pub(crate) struct WorkspaceView<'a> {
    pub(crate) sources: &'a [WorkspaceSource],
    /// Per-source load marker (idle remote / ready / failed).
    pub(crate) source_markers: &'a [SourceLoadMarker],
    pub(crate) loading_tick: u8,
    /// Cursor, marks, and the live range anchor of all three list panes.
    pub(crate) rows: &'a Rows,
    pub(crate) sessions: &'a [WorkspaceSession],
    /// `(source, session id)` pairs whose body hydration failed (spawn or
    /// query error). Rows render an error marker and the loader does not retry
    /// them. Source-qualified so a local session sharing an id with a remote
    /// mirror does not mark the healthy row.
    pub(crate) body_failures: HashSet<(WorkspaceSource, String)>,
    /// Dialogue list titles only (no body materialize on paint).
    pub(crate) dialogue_titles: &'a [&'a str],
    /// WorkSet-derived dialogue selection for this frame.
    pub(crate) dialogue_selection: &'a [bool],
    /// Materialized dialogues for content/copy (focus ∪ multi-select bodies).
    pub(crate) dialogues: &'a [WorkspaceDialogue],
    pub(crate) focus: WorkspaceFocus,
    pub(crate) content_scrolls: ContentScrolls,
    pub(crate) content_io_focus: ContentIoFocus,
    pub(crate) content_mode: ContentViewMode,
    pub(crate) content_at: Option<WorkAt>,
    pub(crate) show_help: bool,
    pub(crate) help_state: &'a ListState,
    pub(crate) search: Option<WorkspaceSearchView<'a>>,
    pub(crate) line_filter_input_open: bool,
    pub(crate) line_filter: Option<&'a str>,
    pub(crate) line_filter_error: Option<&'a str>,
    pub(crate) fullscreen: Option<WorkspaceFocus>,
    pub(crate) content_selection: Option<ContentSelection>,
    pub(crate) publish: Option<WorkspacePublishView<'a>>,
    pub(crate) publish_error: Option<&'a str>,
    /// Block under the keyboard/mouse cursor plus its half (derived from
    /// the dialogue-global id); highlighted like a list row in that half.
    pub(crate) content_block_cursor: Option<(ContentIoFocus, usize)>,
    /// Pending `v` block-range span `(anchor block, cursor block)`;
    /// its lines render with the same amber range style as the list panes.
    pub(crate) content_range: Option<(usize, usize)>,
    /// WorkSet-derived block mask (`mask[block_id]` = marked, dialogue-global
    /// ids), consumed by the dot gutter and copy.
    pub(crate) content_marked: &'a [bool],
    /// Dual IO layout + display texts, computed once per redraw by the picker
    /// and shared with the renderer (no per-frame duplicate layout).
    pub(crate) content_frame: &'a ContentIoFrame,
}

pub(crate) struct WorkspaceSearchView<'a> {
    pub(crate) query: &'a str,
    pub(crate) result_count: usize,
    pub(crate) current_match: Option<usize>,
    pub(crate) match_count: usize,
    pub(crate) current_target: Option<String>,
    pub(crate) regex: Option<&'a Regex>,
    pub(crate) error: Option<&'a str>,
    pub(crate) input_open: bool,
}

pub(crate) struct WorkspacePublishView<'a> {
    pub(crate) name: &'a str,
    pub(crate) name_input: bool,
    pub(crate) preview: bool,
    pub(crate) selected: usize,
    pub(crate) redaction_count: usize,
    pub(crate) warning_count: usize,
    pub(crate) item_count: usize,
    pub(crate) schema_version: u32,
}

pub(crate) struct WorkspaceFooterView<'a> {
    pub(crate) focus: WorkspaceFocus,
    pub(crate) show_help: bool,
    pub(crate) search: Option<&'a WorkspaceSearchView<'a>>,
    pub(crate) line_filter_input_open: bool,
    pub(crate) line_filter: Option<&'a str>,
    pub(crate) line_filter_error: Option<&'a str>,
    pub(crate) fullscreen: Option<WorkspaceFocus>,
    pub(crate) content_selection: Option<ContentSelection>,
}
