//! Workspace unit tests.

use super::help::{help_action_for_key, parse_help_key, WorkspaceHelpAction};
use super::layout::can_open_dialogue_vim;
use super::model::{WorkspaceDialogue, WorkspaceFocus, WorkspaceSearchView, WorkspaceSource};
use super::render::{
    content_title, current_content_dialogue, current_content_ref, line_filter_prompt_text,
    search_box_body, search_box_title,
};
use crate::tui::content::io::ExpandedBlocks;
use crate::tui::content::text::{workspace_content_io_texts, workspace_content_text};
use crate::tui::content::view::ContentViewMode;
use sivtr_core::ai::AgentProvider;
use sivtr_core::record::{WorkAt, WorkRecord, WorkRef};

fn tool_test_value(text: String) -> serde_json::Value {
    serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
}

fn part(seq: usize, data: sivtr_core::record::WorkPartData) -> sivtr_core::record::WorkPart {
    sivtr_core::record::WorkPart {
        seq,
        occurred_at: None,
        data,
    }
}

fn chat_record(parts: Vec<sivtr_core::record::WorkPart>) -> WorkRecord {
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

fn codex_dialogue(record: WorkRecord) -> WorkspaceDialogue {
    WorkspaceDialogue {
        source: WorkspaceSource::agent(AgentProvider::Codex),
        work_ref: Some(record.work_ref.clone()),
        record: Some(record),
    }
}

#[test]
fn can_open_dialogue_vim_accepts_sessions_when_dialogues_exist() {
    assert!(can_open_dialogue_vim(WorkspaceFocus::Sessions, 1));
    assert!(can_open_dialogue_vim(WorkspaceFocus::Dialogues, 1));
    assert!(can_open_dialogue_vim(WorkspaceFocus::Content, 1));
    assert!(!can_open_dialogue_vim(WorkspaceFocus::Sessions, 0));
}

#[test]
fn content_preview_text_preserves_raw_text_without_line_number_prefixes() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "alpha".to_string(),
            },
        ),
        part(
            1,
            sivtr_core::record::WorkPartData::Assistant {
                content: "omega".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Raw,
        None,
        &ExpandedBlocks::default(),
    );
    let text = workspace_content_text(&[dialogue], 0, ContentViewMode::Raw, None);
    assert_eq!(io.input.trim(), "alpha");
    assert_eq!(io.output.trim(), "omega");
    assert!(text.contains("alpha"));
    assert!(text.contains("omega"));
    assert!(!text.contains("## Input"));
    assert!(!text.contains("[r expand]"));
}

#[test]
fn content_preview_text_uses_targeted_part_text_in_raw_mode() {
    let record = chat_record(vec![part(
        1,
        sivtr_core::record::WorkPartData::ToolCall {
            call_id: None,
            tool: Some("tool".to_string()),
            input: tool_test_value("hidden tool call".to_string()),
        },
    )]);
    let dialogue = codex_dialogue(record);

    let text = workspace_content_text(&[dialogue], 0, ContentViewMode::Raw, Some(WorkAt::Part(1)));
    assert!(text.contains("<:tool:tool call:>"));
    assert!(text.contains("hidden tool call"));
    assert!(text.contains("<:/tool:tool call:>"));
}

#[test]
fn content_preview_text_uses_structured_targeted_part_text_in_reading_mode() {
    let record = chat_record(vec![part(
        1,
        sivtr_core::record::WorkPartData::ToolCall {
            call_id: None,
            tool: Some("tool".to_string()),
            input: tool_test_value("hidden tool call".to_string()),
        },
    )]);
    let dialogue = codex_dialogue(record);

    let text = workspace_content_text(
        &[dialogue],
        0,
        ContentViewMode::Reading,
        Some(WorkAt::Part(1)),
    );

    // Reading folds structure to one open marker only.
    assert_eq!(text.trim(), "<:tool:tool call:>");
    assert!(!text.contains("hidden tool call"));
    assert!(!text.contains("codex/session"));
    assert!(!text.contains("[r expand]"));
}

#[test]
fn targeted_part_uses_its_dialogue_global_block_id() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "question".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::Assistant {
                content: "answer".to_string(),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("tool".to_string()),
                input: tool_test_value("target body".to_string()),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);
    let mut expanded = ExpandedBlocks::default();
    let folded =
        dialogue.content_io_texts(ContentViewMode::Reading, Some(WorkAt::Part(3)), &expanded);
    assert_eq!(
        crate::tui::content::block::dialogue_block_id(dialogue.record.as_ref().unwrap(), 3),
        Some(2)
    );
    assert_eq!(folded.output.trim(), "<:tool:tool call:>");
    expanded.toggle(2);
    let open =
        dialogue.content_io_texts(ContentViewMode::Reading, Some(WorkAt::Part(3)), &expanded);
    assert!(open.output.contains("target body"));
}

