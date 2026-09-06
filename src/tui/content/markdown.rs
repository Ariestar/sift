use pulldown_cmark::{Event, Options as MarkdownOptions, Parser, Tag, TagEnd};
use ratatui::prelude::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkdownLineKind {
    Normal,
    CodeFence,
    CodeBlock,
    Table,
}

#[derive(Clone, Debug)]
pub(crate) struct MarkdownLine {
    pub(crate) line: Line<'static>,
    pub(crate) kind: MarkdownLineKind,
    pub(crate) links: Vec<MarkdownLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownLink {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) target: String,
}

#[derive(Clone)]
struct TableRenderRow {
    cells: Vec<String>,
    widths: Vec<usize>,
    kind: TableRenderRowKind,
}

#[derive(Clone, Copy)]
enum TableRenderRowKind {
    Top,
    Header,
    Separator,
    Body,
    Bottom,
}

struct MarkdownLineParts<'a> {
    prefix: String,
    prefix_style: Style,
    content: &'a str,
    content_style: Style,
}

pub(crate) fn render_markdown_lines(lines: &[&str], width: usize) -> Vec<MarkdownLine> {
    let table_rows = aligned_table_rows(lines);
    let mut rendered = Vec::with_capacity(lines.len());
    let mut idx = 0;

    while idx < lines.len() {
        if let Some(table_rows) = &table_rows[idx] {
            rendered.extend(table_rows.iter().map(render_table_row));
            idx += 1;
            continue;
        }
        let line = lines[idx];
        // Fenced code blocks are gathered as one unit so diff-aware coloring
        // can see the whole block before deciding how to paint its lines.
        if let Some(language) = code_fence_language(line) {
            rendered.push(MarkdownLine {
                line: code_fence_line(language.clone()),
                kind: MarkdownLineKind::CodeFence,
                links: Vec::new(),
            });
            let mut body = Vec::new();
            idx += 1;
            while idx < lines.len() && code_fence_language(lines[idx]).is_none() {
                body.push(lines[idx]);
                idx += 1;
            }
            rendered.extend(render_code_block(&body, language.as_deref()));
            if idx < lines.len() {
                rendered.push(MarkdownLine {
                    line: code_fence_line(None),
                    kind: MarkdownLineKind::CodeFence,
                    links: Vec::new(),
                });
                idx += 1;
            }
            continue;
        }
        rendered.push(render_markdown_line(line, width));
        idx += 1;
    }

    rendered
}

/// One fenced block: ` ```rust ` … ` ``` `. GitHub-style: every code line
/// carries a right-aligned line number sharing the block background, so the
/// same gutter appears in markdown fences, tool result previews, and diffs.
/// Diff-aware when the fence names `diff` or the body carries a unified-diff
/// hunk header (`@@ …`). A numeric fence (```` ```775 ````) numbers the block
/// from that file line — read-result previews keep their real line numbers.
fn render_code_block(body: &[&str], language: Option<&str>) -> Vec<MarkdownLine> {
    if language == Some("grep") {
        return render_grep_block(body);
    }
    let is_diff = language == Some("diff") || body.iter().any(|line| is_diff_hunk_header(line));
    let start_line = language
        .and_then(|lang| lang.parse::<usize>().ok())
        .unwrap_or(1);
    // GitHub-style gutter: right-aligned line numbers, the code follows;
    // widths grow with the block's last line number. No background — the
    // current-row highlight owns the background on the content pane.
    let gutter_width = (start_line.saturating_add(body.len()).saturating_sub(1))
        .to_string()
        .len();
    body.iter()
        .enumerate()
        .map(|(idx, line)| MarkdownLine {
            line: Line::from(vec![
                Span::styled(
                    format!("{:>gutter_width$} ", start_line.saturating_add(idx)),
                    code_line_number_style(),
                ),
                Span::styled(line.to_string(), code_line_style(line, is_diff)),
            ]),
            kind: MarkdownLineKind::CodeBlock,
            links: Vec::new(),
        })
        .collect()
}

/// Unified-diff hunk header: `@@ -1,3 +1,4 @@` (an optional section heading
/// may follow the closing `@@`). Strict on shape and column: the header
/// starts at column 0, the old range is minus-led and the new range
/// plus-led, and each range carries at most one line-count segment — so
/// context lines and lookalike code never switch the block into diff mode.
fn is_diff_hunk_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("@@") else {
        return false;
    };
    let Some((ranges, _heading)) = rest.trim_start().split_once("@@") else {
        return false;
    };
    let Some((minus, plus)) = ranges.trim().split_once(' ') else {
        return false;
    };
    minus.strip_prefix('-').is_some_and(hunk_count)
        && plus.strip_prefix('+').is_some_and(hunk_count)
}

