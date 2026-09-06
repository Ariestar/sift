//! Shape-driven tool display for the content pane.
//!
//! Tool actions are classified by category — read (`$ read path` + code
//! preview), search (`$ grep pattern`), edit (diff preview), web (`$
//! webfetch url`) — and get per-tool tags (`<:read: src/main.rs:12-30:>`)
//! instead of the generic `<:tool:Name call:>` marker. Shell actions read
//! as the terminal transcript they are: the `$` command line, then the
//! output, whoever ran them. The formatter keys off the action target and
//! its input / output, so every provider (claude, grok, codex, opencode,
//! …) flows through one code path — display only; evidence export keeps
//! its original markers.

use similar::{ChangeTag, TextDiff};

use serde_json::Value;
use sivtr_core::record::{
    ProjectionSlice, WorkContent, WorkContentBlock, WorkPart, WorkPartBody, WorkTarget,
};

/// Long input expressions are truncated to fit a tag line.
const MAX_EXPR: usize = 40;

/// Tool category: drives how a tool call is displayed. Shell execution is
/// not a category here — the reducer normalizes shell tools to
/// `WorkTarget::Shell` before display.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolCategory {
    /// File read: `$ read path:lines`, code preview from the result.
    Read,
    /// Text search: `$ grep pattern`.
    Search,
    /// File modification: diff preview from the input (`edit`, `write`,
    /// `apply_patch`).
    Edit,
    /// Remote fetch: `$ webfetch url`.
    Web,
}

/// Display spec of a known tool: its category and canonical tag name.
struct ToolSpec {
    category: ToolCategory,
    name: &'static str,
}

/// Known tools by provider name (claude `Read`, opencode `read`, codex
/// `apply_patch`, grok build `read_file`/`search_replace`); `None` for
/// unknown tools that keep the generic marker.
fn tool_spec(tool: &str) -> Option<&'static ToolSpec> {
    use ToolCategory::*;
    Some(match tool.to_ascii_lowercase().as_str() {
        "read" | "read_file" => &ToolSpec {
            category: Read,
            name: "read",
        },
        "grep" | "search_files" => &ToolSpec {
            category: Search,
            name: "grep",
        },
        "edit" | "search_replace" => &ToolSpec {
            category: Edit,
            name: "edit",
        },
        "write" => &ToolSpec {
            category: Edit,
            name: "write",
        },
        "apply_patch" | "patch" => &ToolSpec {
            category: Edit,
            name: "patch",
        },
        "notebookedit" | "notebook_edit" => &ToolSpec {
            category: Edit,
            name: "notebook-edit",
        },
        "webfetch" | "web_fetch" => &ToolSpec {
            category: Web,
            name: "webfetch",
        },
        "websearch" | "web_search" => &ToolSpec {
            category: Web,
            name: "websearch",
        },
        _ => return None,
    })
}

/// Grok's MCP dispatcher: `use_tool` calls a `server__tool` by name.
fn is_use_tool(tool: &str) -> bool {
    tool.eq_ignore_ascii_case("use_tool")
}

/// Display name for any tool: MCP tools as `server: tool`, known tools
/// canonically, unknown tools as their lowercased name.
pub(crate) fn tool_display_name(tool: &str) -> String {
    if let Some(rest) = tool.strip_prefix("mcp__") {
        if let Some((server, name)) = rest.split_once("__") {
            return format!("{server}: {name}");
        }
    }
    tool_spec(tool)
        .map(|spec| spec.name.to_string())
        .unwrap_or_else(|| tool.to_ascii_lowercase())
}

/// Whether the tool gets the new per-tool rendering (known names + MCP).
fn is_known_tool(tool: &str) -> bool {
    tool_spec(tool).is_some() || tool.starts_with("mcp__") || is_use_tool(tool)
}

/// Display name of a call: `use_tool` shows its target tool, others the
/// tool's own name.
fn tool_call_name(tool: &str, input: &Value) -> String {
    if is_use_tool(tool) {
        use_tool_name(input).unwrap_or_else(|| tool_display_name(tool))
    } else {
        tool_display_name(tool)
    }
}

