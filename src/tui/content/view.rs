use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Alignment, Color, Frame, Modifier, Position, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use regex::Regex;
use std::rc::Rc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::content::block::{BlockRole, BlockText};
use crate::tui::content::markdown::{render_markdown_lines, MarkdownLineKind};
use crate::tui::pane::{panel_block, render_panel_scrollbar, Panel, PanelScroll};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentViewMode {
    Raw,
    Reading,
}

impl ContentViewMode {
    pub(crate) fn toggle(self) -> Self {
        match self {
            Self::Raw => Self::Reading,
            Self::Reading => Self::Raw,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            // Read folds structure blocks to their `<:…:>` tags; raw shows
            // full payloads. Both render tags with the same gray style.
            Self::Raw => "raw",
            Self::Reading => "read",
        }
    }
}

pub(crate) struct ContentView<'a> {
    /// Precomputed layout (lines + ownership + kinds) of the displayed text;
    /// scroll only slices it.
    pub(crate) layout: &'a ContentLayout,
    pub(crate) scroll: usize,
    pub(crate) search_regex: Option<&'a Regex>,
    pub(crate) selection: Option<ContentSelection>,
    /// Block under the keyboard/mouse cursor; its displayed line range is
    /// highlighted with the list-row focus style.
    pub(crate) cursor_block: Option<usize>,
    /// Pending `v` block-range span (anchor block, cursor block). Lines owned
    /// by any block in the span use the list panes' amber range style.
    pub(crate) range_blocks: Option<(usize, usize)>,
    /// Marked block mask (`mask[block_id]` = marked) for the batch-copy dots.
    pub(crate) marked: &'a [bool],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentPosition {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentSelection {
    pub(crate) anchor: ContentPosition,
    pub(crate) cursor: ContentPosition,
    pub(crate) kind: ContentSelectionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentSelectionKind {
    Linear,
    Block,
}

#[derive(Clone)]
pub(crate) struct ContentLine {
    pub(crate) line: Line<'static>,
    pub(crate) kind: MarkdownLineKind,
    pub(crate) links: Vec<ContentLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContentLink {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) target: String,
}

/// Precomputed display layout of one content half: the markdown+wrap lines,
/// the block ownership per line, and the block-kind colors for the dot
/// gutter. Scroll only slices [`Self::lines`]; the layout is rebuilt when
/// the content changes, so wheel events never re-lay-out the text.
#[derive(Clone, Default)]
pub(crate) struct ContentLayout {
    pub(crate) lines: Vec<ContentLine>,
    pub(crate) ownership: Vec<Option<usize>>,
    /// Dot color per line, parallel to [`Self::ownership`]: block ids are
    /// dialogue-global, so an id-keyed table would have to be padded across
    /// the other half's range and would report a foreign id as a real kind.
    pub(crate) kinds: Vec<Option<BlockRole>>,
}

/// Lay one half out once from its blocks. Empty halves use `text` (`<empty>`).
pub(crate) fn layout_content(
    area: Rect,
    text: &str,
    blocks: &[BlockText],
    mode: ContentViewMode,
) -> ContentLayout {
    let inner = panel_inner(area);
    let width = inner.width.saturating_sub(GUTTER_WIDTH) as usize;
    if blocks.is_empty() {
        let lines = all_content_lines(text, width, mode);
        let n = lines.len().max(1);
        return ContentLayout {
            lines,
            ownership: vec![None; n],
            kinds: vec![None; n],
        };
    }
    let mut lines = Vec::new();
    let mut ownership = Vec::new();
    let mut kinds = Vec::new();
    for (idx, segment) in blocks.iter().enumerate() {
        let block_lines = all_content_lines(&segment.text, width, mode);
        ownership.extend(std::iter::repeat_n(Some(segment.id), block_lines.len()));
        kinds.extend(std::iter::repeat_n(Some(segment.role), block_lines.len()));
        lines.extend(block_lines);
        if idx + 1 < blocks.len() && !segment.tight {
            ownership.push(None);
            kinds.push(None);
            lines.push(raw_content_line(""));
        }
    }
    ContentLayout {
        lines,
        ownership,
        kinds,
    }
}

pub(crate) fn render_content_view(
    frame: &mut Frame,
    area: Rect,
    panel: Panel,
    view: ContentView<'_>,
) {
    let block = panel_block(&panel);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Scroll slices the cached layout; the layout itself is rebuilt only
    // when the content changes.
    let chunks = content_chunks(inner);
    let visible_height = inner.height as usize;
    let total_lines = view.layout.lines.len().max(1);
    let scroll = view.scroll.min(total_lines.saturating_sub(1));
    let visible: Vec<ContentLine> = view
        .layout
        .lines
        .iter()
        .skip(scroll)
        .take(visible_height)
        .cloned()
        .collect();
    let block_highlight = view
        .cursor_block
        .and_then(|block| content_block_range(view.layout, block));
    let range_highlight = view
        .range_blocks
        .and_then(|(anchor, cursor)| content_range_line_ranges(view.layout, anchor, cursor));
    frame.render_widget(
        Paragraph::new(block_dot_lines(view.layout, view.marked, scroll, &visible)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(content_lines(
            visible,
            scroll,
            view.search_regex,
            view.selection,
            range_highlight.as_deref(),
            block_highlight,
            chunks[1].width as usize,
        )),
        chunks[1],
    );
    render_panel_scrollbar(
        frame,
        area,
        PanelScroll::new(scroll, total_lines, visible_height),
        panel.active(),
    );
}

/// Content panel's inner rect (borders already accounted) — the geometry
/// every layout and hit-test helper asks for, instead of re-deriving a
/// borderless chrome block each time.
fn panel_inner(area: Rect) -> Rect {
    panel_block(&Panel::new("", "", false)).inner(area)
}

/// Horizontal split of a panel's inner area: fixed dot gutter + content.
fn content_chunks(inner: Rect) -> Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(GUTTER_WIDTH), Constraint::Min(1)])
        .split(inner)
}

/// Content column rect (the panel's inner area minus the fixed dot gutter);
/// pure geometry, no layout.
pub(crate) fn content_text_area(area: Rect) -> Rect {
    let inner = panel_inner(area);
    Rect {
        x: inner.x.saturating_add(GUTTER_WIDTH),
        y: inner.y,
        width: inner.width.saturating_sub(GUTTER_WIDTH),
        height: inner.height,
    }
}

pub(crate) fn content_link_at(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    column: u16,
    row: u16,
) -> Option<String> {
    let inner = panel_inner(area);
    if inner.width == 0
        || inner.height == 0
        || column < inner.x
        || row < inner.y
        || column >= inner.x.saturating_add(inner.width)
        || row >= inner.y.saturating_add(inner.height)
    {
        return None;
    }

    let (lines, chunks) = content_layout(area, text, mode);
    let content_area = chunks[1];
    if column < content_area.x
        || column >= content_area.x.saturating_add(content_area.width)
        || row < content_area.y
        || row >= content_area.y.saturating_add(content_area.height)
    {
        return None;
    }

    let scroll = scroll.min(lines.len().saturating_sub(1));
    let line = lines.get(scroll.saturating_add((row - content_area.y) as usize))?;
    let hit_col = (column - content_area.x) as usize;
    line.links
        .iter()
        .find(|link| link.start <= hit_col && hit_col < link.end)
        .map(|link| link.target.clone())
}