#[test]
fn reading_mode_folds_structure_and_raw_expands() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "question".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("cargo test".to_string()),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolResult {
                call_id: None,
                tool: Some("Bash".to_string()),
                output: tool_test_value("ok".to_string()),
                start_line: None,
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::Assistant {
                content: "answer".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading = workspace_content_text(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
    );
    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
        &ExpandedBlocks::default(),
    );
    assert!(reading_io.input.contains("question"));
    // Reading folds each structure group to its tag line; the call+result
    // pair collapses to one tag, so the result tag and payload are dropped.
    assert!(reading_io.output.contains("<:bash: cargo test:>"));
    assert!(!reading_io.output.contains("<:tool:Bash result:>"));
    assert!(reading_io.output.contains("answer"));
    assert!(!reading.contains("$ cargo test"));
    assert!(!reading.contains("ok"));
    assert!(!reading.contains("codex/session"));
    assert!(!reading.contains("## User"));
    assert!(!reading.contains("## Input"));
    assert!(!reading.contains("[r expand]"));

    let raw_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Raw,
        None,
        &ExpandedBlocks::default(),
    );
    let raw = workspace_content_text(&[dialogue], 0, ContentViewMode::Raw, None);
    assert!(raw_io.input.contains("question"));
    assert!(raw_io.output.contains("cargo test"));
    assert!(raw_io.output.contains("$ cargo test"));
    // Known results use the `>` output format.
    assert!(raw_io.output.contains("> ok"));
    assert!(raw_io.output.contains("ok"));
    assert!(raw_io.output.contains("answer"));
    assert!(!raw.contains("codex/session"));
    assert!(!raw.contains("## User"));
    assert!(!raw.contains("## Input"));
}

#[test]
fn reading_mode_collapses_adjacent_structure_runs() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "do it".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("ls".to_string()),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Read".to_string()),
                input: tool_test_value("file".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::Skill {
                skill: Some("review".to_string()),
                content: "skill body".to_string(),
            },
        ),
        part(
            5,
            sivtr_core::record::WorkPartData::Skill {
                skill: Some("deploy".to_string()),
                content: "skill body 2".to_string(),
            },
        ),
        part(
            1,
            sivtr_core::record::WorkPartData::Assistant {
                content: "done".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
        &ExpandedBlocks::default(),
    );
    let reading = workspace_content_text(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
    );
    assert!(reading_io.input.contains("do it"));
    // Adjacent same-kind structure parts fold into one run tag each.
    let output = &reading_io.output;
    assert!(output.contains("<:bash, read:>"));
    assert!(!output.contains("<:tool:Bash call:>"));
    assert!(!output.contains("<:tool:Read call:>"));
    let input = &reading_io.input;
    assert!(input.contains("<:skill x2:>"));
    assert!(!input.contains("<:skill:review:>"));
    assert!(!input.contains("<:skill:deploy:>"));
    assert!(reading_io.output.contains("done"));
    assert!(!reading.contains("file"));
    assert!(!reading.contains("skill body"));
    assert!(!reading.contains("## Input"));
}