/// Folded tag for a tool *call*: MCP tools always (`<:sivtr: sivtr_search:>`),
/// `use_tool` as its target tool, known tools when the input shape is
/// understood (`<:bash: ls:>`); `None` keeps the generic `<:tool:Name call:>`
/// marker. Long expressions are truncated to fit the tag line.
pub(crate) fn tool_tag(tool: &str, value: &Value) -> Option<String> {
    if tool.starts_with("mcp__") || is_use_tool(tool) {
        return Some(format!("<:{}:>", tool_call_name(tool, value)));
    }
    let spec = tool_spec(tool)?;
    if spec.name == "patch" {
        // apply_patch always carries a raw patch string; no short expr.
        return Some("<:patch:>".to_string());
    }
    let expr = tool_input_expr(tool, value)?;
    Some(format!(
        "<:{}: {}:>",
        spec.name,
        crate::tui::content::truncate_chars(&expr, MAX_EXPR)
    ))
}

/// New-style folded tag for an action part, when the shape is understood:
/// MCP tools always (`<:sivtr: sivtr_search:>`), shell commands as the
/// expression (`<:shell: ls:>`), known tools as the expression tag for a
/// call (`<:bash: ls:>`) or the bare name for a result. `None` keeps the
/// generic `<:tool:Name call:>` marker. Long expressions are truncated to
/// fit the tag line.
pub(crate) fn tool_tag_for_part(part: &WorkPart) -> Option<String> {
    let WorkPartBody::Action {
        target,
        input,
        output,
        ..
    } = &part.body
    else {
        return None;
    };
    match target {
        WorkTarget::Mcp { server, tool } => Some(format!("<:{server}: {tool}:>")),
        WorkTarget::Shell => {
            let command = shell_command(input.as_ref())?;
            Some(format!(
                "<:shell: {}:>",
                crate::tui::content::truncate_chars(&command, MAX_EXPR)
            ))
        }
        WorkTarget::Tool { name } => {
            let tool = name.as_deref().unwrap_or_default();
            if output.is_empty() {
                let input = input.as_ref()?;
                let WorkContent::Json(value) = input else {
                    return None;
                };
                tool_tag(tool, value)
            } else {
                is_known_tool(tool).then(|| format!("<:{}:>", tool_display_name(tool)))
            }
        }
        WorkTarget::Agent { .. } => None,
    }
}

fn shell_command(input: Option<&WorkContent>) -> Option<String> {
    let command = input?.text().trim_end().to_string();
    (!command.is_empty()).then_some(command)
}

/// Whether the new `$` format applies to a tool call: MCP tools and
/// `use_tool` always, known tools when the input shape is understood (an
/// expression or a preview). Unrecognized shapes keep the evidence format so
/// no payload is lost.
fn tool_renderable_call(tool: &str, input: &Value) -> bool {
    tool.starts_with("mcp__")
        || is_use_tool(tool)
        || tool_input_expr(tool, input).is_some()
        || diff_preview(tool, input).is_some()
}
/// Display body of one part in the slice a projection asks for: the `$`/`>`
/// tool format for understood tool shapes, shell actions as terminal
/// transcripts, the evidence format otherwise.
pub(crate) fn part_body_text(part: &WorkPart, slice: ProjectionSlice) -> String {
    let WorkPartBody::Action {
        target,
        input,
        output,
        ..
    } = &part.body
    else {
        return sivtr_core::record::format_work_part(part);
    };
    match target {
        WorkTarget::Shell => sivtr_core::record::format_shell_action(part, slice),
        WorkTarget::Tool { name } => tool_body(
            part,
            slice,
            name.as_deref().unwrap_or_default(),
            input.as_ref(),
            output,
        ),
        WorkTarget::Mcp { server, tool } => tool_body(
            part,
            slice,
            &format!("mcp__{server}__{tool}"),
            input.as_ref(),
            output,
        ),
        WorkTarget::Agent { .. } => sivtr_core::record::format_work_part(part),
    }
}

