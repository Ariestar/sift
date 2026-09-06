//! Part → dual Input/Output display text (per-block fold in reading mode).

use sivtr_core::record::{Projection, WorkAt, WorkRecord};

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
        render_half(record, &input_blocks, Projection::Input, reading, expanded),
        render_half(
            record,
            &output_blocks,
            Projection::Output,
            reading,
            expanded,
        ),
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
    use sivtr_core::record::{
        MessageRole, WorkActionStatus, WorkActor, WorkChannel, WorkContent, WorkContentBlock,
        WorkPart, WorkPartBody, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource,
        WorkTarget, WorkTime,
    };

    fn shell_action(seq: usize, command: &str, output: Option<&str>) -> WorkPart {
        crate::test_fixtures::shell_action_part(seq, command, output)
    }

    fn read_action(seq: usize, path: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            body: WorkPartBody::Action {
                id: format!("read-{seq}"),
                actor: WorkActor::Agent,
                target: WorkTarget::Tool {
                    name: Some("Read".to_string()),
                },
                title: None,
                input: Some(WorkContent::Json(serde_json::json!({ "file_path": path }))),
                output: Vec::new(),
                status: WorkActionStatus::InProgress,
                exit_code: None,
            },
        }
    }

    fn user_message(seq: usize, content: &str) -> WorkPart {
        crate::test_fixtures::message_part(seq, MessageRole::User, content)
    }

    fn record(parts: Vec<WorkPart>) -> WorkRecord {
        WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts,
        }
    }

    #[test]
    fn reading_folds_runs_and_expanding_reveals_member_tags() {
        let rec = record(vec![shell_action(1, "ls", None), read_action(2, "file")]);
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        let output = &io.output;
        // The two consecutive agent actions fold into one run tag.
        assert!(output.contains("<:shell, read:>"));
        assert!(!output.contains("ls"));
        assert!(!output.contains("file"));

        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        let io = content_io_from_record(&rec, true, &expanded);
        let output = &io.output;
        // The run opens to its member tags; bodies stay folded.
        assert!(output.contains("<:shell: ls:>"));
        assert!(output.contains("<:read: file:>"));
        assert!(!output.contains("$ ls"));

        // Opening a member shows its body.
        expanded.toggle(1);
        let io = content_io_from_record(&rec, true, &expanded);
        assert!(io.output.contains("$ ls"));
    }

    #[test]
    fn raw_mode_ignores_expand_state() {
        let rec = record(vec![shell_action(1, "ls", None)]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        let io = content_io_from_record(&rec, false, &expanded);
        assert_eq!(io.output, "$ ls");
    }

    #[test]
    fn shell_action_renders_command_then_output() {
        // One action carries the whole lifecycle: call line and result text
        // read as one terminal transcript.
        let rec = record(vec![shell_action(1, "ls", Some("src\ntarget"))]);
        let io = content_io_from_record(&rec, false, &ExpandedBlocks::default());
        assert_eq!(io.output, "$ ls\nsrc\ntarget");
    }

    #[test]
    fn dialogue_and_human_commands_land_in_the_input_half() {
        let rec = record(vec![
            user_message(1, "run the build"),
            WorkPart {
                seq: 2,
                occurred_at: None,
                body: WorkPartBody::Action {
                    id: "term-1".to_string(),
                    actor: WorkActor::User,
                    target: WorkTarget::Shell,
                    title: None,
                    input: Some(WorkContent::Text {
                        content: "cargo build".to_string(),
                        ansi: None,
                    }),
                    output: vec![WorkContentBlock {
                        content: WorkContent::Text {
                            content: "compiled".to_string(),
                            ansi: None,
                        },
                        start_line: None,
                    }],
                    status: WorkActionStatus::Completed,
                    exit_code: Some(0),
                },
            },
        ]);
        let io = content_io_from_record(&rec, false, &ExpandedBlocks::default());
        // Input half: the user message and the human command…
        assert!(io.input.contains("run the build"));
        assert!(io.input.contains("$ cargo build"));
        // …and the command's output belongs to the output half.
        assert!(io.output.contains("compiled"));
    }

    #[test]
    fn fold_label_uses_the_expression_over_the_context_line() {
        let mut rec = record(vec![shell_action(1, "git diff", None)]);
        rec.parts[0].body = WorkPartBody::Action {
            id: "shell-1".to_string(),
            actor: WorkActor::Agent,
            target: WorkTarget::Shell,
            title: Some("Review working tree".to_string()),
            input: Some(WorkContent::Text {
                content: "git diff".to_string(),
                ansi: None,
            }),
            output: Vec::new(),
            status: WorkActionStatus::InProgress,
            exit_code: None,
        };
        let io = content_io_from_record(&rec, true, &ExpandedBlocks::default());
        // The expression replaces the context line in the tag.
        assert!(io.output.contains("<:shell: git diff:>"));
        assert!(!io.output.contains("Review working tree"));
    }

    #[test]
    fn fold_label_normalizes_multiline_context_to_one_tag_line() {
        // An unknown tool takes the generic marker path, where the context
        // line lands in the tag.
        let mut action = read_action(1, "a.rs");
        action.body = WorkPartBody::Action {
            id: "a1".to_string(),
            actor: WorkActor::Agent,
            target: WorkTarget::Tool {
                name: Some("UnknownTool".to_string()),
            },
            title: Some("line one\nline two".to_string()),
            input: Some(WorkContent::Json(
                serde_json::json!({ "command": "git diff" }),
            )),
            output: Vec::new(),
            status: WorkActionStatus::InProgress,
            exit_code: None,
        };
        let tag = crate::tui::content::block::fold_label_for_part(&action);
        // The tag stays a single line: internal whitespace collapses.
        assert_eq!(tag.lines().count(), 1);
        assert!(tag.contains("line one line two"));
    }

    #[test]
    fn separate_call_and_result_parts_stay_two_run_members() {
        // Pairing happens once, in the core reducer, keyed by the action id.
        // Parts that reach the TUI unpaired never merge by tool name.
        let rec = record(vec![
            read_action(1, "a.rs"),
            WorkPart {
                seq: 2,
                occurred_at: None,
                body: WorkPartBody::Action {
                    id: "other".to_string(),
                    actor: WorkActor::Agent,
                    target: WorkTarget::Tool {
                        name: Some("Read".to_string()),
                    },
                    title: None,
                    input: None,
                    output: vec![WorkContentBlock {
                        content: WorkContent::Json(serde_json::json!("ok")),
                        start_line: None,
                    }],
                    status: WorkActionStatus::Completed,
                    exit_code: None,
                },
            },
        ]);
        let blocks = crate::tui::content::block::half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
    }
}