pub(crate) fn content_position_at(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    column: u16,
    row: u16,
) -> Option<ContentPosition> {
    let content_area = content_text_area(area);
    if content_area.width == 0
        || content_area.height == 0
        || column < content_area.x
        || row < content_area.y
        || column >= content_area.x.saturating_add(content_area.width)
        || row >= content_area.y.saturating_add(content_area.height)
    {
        return None;
    }

    let line = scroll.saturating_add((row - content_area.y) as usize);
    let column = (column - content_area.x) as usize;
    Some(clamp_content_position(
        area,
        text,
        mode,
        ContentPosition { line, column },
    ))
}

/// Whether a mouse row inside the content area lands on a real rendered
/// line. [`content_position_at`] clamps rows below the last line to it, so
/// callers that treat a hit as content (e.g. block toggling) must check the
/// raw row against the wrapped line count first. Call after
/// [`content_position_at`] returned `Some`, when the row is already known to
/// be inside the content area.
pub(crate) fn content_row_in_text(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    scroll: usize,
    row: u16,
) -> bool {
    let content_area = content_text_area(area);
    let raw_line = scroll.saturating_add((row.saturating_sub(content_area.y)) as usize);
    raw_line < content_view_line_count(area, text, mode)
}

/// Block owning `line` (a displayed line index), if any. Blocks are laid out
/// from their own segments (one blank separator line between blocks, none
/// between members of the same run), so every displayed line maps directly
/// onto the block that produced it — tags, bodies, and close markers all
/// belong to the same block.
pub(crate) fn content_block_at(layout: &ContentLayout, line: usize) -> Option<usize> {
    layout.ownership.get(line).copied().flatten()
}

/// Displayed line range (start..end, end exclusive) owned by `block`, if the
/// block is present in the layout.
pub(crate) fn content_block_range(
    layout: &ContentLayout,
    block: usize,
) -> Option<std::ops::Range<usize>> {
    let start = layout
        .ownership
        .iter()
        .position(|owner| *owner == Some(block))?;
    let end = layout
        .ownership
        .iter()
        .rposition(|owner| *owner == Some(block))?;
    Some(start..end + 1)
}

/// Displayed line ranges owned by any block in `anchor..=cursor`, grouped
/// into contiguous chunks — the pending `v` span highlight.
pub(crate) fn content_range_line_ranges(
    layout: &ContentLayout,
    anchor: usize,
    cursor: usize,
) -> Option<Vec<std::ops::Range<usize>>> {
    let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for (line_idx, owner) in layout.ownership.iter().enumerate() {
        if owner.is_some_and(|id| id >= lo && id <= hi) {
            match ranges.last_mut() {
                Some(range) if range.end == line_idx => range.end = line_idx + 1,
                _ => ranges.push(line_idx..line_idx + 1),
            }
        }
    }
    (!ranges.is_empty()).then_some(ranges)
}

pub(crate) fn content_position_in_text_row(
    area: Rect,
    text: &str,
    scroll: usize,
    mode: ContentViewMode,
    column: u16,
    row: u16,
) -> Option<ContentPosition> {
    let inner = panel_inner(area);
    let content_area = content_text_area(area);
    if inner.width == 0
        || inner.height == 0
        || content_area.width == 0
        || content_area.height == 0
        || row < content_area.y
        || row >= content_area.y.saturating_add(content_area.height)
        || column < inner.x
        || column >= inner.x.saturating_add(inner.width)
    {
        return None;
    }

    if column >= content_area.x && column < content_area.x.saturating_add(content_area.width) {
        return content_position_at(area, text, scroll, mode, column, row);
    }

    let raw_line = scroll.saturating_add((row - content_area.y) as usize);
    let line_count = all_content_lines(text, content_area.width as usize, mode)
        .len()
        .max(1);
    let line = raw_line.min(line_count.saturating_sub(1));
    let column = column
        .saturating_sub(content_area.x)
        .min(content_area.width.saturating_sub(1)) as usize;
    Some(ContentPosition { line, column })
}

pub(crate) fn content_cursor_position(
    area: Rect,
    scroll: usize,
    position: ContentPosition,
) -> Option<Position> {
    let content_area = content_text_area(area);
    if content_area.width == 0
        || content_area.height == 0
        || position.line < scroll
        || position.line >= scroll.saturating_add(content_area.height as usize)
    {
        return None;
    }

    let column = position
        .column
        .min(content_area.width.saturating_sub(1) as usize);
    Some(Position::new(
        content_area.x.saturating_add(column as u16),
        content_area
            .y
            .saturating_add(position.line.saturating_sub(scroll) as u16),
    ))
}

pub(crate) fn clamp_content_position(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    position: ContentPosition,
) -> ContentPosition {
    let content_area = content_text_area(area);
    let width = content_area.width as usize;
    let lines = all_content_lines(text, width, mode);
    let line = position.line.min(lines.len().saturating_sub(1));
    let max_column = line_text_width(lines.get(line).map(|line| &line.line)).saturating_sub(1);
    ContentPosition {
        line,
        column: position.column.min(max_column),
    }
}