/// Digits, optionally followed by one `,digits` line count.
fn hunk_count(digits: &str) -> bool {
    match digits.split_once(',') {
        Some((start, count)) => is_digits(start) && is_digits(count),
        None => is_digits(digits),
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// Unified-diff file header (`+++ b/file`, `--- a/file`): the marker is
/// followed by a space and a path, so a change line whose content itself
/// starts with `++`/`--` stays a change line.
fn is_diff_file_header(line: &str) -> bool {
    line.strip_prefix("+++")
        .or_else(|| line.strip_prefix("---"))
        .and_then(|rest| rest.strip_prefix(' '))
        .is_some_and(|path| !path.is_empty())
}

/// grep search results (` ```grep `): a structured block, not a code gutter.
/// Matches are scattered single lines, so each part gets its own color — the
/// `Found N matching lines` summary stands out, file paths read as targets,
/// and the `line:content` matches highlight their line number. Provider
/// envelopes (grok's `<workspace_result …>`) are already stripped by the
/// formatter, so only the generic shape is rendered here.
fn render_grep_block(body: &[&str]) -> Vec<MarkdownLine> {
    body.iter()
        .map(|line| {
            let trimmed = line.trim_start();
            let spans = if trimmed.starts_with("Found ") && trimmed.contains("matching lines") {
                vec![Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(crate::tui::theme::accent())
                        .add_modifier(Modifier::BOLD),
                )]
            } else if let Some((num, rest)) = line.split_once(':') {
                if num.trim().chars().all(|ch| ch.is_ascii_digit()) {
                    vec![
                        Span::styled(
                            format!("{num}:"),
                            Style::default()
                                .fg(crate::tui::theme::accent())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(rest.to_string(), code_block_style()),
                    ]
                } else {
                    vec![Span::styled(
                        line.to_string(),
                        Style::default().fg(crate::tui::theme::accent()),
                    )]
                }
            } else {
                vec![Span::styled(
                    line.to_string(),
                    Style::default().fg(crate::tui::theme::accent()),
                )]
            };
            MarkdownLine {
                line: Line::from(spans),
                kind: MarkdownLineKind::CodeBlock,
                links: Vec::new(),
            }
        })
        .collect()
}

fn code_line_style(line: &str, is_diff: bool) -> Style {
    if !is_diff {
        return code_block_style();
    }
    if is_diff_hunk_header(line) {
        Style::default()
            .fg(crate::tui::theme::accent())
            .add_modifier(Modifier::BOLD)
    } else if is_diff_file_header(line) {
        Style::default()
            .fg(crate::tui::theme::muted())
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        Style::default().fg(crate::tui::theme::success())
    } else if line.starts_with('-') {
        Style::default().fg(crate::tui::theme::failure())
    } else {
        // Context lines (leading marker space) and anything else keep the
        // plain code style; the context space stays in the rendered text.
        code_block_style()
    }
}

#[cfg(test)]
fn render_markdown_window(
    lines: &[&str],
    scroll: usize,
    height: usize,
    width: usize,
) -> Vec<MarkdownLine> {
    render_markdown_lines(lines, width)
        .into_iter()
        .skip(scroll)
        .take(height)
        .collect()
}

fn render_markdown_line(line: &str, width: usize) -> MarkdownLine {
    if is_horizontal_rule(line) {
        return MarkdownLine {
            line: Line::from(Span::styled(
                "─".repeat(width.max(3)),
                structure_prefix_style(),
            )),
            kind: MarkdownLineKind::Normal,
            links: Vec::new(),
        };
    }

    let parts = markdown_line_parts(line);
    let mut spans = Vec::new();
    let mut links = Vec::new();
    let mut prefix_width = 0;
    if !parts.prefix.is_empty() {
        prefix_width = UnicodeWidthStr::width(parts.prefix.as_str());
        spans.push(Span::styled(parts.prefix, parts.prefix_style));
    }
    let inline = markdown_inline(parts.content, parts.content_style);
    links.extend(inline.links.into_iter().map(|link| MarkdownLink {
        start: link.start + prefix_width,
        end: link.end + prefix_width,
        target: link.target,
    }));
    spans.extend(inline.spans);
    MarkdownLine {
        line: Line {
            spans,
            style: parts.content_style,
            alignment: None,
        },
        kind: MarkdownLineKind::Normal,
        links,
    }
}

fn code_fence_language(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("```")
        .or_else(|| trimmed.strip_prefix("~~~"))?;
    let language = rest.split_whitespace().next().filter(|lang| {
        !lang.is_empty() && !matches!(*lang, "text" | "txt" | "plain" | "plaintext")
    });
    Some(language.map(str::to_string))
}

fn code_fence_line(language: Option<String>) -> Line<'static> {
    // A numeric fence is a line-number offset, not a language label — the
    // fence row stays empty so the gutter reads as the file's own numbers.
    match language.filter(|lang| lang.parse::<usize>().is_err()) {
        Some(language) => Line::from(Span::styled(language, code_fence_style())),
        None => Line::default(),
    }
}

fn markdown_line_parts(line: &str) -> MarkdownLineParts<'_> {
    let leading_width = line.len() - line.trim_start().len();
    let leading = &line[..leading_width];
    let trimmed = &line[leading_width..];

    if let Some((level, rest)) = markdown_heading(trimmed) {
        let style = heading_style(level);
        return MarkdownLineParts {
            prefix: format!("{leading}{} ", "#".repeat(level)),
            prefix_style: structure_prefix_style(),
            content: rest,
            content_style: style,
        };
    }

    // Content-block structure markers (`<:tool:…:>`, `<:skill:…:>`, …).
    if let Some(style) = structure_marker_style(trimmed) {
        return MarkdownLineParts {
            prefix: leading.to_string(),
            prefix_style: Style::default(),
            content: trimmed,
            content_style: style,
        };
    }

    // Tool input line (`$ read src/main.rs:12-30`): the `$` marker is the
    // highlighted prompt; the command itself reads in the default color.
    if let Some(rest) = trimmed.strip_prefix("$ ") {
        return MarkdownLineParts {
            prefix: format!("{leading}$ "),
            prefix_style: tool_input_marker_style(),
            content: rest,
            content_style: Style::default(),
        };
    }

    if let Some(rest) = trimmed.strip_prefix("> ") {
        return MarkdownLineParts {
            prefix: format!("{leading}> "),
            prefix_style: structure_prefix_style(),
            content: rest,
            content_style: blockquote_style(),
        };
    }

    if let Some((marker, rest, marker_style)) = markdown_task_item(trimmed) {
        return MarkdownLineParts {
            prefix: format!("{leading}{marker}"),
            prefix_style: marker_style,
            content: rest,
            content_style: Style::default(),
        };
    }

    if let Some((marker, rest)) = markdown_list_item(trimmed) {
        return MarkdownLineParts {
            prefix: format!("{leading}{marker}"),
            prefix_style: structure_prefix_style(),
            content: rest,
            content_style: Style::default(),
        };
    }

    MarkdownLineParts {
        prefix: String::new(),
        prefix_style: Style::default(),
        content: line,
        content_style: Style::default(),
    }
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
}

