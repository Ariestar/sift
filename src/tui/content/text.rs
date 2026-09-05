//! Part → dual Input/Output display text (per-block fold in reading mode).

use sivtr_core::record::{WorkAt, WorkRecord};

use crate::tui::content::block::{dialogue_blocks, render_half};
use crate::tui::content::io::{ContentIoTexts, ExpandedBlocks};
use crate::tui::content::view::ContentViewMode;
use crate::tui::workspace::model::WorkspaceDialogue;

/// Read mode folds every block to its `<:…:>` tag (structure blocks by
/// default, body blocks only when flipped); blocks listed in `expanded`
/// show their full body instead. Raw mode always shows full blocks (the
/// expand state only affects reading). Block ids are dialogue-global, so
/// the fold state spans the input/output boundary.
pub(crate) fn content_io_from_record(
    record: &WorkRecord,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> ContentIoTexts {
    let (input_blocks, output_blocks) = dialogue_blocks(record);
    ContentIoTexts::new(
        render_half(record, &input_blocks, reading, expanded),
        render_half(record, &output_blocks, reading, expanded),
    )
}

pub(crate) fn workspace_content_text(
    dialogues: &[WorkspaceDialogue],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
) -> String {
    workspace_content_io_texts(
        dialogues,
        highlighted_idx,
        mode,
        target,
        &ExpandedBlocks::default(),
    )
    .join_displayed()
}

/// Input / Output bodies for the dual content panes with per-block fold
/// state. Every workpart is a block; the segments stay attached to the pane
/// text so the content view can map displayed lines back to their block.
/// Multi-selection is paged: multi-selected dialogues render one at a time,
/// so the caller passes the index of the dialogue shown on the current page.
pub(crate) fn workspace_content_io_texts(
    dialogues: &[WorkspaceDialogue],
    highlighted_idx: usize,
    mode: ContentViewMode,
    target: Option<WorkAt>,
    expanded: &ExpandedBlocks,
) -> ContentIoTexts {
    dialogues
        .get(highlighted_idx)
        .map(|dialogue| dialogue.content_io_texts(mode, target, expanded))
        .unwrap_or_else(|| ContentIoTexts::new(Vec::new(), Vec::new()))
}

/// A line that opens or closes a structure block (`<:tool:…:>`,
/// `<:skill:…:>`, `<:thinking:>`, or the generic `<:structure:>` fallback).
/// Validates the full marker shape, not just the `<:` prefix, so plain text
/// that happens to start with `<:` is not treated as a marker.
pub(crate) fn is_structure_marker(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix("<:")
        .and_then(|rest| rest.strip_suffix(":>"))
    else {
        return false;
    };
    let kind = inner
        .strip_prefix('/')
        .unwrap_or(inner)
        .split(':')
        .next()
        .unwrap_or_default();
    matches!(kind, "tool" | "skill" | "thinking" | "structure")
}

#[cfg(test)]
mod tests {
    use super::content_io_from_record;
    use crate::tui::content::io::ExpandedBlocks;
    use sivtr_core::agents::AgentProvider;
    use sivtr_core::record::{WorkPart, WorkPartData, WorkRecord, WorkRef};

    fn tool_part(seq: usize, tool: &str, input: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some(tool.to_string()),
                input: serde_json::json!({ "command": input }),
            },
        }
    }

    fn tool_result_part(seq: usize, tool: &str, output: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: None,
                tool: Some(tool.to_string()),
                output: serde_json::json!({ "stdout": output }),
                start_line: None,
            },
        }
    }

    fn record(parts: Vec<WorkPart>) -> WorkRecord {
        WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: sivtr_core::record::WorkRecordKind::ChatTurn,
            source: sivtr_core::record::WorkSource {
                channel: sivtr_core::record::WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: sivtr_core::record::WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: sivtr_core::record::WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts,
        }
    }

    #[test]
    fn reading_folds_runs_and_expanding_reveals_member_tags() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        // The two consecutive tool calls fold into one run tag.
        assert!(output.contains("<:bash, read:>"));
        assert!(!output.contains("ls"));
        assert!(!output.contains("file"));

        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        let io = content_io_from_record(&rec, true, &expanded);
        let output = &io.output;
        // The run opens to its member tags; bodies stay folded.
        assert!(output.contains("<:bash: ls:>"));
        assert!(output.contains("<:tool:Read call:>"));
        assert!(!output.contains("$ ls"));
        assert!(!output.contains("file"));

        // Opening a member shows its body.
        expanded.toggle(1);
        let io = content_io_from_record(&rec, true, &expanded);
        assert!(io.output.contains("$ ls"));
    }

    #[test]
    fn raw_mode_ignores_expand_state() {
        let rec = record(vec![tool_part(1, "Bash", "ls")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        let io = content_io_from_record(&rec, false, &expanded);
        assert_eq!(io.output, "$ ls");
    }

    #[test]
    fn tool_call_and_result_fold_into_one_run() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Bash", "ok"),
            tool_part(3, "Read", "file"),
        ]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        // Collapsed: the call+result pair and the Read call fold into one
        // run tag; the result payload stays hidden.
        assert!(output.contains("<:bash, read:>"));
        assert!(!output.contains("<:tool:Bash result:>"));
        assert!(!output.contains("ok"));
        assert!(!output.contains("file"));

        // Expanding the run reveals member tags on adjacent lines (no blank
        // line between); the result payload stays folded.
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        let io = content_io_from_record(&rec, true, &expanded);
        let output = &io.output;
        // The call+result pair is one member; its collapsed tag is the call
        // tag, so the result marker only appears once the member opens.
        assert!(output.contains("<:bash: ls:>"));
        assert!(output.contains("<:tool:Read call:>"));
        assert!(!output.contains("<:tool:Bash result:>"));
        assert!(!output.contains("ok"));
        assert!(!output.contains("file"));

        // Opening the result member shows its payload.
        expanded.toggle(1);
        let io = content_io_from_record(&rec, true, &expanded);
        assert!(io.output.contains("ok"));
    }

    #[test]
    fn fold_label_uses_input_expression_over_description() {
        let rec = record(vec![WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: serde_json::json!({
                    "command": "git diff",
                    "description": "Review working tree",
                }),
            },
        }]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        // The input expression replaces the description in the tag.
        assert!(io.output.contains("<:bash: git diff:>"));
        assert!(!io.output.contains("Review working tree"));

        // A plain tool part still folds to its expression tag.
        let rec = record(vec![tool_part(1, "Bash", "ls")]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        assert!(io.output.contains("<:bash: ls:>"));
    }

    #[test]
    fn fold_label_normalizes_multiline_descriptions_to_one_tag_line() {
        // An unknown tool takes the generic marker path, where the
        // description lands in the tag.
        let part = WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some("UnknownTool".to_string()),
                input: serde_json::json!({
                    "command": "git diff",
                    "description": "line one\nline two",
                }),
            },
        };
        let tag = crate::tui::content::block::fold_label_for_part(&part);
        // The tag stays a single line: internal whitespace collapses.
        assert_eq!(tag.lines().count(), 1);
        assert!(tag.contains("line one line two"));
    }

    #[test]
    fn id_less_call_and_result_group_only_for_the_same_tool() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Read", "ok"),
        ]);
        // Different tools without call ids stay separate blocks: the run
        // keeps the result out of the call's leaf.
        let blocks = crate::tui::content::block::half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);

        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Bash", "ok"),
        ]);
        // The same tool without call ids pairs into one leaf.
        let blocks = crate::tui::content::block::half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].children.is_empty());
        assert_eq!(blocks[0].parts, vec![0, 1]);
    }

    #[test]
    fn distinct_call_ids_never_group() {
        let rec = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("a".to_string()),
                    tool: Some("Bash".to_string()),
                    input: serde_json::json!({ "command": "ls" }),
                },
            },
            WorkPart {
                seq: 2,
                occurred_at: None,
                data: WorkPartData::ToolResult {
                    call_id: Some("b".to_string()),
                    tool: Some("Bash".to_string()),
                    output: serde_json::json!({ "stdout": "ok" }),
                    start_line: None,
                },
            },
        ]);
        // Distinct call ids never merge the result into the call's leaf.
        let blocks = crate::tui::content::block::half_blocks(&rec, false);
        assert_eq!(blocks[0].children.len(), 2);
    }
}