/// Body of a tool action: the `$` call line when the input shape is
/// understood, the result body for known tools; otherwise the evidence
/// format so no payload is lost.
fn tool_body(
    part: &WorkPart,
    slice: ProjectionSlice,
    tool: &str,
    input: Option<&WorkContent>,
    output: &[WorkContentBlock],
) -> String {
    let json_input = match input {
        Some(WorkContent::Json(value)) => Some(value),
        _ => None,
    };
    let call = json_input
        .filter(|_| matches!(slice, ProjectionSlice::Whole | ProjectionSlice::Input))
        .filter(|value| tool_renderable_call(tool, value))
        .map(|value| tool_call_text(tool, value));
    let result = output
        .first()
        .filter(|_| matches!(slice, ProjectionSlice::Whole | ProjectionSlice::Output))
        .filter(|_| is_known_tool(tool))
        .and_then(|block| match &block.content {
            WorkContent::Json(value) => Some(tool_result_text(tool, value, block.start_line)),
            _ => None,
        });
    match (call, result) {
        (Some(call), Some(result)) => format!("{call}\n{result}"),
        (Some(call), None) => call,
        (None, Some(result)) => result,
        // The evidence format renders only when the projection asks for the
        // whole part; a sliced view shows just its own side.
        (None, None) => match slice {
            ProjectionSlice::Whole => sivtr_core::record::format_work_part(part),
            ProjectionSlice::Input => input
                .map(sivtr_core::record::WorkContent::text)
                .map(std::borrow::Cow::into_owned)
                .unwrap_or_default(),
            ProjectionSlice::Output => sivtr_core::record::output_blocks_text(output),
        },
    }
}

/// Display name for `use_tool` (grok's MCP dispatcher): the `tool_name`
/// argument, `server__tool` rendered as `server: tool`.
fn use_tool_name(input: &Value) -> Option<String> {
    let tool_name = input.as_object()?.get("tool_name")?.as_str()?;
    Some(match tool_name.split_once("__") {
        Some((server, name)) => format!("{server}: {name}"),
        None => tool_name.to_string(),
    })
}

/// Input expression from a tool call's input JSON: `src/main.rs:12-30`,
/// `export function foo`… `None` when the shape is unknown. Full text;
/// callers truncate for tag lines.
fn tool_input_expr(tool: &str, input: &Value) -> Option<String> {
    let spec = tool_spec(tool)?;
    let expr = match spec.category {
        ToolCategory::Read => {
            let obj = input.as_object()?;
            let path = path_field(obj)?;
            match line_range(obj) {
                Some(range) => format!("{path}:{range}"),
                None => path.to_string(),
            }
        }
        ToolCategory::Search => input
            .as_object()?
            .get("pattern")?
            .as_str()?
            .trim()
            .to_string(),
        ToolCategory::Edit => match spec.name {
            "patch" => return None,
            _ => path_field(input.as_object()?)?.to_string(),
        },
        ToolCategory::Web => input
            .as_object()?
            .get("url")
            .or_else(|| input.as_object()?.get("query"))?
            .as_str()?
            .trim()
            .to_string(),
    };
    if expr.is_empty() {
        return None;
    }
    Some(expr)
}