fn markdown_task_item(line: &str) -> Option<(String, &str, Style)> {
    let (_marker, rest) = markdown_list_item(line)?;
    let (symbol, rest, style) = if let Some(rest) = rest.strip_prefix("[ ] ") {
        ("□ ", rest, structure_prefix_style())
    } else {
        let rest = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))?;
        ("■ ", rest, task_done_marker_style())
    };

    Some((symbol.to_string(), rest, style))
}

fn markdown_list_item(line: &str) -> Option<(String, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some((marker.to_string(), rest));
        }
    }

    let dot = line.find(". ")?;
    if dot == 0 || !line[..dot].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((line[..dot + 2].to_string(), &line[dot + 2..]))
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let mut chars = trimmed.chars();
    let Some(marker @ ('-' | '*' | '_')) = chars.next() else {
        return false;
    };
    let mut count = 1;
    for ch in chars {
        if ch != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn aligned_table_rows(lines: &[&str]) -> Vec<Option<Vec<TableRenderRow>>> {
    let mut rows = vec![None; lines.len()];
    let mut in_code_block = false;
    let mut idx = 0;

    while idx < lines.len() {
        if code_fence_language(lines[idx]).is_some() {
            in_code_block = !in_code_block;
            idx += 1;
            continue;
        }
        if in_code_block {
            idx += 1;
            continue;
        }

        let Some(header) = parse_table_row(lines[idx]) else {
            idx += 1;
            continue;
        };
        let Some(separator_line) = lines.get(idx + 1) else {
            idx += 1;
            continue;
        };
        let Some(separator) = parse_table_row(separator_line).filter(|row| row.separator) else {
            idx += 1;
            continue;
        };
        if separator.cells.len() != header.cells.len() {
            idx += 1;
            continue;
        }

        let mut parsed_rows = vec![header, separator];
        let mut end = idx + 2;
        while let Some(row) = lines.get(end).and_then(|line| parse_table_row(line)) {
            if row.separator || row.cells.len() != parsed_rows[0].cells.len() {
                break;
            }
            parsed_rows.push(row);
            end += 1;
        }

        let widths = table_column_widths(&parsed_rows);
        let header = parsed_rows[0].cells.clone();
        rows[idx] = Some(vec![
            TableRenderRow {
                cells: Vec::new(),
                widths: widths.clone(),
                kind: TableRenderRowKind::Top,
            },
            TableRenderRow {
                cells: header,
                widths: widths.clone(),
                kind: TableRenderRowKind::Header,
            },
        ]);
        rows[idx + 1] = Some(vec![TableRenderRow {
            cells: Vec::new(),
            widths: widths.clone(),
            kind: TableRenderRowKind::Separator,
        }]);
        let body_count = parsed_rows.len().saturating_sub(2);
        for (body_idx, row) in parsed_rows.into_iter().skip(2).enumerate() {
            let is_last_body = body_idx + 1 == body_count;
            let mut render_rows = vec![TableRenderRow {
                cells: row.cells,
                widths: widths.clone(),
                kind: TableRenderRowKind::Body,
            }];
            if is_last_body {
                render_rows.push(TableRenderRow {
                    cells: Vec::new(),
                    widths: widths.clone(),
                    kind: TableRenderRowKind::Bottom,
                });
            }
            rows[idx + 2 + body_idx] = Some(render_rows);
        }
        idx = end;
    }

    rows
}

#[derive(Clone)]
struct ParsedTableRow {
    cells: Vec<String>,
    separator: bool,
}

fn parse_table_row(line: &str) -> Option<ParsedTableRow> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }

    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let cells = inner
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect::<Vec<_>>();
    if cells.len() < 2 {
        return None;
    }

    let separator = cells.iter().all(|cell| is_table_separator_cell(cell));
    Some(ParsedTableRow { cells, separator })
}