pub(crate) fn selected_content_text(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    selection: ContentSelection,
) -> String {
    let content_area = content_text_area(area);
    let width = content_area.width as usize;
    let lines = all_content_lines(text, width, mode);
    if lines.is_empty() {
        return String::new();
    }

    let selection = clamp_content_selection(area, text, mode, selection);
    let (start, end) = normalized_selection(selection);
    (start.line..=end.line)
        .filter_map(|line_idx| {
            let line = lines.get(line_idx)?;
            let width = line_text_width(Some(&line.line));
            let range = selection_range_for_line(selection, start, end, line_idx, width)?;
            Some(line_text_columns(&line.line, range.start, range.end))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clamp_content_selection(
    area: Rect,
    text: &str,
    mode: ContentViewMode,
    selection: ContentSelection,
) -> ContentSelection {
    if selection.kind == ContentSelectionKind::Linear {
        return ContentSelection {
            anchor: clamp_content_position(area, text, mode, selection.anchor),
            cursor: clamp_content_position(area, text, mode, selection.cursor),
            kind: selection.kind,
        };
    }

    let content_area = content_text_area(area);
    let lines = all_content_lines(text, content_area.width as usize, mode);
    let max_line = lines.len().saturating_sub(1);
    let max_column = content_area.width.saturating_sub(1) as usize;
    ContentSelection {
        anchor: ContentPosition {
            line: selection.anchor.line.min(max_line),
            column: selection.anchor.column.min(max_column),
        },
        cursor: ContentPosition {
            line: selection.cursor.line.min(max_line),
            column: selection.cursor.column.min(max_column),
        },
        kind: selection.kind,
    }
}

pub(crate) fn content_view_line_count(area: Rect, text: &str, mode: ContentViewMode) -> usize {
    let inner = panel_inner(area);
    if inner.width == 0 || inner.height == 0 {
        return 1;
    }
    content_layout_lines_metrics(inner.width, text, mode)
        .len()
        .max(1)
}

/// Gutter width: one dot column plus one trailing space — no line numbers.
const GUTTER_WIDTH: u16 = 2;

fn content_layout(area: Rect, text: &str, mode: ContentViewMode) -> (Vec<ContentLine>, Rc<[Rect]>) {
    let inner = panel_inner(area);
    // Gutter: dialogue dots per block, right-aligned dot plus one trailing
    // space — no separator bar between the dots and the content.
    let lines = content_layout_lines_metrics(inner.width, text, mode);
    (lines, content_chunks(inner))
}

#[cfg(test)]
fn visible_content_lines(
    text: &str,
    scroll: usize,
    height: usize,
    width: usize,
    mode: ContentViewMode,
) -> Vec<ContentLine> {
    all_content_lines(text, width, mode)
        .into_iter()
        .skip(scroll)
        .take(height)
        .collect()
}

/// Lay the document out per mode. Read/fold renders markdown (tables, code
/// fences, headings, links); raw/full keeps the text literal but still
/// styles structure markers in gray — so both modes share one body/structure
/// color treatment and differ only in markdown layout.
fn all_content_lines(text: &str, width: usize, mode: ContentViewMode) -> Vec<ContentLine> {
    let lines = raw_lines(text);
    let logical_lines = match mode {
        ContentViewMode::Raw => lines
            .iter()
            .map(|line| raw_content_line(line))
            .collect::<Vec<_>>(),
        ContentViewMode::Reading => render_markdown_lines(&lines, width)
            .into_iter()
            .map(|line| ContentLine {
                line: line.line,
                kind: line.kind,
                links: line
                    .links
                    .into_iter()
                    .map(|link| ContentLink {
                        start: link.start,
                        end: link.end,
                        target: link.target,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>(),
    };

    logical_lines
        .into_iter()
        .flat_map(|line| fit_content_line(line, width))
        .collect()
}

/// Literal rendering for raw/full content: no markdown layout, but
/// `<:channel:…:>` structure markers use the same structural gray as
/// read/fold summaries so the two modes stay visually consistent.
fn raw_content_line(line: &str) -> ContentLine {
    let style = if crate::tui::content::text::is_structure_marker(line) {
        Style::default().fg(crate::tui::theme::muted())
    } else {
        Style::default()
    };
    ContentLine {
        line: Line::from(Span::styled(line.to_string(), style)),
        kind: MarkdownLineKind::Normal,
        links: Vec::new(),
    }
}

/// Lay the document out. The gutter is a fixed dot column, so no width
/// convergence is needed: the content column is the inner width minus the
/// gutter (dot + one trailing space).
fn content_layout_lines_metrics(
    inner_width: u16,
    text: &str,
    mode: ContentViewMode,
) -> Vec<ContentLine> {
    let content_width = inner_width.saturating_sub(GUTTER_WIDTH) as usize;
    all_content_lines(text, content_width, mode)
}

fn content_lines(
    visible: Vec<ContentLine>,
    scroll: usize,
    search_regex: Option<&Regex>,
    selection: Option<ContentSelection>,
    range_highlight: Option<&[std::ops::Range<usize>]>,
    block_highlight: Option<std::ops::Range<usize>>,
    width: usize,
) -> Text<'static> {
    let lines = visible
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let line_idx = scroll.saturating_add(idx);
            let line = styled_content_line(line.line, search_regex);
            let line = styled_selection_line(line, selection, line_idx);
            style_block_line(
                line,
                range_highlight,
                block_highlight.as_ref(),
                line_idx,
                width,
            )
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

/// Tint every line of the pending range span (amber) or the cursor block
/// (list-row focus) and pad the remainder of the row so the whole block
/// lights up — the same full-width highlight the session/dialogue lists use.
/// The style decision itself is the shared pane `span_style`.
fn style_block_line(
    line: Line<'static>,
    range_highlight: Option<&[std::ops::Range<usize>]>,
    block_highlight: Option<&std::ops::Range<usize>>,
    line_idx: usize,
    width: usize,
) -> Line<'static> {
    let in_span =
        range_highlight.is_some_and(|ranges| ranges.iter().any(|range| range.contains(&line_idx)));
    let focused = block_highlight.is_some_and(|range| range.contains(&line_idx));
    let Some(overlay) = crate::tui::pane::span_style(in_span, focused) else {
        return line;
    };
    let text_width = line_text_width(Some(&line));
    let mut spans = line
        .spans
        .into_iter()
        .map(|span| Span::styled(span.content, span.style.patch(overlay)))
        .collect::<Vec<_>>();
    let fill = width.saturating_sub(text_width);
    if fill > 0 {
        spans.push(Span::styled(" ".repeat(fill), overlay));
    }
    Line {
        spans,
        style: line.style.patch(overlay),
        alignment: line.alignment,
    }
}

fn styled_selection_line(
    line: Line<'static>,
    selection: Option<ContentSelection>,
    line_idx: usize,
) -> Line<'static> {
    let Some(selection) = selection else {
        return line;
    };
    let width = line_text_width(Some(&line));
    let (start, end) = normalized_selection(selection);
    let Some(range) = selection_range_for_line(selection, start, end, line_idx, width) else {
        return line;
    };
    style_line_columns(
        line,
        range.start,
        range.end,
        crate::tui::theme::text_selection_row(),
    )
}

fn normalized_selection(selection: ContentSelection) -> (ContentPosition, ContentPosition) {
    if (selection.anchor.line, selection.anchor.column)
        <= (selection.cursor.line, selection.cursor.column)
    {
        (selection.anchor, selection.cursor)
    } else {
        (selection.cursor, selection.anchor)
    }
}

fn selection_range_for_line(
    selection: ContentSelection,
    start: ContentPosition,
    end: ContentPosition,
    line_idx: usize,
    line_width: usize,
) -> Option<std::ops::Range<usize>> {
    if line_idx < start.line || line_idx > end.line {
        return None;
    }
    if selection.kind == ContentSelectionKind::Block {
        let start_column = selection.anchor.column.min(selection.cursor.column);
        let end_column = selection.anchor.column.max(selection.cursor.column);
        let start = start_column.min(line_width);
        let end = end_column.min(line_width).saturating_add(1);
        return (start < end).then_some(start..end);
    }

    let fallback_width = line_width.max(1);
    let range = if start.line == end.line {
        start.column.min(line_width)..end.column.min(line_width).saturating_add(1)
    } else if line_idx == start.line {
        start.column.min(line_width)..fallback_width
    } else if line_idx == end.line {
        0..end.column.min(line_width).saturating_add(1)
    } else {
        0..fallback_width
    };
    (range.start < range.end).then_some(range)
}

fn style_line_columns(
    line: Line<'static>,
    start: usize,
    end: usize,
    overlay: Style,
) -> Line<'static> {
    let mut output = Vec::new();
    let mut column = 0usize;
    for span in line.spans {
        for ch in span.content.chars() {
            let width = char_width(ch);
            let ch_start = column;
            let ch_end = column.saturating_add(width.max(1));
            let style = if ch_start < end && ch_end > start {
                span.style.patch(overlay)
            } else {
                span.style
            };
            push_char_span(&mut output, ch, style);
            column = column.saturating_add(width);
        }
    }

    if output.is_empty() && start < end {
        output.push(Span::styled(" ", overlay));
    }

    Line {
        spans: output,
        style: line.style,
        alignment: line.alignment,
    }
}

fn line_text_columns(line: &Line<'static>, start: usize, end: usize) -> String {
    let mut output = String::new();
    let mut column = 0usize;
    for span in &line.spans {
        for ch in span.content.chars() {
            let width = char_width(ch);
            let ch_start = column;
            let ch_end = column.saturating_add(width.max(1));
            if ch_start < end && ch_end > start {
                output.push(ch);
            }
            column = column.saturating_add(width);
        }
    }
    output
}

fn line_text_width(line: Option<&Line<'static>>) -> usize {
    line.map(|line| {
        line.spans
            .iter()
            .map(|span| text_width(span.content.as_ref()))
            .sum()
    })
    .unwrap_or(0)
}

fn fit_content_line(line: ContentLine, width: usize) -> Vec<ContentLine> {
    if width == 0 {
        return vec![ContentLine {
            line: Line::default(),
            kind: line.kind,
            links: Vec::new(),
        }];
    }

    match line.kind {
        MarkdownLineKind::Normal => wrap_content_line(line, width),
        MarkdownLineKind::CodeFence | MarkdownLineKind::CodeBlock | MarkdownLineKind::Table => {
            vec![ContentLine {
                links: clip_links_to_width(&line.links, width),
                line: clip_line_to_width(line.line, width),
                kind: line.kind,
            }]
        }
    }
}

fn wrap_content_line(line: ContentLine, width: usize) -> Vec<ContentLine> {
    if line.line.spans.is_empty() {
        return vec![line];
    }

    let style = line.line.style;
    let alignment = line.line.alignment;
    let mut wrapped = Vec::new();
    let mut current = Vec::new();
    let mut current_links = Vec::new();
    let mut current_width = 0usize;
    let mut source_width = 0usize;

    for span in line.line.spans {
        let span_start = source_width;
        source_width += text_width(span.content.as_ref());
        for token in span_wrap_tokens(&span, span_start) {
            let token_width = text_width(&token.content);
            if token_width == 0 {
                append_token(&mut current, token);
                continue;
            }

            if token.whitespace && current_width == 0 {
                continue;
            }

            if token_width > width && !token.whitespace && !token.unbreakable {
                if current_width > 0 {
                    trim_trailing_spaces(&mut current, &mut current_width);
                    push_wrapped_line(
                        &mut wrapped,
                        &mut current,
                        &mut current_links,
                        &mut current_width,
                        style,
                        alignment,
                        line.kind,
                    );
                }
                for part in break_token_to_width(token, width) {
                    let part_width = text_width(&part.content);
                    if part_width == width {
                        append_token_links(&mut current_links, &line.links, current_width, &part);
                        append_token(&mut current, part);
                        push_wrapped_line(
                            &mut wrapped,
                            &mut current,
                            &mut current_links,
                            &mut current_width,
                            style,
                            alignment,
                            line.kind,
                        );
                    } else {
                        append_token_links(&mut current_links, &line.links, current_width, &part);
                        append_token(&mut current, part);
                        current_width += part_width;
                    }
                }
                continue;
            }

            if current_width > 0 && current_width + token_width > width {
                trim_trailing_spaces(&mut current, &mut current_width);
                push_wrapped_line(
                    &mut wrapped,
                    &mut current,
                    &mut current_links,
                    &mut current_width,
                    style,
                    alignment,
                    line.kind,
                );
                if token.whitespace {
                    continue;
                }
            }

            append_token_links(&mut current_links, &line.links, current_width, &token);
            append_token(&mut current, token);
            current_width += token_width;
        }
    }

    trim_trailing_spaces(&mut current, &mut current_width);
    if current_width > 0 || wrapped.is_empty() {
        wrapped.push(ContentLine {
            line: Line {
                spans: current,
                style,
                alignment,
            },
            kind: line.kind,
            links: current_links,
        });
    }

    wrapped
}

struct WrapToken {
    content: String,
    style: Style,
    whitespace: bool,
    unbreakable: bool,
    source_start: usize,
    source_end: usize,
}

fn span_wrap_tokens(span: &Span<'static>, source_start: usize) -> Vec<WrapToken> {
    let content = span.content.as_ref();
    if is_unbreakable_span(span) {
        let width = text_width(content);
        return vec![WrapToken {
            content: content.to_string(),
            style: span.style,
            whitespace: false,
            unbreakable: true,
            source_start,
            source_end: source_start + width,
        }];
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_whitespace = None;
    let mut current_source_start = source_start;
    let mut current_source_end = source_start;
    for ch in content.chars() {
        let whitespace = ch.is_whitespace();
        if current_whitespace == Some(whitespace) || current.is_empty() {
            current.push(ch);
            current_whitespace = Some(whitespace);
            current_source_end += char_width(ch);
            continue;
        }

        tokens.push(WrapToken {
            content: std::mem::take(&mut current),
            style: span.style,
            whitespace: current_whitespace.unwrap_or(false),
            unbreakable: false,
            source_start: current_source_start,
            source_end: current_source_end,
        });
        current_source_start = current_source_end;
        current.push(ch);
        current_whitespace = Some(whitespace);
        current_source_end += char_width(ch);
    }

    if !current.is_empty() {
        tokens.push(WrapToken {
            content: current,
            style: span.style,
            whitespace: current_whitespace.unwrap_or(false),
            unbreakable: false,
            source_start: current_source_start,
            source_end: current_source_end,
        });
    }

    tokens
}

fn is_unbreakable_span(span: &Span<'static>) -> bool {
    let text = span.content.as_ref();
    span.style.fg == Some(Color::DarkGray)
        && text.starts_with(" (")
        && text.ends_with(')')
        && (text.contains("://")
            || text.contains(":/")
            || text.contains(":\\")
            || text.contains('/')
            || text.contains('\\'))
}

fn break_token_to_width(token: WrapToken, width: usize) -> Vec<WrapToken> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut current_source_start = token.source_start;
    let mut current_source_end = token.source_start;

    for ch in token.content.chars() {
        let ch_width = char_width(ch);
        if ch_width > width {
            continue;
        }
        if current_width > 0 && current_width + ch_width > width {
            parts.push(WrapToken {
                content: std::mem::take(&mut current),
                style: token.style,
                whitespace: false,
                unbreakable: false,
                source_start: current_source_start,
                source_end: current_source_end,
            });
            current_source_start = current_source_end;
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
        current_source_end += ch_width;
    }

    if !current.is_empty() || parts.is_empty() {
        parts.push(WrapToken {
            content: current,
            style: token.style,
            whitespace: false,
            unbreakable: false,
            source_start: current_source_start,
            source_end: current_source_end,
        });
    }

    parts
}

fn append_token_links(
    current_links: &mut Vec<ContentLink>,
    source_links: &[ContentLink],
    current_width: usize,
    token: &WrapToken,
) {
    for link in source_links {
        let start = link.start.max(token.source_start);
        let end = link.end.min(token.source_end);
        if start >= end {
            continue;
        }
        current_links.push(ContentLink {
            start: current_width + start.saturating_sub(token.source_start),
            end: current_width + end.saturating_sub(token.source_start),
            target: link.target.clone(),
        });
    }
}

fn append_token(spans: &mut Vec<Span<'static>>, token: WrapToken) {
    if token.content.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut().filter(|span| span.style == token.style) {
        last.content.to_mut().push_str(&token.content);
    } else {
        spans.push(Span::styled(token.content, token.style));
    }
}

fn trim_trailing_spaces(spans: &mut Vec<Span<'static>>, width: &mut usize) {
    while let Some(last) = spans.last_mut() {
        let trimmed = last.content.as_ref().trim_end().to_string();
        if trimmed.len() == last.content.len() {
            break;
        }
        *width = width.saturating_sub(text_width(&last.content) - text_width(&trimmed));
        if trimmed.is_empty() {
            spans.pop();
        } else {
            last.content = trimmed.into();
            break;
        }
    }
}

fn push_wrapped_line(
    wrapped: &mut Vec<ContentLine>,
    current: &mut Vec<Span<'static>>,
    current_links: &mut Vec<ContentLink>,
    current_width: &mut usize,
    style: Style,
    alignment: Option<Alignment>,
    kind: MarkdownLineKind,
) {
    wrapped.push(ContentLine {
        line: Line {
            spans: std::mem::take(current),
            style,
            alignment,
        },
        kind,
        links: std::mem::take(current_links),
    });
    *current_width = 0;
}

fn clip_line_to_width(line: Line<'static>, width: usize) -> Line<'static> {
    if width == 0 {
        return Line {
            spans: Vec::new(),
            style: line.style,
            alignment: line.alignment,
        };
    }

    let mut clipped = Vec::new();
    let mut used_width = 0usize;

    'spans: for span in line.spans {
        for ch in span.content.chars() {
            let ch_width = char_width(ch);
            if ch_width > width || used_width + ch_width > width {
                break 'spans;
            }
            push_char_span(&mut clipped, ch, span.style);
            used_width += ch_width;
        }
    }

    Line {
        spans: clipped,
        style: line.style,
        alignment: line.alignment,
    }
}

fn clip_links_to_width(links: &[ContentLink], width: usize) -> Vec<ContentLink> {
    links
        .iter()
        .filter_map(|link| {
            let end = link.end.min(width);
            (link.start < end).then(|| ContentLink {
                start: link.start,
                end,
                target: link.target.clone(),
            })
        })
        .collect()
}

fn push_char_span(spans: &mut Vec<Span<'static>>, ch: char, style: Style) {
    if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
        last.content.to_mut().push(ch);
    } else {
        spans.push(Span::styled(ch.to_string(), style));
    }
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// One width source: `unicode-width`'s string algorithm (identical to summing
/// `char_width` per char) instead of a second hand-rolled sum.
fn text_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Gutter lines: one dialogue dot per block, shown on the block's first
/// displayed line. Filled dots mark blocks selected for batch copy; the dot
/// color follows the block kind so a conversation reads like a chat list.
fn block_dot_lines(
    layout: &ContentLayout,
    marked: &[bool],
    scroll: usize,
    visible: &[ContentLine],
) -> Text<'static> {
    let ownership = &layout.ownership;
    let lines = (scroll..scroll.saturating_add(visible.len()))
        .map(|idx| {
            let owner = ownership.get(idx).copied().flatten();
            let starts = owner.is_some_and(|block| {
                idx == 0 || ownership.get(idx.saturating_sub(1)).copied().flatten() != Some(block)
            });
            match owner {
                Some(block) if starts => {
                    let marked = marked.get(block).copied().unwrap_or(false);
                    let glyph = crate::tui::pane::selection_dot(marked);
                    Line::from(Span::styled(
                        glyph,
                        Style::default()
                            .fg(block_dot_color(layout.kinds.get(idx).copied().flatten())),
                    ))
                }
                _ => Line::from("  "),
            }
        })
        .collect::<Vec<_>>();
    Text::from(lines)
}

/// Dot color by part kind: dialogue parts (user/assistant) carry the default
/// text color; structure parts (thinking/system/shell/tool/agent) share one
/// muted light color; failure is status, not decoration.
fn block_dot_color(role: Option<BlockRole>) -> Color {
    match role {
        Some(BlockRole::Failed) => crate::tui::theme::failure(),
        Some(
            BlockRole::Reasoning
            | BlockRole::System
            | BlockRole::Shell
            | BlockRole::Tool
            | BlockRole::Agent,
        ) => crate::tui::theme::muted(),
        Some(BlockRole::User | BlockRole::Assistant) | None => Color::Reset,
    }
}

/// Block under a click in the dot gutter (the column left of the content
/// text): any line of a block maps to that block, so the whole gutter column
/// is a click target for marking.
pub(crate) fn content_dot_at(
    area: Rect,
    layout: &ContentLayout,
    scroll: usize,
    column: u16,
    row: u16,
) -> Option<usize> {
    let inner = panel_inner(area);
    let content_area = content_text_area(area);
    if content_area.width == 0
        || content_area.height == 0
        || column < inner.x
        || column >= content_area.x
        || row < content_area.y
        || row >= content_area.y.saturating_add(content_area.height)
    {
        return None;
    }
    let line = scroll.saturating_add((row - content_area.y) as usize);
    content_block_at(layout, line)
}

fn styled_content_line(line: Line<'static>, search_regex: Option<&Regex>) -> Line<'static> {
    if search_regex.is_some() {
        return highlight_line(line, search_regex);
    }

    line
}

pub(crate) fn highlight_spans(
    text: &str,
    regex: Option<&Regex>,
    base_style: Style,
) -> Vec<Span<'static>> {
    let Some(regex) = regex else {
        return vec![Span::styled(text.to_string(), base_style)];
    };

    let mut spans = Vec::new();
    let mut cursor = 0;
    for matched in regex.find_iter(text) {
        if matched.start() == matched.end() {
            continue;
        }
        if matched.start() > cursor {
            spans.push(Span::styled(
                text[cursor..matched.start()].to_string(),
                base_style,
            ));
        }
        spans.push(Span::styled(
            text[matched.start()..matched.end()].to_string(),
            base_style
                .fg(crate::tui::theme::range_fg())
                .add_modifier(Modifier::BOLD),
        ));
        cursor = matched.end();
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }
    spans
}

fn highlight_line(line: Line<'static>, regex: Option<&Regex>) -> Line<'static> {
    let Some(regex) = regex else {
        return line;
    };
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if text.is_empty() {
        return line;
    }

    let matches = regex
        .find_iter(&text)
        .filter(|matched| matched.start() != matched.end())
        .map(|matched| matched.start()..matched.end())
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return line;
    }

    let mut offset = 0usize;
    let spans = line
        .spans
        .into_iter()
        .flat_map(|span| {
            let start = offset;
            let end = start + span.content.len();
            offset = end;
            split_span_by_matches(span, start, end, &matches)
        })
        .collect::<Vec<_>>();

    Line {
        spans,
        style: line.style,
        alignment: line.alignment,
    }
}

fn split_span_by_matches(
    span: Span<'static>,
    span_start: usize,
    span_end: usize,
    matches: &[std::ops::Range<usize>],
) -> Vec<Span<'static>> {
    if span_start == span_end {
        return vec![span];
    }

    let mut pieces = Vec::new();
    let text = span.content.to_string();
    let mut cursor = span_start;
    for matched in matches {
        if matched.end <= span_start || matched.start >= span_end {
            continue;
        }
        let start = matched.start.max(span_start);
        let end = matched.end.min(span_end);
        if start > cursor {
            pieces.push(Span::styled(
                text[cursor - span_start..start - span_start].to_string(),
                span.style,
            ));
        }
        pieces.push(Span::styled(
            text[start - span_start..end - span_start].to_string(),
            span.style
                .fg(crate::tui::theme::range_fg())
                .add_modifier(Modifier::BOLD),
        ));
        cursor = end;
    }
    if cursor < span_end {
        pieces.push(Span::styled(
            text[cursor - span_start..].to_string(),
            span.style,
        ));
    }

    if pieces.is_empty() {
        vec![span]
    } else {
        pieces
    }
}

pub(crate) fn line_count(text: &str) -> usize {
    raw_lines(text).len()
}

fn raw_lines(text: &str) -> Vec<&str> {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        vec![""]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        content_block_at, content_dot_at, content_lines, content_link_at, content_position_at,
        content_position_in_text_row, layout_content, line_count, render_content_view,
        selected_content_text, visible_content_lines, ContentPosition, ContentSelection,
        ContentSelectionKind, ContentView, ContentViewMode,
    };
    use crate::tui::content::block::{BlockRole, BlockText};
    use crate::tui::pane::Panel;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::prelude::Modifier;
    use ratatui::text::Text;
    use ratatui::Terminal;
    use regex::Regex;
    use unicode_width::UnicodeWidthStr;

    /// Displayed line index of the first layout line containing `needle`.
    fn displayed_line_of(lines: &[super::ContentLine], needle: &str) -> usize {
        lines
            .iter()
            .enumerate()
            .find_map(|(idx, line)| {
                let joined: String = line
                    .line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                joined.contains(needle).then_some(idx)
            })
            .expect("needle appears in layout")
    }

    /// Join test blocks the way the pane text joins them.
    fn blocks_text(blocks: &[BlockText]) -> String {
        blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn test_block(id: usize, text: &str) -> BlockText {
        BlockText {
            id,
            text: text.to_string(),
            tight: false,
            role: BlockRole::Tool,
        }
    }

    fn render_content_lines(
        text: &str,
        scroll: usize,
        height: usize,
        search_regex: Option<&Regex>,
        mode: ContentViewMode,
    ) -> Text<'static> {
        content_lines(
            visible_content_lines(text, scroll, height, 80, mode),
            scroll,
            search_regex,
            None,
            None,
            None,
            80,
        )
    }

    fn rendered_line_text(text: &Text<'static>, idx: usize) -> String {
        text.lines[idx]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn backend_row(backend: &TestBackend, y: u16) -> String {
        (0..backend.buffer().area.width)
            .filter_map(|x| backend.buffer().cell((x, y)).map(|cell| cell.symbol()))
            .collect::<String>()
    }

    #[test]
    fn counts_empty_content_as_one_display_line() {
        assert_eq!(line_count(""), 1);
    }

    #[test]
    fn content_lines_preserve_blank_lines_without_number_prefixes() {
        let rendered = render_content_lines("alpha\n\nomega", 0, 3, None, ContentViewMode::Reading);

        assert_eq!(rendered.lines.len(), 3);
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "alpha");
        assert!(rendered.lines[1].spans.is_empty());
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "omega");
    }

    #[test]
    fn content_lines_render_markdown_without_changing_line_count() {
        let rendered = render_content_lines(
            "## User\n**bold** and `code`",
            0,
            2,
            None,
            ContentViewMode::Reading,
        );

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "## ");
        assert_eq!(rendered.lines[0].spans[1].content.as_ref(), "User");
        // Heading color depends on the level alone, not the text: an h2
        // renders in the default foreground.
        assert_eq!(rendered.lines[0].spans[1].style.fg, None);
        assert!(rendered.lines[1].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(rendered.lines[1].spans[2].content.as_ref(), "code");
        assert_eq!(
            rendered.lines[1].spans[2].style.fg,
            Some(crate::tui::theme::code())
        );
        assert_eq!(rendered.lines[1].spans[2].style.bg, None);
        assert_eq!(line_count("## User\n**bold** and `code`"), 2);
    }

    #[test]
    fn content_search_highlight_overrides_markdown_spans() {
        let regex = Regex::new("bold").unwrap();
        let rendered = render_content_lines(
            "**bold** text",
            0,
            1,
            Some(&regex),
            ContentViewMode::Reading,
        );

        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "bold");
        assert_eq!(
            rendered.lines[0].spans[0].style.fg,
            Some(crate::tui::theme::range_fg())
        );
        assert!(rendered.lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn content_lines_wrap_wide_characters_before_terminal_clipping() {
        let rendered = content_lines(
            visible_content_lines("甲乙丙", 0, 3, 4, ContentViewMode::Reading),
            0,
            None,
            None,
            None,
            None,
            4,
        );

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered_line_text(&rendered, 0), "甲乙");
        assert_eq!(rendered_line_text(&rendered, 1), "丙");
        assert!(UnicodeWidthStr::width(rendered_line_text(&rendered, 0).as_str()) <= 4);
        assert!(UnicodeWidthStr::width(rendered_line_text(&rendered, 1).as_str()) <= 4);
    }

    #[test]
    fn markdown_link_targets_stay_atomic_when_wrapping() {
        let visible = visible_content_lines(
            "[docker-compose.funasr.yml](V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15)",
            0,
            4,
            16,
            ContentViewMode::Reading,
        );
        let rendered = content_lines(visible.clone(), 0, None, None, None, None, 16);

        assert_eq!(rendered.lines.len(), 2);
        assert_eq!(rendered_line_text(&rendered, 0), "docker-compose.f");
        assert_eq!(rendered_line_text(&rendered, 1), "unasr.yml");
        assert_eq!(
            visible[0].links[0].target,
            "V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15"
        );
        assert_eq!(
            visible[1].links[0].target,
            "V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15"
        );
    }

    #[test]
    fn content_link_hit_test_returns_full_target_after_wrapping() {
        let target = content_link_at(
            Rect::new(0, 0, 24, 5),
            "[docker-compose.funasr.yml](V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15)",
            0,
            ContentViewMode::Reading,
            4,
            1,
        );

        assert_eq!(
            target.as_deref(),
            Some("V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15")
        );
    }

    #[test]
    fn content_position_at_rejects_dot_gutter() {
        let position = content_position_at(
            Rect::new(0, 0, 24, 5),
            "alpha\nbeta",
            0,
            ContentViewMode::Reading,
            1,
            1,
        );

        assert_eq!(position, None);
    }

    #[test]
    fn content_position_in_text_row_maps_gutter_to_text_start() {
        let position = content_position_in_text_row(
            Rect::new(0, 0, 24, 5),
            "alpha\nbeta",
            0,
            ContentViewMode::Reading,
            1,
            1,
        );

        assert_eq!(position, Some(ContentPosition { line: 0, column: 0 }));
    }

    #[test]
    fn content_view_line_count_uses_wrapped_reading_lines() {
        assert_eq!(
            super::content_view_line_count(
                Rect::new(0, 0, 24, 5),
                "[docker-compose.funasr.yml](V:/Coding/Meeting-Assistant-/docker-compose.funasr.yml:15)",
                ContentViewMode::Reading,
            ),
            2
        );
    }

    #[test]
    fn fixed_gutter_reserves_two_columns() {
        // inner_width == 3 leaves a single content column; the fixed two-column
        // gutter (dot + space) means a ten-character unbroken line wraps to
        // ten rows instead of oscillating against the digit width.
        let lines = super::content_layout_lines_metrics(3, "0123456789", ContentViewMode::Reading);
        assert_eq!(lines.len(), 10, "one character per wrapped row");
        assert!(!lines[0].line.spans.is_empty(), "content must not vanish");
    }

    #[test]
    fn fixed_shape_lines_clip_without_cutting_wide_characters() {
        let rendered = content_lines(
            visible_content_lines(
                "| 编号 | 项目 |\n| --- | --- |\n| 001 | 用户调研 |",
                0,
                5,
                10,
                ContentViewMode::Reading,
            ),
            0,
            None,
            None,
            None,
            None,
            10,
        );

        assert!(!rendered.lines.is_empty());
        for idx in 0..rendered.lines.len() {
            let text = rendered_line_text(&rendered, idx);
            assert!(
                UnicodeWidthStr::width(text.as_str()) <= 10,
                "line exceeds render width: {text:?}"
            );
        }
    }

    #[test]
    fn raw_mode_keeps_literal_layout_but_styles_markers_gray() {
        // Raw/full renders tables, code fences, and inline markup literally —
        // no markdown layout — while `<:…:>` structure markers share the same
        // structural gray as read-mode fold summaries.
        let rendered = render_content_lines(
            "plain body\n<:tool:bash call:>\n```\n**bold**\n```\n| a | b |",
            0,
            6,
            None,
            ContentViewMode::Raw,
        );

        assert_eq!(rendered.lines[0].spans[0].content.as_ref(), "plain body");
        assert_eq!(rendered.lines[0].spans[0].style.fg, None);
        assert_eq!(
            rendered.lines[1].spans[0].content.as_ref(),
            "<:tool:bash call:>"
        );
        assert_eq!(
            rendered.lines[1].spans[0].style.fg,
            Some(crate::tui::theme::muted())
        );
        // Literal: fences, emphasis, and table pipes stay as-is.
        assert_eq!(rendered.lines[2].spans[0].content.as_ref(), "```");
        assert_eq!(rendered.lines[3].spans[0].content.as_ref(), "**bold**");
        assert_eq!(rendered.lines[4].spans[0].content.as_ref(), "```");
        assert_eq!(rendered.lines[5].spans[0].content.as_ref(), "| a | b |");
        assert_eq!(rendered.lines[5].spans[0].style.fg, None);
    }

    #[test]
    fn selected_content_text_uses_rendered_content_coordinates() {
        let selected = selected_content_text(
            Rect::new(0, 0, 40, 6),
            "alpha\nbeta\nomega",
            ContentViewMode::Reading,
            ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 1, column: 2 },
                kind: ContentSelectionKind::Linear,
            },
        );

        assert_eq!(selected, "lpha\nbet");
    }

    #[test]
    fn selected_content_text_never_includes_dot_gutter() {
        let selected = selected_content_text(
            Rect::new(0, 0, 24, 5),
            "alpha\nbeta",
            ContentViewMode::Reading,
            ContentSelection {
                anchor: ContentPosition { line: 0, column: 0 },
                cursor: ContentPosition { line: 1, column: 1 },
                kind: ContentSelectionKind::Linear,
            },
        );

        assert_eq!(selected, "alpha\nbe");
    }

    #[test]
    fn selected_content_text_supports_block_selection() {
        let selected = selected_content_text(
            Rect::new(0, 0, 32, 6),
            "abcdef\n12\nuvwxyz",
            ContentViewMode::Reading,
            ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 2, column: 3 },
                kind: ContentSelectionKind::Block,
            },
        );

        assert_eq!(selected, "bcd\n2\nvwx");
    }

    #[test]
    fn content_lines_highlight_block_selection_per_row() {
        let rendered = content_lines(
            visible_content_lines("abcdef\nuvwxyz", 0, 2, 80, ContentViewMode::Reading),
            0,
            None,
            Some(ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 1, column: 3 },
                kind: ContentSelectionKind::Block,
            }),
            None,
            None,
            80,
        );

        assert_eq!(rendered.lines[0].spans[1].content.as_ref(), "bcd");
        assert_eq!(rendered.lines[1].spans[1].content.as_ref(), "vwx");
        assert_eq!(
            rendered.lines[0].spans[1].style.bg,
            crate::tui::theme::text_selection_row().bg
        );
        assert_eq!(
            rendered.lines[1].spans[1].style.bg,
            crate::tui::theme::text_selection_row().bg
        );
    }

    #[test]
    fn content_lines_highlight_visual_selection() {
        let rendered = content_lines(
            visible_content_lines("alpha", 0, 1, 80, ContentViewMode::Reading),
            0,
            None,
            Some(ContentSelection {
                anchor: ContentPosition { line: 0, column: 1 },
                cursor: ContentPosition { line: 0, column: 3 },
                kind: ContentSelectionKind::Linear,
            }),
            None,
            None,
            80,
        );

        assert_eq!(rendered_line_text(&rendered, 0), "alpha");
        assert_eq!(rendered.lines[0].spans[1].content.as_ref(), "lph");
        assert_eq!(
            rendered.lines[0].spans[1].style.bg,
            crate::tui::theme::text_selection_row().bg
        );
    }

    #[test]
    fn render_content_view_shows_block_dots_in_the_gutter() {
        let backend = TestBackend::new(24, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 24, 6);
        let blocks = vec![test_block(0, "alpha\nbeta"), test_block(1, "omega")];
        let text = blocks_text(&blocks);
        let layout = layout_content(area, &text, &blocks, ContentViewMode::Reading);
        // Marked mask indexed by block id: block 1 is marked.
        let marked = [false, true];

        terminal
            .draw(|frame| {
                render_content_view(
                    frame,
                    area,
                    Panel::new("3", "Content (read)", true),
                    ContentView {
                        layout: &layout,
                        scroll: 0,
                        search_regex: None,
                        selection: None,
                        cursor_block: None,
                        range_blocks: None,
                        marked: &marked,
                    },
                );
            })
            .unwrap();

        let backend = terminal.backend();
        // Block 0 starts on row 1 (unmarked ○), block 1 on row 4 (marked ●);
        // the block's other lines keep a blank gutter.
        assert!(backend_row(backend, 1).contains("○ alpha"));
        assert!(backend_row(backend, 2).contains("  beta"));
        assert!(backend_row(backend, 4).contains("● omega"));
    }

    #[test]
    fn content_dot_at_hits_blocks_in_the_gutter_column() {
        let area = Rect::new(0, 0, 24, 8);
        let blocks = vec![test_block(0, "alpha\nbeta"), test_block(1, "omega")];
        let text = blocks_text(&blocks);
        let layout = layout_content(area, &text, &blocks, ContentViewMode::Reading);
        // Columns 1..3 are the dot gutter (column 0 is the panel border):
        // any line of a block maps to it.
        assert_eq!(content_dot_at(area, &layout, 0, 1, 1), Some(0));
        assert_eq!(content_dot_at(area, &layout, 0, 2, 2), Some(0));
        assert_eq!(content_dot_at(area, &layout, 0, 1, 4), Some(1));
        // The content column is not the dot gutter.
        assert_eq!(content_dot_at(area, &layout, 0, 3, 1), None);
        // The panel border is not the dot gutter.
        assert_eq!(content_dot_at(area, &layout, 0, 0, 1), None);
    }

    #[test]
    fn render_content_view_highlights_the_cursor_block() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal from test backend");
        let area = Rect::new(0, 0, 40, 8);
        let blocks = vec![
            test_block(0, "<:tool:Bash call:>\nbody\n<:/tool:Bash call:>"),
            test_block(1, "<:tool:Read call:>"),
        ];
        let layout = layout_content(
            area,
            &blocks_text(&blocks),
            &blocks,
            ContentViewMode::Reading,
        );
        terminal
            .draw(|frame| {
                render_content_view(
                    frame,
                    area,
                    Panel::new("3", "Content (read)", true),
                    ContentView {
                        layout: &layout,
                        scroll: 0,
                        search_regex: None,
                        selection: None,
                        cursor_block: Some(0),
                        range_blocks: None,
                        marked: &[],
                    },
                );
            })
            .expect("draw content frame");
        let buffer = terminal.backend().buffer();
        // Block 0 spans the tag, body, and close rows; its lines use the same
        // focus-row style as the session/dialogue list cursor.
        let focus_bg = crate::tui::theme::focus_row().bg;
        for row in 1..=3 {
            let cell = buffer.cell((4, row)).expect("cell within buffer bounds");
            assert_eq!(cell.style().bg, focus_bg, "row {row} should be highlighted");
        }
        // The separator and the next block's tag are not part of the highlight.
        for row in 4..=5 {
            let cell = buffer.cell((4, row)).expect("cell within buffer bounds");
            assert_ne!(cell.style().bg, focus_bg);
        }
        // The highlight fills the whole row width, not just the text columns.
        for row in 1..=3 {
            let cell = buffer.cell((38, row)).unwrap();
            assert_eq!(
                cell.style().bg,
                focus_bg,
                "row {row} should be highlighted to the right edge"
            );
        }
    }

    #[test]
    fn render_content_view_highlights_the_range_span() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 8);
        let blocks = vec![
            test_block(0, "alpha\nbeta"),
            test_block(1, "omega"),
            test_block(2, "gamma"),
        ];
        let layout = layout_content(
            area,
            &blocks_text(&blocks),
            &blocks,
            ContentViewMode::Reading,
        );
        terminal
            .draw(|frame| {
                render_content_view(
                    frame,
                    area,
                    Panel::new("3", "Content (read)", true),
                    ContentView {
                        layout: &layout,
                        scroll: 0,
                        search_regex: None,
                        selection: None,
                        // The cursor block is inside the span, so the amber
                        // range style overrides the focus style — the same
                        // in-range priority the list panes use.
                        cursor_block: Some(0),
                        range_blocks: Some((0, 1)),
                        marked: &[],
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // The span owns block 0 (rows 1-2) and block 1 (row 4), split by the
        // block separator.
        let range_fg = crate::tui::theme::range_row().fg;
        for row in [1, 2, 4] {
            let cell = buffer.cell((4, row)).unwrap();
            assert_eq!(cell.style().fg, range_fg, "row {row} should be in range");
        }
        // The span wins over the cursor block: no focus background on block 0.
        let focus_bg = crate::tui::theme::focus_row().bg;
        for row in 1..=2 {
            let cell = buffer.cell((4, row)).unwrap();
            assert_ne!(cell.style().bg, focus_bg);
        }
        // Block 2 (row 6) is outside the span.
        let cell = buffer.cell((4, 6)).unwrap();
        assert_ne!(cell.style().fg, range_fg);
    }

    #[test]
    fn content_layout_single_pass_matches_metrics_and_fits_width() {
        let text = "line one\nline two\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n";
        let area = Rect::new(0, 0, 60, 20);

        let (lines, chunks) = super::content_layout(area, text, ContentViewMode::Reading);
        assert_eq!(chunks.len(), 2);
        let inner_width = area.width.saturating_sub(2) as usize; // panel borders
        let column_widths = chunks.iter().map(|c| c.width as usize).sum::<usize>();
        assert_eq!(column_widths, inner_width);

        // Metrics (used by ContentIoFrame) and the layout used for painting
        // agree on the total line count — one markdown + wrap pass.
        let metrics = super::content_view_line_count(area, text, ContentViewMode::Reading);
        assert_eq!(metrics, lines.len().max(1));
        assert!(!lines.is_empty());

        // Every wrapped line fits the content column.
        for line in &lines {
            let joined: String = line
                .line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            let width = UnicodeWidthStr::width(joined.as_str());
            assert!(
                width <= chunks[1].width as usize,
                "line {width} exceeds content width {}",
                chunks[1].width
            );
        }
    }

    #[test]
    fn content_block_at_maps_displayed_lines_with_wrapping() {
        let area = Rect::new(0, 0, 24, 10);
        let long = "alpha ".repeat(20);
        let blocks = vec![
            test_block(0, &long),
            test_block(1, "<:tool:A call:>"),
            test_block(2, "<:tool:B call:>"),
        ];
        let text = blocks_text(&blocks);
        let layout = layout_content(area, &text, &blocks, ContentViewMode::Reading);
        let lines = &layout.lines;
        let tag_lines: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                let joined: String = line
                    .line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                joined.starts_with("<:tool:").then_some(idx)
            })
            .collect();
        assert_eq!(tag_lines.len(), 2);
        // The long body wraps, so the tags' displayed lines are no longer
        // their raw text-line indices; the block lookup must follow the
        // display, not `text.lines()`.
        assert_ne!(tag_lines[0], 1, "wrapped body shifts the tag line index");
        assert_eq!(content_block_at(&layout, tag_lines[0]), Some(1));
        assert_eq!(content_block_at(&layout, tag_lines[1]), Some(2));
        // A wrapped body line belongs to its own block (every part is one).
        assert_eq!(content_block_at(&layout, 0), Some(0));
    }

    #[test]
    fn content_block_at_owns_every_line_of_a_block() {
        let area = Rect::new(0, 0, 40, 20);
        let blocks = vec![
            test_block(0, "<:tool:Bash call:>\noutput line\n<:/tool:Bash call:>"),
            test_block(1, "<:tool:Read call:>\nplain"),
        ];
        let text = blocks_text(&blocks);
        let layout = layout_content(area, &text, &blocks, ContentViewMode::Reading);
        let lines = &layout.lines;
        // The tag, the expanded body, and the close marker all own block 0.
        for needle in ["<:tool:Bash call:>", "output line", "<:/tool:Bash call:>"] {
            assert_eq!(
                content_block_at(&layout, displayed_line_of(lines, needle)),
                Some(0),
                "{needle} should belong to block 0"
            );
        }
        // The next block owns its tag and its plain body.
        for needle in ["<:tool:Read call:>", "plain"] {
            assert_eq!(
                content_block_at(&layout, displayed_line_of(lines, needle)),
                Some(1),
                "{needle} should belong to block 1"
            );
        }
    }

    #[test]
    fn content_block_at_keeps_merged_tool_group_as_one_block() {
        let area = Rect::new(0, 0, 40, 20);
        // A merged tool group: call section + result section share one block.
        let blocks = vec![
            test_block(
                0,
                "<:tool:Bash call:>\ninput\n<:/tool:Bash call:>\n<:tool:Bash result:>\noutput\n<:/tool:Bash result:>",
            ),
            test_block(1, "<:tool:Read call:>"),
        ];
        let text = blocks_text(&blocks);
        let layout = layout_content(area, &text, &blocks, ContentViewMode::Reading);
        let lines = &layout.lines;
        // Everything from the call tag through the result close is block 0.
        for needle in [
            "<:tool:Bash call:>",
            "input",
            "<:/tool:Bash call:>",
            "<:tool:Bash result:>",
            "output",
            "<:/tool:Bash result:>",
        ] {
            let line = displayed_line_of(lines, needle);
            assert_eq!(
                content_block_at(&layout, line),
                Some(0),
                "line {line} ({needle}) should belong to the merged tool group"
            );
        }
        assert_eq!(
            content_block_at(&layout, displayed_line_of(lines, "<:tool:Read call:>")),
            Some(1)
        );
    }
}