/// File path field: `file_path` (claude/grok), `filePath` (opencode),
/// `target_file` (grok build), or `path` (codex).
fn path_field(obj: &serde_json::Map<String, Value>) -> Option<&str> {
    obj.get("file_path")
        .or_else(|| obj.get("filePath"))
        .or_else(|| obj.get("target_file"))
        .or_else(|| obj.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// Line range: grok-style `line_start`/`line_end` (1-based inclusive), else
/// claude-style `offset` (0-based) / `limit` lines.
fn line_range(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let start = obj.get("line_start").and_then(Value::as_i64);
    let end = obj.get("line_end").and_then(Value::as_i64);
    match (start, end) {
        (Some(start), Some(end)) => Some(format!("{start}-{end}")),
        _ => {
            let offset = obj.get("offset").and_then(Value::as_i64);
            let limit = obj.get("limit").and_then(Value::as_i64);
            match (offset, limit) {
                (Some(offset), Some(limit)) => Some(format!(
                    "{}-{}",
                    offset.saturating_add(1),
                    offset.saturating_add(limit)
                )),
                (Some(offset), None) => Some(format!("{}", offset.saturating_add(1))),
                _ => None,
            }
        }
    }
}

/// Expanded body of a tool call: the `$` input line, plus a diff preview
/// for edit/write (which carry content in the input).
pub(crate) fn tool_call_text(tool: &str, input: &Value) -> String {
    let name = tool_call_name(tool, input);
    let expr = tool_input_expr(tool, input);
    let line = match expr {
        Some(expr) => format!("$ {name} {expr}"),
        None => format!("$ {name}"),
    };
    match diff_preview(tool, input) {
        Some(preview) => format!("{line}\n{preview}"),
        None => line,
    }
}

/// Diff preview from the tool input: `+` lines for write content, a real
/// line diff for edit old/new strings (aligned insertions, deletions, and
/// context like grok build), the raw unified diff for apply_patch, inside a
/// ```diff fence the pane colors.
fn diff_preview(tool: &str, input: &Value) -> Option<String> {
    let spec = tool_spec(tool)?;
    if spec.category != ToolCategory::Edit {
        return None;
    }
    let diff = match spec.name {
        "patch" => {
            let Value::String(patch) = input else {
                return None;
            };
            let patch = patch.trim_end();
            if patch.is_empty() {
                return None;
            }
            patch.to_string()
        }
        "write" => {
            let content = input.as_object()?.get("content")?.as_str()?;
            content
                .lines()
                .map(|line| format!("+{line}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => {
            // edit / notebook-edit: diff old_string against new_string so the
            // preview shows what actually changed, not two blind blocks.
            let obj = input.as_object()?;
            let old_string = obj
                .get("old_string")
                .or_else(|| obj.get("oldString"))
                .and_then(Value::as_str);
            let new_string = obj
                .get("new_string")
                .or_else(|| obj.get("newString"))
                .and_then(Value::as_str);
            let (Some(old), Some(new)) = (old_string, new_string) else {
                // Notebook-style edits may carry only one side: show it as a
                // plain insertion or deletion.
                let marked = |text: &str, sign: char| {
                    text.lines()
                        .map(|line| format!("{sign}{line}"))
                        .collect::<Vec<_>>()
                };
                let single = new_string
                    .map(|new| marked(new, '+'))
                    .or_else(|| old_string.map(|old| marked(old, '-')))
                    .filter(|lines| !lines.is_empty())
                    .map(|lines| format!("```diff\n{}\n```", lines.join("\n")));
                return single;
            };
            diff_hunk(old, new)?
        }
    };
    if diff.trim().is_empty() {
        return None;
    }
    Some(format!("```diff\n{diff}\n```"))
}

/// Unified-style line diff of `old` → `new`, keeping up to three context
/// lines around the changes — the same shape `similar` produces for grok
/// build's edit previews.
fn diff_hunk(old: &str, new: &str) -> Option<String> {
    const CONTEXT: usize = 3;
    let lines: Vec<(ChangeTag, String)> = TextDiff::from_lines(old, new)
        .iter_all_changes()
        .map(|change| {
            (
                change.tag(),
                change.value().trim_end_matches('\n').to_string(),
            )
        })
        .collect();
    let first = lines.iter().position(|(tag, _)| *tag != ChangeTag::Equal)?;
    let last = lines
        .iter()
        .rposition(|(tag, _)| *tag != ChangeTag::Equal)?;
    let start = first.saturating_sub(CONTEXT);
    let end = (last + CONTEXT + 1).min(lines.len());
    let text = lines[start..end]
        .iter()
        .map(|(tag, text)| match tag {
            ChangeTag::Delete => format!("-{text}"),
            ChangeTag::Insert => format!("+{text}"),
            ChangeTag::Equal => format!(" {text}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

/// Expanded body of a tool result: `>` output lines, or a fenced block for
/// read (the file content preview) and every search tool. Text matches fence
/// as a structured ` ```grep ` block (summary, paths, line numbers); JSON
/// results (e.g. opencode's `search_files`) keep their data shape as a
/// ` ```json ` block. Provider envelopes (grok's `<workspace_result …>`,
/// `N→` line gutters) are already stripped by the parser, whose `start_line`
/// metadata shifts the read gutter to real file lines.
pub(crate) fn tool_result_text(tool: &str, output: &Value, start_line: Option<u64>) -> String {
    let text = match output {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(output).unwrap_or_default(),
    };
    let Some(spec) = tool_spec(tool) else {
        return output_lines(&text);
    };
    match spec.category {
        ToolCategory::Read => match start_line {
            Some(start) => format!("```{start}\n{}\n```", text.trim_end()),
            None => format!("```\n{}\n```", text.trim_end()),
        },
        ToolCategory::Search => match output {
            Value::String(_) => format!("```grep\n{}\n```", text.trim_end()),
            _ => format!("```json\n{}\n```", text.trim_end()),
        },
        _ => output_lines(&text),
    }
}

fn output_lines(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{WorkActionStatus, WorkActor, WorkTarget};

    const SLICE: ProjectionSlice = ProjectionSlice::Whole;

    fn tool_action(
        seq: usize,
        tool: &str,
        input: Option<Value>,
        output: Option<(Value, Option<u64>)>,
    ) -> WorkPart {
        let start_line = output.as_ref().and_then(|(_, line)| *line);
        let mut part = crate::test_fixtures::tool_action_part(
            seq,
            &format!("a{seq}"),
            Some(tool),
            input,
            output.map(|(value, _)| value),
        );
        if let WorkPartBody::Action { output, status, .. } = &mut part.body {
            if let Some(block) = output.first_mut() {
                block.start_line = start_line;
            }
            *status = WorkActionStatus::Completed;
        }
        part
    }

    fn call(tool: &str, input: Value) -> WorkPart {
        tool_action(1, tool, Some(input), None)
    }

    fn result(tool: &str, output: Value) -> WorkPart {
        result_with_line(tool, output, None)
    }

    fn result_with_line(tool: &str, output: Value, start_line: Option<u64>) -> WorkPart {
        tool_action(2, tool, None, Some((output, start_line)))
    }

    fn shell(seq: usize, command: Option<&str>, output: Option<&str>) -> WorkPart {
        crate::test_fixtures::shell_action_part(seq, command.unwrap_or(""), output)
    }

    #[test]
    fn mcp_tools_render_as_server_colon_tool() {
        assert_eq!(
            tool_display_name("mcp__sivtr__sivtr_search"),
            "sivtr: sivtr_search"
        );
        assert_eq!(
            tool_tag("mcp__codegraph__codegraph_context", &Value::Null).unwrap(),
            "<:codegraph: codegraph_context:>"
        );
    }

    #[test]
    fn mcp_targets_render_as_server_colon_tool() {
        let part = WorkPart {
            seq: 1,
            occurred_at: None,
            body: WorkPartBody::Action {
                id: "mcp".to_string(),
                actor: WorkActor::Agent,
                target: WorkTarget::Mcp {
                    server: "sivtr".to_string(),
                    tool: "sivtr_search".to_string(),
                },
                title: None,
                input: Some(WorkContent::Json(serde_json::json!({"query": "x"}))),
                output: Vec::new(),
                status: WorkActionStatus::InProgress,
                exit_code: None,
            },
        };
        assert_eq!(tool_tag_for_part(&part).unwrap(), "<:sivtr: sivtr_search:>");
        assert_eq!(part_body_text(&part, SLICE), "$ sivtr: sivtr_search");
    }

    #[test]
    fn known_tools_get_canonical_names_and_exprs() {
        let read = call(
            "Read",
            serde_json::json!({"file_path": "src/main.rs", "offset": 11, "limit": 19}),
        );
        assert_eq!(
            tool_tag_for_part(&read).unwrap(),
            "<:read: src/main.rs:12-30:>"
        );
        assert_eq!(part_body_text(&read, SLICE), "$ read src/main.rs:12-30");

        let read_grok = call(
            "read",
            serde_json::json!({"file_path": "a.rs", "line_start": 5, "line_end": 9}),
        );
        assert_eq!(tool_tag_for_part(&read_grok).unwrap(), "<:read: a.rs:5-9:>");

        let grep = call(
            "Grep",
            serde_json::json!({"pattern": "export function", "glob": "*.ts"}),
        );
        assert_eq!(
            tool_tag_for_part(&grep).unwrap(),
            "<:grep: export function:>"
        );
        assert_eq!(part_body_text(&grep, SLICE), "$ grep export function");
    }

    #[test]
    fn shell_actions_render_command_and_output() {
        let action = shell(1, Some("cd src && cargo build"), None);
        assert_eq!(
            tool_tag_for_part(&action).unwrap(),
            "<:shell: cd src && cargo build:>"
        );
        assert_eq!(part_body_text(&action, SLICE), "$ cd src && cargo build");

        let with_output = shell(2, Some("ls"), Some("src\ntarget"));
        assert_eq!(part_body_text(&with_output, SLICE), "$ ls\nsrc\ntarget");

        // Output-only and command-only halves render their slice.
        assert_eq!(part_body_text(&with_output, ProjectionSlice::Input), "$ ls");
        assert_eq!(
            part_body_text(&with_output, ProjectionSlice::Output),
            "src\ntarget"
        );
    }

    #[test]
    fn write_and_edit_show_diff_previews() {
        let write = call(
            "Write",
            serde_json::json!({
                "file_path": "notes.md",
                "content": "line one\nline two",
            }),
        );
        assert_eq!(tool_tag_for_part(&write).unwrap(), "<:write: notes.md:>");
        assert_eq!(
            part_body_text(&write, SLICE),
            "$ write notes.md\n```diff\n+line one\n+line two\n```"
        );

        let edit = call(
            "Edit",
            serde_json::json!({
                "file_path": "a.rs",
                "old_string": "old",
                "new_string": "new",
                "replace_all": false,
            }),
        );
        assert_eq!(tool_tag_for_part(&edit).unwrap(), "<:edit: a.rs:>");
        assert_eq!(
            part_body_text(&edit, SLICE),
            "$ edit a.rs\n```diff\n-old\n+new\n```"
        );
    }

    #[test]
    fn edit_diff_aligns_changes_with_context() {
        // Replacing one line inside a block shows the surrounding context
        // and the real change — not two blind old/new blocks.
        let edit = call(
            "Edit",
            serde_json::json!({
                "file_path": "a.rs",
                "old_string": "fn main() {\n    println!(\"old\");\n}",
                "new_string": "fn main() {\n    println!(\"new\");\n}",
            }),
        );
        let text = part_body_text(&edit, SLICE);
        assert!(text.contains(" fn main() {"), "context missing: {text}");
        assert!(text.contains("-    println!(\"old\");"), "{text}");
        assert!(text.contains("+    println!(\"new\");"), "{text}");
        assert!(text.contains(" }"), "trailing context missing: {text}");
    }

    #[test]
    fn read_result_previews_as_code_block_and_others_as_output_lines() {
        let read_result = result("Read", serde_json::json!("fn main() {}\n"));
        assert_eq!(
            part_body_text(&read_result, SLICE),
            "```\nfn main() {}\n```"
        );

        // A numbered read result shifts the code gutter to the file's line.
        let numbered = result_with_line("Read", serde_json::json!("line one\nline two"), Some(775));
        assert_eq!(
            part_body_text(&numbered, SLICE),
            "```775\nline one\nline two\n```"
        );

        let fetch_result = result("webfetch", serde_json::json!("ok\nwarning"));
        assert_eq!(part_body_text(&fetch_result, SLICE), "> ok\n> warning");

        // Text grep results fence as a structured search block; JSON results
        // (opencode's search_files) keep their data shape as a JSON block.
        let text_match = result(
            "Grep",
            serde_json::json!("Found 2 matching lines\nD:\\Coding\\AGENTS.md\n31:- rule\n"),
        );
        assert_eq!(
            part_body_text(&text_match, SLICE),
            "```grep\nFound 2 matching lines\nD:\\Coding\\AGENTS.md\n31:- rule\n```"
        );
        let json_result = result("Grep", serde_json::json!([{"file": "a.rs", "line": 1}]));
        assert_eq!(
            part_body_text(&json_result, SLICE),
            "```json\n[\n  {\n    \"file\": \"a.rs\",\n    \"line\": 1\n  }\n]\n```"
        );
    }

    #[test]
    fn opencode_camel_case_shapes_are_recognized() {
        let read = call(
            "read",
            serde_json::json!({"filePath": "src/main.rs", "offset": 11, "limit": 19}),
        );
        assert_eq!(
            tool_tag_for_part(&read).unwrap(),
            "<:read: src/main.rs:12-30:>"
        );

        let edit = call(
            "edit",
            serde_json::json!({
                "filePath": "a.rs",
                "oldString": "old",
                "newString": "new",
            }),
        );
        assert_eq!(tool_tag_for_part(&edit).unwrap(), "<:edit: a.rs:>");
        assert_eq!(
            part_body_text(&edit, SLICE),
            "$ edit a.rs\n```diff\n-old\n+new\n```"
        );

        let grep = call(
            "grep",
            serde_json::json!({"path": "a.rs", "pattern": "fn main"}),
        );
        assert_eq!(tool_tag_for_part(&grep).unwrap(), "<:grep: fn main:>");
    }

    #[test]
    fn grok_build_tool_names_are_recognized() {
        let read = call(
            "read_file",
            serde_json::json!({"target_file": "src/main.rs", "offset": 11, "limit": 19}),
        );
        assert_eq!(
            tool_tag_for_part(&read).unwrap(),
            "<:read: src/main.rs:12-30:>"
        );

        let edit = call(
            "search_replace",
            serde_json::json!({
                "file_path": "a.rs",
                "old_string": "old",
                "new_string": "new",
            }),
        );
        assert_eq!(tool_tag_for_part(&edit).unwrap(), "<:edit: a.rs:>");

        // use_tool dispatches MCP tools: the tag shows the target tool.
        let use_tool = call(
            "use_tool",
            serde_json::json!({
                "tool_name": "context7__resolve-library-id",
                "tool_input": {"query": "zed"},
            }),
        );
        assert_eq!(
            tool_tag_for_part(&use_tool).unwrap(),
            "<:context7: resolve-library-id:>"
        );
        assert_eq!(
            part_body_text(&use_tool, SLICE),
            "$ context7: resolve-library-id"
        );
    }

    #[test]
    fn codex_apply_patch_previews_as_diff() {
        let patch = call(
            "apply_patch",
            serde_json::json!("*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n"),
        );
        assert_eq!(tool_tag_for_part(&patch).unwrap(), "<:patch:>");
        assert_eq!(
            part_body_text(&patch, SLICE),
            "$ patch\n```diff\n*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n```"
        );
    }

    #[test]
    fn shell_scripts_truncate_in_the_tag_line() {
        let script =
            "const skill = await tools.shell_command({command:\"cargo build\"});\ntext(skill);";
        let exec = shell(1, Some(script), None);
        // The whole script is the command; long ones truncate to fit the tag.
        let tag = tool_tag_for_part(&exec).unwrap();
        assert!(tag.starts_with("<:shell: const skill = await tools.shell_command("));
        assert!(tag.ends_with("…:>"));
        assert_eq!(part_body_text(&exec, SLICE), format!("$ {script}"));
    }

    #[test]
    fn unknown_tools_keep_the_generic_marker() {
        let unknown = call("wait", serde_json::json!({"cell_id": "1"}));
        assert_eq!(tool_tag_for_part(&unknown), None);
        assert!(part_body_text(&unknown, SLICE).contains("<:tool:wait call:>"));
    }

    #[test]
    fn long_expressions_are_truncated() {
        let long_command = "x".repeat(100);
        let action = shell(1, Some(&long_command), None);
        let tag = tool_tag_for_part(&action).unwrap();
        assert_eq!(
            tag.chars().count(),
            "<:shell: :>".chars().count() + MAX_EXPR + 1 // + "…"
        );
        assert!(tag.ends_with("…:>"));
    }
}