fn is_table_separator_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    let body = trimmed
        .strip_prefix(':')
        .unwrap_or(trimmed)
        .strip_suffix(':')
        .unwrap_or(trimmed);
    body.len() >= 3 && body.chars().all(|ch| ch == '-')
}

fn table_column_widths(rows: &[ParsedTableRow]) -> Vec<usize> {
    let column_count = rows[0].cells.len();
    let mut widths = vec![3; column_count];
    for row in rows.iter().filter(|row| !row.separator) {
        for (idx, cell) in row.cells.iter().enumerate() {
            widths[idx] = widths[idx].max(inline_render_width(cell));
        }
    }
    widths
}

fn render_table_row(row: &TableRenderRow) -> MarkdownLine {
    if let Some(line) = render_table_border(row) {
        return line;
    }

    let mut spans = Vec::new();
    spans.push(Span::styled("│ ", structure_prefix_style()));

    for (idx, width) in row.widths.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" │ ", structure_prefix_style()));
        }

        let cell = row.cells.get(idx).map(String::as_str).unwrap_or_default();
        let cell_width = inline_render_width(cell);
        spans.extend(markdown_inline_spans(cell, Style::default()));
        if *width > cell_width {
            spans.push(Span::raw(" ".repeat(*width - cell_width)));
        }
    }

    spans.push(Span::styled(" │", structure_prefix_style()));
    MarkdownLine {
        line: Line::from(spans),
        kind: MarkdownLineKind::Table,
        links: Vec::new(),
    }
}

fn render_table_border(row: &TableRenderRow) -> Option<MarkdownLine> {
    let (left, join, right) = match row.kind {
        TableRenderRowKind::Top => ("┌", "┬", "┐"),
        TableRenderRowKind::Separator => ("├", "┼", "┤"),
        TableRenderRowKind::Bottom => ("└", "┴", "┘"),
        TableRenderRowKind::Header | TableRenderRowKind::Body => return None,
    };

    let mut line = String::new();
    line.push_str(left);
    for (idx, width) in row.widths.iter().enumerate() {
        if idx > 0 {
            line.push_str(join);
        }
        line.push_str(&"─".repeat(width.saturating_add(2)));
    }
    line.push_str(right);

    Some(MarkdownLine {
        line: Line::from(Span::styled(line, structure_prefix_style())),
        kind: MarkdownLineKind::Table,
        links: Vec::new(),
    })
}

fn inline_render_width(text: &str) -> usize {
    markdown_inline(text, Style::default())
        .spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn markdown_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    markdown_inline(text, base_style).spans
}

struct MarkdownInline {
    spans: Vec<Span<'static>>,
    links: Vec<MarkdownLink>,
}

fn markdown_inline(text: &str, base_style: Style) -> MarkdownInline {
    if text.is_empty() {
        return MarkdownInline {
            spans: Vec::new(),
            links: Vec::new(),
        };
    }

    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(text, options);
    let mut spans = Vec::new();
    let mut links = Vec::new();
    let mut styles = vec![base_style];
    let mut pending_link = None;
    let mut pending_link_start = 0usize;
    let mut cursor = 0usize;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Emphasis => {
                    push_style(&mut styles, Style::default().add_modifier(Modifier::ITALIC))
                }
                Tag::Strong => {
                    push_style(&mut styles, Style::default().add_modifier(Modifier::BOLD));
                }
                Tag::Strikethrough => {
                    push_style(
                        &mut styles,
                        Style::default().add_modifier(Modifier::CROSSED_OUT),
                    );
                }
                Tag::Link { dest_url, .. } => {
                    pending_link = Some(dest_url.to_string());
                    pending_link_start = cursor;
                    push_style(&mut styles, link_style());
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    pop_style(&mut styles);
                }
                TagEnd::Link => {
                    pop_style(&mut styles);
                    if let Some(link) = pending_link.take() {
                        if cursor > pending_link_start {
                            links.push(MarkdownLink {
                                start: pending_link_start,
                                end: cursor,
                                target: link,
                            });
                        }
                    }
                }
                _ => {}
            },
            Event::Text(value) => {
                if pending_link.is_some() {
                    push_inline_span(
                        &mut spans,
                        value.to_string(),
                        current_style(&styles),
                        &mut cursor,
                    );
                } else {
                    push_text_with_autolinks(
                        &mut spans,
                        &mut links,
                        value.as_ref(),
                        current_style(&styles),
                        &mut cursor,
                    );
                }
            }
            Event::Code(value) => {
                push_inline_span(&mut spans, value.to_string(), code_style(), &mut cursor);
            }
            Event::SoftBreak | Event::HardBreak => {
                push_inline_span(
                    &mut spans,
                    " ".to_string(),
                    current_style(&styles),
                    &mut cursor,
                );
            }
            Event::Html(value) | Event::InlineHtml(value) => {
                push_inline_span(
                    &mut spans,
                    value.to_string(),
                    current_style(&styles),
                    &mut cursor,
                );
            }
            _ => {}
        }
    }

    if spans.is_empty() {
        push_inline_span(&mut spans, text.to_string(), base_style, &mut cursor);
    }
    MarkdownInline { spans, links }
}