#[test]
fn reading_mode_folds_consecutive_same_kind_runs() {
    let record = chat_record(vec![
        // Interleaved with dialogue — the output half still sees the tool
        // calls as one consecutive run.
        part(
            1,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("ls".to_string()),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::User {
                content: "middle note".to_string(),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Read".to_string()),
                input: tool_test_value("file".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("pwd".to_string()),
            },
        ),
        part(
            5,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("date".to_string()),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
        &ExpandedBlocks::default(),
    );
    // The four output-half tool calls fold into one run tag.
    assert!(reading_io.input.contains("middle note"));
    let output = &reading_io.output;
    assert!(output.contains("<:bash x3, read:>"));
    assert!(!output.contains("<:tool:Bash call:>"));
    assert!(!output.contains("file"));
    assert!(!output.contains("pwd"));
    assert!(!output.contains("date"));
}

#[test]
fn reading_mode_keeps_structure_runs_in_call_order() {
    let record = chat_record(vec![
        part(
            1,
            sivtr_core::record::WorkPartData::User {
                content: "do it".to_string(),
            },
        ),
        part(
            2,
            sivtr_core::record::WorkPartData::Assistant {
                content: "checking first".to_string(),
            },
        ),
        part(
            3,
            sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("Bash".to_string()),
                input: tool_test_value("ls".to_string()),
            },
        ),
        part(
            4,
            sivtr_core::record::WorkPartData::ToolResult {
                call_id: None,
                tool: Some("Bash".to_string()),
                output: tool_test_value("ok".to_string()),
                start_line: None,
            },
        ),
        part(
            5,
            sivtr_core::record::WorkPartData::Assistant {
                content: "all done".to_string(),
            },
        ),
    ]);
    let dialogue = codex_dialogue(record);

    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
        &ExpandedBlocks::default(),
    );
    // Tags sit between the assistant chunks, matching the call order.
    let output = &reading_io.output;
    let first_text = output.find("checking first").expect("first assistant text");
    let tag = output.find("<:bash: ls:>").expect("tool tag");
    let last_text = output.find("all done").expect("last assistant text");
    assert!(first_text < tag);
    assert!(tag < last_text);
    // Payloads are dropped: the tag line mentions result, the body never shows.
    assert!(!output.contains("ok"));
}

#[test]
fn content_title_includes_view_mode() {
    assert_eq!(
        content_title(ContentViewMode::Reading, 0, None),
        "Content (read)"
    );
    assert_eq!(
        content_title(ContentViewMode::Raw, 1, None),
        "Content (raw): 1 dialogue selected"
    );
}

#[test]
fn content_title_includes_current_dialogue_ref() {
    let work_ref = WorkRef::agent(AgentProvider::Codex, "session", 2);

    assert_eq!(
        content_title(ContentViewMode::Reading, 0, Some(&work_ref)),
        "Content (read) [codex/session/2]"
    );
}

#[test]
fn line_filter_prompt_text_shows_current_input() {
    let prompt = line_filter_prompt_text(Some("2:8"), None, true);
    assert!(prompt.contains("2:8"));
    assert!(prompt.contains("Enter keeps displayed lines."));
}

#[test]
fn line_filter_prompt_text_shows_error_and_current_value() {
    let prompt = line_filter_prompt_text(Some("23"), Some("Invalid line number"), false);
    assert!(prompt.contains("Invalid line number"));
    assert!(prompt.contains("Current: 23"));
}

#[test]
fn parse_help_key_recognizes_named_and_ctrl_specs() {
    use crossterm::event::{KeyCode, KeyModifiers};
    assert_eq!(
        parse_help_key("Tab"),
        Some((KeyCode::Tab, KeyModifiers::NONE))
    );
    assert_eq!(
        parse_help_key("Ctrl-d"),
        Some((KeyCode::Char('d'), KeyModifiers::CONTROL))
    );
    assert_eq!(
        parse_help_key("Space"),
        Some((KeyCode::Char(' '), KeyModifiers::NONE))
    );
    assert_eq!(
        parse_help_key("PgDn"),
        Some((KeyCode::PageDown, KeyModifiers::NONE))
    );
}

#[test]
fn help_action_for_key_is_focus_scoped() {
    use crossterm::event::{KeyCode, KeyModifiers};
    assert_eq!(
        help_action_for_key(KeyCode::Tab, KeyModifiers::NONE, WorkspaceFocus::Content),
        Some(WorkspaceHelpAction::ToggleContentIo)
    );
    assert_eq!(
        help_action_for_key(KeyCode::Tab, KeyModifiers::NONE, WorkspaceFocus::Source),
        None
    );
    // Source-only binding does not fire on Content.
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            WorkspaceFocus::Source
        ),
        Some(WorkspaceHelpAction::SelectAgentSources)
    );
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
            WorkspaceFocus::Content
        ),
        Some(WorkspaceHelpAction::ScrollContentTop)
    );
    // Ctrl-d is scroll, bare d is not.
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
            WorkspaceFocus::Content
        ),
        Some(WorkspaceHelpAction::ScrollDown)
    );
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
            WorkspaceFocus::Content
        ),
        None
    );
    assert_eq!(
        help_action_for_key(
            KeyCode::Char('p'),
            KeyModifiers::NONE,
            WorkspaceFocus::Dialogues
        ),
        Some(WorkspaceHelpAction::Publish)
    );
    for focus in [
        WorkspaceFocus::Source,
        WorkspaceFocus::Sessions,
        WorkspaceFocus::Dialogues,
        WorkspaceFocus::Content,
    ] {
        assert_eq!(
            help_action_for_key(KeyCode::Char('a'), KeyModifiers::NONE, focus),
            Some(WorkspaceHelpAction::ToggleAll)
        );
    }
}

#[test]
fn current_content_dialogue_uses_single_selected_dialogue() {
    let dialogues = vec![
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            record: None,
        },
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 2)),
            record: None,
        },
    ];

    let current = current_content_dialogue(&dialogues, &[false, true], 0).unwrap();

    assert_eq!(
        current.work_ref.as_ref().unwrap().to_string(),
        "codex/session/2"
    );
}

#[test]
fn current_content_ref_round_trips_active_part_target() {
    let dialogues = vec![WorkspaceDialogue {
        source: WorkspaceSource::agent(AgentProvider::Codex),
        work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 2)),
        record: None,
    }];

    let current = current_content_ref(&dialogues, &[false], 0, Some(WorkAt::Part(1))).unwrap();

    assert_eq!(current.to_string(), "codex/session/2/p1");
}

#[test]
fn search_box_body_includes_current_target_ref() {
    let search = WorkspaceSearchView {
        query: "needle",
        result_count: 1,
        current_match: Some(0),
        match_count: 1,
        current_target: Some("codex/session/1/4".to_string()),
        regex: None,
        error: None,
        input_open: true,
    };

    assert_eq!(search_box_title(&search), "Search  ([1/1])");
    assert_eq!(
        search_box_body(&search),
        "needle\n\nTarget: codex/session/1/4"
    );
}
