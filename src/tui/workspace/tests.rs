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
use crate::tui::search::WorkspaceSearchScope;
use sivtr_core::agents::AgentProvider;
use sivtr_core::record::{
    MessageRole, WorkActionStatus, WorkActor, WorkAt, WorkContent, WorkPart, WorkPartBody,
    WorkRecord, WorkRef, WorkTarget,
};

fn part(seq: usize, body: WorkPartBody) -> WorkPart {
    WorkPart {
        seq,
        occurred_at: None,
        body,
    }
}

fn message(seq: usize, role: MessageRole, content: &str) -> WorkPart {
    crate::test_fixtures::message_part(seq, role, content)
}

fn user(seq: usize, content: &str) -> WorkPart {
    message(seq, MessageRole::User, content)
}

fn assistant(seq: usize, content: &str) -> WorkPart {
    message(seq, MessageRole::Assistant, content)
}

fn skill(seq: usize, name: &str, content: &str) -> WorkPart {
    part(
        seq,
        WorkPartBody::Message {
            role: MessageRole::System,
            label: Some(name.to_string()),
            content: WorkContent::Text {
                content: content.to_string(),
                ansi: None,
            },
        },
    )
}

fn shell_call(seq: usize, command: &str) -> WorkPart {
    let mut p = crate::test_fixtures::shell_action_part(seq, command, None);
    if let WorkPartBody::Action { status, .. } = &mut p.body {
        *status = WorkActionStatus::InProgress;
    }
    p
}

fn shell_done(seq: usize, command: &str, output: &str) -> WorkPart {
    crate::test_fixtures::shell_action_part(seq, command, Some(output))
}

fn tool_call(seq: usize, tool: &str, input: serde_json::Value) -> WorkPart {
    part(
        seq,
        WorkPartBody::Action {
            id: format!("tool-{seq}"),
            actor: WorkActor::Agent,
            target: WorkTarget::Tool {
                name: Some(tool.to_string()),
            },
            title: None,
            input: Some(WorkContent::Json(input)),
            output: Vec::new(),
            status: WorkActionStatus::InProgress,
            exit_code: None,
        },
    )
}

fn chat_record(parts: Vec<WorkPart>) -> WorkRecord {
    crate::test_fixtures::chat_record(1, parts)
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
    let record = chat_record(vec![user(1, "alpha"), assistant(2, "omega")]);
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
    let record = chat_record(vec![tool_call(
        1,
        "tool",
        serde_json::Value::String("hidden tool call".to_string()),
    )]);
    let dialogue = codex_dialogue(record);

    let text = workspace_content_text(&[dialogue], 0, ContentViewMode::Raw, Some(WorkAt::Part(1)));
    assert!(text.contains("<:tool:tool call:>"));
    assert!(text.contains("hidden tool call"));
    assert!(text.contains("<:/tool:tool call:>"));
}

#[test]
fn content_preview_text_uses_structured_targeted_part_text_in_reading_mode() {
    let record = chat_record(vec![tool_call(
        1,
        "tool",
        serde_json::Value::String("hidden tool call".to_string()),
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
        user(1, "question"),
        assistant(2, "answer"),
        tool_call(
            3,
            "tool",
            serde_json::Value::String("target body".to_string()),
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
        user(1, "question"),
        // One shell action carries the call and its result.
        shell_done(2, "cargo test", "ok"),
        assistant(3, "answer"),
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
    // Reading folds the agent action to its tag line; the payload drops.
    assert!(reading_io.output.contains("<:shell: cargo test:>"));
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
    // Shell output reads as the terminal transcript it is.
    assert!(raw_io.output.contains("ok"));
    assert!(raw_io.output.contains("answer"));
    assert!(!raw.contains("codex/session"));
    assert!(!raw.contains("## User"));
    assert!(!raw.contains("## Input"));
}

#[test]
fn reading_mode_collapses_adjacent_structure_runs() {
    let record = chat_record(vec![
        user(1, "do it"),
        shell_call(2, "ls"),
        tool_call(3, "Read", serde_json::json!({ "file_path": "file" })),
        skill(4, "review", "skill body"),
        skill(5, "deploy", "skill body 2"),
        assistant(6, "done"),
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
    // Adjacent agent actions fold into one run tag each; the skills carry
    // their names into the input half's run tag.
    let output = &reading_io.output;
    assert!(output.contains("<:shell, read:>"));
    assert!(!output.contains("<:shell: ls:>"));
    assert!(!output.contains("<:read: file:>"));
    let input = &reading_io.input;
    assert!(input.contains("<:review, deploy:>"));
    assert!(!input.contains("skill body"));
    assert!(reading_io.output.contains("done"));
    assert!(!reading.contains("file"));
    assert!(!reading.contains("skill body"));
    assert!(!reading.contains("## Input"));
}

#[test]
fn reading_mode_folds_consecutive_same_kind_runs() {
    let record = chat_record(vec![
        // Interleaved with dialogue — the output half still sees the agent
        // actions as one consecutive run.
        shell_call(1, "ls"),
        user(2, "middle note"),
        tool_call(3, "Read", serde_json::json!({ "file_path": "file" })),
        shell_call(4, "pwd"),
        shell_call(5, "date"),
    ]);
    let dialogue = codex_dialogue(record);

    let reading_io = workspace_content_io_texts(
        std::slice::from_ref(&dialogue),
        0,
        ContentViewMode::Reading,
        None,
        &ExpandedBlocks::default(),
    );
    // The four output-half agent actions fold into one run tag.
    assert!(reading_io.input.contains("middle note"));
    let output = &reading_io.output;
    assert!(output.contains("<:shell x3, read:>"));
    assert!(!output.contains("<:shell: ls:>"));
    assert!(!output.contains("file"));
    assert!(!output.contains("pwd"));
    assert!(!output.contains("date"));
}

#[test]
fn reading_mode_keeps_structure_runs_in_call_order() {
    let record = chat_record(vec![
        user(1, "do it"),
        assistant(2, "checking first"),
        shell_done(3, "ls", "ok"),
        assistant(4, "all done"),
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
    let tag = output.find("<:shell: ls:>").expect("tool tag");
    let last_text = output.find("all done").expect("last assistant text");
    assert!(first_text < tag);
    assert!(tag < last_text);
    // Payloads are dropped: the tag line mentions the command, the body
    // never shows.
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
        scope: WorkspaceSearchScope::Content,
        result_count: 1,
        current_match: Some(0),
        match_count: 1,
        current_target: Some("codex/session/1/4".to_string()),
        input_open: true,
    };

    assert_eq!(search_box_title(&search), "Search  ([1/1])");
    assert_eq!(
        search_box_body(&search),
        "needle\n\nTarget: codex/session/1/4"
    );
}