fn push_text_with_autolinks(
    spans: &mut Vec<Span<'static>>,
    links: &mut Vec<MarkdownLink>,
    text: &str,
    style: Style,
    cursor: &mut usize,
) {
    let mut byte_cursor = 0;
    for (token_start, token) in
        text.split_inclusive(char::is_whitespace)
            .scan(0usize, |offset, token| {
                let start = *offset;
                *offset += token.len();
                Some((start, token))
            })
    {
        let trimmed = token.trim_end_matches(char::is_whitespace);
        let whitespace = &token[trimmed.len()..];
        if let Some((link_start, link_end)) = autolink_bounds(trimmed) {
            if token_start + link_start > byte_cursor {
                push_inline_span(
                    spans,
                    text[byte_cursor..token_start + link_start].to_string(),
                    style,
                    cursor,
                );
            }
            let target = &text[token_start + link_start..token_start + link_end];
            let start = *cursor;
            push_inline_span(spans, target.to_string(), style.patch(link_style()), cursor);
            links.push(MarkdownLink {
                start,
                end: *cursor,
                target: target.to_string(),
            });
            byte_cursor = token_start + link_end;
            if token_start + trimmed.len() > byte_cursor {
                push_inline_span(
                    spans,
                    text[byte_cursor..token_start + trimmed.len()].to_string(),
                    style,
                    cursor,
                );
                byte_cursor = token_start + trimmed.len();
            }
            if !whitespace.is_empty() {
                push_inline_span(spans, whitespace.to_string(), style, cursor);
                byte_cursor = token_start + token.len();
            }
        } else if token_start + token.len() > byte_cursor {
            push_inline_span(
                spans,
                text[byte_cursor..token_start + token.len()].to_string(),
                style,
                cursor,
            );
            byte_cursor = token_start + token.len();
        }
    }

    if byte_cursor < text.len() {
        push_inline_span(spans, text[byte_cursor..].to_string(), style, cursor);
    }
}

fn autolink_bounds(token: &str) -> Option<(usize, usize)> {
    let start = token
        .find("://")
        .map(|idx| {
            token[..idx]
                .rfind(|ch: char| ch.is_whitespace() || ch == '(' || ch == '[' || ch == '<')
                .map_or(0, |pos| pos + 1)
        })
        .or_else(|| token.find("www."))?;
    let mut end = token.len();
    while end > start
        && token[..end].chars().next_back().is_some_and(|ch| {
            matches!(
                ch,
                '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>'
            )
        })
    {
        end -= token[..end]
            .chars()
            .next_back()
            .map(char::len_utf8)
            .unwrap_or(0);
    }
    (end > start).then_some((start, end))
}

fn push_inline_span(
    spans: &mut Vec<Span<'static>>,
    content: String,
    style: Style,
    cursor: &mut usize,
) {
    if content.is_empty() {
        return;
    }
    *cursor += UnicodeWidthStr::width(content.as_str());
    if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
        last.content.to_mut().push_str(&content);
    } else {
        spans.push(Span::styled(content, style));
    }
}

fn push_style(styles: &mut Vec<Style>, style: Style) {
    let next = current_style(styles).patch(style);
    styles.push(next);
}

fn pop_style(styles: &mut Vec<Style>) {
    if styles.len() > 1 {
        styles.pop();
    }
}

fn current_style(styles: &[Style]) -> Style {
    styles.last().copied().unwrap_or_default()
}

/// Style for `<:channel:…:>` structure markers (tools/skills/thinking/mcp):
/// structural content renders in a subdued gray, distinct from the
/// default-foreground body.
fn structure_marker_style(text: &str) -> Option<Style> {
    if !crate::tui::content::text::is_structure_marker(text) {
        return None;
    }
    Some(structural_gray_style())
}

fn structural_gray_style() -> Style {
    Style::default().fg(crate::tui::theme::muted())
}

/// The `$` prompt marker of a tool input line: muted but bold so the
/// instruction stands out from surrounding output.
fn tool_input_marker_style() -> Style {
    Style::default()
        .fg(crate::tui::theme::muted())
        .add_modifier(Modifier::BOLD)
}

fn heading_style(level: usize) -> Style {
    match level {
        1 => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        _ => Style::default(),
    }
}

fn structure_prefix_style() -> Style {
    Style::default().fg(crate::tui::theme::dim())
}

fn task_done_marker_style() -> Style {
    Style::default()
        .fg(crate::tui::theme::success())
        .add_modifier(Modifier::BOLD)
}

fn code_style() -> Style {
    Style::default().fg(crate::tui::theme::code())
}

fn code_block_style() -> Style {
    Style::default().fg(crate::tui::theme::code())
}

/// GitHub-style gutter numbers: dim, same width as the block's line count.
fn code_line_number_style() -> Style {
    Style::default().fg(crate::tui::theme::dim())
}

fn code_fence_style() -> Style {
    Style::default()
        .fg(crate::tui::theme::dim())
        .add_modifier(Modifier::ITALIC)
}

fn link_style() -> Style {
    crate::tui::theme::link_style()
}

fn blockquote_style() -> Style {
    Style::default().fg(crate::tui::theme::quote())
}

#[cfg(test)]
mod tests {
    use super::{render_markdown_window, MarkdownLineKind};
    use ratatui::prelude::Modifier;

    #[test]
    fn renders_headings_by_level_not_text() {
        let lines = render_markdown_window(&["## User", "## Assistant"], 0, 2, 80);
        let user = &lines[0].line;
        let assistant = &lines[1].line;

        assert_eq!(user.spans[0].content.as_ref(), "## ");
        assert_eq!(user.spans[1].content.as_ref(), "User");
        // Heading color depends on the heading level alone — never on the
        // text content.
        assert_eq!(user.spans[1].style, assistant.spans[1].style);
    }

    #[test]
    fn renders_structure_markers_in_muted_gray() {
        let lines = render_markdown_window(
            &[
                "## Command",
                "<:tool:Bash call:>",
                "<:tool:Bash result:>",
                "<:skill:sivtr-memory:>",
                "<:thinking:>",
                "## Error",
            ],
            0,
            6,
            80,
        );

        for marker in [1, 2, 3, 4] {
            assert_eq!(
                lines[marker].line.spans[0].style.fg,
                Some(crate::tui::theme::muted()),
                "structure marker line {marker} must render gray"
            );
        }
    }

    #[test]
    fn renders_only_h1_as_strong_document_heading() {
        let lines = render_markdown_window(&["# Title", "## Section"], 0, 2, 80);
        let h1 = &lines[0].line;
        let h2 = &lines[1].line;

        assert_eq!(h1.spans[0].content.as_ref(), "# ");
        assert_eq!(h1.spans[1].content.as_ref(), "Title");
        assert!(h1.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(h1.spans[1]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(h1.spans[1].style.fg, None);

        assert_eq!(h2.spans[0].content.as_ref(), "## ");
        assert_eq!(h2.spans[1].content.as_ref(), "Section");
        assert_eq!(h2.spans[1].style, ratatui::prelude::Style::default());
    }

    #[test]
    fn renders_inline_markdown_without_removing_structural_prefixes() {
        let lines = render_markdown_window(&["- **bold** and `code`"], 0, 1, 80);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "- ");
        assert_eq!(line.spans[0].style.fg, Some(crate::tui::theme::dim()));
        assert_eq!(line.spans[1].content.as_ref(), "bold");
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[3].content.as_ref(), "code");
        assert_eq!(line.spans[3].style.fg, Some(crate::tui::theme::code()));
        assert_eq!(line.spans[3].style.bg, None);
    }

    #[test]
    fn renders_emphasis_and_links_for_terminal_reading() {
        let lines = render_markdown_window(&["*em* [site](https://example.com)"], 0, 1, 80);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "em");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(line.spans[2].content.as_ref(), "site");
        assert_eq!(
            line.spans[2].style.fg,
            Some(crate::tui::theme::link_style().fg.unwrap())
        );
        assert!(line.spans[2]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(lines[0].links.len(), 1);
        assert_eq!(lines[0].links[0].start, 3);
        assert_eq!(lines[0].links[0].end, 7);
        assert_eq!(lines[0].links[0].target, "https://example.com");
    }

    #[test]
    fn detects_plain_urls_as_clickable_links() {
        let lines = render_markdown_window(&["See https://example.com/docs."], 0, 1, 80);
        let line = &lines[0].line;

        assert_eq!(line.spans[0].content.as_ref(), "See ");
        assert_eq!(line.spans[1].content.as_ref(), "https://example.com/docs");
        assert_eq!(
            line.spans[1].style.fg,
            Some(crate::tui::theme::link_style().fg.unwrap())
        );
        assert_eq!(line.spans[2].content.as_ref(), ".");
        assert_eq!(lines[0].links[0].start, 4);
        assert_eq!(lines[0].links[0].end, 28);
        assert_eq!(lines[0].links[0].target, "https://example.com/docs");
    }

    #[test]
    fn renders_structural_markers_as_dim_prefixes() {
        let lines = render_markdown_window(&["> quote", "1. item"], 0, 2, 80);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "> ");
        assert_eq!(
            lines[0].line.spans[0].style.fg,
            Some(crate::tui::theme::dim())
        );
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "quote");
        assert_eq!(
            lines[0].line.spans[1].style.fg,
            Some(crate::tui::theme::quote())
        );

        assert_eq!(lines[1].line.spans[0].content.as_ref(), "1. ");
        assert_eq!(
            lines[1].line.spans[0].style.fg,
            Some(crate::tui::theme::dim())
        );
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "item");
    }

    #[test]
    fn renders_task_list_markers_as_square_symbols() {
        let lines = render_markdown_window(&["- [ ] open", "- [x] done"], 0, 2, 80);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "□ ");
        assert_eq!(
            lines[0].line.spans[0].style.fg,
            Some(crate::tui::theme::dim())
        );
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "open");

        assert_eq!(lines[1].line.spans[0].content.as_ref(), "■ ");
        assert_eq!(
            lines[1].line.spans[0].style.fg,
            Some(crate::tui::theme::success())
        );
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "done");
    }

    #[test]
    fn renders_horizontal_rules_to_content_width() {
        let lines = render_markdown_window(&["---"], 0, 1, 12);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "────────────");
        assert_eq!(
            lines[0].line.spans[0].style.fg,
            Some(crate::tui::theme::dim())
        );
    }

    #[test]
    fn aligns_markdown_tables_by_visible_cell_width() {
        let lines = render_markdown_window(
            &[
                "| Name | Status |",
                "| --- | --- |",
                "| sivtr | ok |",
                "| cli | needs review |",
            ],
            0,
            6,
            80,
        );

        assert_eq!(lines.len(), 6);
        assert_eq!(
            lines[0].line.spans[0].content.as_ref(),
            "┌───────┬──────────────┐"
        );
        assert_eq!(lines[1].line.spans[0].content.as_ref(), "│ ");
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "Name");
        assert_eq!(lines[1].line.spans[2].content.as_ref(), " ");
        assert_eq!(lines[1].line.spans[3].content.as_ref(), " │ ");
        assert_eq!(
            lines[2].line.spans[0].content.as_ref(),
            "├───────┼──────────────┤"
        );
        assert_eq!(lines[3].line.spans[1].content.as_ref(), "sivtr");
        assert_eq!(lines[3].line.spans[2].content.as_ref(), " │ ");
        assert_eq!(lines[4].line.spans[1].content.as_ref(), "cli");
        assert_eq!(lines[4].line.spans[2].content.as_ref(), "  ");
        assert_eq!(
            lines[5].line.spans[0].content.as_ref(),
            "└───────┴──────────────┘"
        );
    }

    #[test]
    fn styles_tool_input_lines_with_a_marker_prefix() {
        let lines = render_markdown_window(&["$ read src/main.rs:12-30"], 0, 1, 80);

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "$ ");
        assert_eq!(
            lines[0].line.spans[0].style.fg,
            Some(crate::tui::theme::muted())
        );
        assert!(lines[0].line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(
            lines[0].line.spans[1].content.as_ref(),
            "read src/main.rs:12-30"
        );
    }

    #[test]
    fn renders_fenced_code_blocks_with_github_style_line_numbers() {
        let lines = render_markdown_window(&["```text", "-> value", "```"], 0, 3, 80);

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert!(lines[0].line.spans.is_empty());
        assert_eq!(lines[1].kind, MarkdownLineKind::CodeBlock);
        assert_eq!(lines[1].line.spans[0].content.as_ref(), "1 ");
        assert_eq!(lines[1].line.spans[1].content.as_ref(), "-> value");
        assert_eq!(
            lines[1].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
        assert_eq!(lines[2].kind, MarkdownLineKind::CodeFence);
        assert!(lines[2].line.spans.is_empty());
    }

    #[test]
    fn keeps_code_block_state_when_window_starts_inside_block() {
        let lines = render_markdown_window(&["```text", "alpha", "beta", "```"], 2, 1, 80);

        assert_eq!(lines[0].kind, MarkdownLineKind::CodeBlock);
        // Numbers stay true to the block: "beta" is the second line, so its
        // gutter says 2 even when the window starts at line two.
        assert_eq!(lines[0].line.spans[0].content.as_ref(), "2 ");
        assert_eq!(lines[0].line.spans[1].content.as_ref(), "beta");
        assert_eq!(
            lines[0].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
    }

    #[test]
    fn code_block_gutter_width_tracks_the_block_length() {
        let lines = render_markdown_window(
            &[
                "```text", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "```",
            ],
            0,
            12,
            80,
        );
        // Ten lines → two-digit numbers, right-aligned to the same width.
        assert_eq!(lines[1].line.spans[0].content.as_ref(), " 1 ");
        assert_eq!(lines[9].line.spans[0].content.as_ref(), " 9 ");
        assert_eq!(lines[10].line.spans[0].content.as_ref(), "10 ");
    }

    #[test]
    fn numeric_fence_numbers_the_block_from_that_file_line() {
        // read results fence as ```` ```775 ````: the gutter shows the file's
        // real lines, and the fence row does not print "775" as a language.
        let lines = render_markdown_window(
            &["```775", "line one", "line two", "line three", "```"],
            0,
            5,
            80,
        );
        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert!(lines[0].line.spans.is_empty());
        assert_eq!(lines[1].line.spans[0].content.as_ref(), "775 ");
        assert_eq!(lines[2].line.spans[0].content.as_ref(), "776 ");
        assert_eq!(lines[3].line.spans[0].content.as_ref(), "777 ");
        assert_eq!(lines[4].kind, MarkdownLineKind::CodeFence);
        assert!(lines[4].line.spans.is_empty());
    }

    #[test]
    fn grep_block_styles_summary_paths_and_matches_separately() {
        let lines = render_markdown_window(
            &[
                "```grep",
                "Found 2 matching lines",
                "D:\\Coding\\AGENTS.md",
                "31:- Adding, updating, or removing dependencies",
                "```",
            ],
            0,
            5,
            100,
        );
        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert_eq!(lines[0].line.spans[0].content.as_ref(), "grep");

        // The summary reads as a bold accent.
        assert_eq!(
            lines[1].line.spans[0].style.fg,
            Some(crate::tui::theme::accent())
        );
        assert!(lines[1].line.spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        // The file path gets accent, no line gutter.
        assert_eq!(
            lines[2].line.spans[0].content.as_ref(),
            "D:\\Coding\\AGENTS.md"
        );
        assert_eq!(
            lines[2].line.spans[0].style.fg,
            Some(crate::tui::theme::accent())
        );
        // The match splits into a highlighted line number and code content.
        assert_eq!(lines[3].line.spans[0].content.as_ref(), "31:");
        assert_eq!(
            lines[3].line.spans[0].style.fg,
            Some(crate::tui::theme::accent())
        );
        assert_eq!(
            lines[3].line.spans[1].content.as_ref(),
            "- Adding, updating, or removing dependencies"
        );
        assert_eq!(
            lines[3].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
    }

    #[test]
    fn renders_useful_code_language_as_a_subtle_label() {
        let lines = render_markdown_window(&["```sql", "select 1", "```"], 0, 1, 80);

        assert_eq!(lines[0].kind, MarkdownLineKind::CodeFence);
        assert_eq!(lines[0].line.spans[0].content.as_ref(), "sql");
    }

    #[test]
    fn renders_diff_fenced_blocks_with_change_colors() {
        let lines = render_markdown_window(
            &[
                "```diff",
                "@@ -1,3 +1,4 @@",
                " base",
                "-removed",
                "+added",
                "+++ b/file",
                "```",
            ],
            0,
            6,
            80,
        );

        assert_eq!(lines[0].line.spans[0].content.as_ref(), "diff");
        assert_eq!(
            lines[1].line.spans[1].style.fg,
            Some(crate::tui::theme::accent())
        ); // @@ hunk
        assert_eq!(
            lines[2].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        ); // context
        assert_eq!(
            lines[3].line.spans[1].style.fg,
            Some(crate::tui::theme::failure())
        ); // - removed
        assert_eq!(
            lines[4].line.spans[1].style.fg,
            Some(crate::tui::theme::success())
        ); // + added
        assert!(lines[5].line.spans[1]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn auto_detects_unified_diffs_in_plain_fenced_blocks() {
        let lines = render_markdown_window(&["```", "@@ -1 +1 @@", "-a", "+b", "```"], 0, 5, 80);

        assert_eq!(
            lines[1].line.spans[1].style.fg,
            Some(crate::tui::theme::accent())
        );
        assert_eq!(
            lines[2].line.spans[1].style.fg,
            Some(crate::tui::theme::failure())
        );
        assert_eq!(
            lines[3].line.spans[1].style.fg,
            Some(crate::tui::theme::success())
        );
    }

    #[test]
    fn non_diff_code_blocks_keep_plain_code_style() {
        let lines = render_markdown_window(&["```text", "- not a diff", "+ nope", "```"], 0, 3, 80);

        // No hunk header: the block is not a diff, so +/- lines stay code-colored.
        assert_eq!(
            lines[1].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
        assert_eq!(
            lines[2].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
    }

    #[test]
    fn plain_code_lines_starting_with_at_at_do_not_enable_diff_mode() {
        // `@@derive` is not a hunk header, so the block stays plain.
        let lines = render_markdown_window(
            &["```rust", "@@derive(Clone)", "+ not a change", "```"],
            0,
            3,
            80,
        );
        assert_eq!(
            lines[1].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
        assert_eq!(
            lines[2].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
    }

    #[test]
    fn non_canonical_hunk_headers_do_not_enable_diff_mode() {
        // A context line, a swapped sign order, and a multi-count range are
        // all lookalikes, not hunk headers.
        for header in [" @@ -1 +1 @@", "@@ +1 -1 @@", "@@ -1,2,3 +1 @@"] {
            let lines = render_markdown_window(&["```", header, "+ not a change", "```"], 0, 3, 80);
            assert_eq!(
                lines[2].line.spans[1].style.fg,
                Some(crate::tui::theme::code()),
                "header `{header}` must not enable diff mode"
            );
        }
    }

    #[test]
    fn hunk_headers_with_section_headings_still_detect() {
        let lines = render_markdown_window(&["```", "@@ -1,3 +1,4 @@ fn main()", "```"], 0, 2, 80);
        assert_eq!(
            lines[1].line.spans[1].style.fg,
            Some(crate::tui::theme::accent())
        );
    }

    #[test]
    fn diff_context_lines_with_plus_content_stay_plain() {
        // The leading context space is preserved: " + x" is context, not an
        // addition.
        let lines =
            render_markdown_window(&["```diff", "@@ -1 +1 @@", " + not added", "```"], 0, 3, 80);
        assert_eq!(
            lines[2].line.spans[1].style.fg,
            Some(crate::tui::theme::code())
        );
        assert_eq!(lines[2].line.spans[1].content.as_ref(), " + not added");
    }

    #[test]
    fn change_lines_starting_with_triple_pluses_stay_additions() {
        let lines = render_markdown_window(
            &["```diff", "@@ -1 +1 @@", "+++added", "--- b/file", "```"],
            0,
            4,
            80,
        );
        // "+++added" is added content, not a file header.
        assert_eq!(
            lines[2].line.spans[1].style.fg,
            Some(crate::tui::theme::success())
        );
        assert!(!lines[2].line.spans[1]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        // "--- b/file" is a file header: muted + bold.
        assert_eq!(
            lines[3].line.spans[1].style.fg,
            Some(crate::tui::theme::muted())
        );
        assert!(lines[3].line.spans[1]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }
}
