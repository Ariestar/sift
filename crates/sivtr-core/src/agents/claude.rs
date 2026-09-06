use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, list_recent_jsonl_sessions, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, AgentBlockKind, AgentProvider, AgentSession, AgentSessionMeta,
    AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Claude";

#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeProvider;

impl AgentSessionProvider for ClaudeProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Claude
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_recent_jsonl_sessions(
            PROVIDER_NAME,
            &claude_home().join("projects"),
            cwd,
            parse_session_meta,
        )
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

pub fn claude_home() -> PathBuf {
    if let Ok(path) = std::env::var("CLAUDE_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 50, update_meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value) {
    if meta.id.is_none() {
        meta.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
        meta.add_cwd(cwd);
    }
    if meta.title.is_none() {
        meta.title = event_title(value);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    update_session_meta(session, value);

    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return;
    }

    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);

    match value.get("type").and_then(Value::as_str) {
        Some("user") => apply_message(session, value, AgentBlockKind::User, timestamp),
        Some("assistant") => apply_message(session, value, AgentBlockKind::Assistant, timestamp),
        _ => {}
    }
}

fn update_session_meta(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(title) = event_title(value) {
        session.title = Some(title);
    }
}

fn event_title(value: &Value) -> Option<String> {
    value
        .get("customTitle")
        .or_else(|| value.get("title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
}

fn apply_message(
    session: &mut AgentSession,
    value: &Value,
    kind: AgentBlockKind,
    timestamp: Option<String>,
) {
    let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
    else {
        return;
    };
    push_content_blocks(session, kind, timestamp, content);
}

fn push_content_blocks(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    match content {
        Value::Array(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => push_text_content_blocks(
                        session,
                        kind,
                        timestamp.clone(),
                        extract_content_text(item),
                    ),
                    Some("thinking") => {
                        let thinking = item
                            .get("thinking")
                            .map(extract_content_text)
                            .filter(|text| !text.trim().is_empty())
                            .unwrap_or_else(|| extract_content_text(item));
                        push_block(
                            session,
                            AgentBlockKind::Thinking,
                            timestamp.clone(),
                            None,
                            thinking,
                        );
                    }
                    Some("tool_use") => push_tool_use(session, timestamp.clone(), item),
                    Some("tool_result") => {
                        if let Some(content) = item.get("content") {
                            push_block(
                                session,
                                AgentBlockKind::ToolOutput,
                                timestamp.clone(),
                                None,
                                extract_content_text(content),
                            );
                        }
                    }
                    Some("image") => {}
                    _ => {}
                }
            }
        }
        Value::String(text) => push_text_content_blocks(session, kind, timestamp, text.clone()),
        _ => {}
    }
}

fn push_tool_use(session: &mut AgentSession, timestamp: Option<String>, item: &Value) {
    if let Some(input) = item.get("input") {
        push_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            item.get("name").and_then(Value::as_str).map(str::to_string),
            pretty_json_value(input),
        );
    }
}

fn push_text_content_blocks(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    text: String,
) {
    if kind == AgentBlockKind::Assistant {
        push_assistant_text_blocks(session, timestamp, &text);
    } else {
        push_block(session, kind, timestamp, None, text);
    }
}

fn push_assistant_text_blocks(session: &mut AgentSession, timestamp: Option<String>, text: &str) {
    let mut rest = text;

    rest = skip_completed_tool_close_tags(rest);
    while let Some((start, tool_kind)) = find_next_embedded_tool(rest) {
        push_block(
            session,
            AgentBlockKind::Assistant,
            timestamp.clone(),
            None,
            &rest[..start],
        );

        let parsed = parse_embedded_tool_span(&rest[start..], tool_kind);
        if let Some(body) = tool_child_body(parsed.inner, tool_kind) {
            push_block(
                session,
                tool_kind.block_kind(),
                timestamp.clone(),
                parsed.name,
                body,
            );
        } else {
            push_block(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                None,
                &rest[start..start + parsed.consumed],
            );
        }

        rest = &rest[start + parsed.consumed..];
        rest = skip_completed_tool_close_tags(rest);
    }

    rest = skip_completed_tool_close_tags(rest);
    push_block(session, AgentBlockKind::Assistant, timestamp, None, rest);
}

#[derive(Clone, Copy)]
enum EmbeddedToolKind {
    ToolUse,
    ToolResult,
}

impl EmbeddedToolKind {
    fn open_tag(self) -> &'static str {
        match self {
            Self::ToolUse => "<tool_use",
            Self::ToolResult => "<tool_result",
        }
    }

    fn close_tag(self) -> &'static str {
        match self {
            Self::ToolUse => "</tool_use>",
            Self::ToolResult => "</tool_result>",
        }
    }

    fn child_tag(self) -> &'static str {
        match self {
            Self::ToolUse => "tool_input",
            Self::ToolResult => "tool_output",
        }
    }

    fn block_kind(self) -> AgentBlockKind {
        match self {
            Self::ToolUse => AgentBlockKind::ToolCall,
            Self::ToolResult => AgentBlockKind::ToolOutput,
        }
    }
}

struct EmbeddedTool<'a> {
    name: Option<String>,
    inner: &'a str,
    consumed: usize,
}

fn find_next_embedded_tool(text: &str) -> Option<(usize, EmbeddedToolKind)> {
    [EmbeddedToolKind::ToolUse, EmbeddedToolKind::ToolResult]
        .into_iter()
        .filter_map(|kind| text.find(kind.open_tag()).map(|index| (index, kind)))
        .min_by_key(|(index, _)| *index)
}

fn parse_embedded_tool_span<'a>(text: &'a str, tool_kind: EmbeddedToolKind) -> EmbeddedTool<'a> {
    let open_end = text
        .find('>')
        .expect("Claude embedded tool tag must have an opening delimiter");
    let opening = &text[..=open_end];
    let body_start = open_end + 1;
    let close = tool_kind.close_tag();
    let close_start = text[body_start..]
        .find(close)
        .map(|offset| body_start + offset);
    let next_tool_start =
        find_next_embedded_tool(&text[body_start..]).map(|(offset, _)| body_start + offset);
    let body_end = match (close_start, next_tool_start) {
        (Some(close_start), Some(next_tool_start)) => close_start.min(next_tool_start),
        (Some(close_start), None) => close_start,
        (None, Some(next_tool_start)) => next_tool_start,
        (None, None) => text.len(),
    };
    let consumed = if close_start == Some(body_end) {
        body_end + close.len()
    } else {
        body_end
    };

    EmbeddedTool {
        name: tool_name_attr(opening),
        inner: &text[body_start..body_end],
        consumed,
    }
}

fn skip_completed_tool_close_tags(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim_start();
        let mut stripped = false;

        for close in [
            "</tool_input>",
            "</tool_output>",
            "</tool_use>",
            "</tool_result>",
        ] {
            if let Some(rest) = trimmed.strip_prefix(close) {
                text = rest;
                stripped = true;
                break;
            }
        }

        if !stripped {
            return text;
        }
    }
}

fn tool_name_attr(opening: &str) -> Option<String> {
    let rest = opening.split_once("name=\"")?.1;
    let name = rest.split_once('"')?.0.trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn tool_child_body(inner: &str, tool_kind: EmbeddedToolKind) -> Option<String> {
    let child = tool_kind.child_tag();
    let open_start = inner.find(&format!("<{child}"))?;
    let open_end = inner[open_start..]
        .find('>')
        .map(|offset| open_start + offset)?;
    let close = format!("</{child}>");
    let body_start = open_end + 1;
    let close_start = match inner[body_start..].find(&close) {
        Some(offset) => body_start + offset,
        None => inner.len(),
    };

    Some(inner[body_start..close_start].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::ClaudeProvider;
    use crate::agents::{select_blocks, AgentBlockKind, AgentSelection, AgentSessionProvider};
    use std::env;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env_lock()
    }

    #[test]
    fn tracks_later_claude_cwd_events_in_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"/home/jacob","customTitle":"Named session","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"abc","cwd":"/home/jacob/ref/oh-my-ppt-fork","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let meta = super::parse_session_meta(&path).unwrap();

        assert_eq!(meta.id.as_deref(), Some("abc"));
        assert_eq!(meta.cwd.as_deref(), Some("/home/jacob"));
        assert_eq!(meta.title.as_deref(), Some("Named session"));
        assert!(meta
            .cwd_history
            .iter()
            .any(|cwd| cwd == "/home/jacob/ref/oh-my-ppt-fork"));
    }

    #[test]
    fn parses_claude_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:00Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"I will check."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi"}}]}}
{"type":"user","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"hi"}]}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","timestamp":"2026-05-01T00:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
    }

    #[test]
    fn ignores_claude_ai_title_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","message":{"role":"user","content":"hello"}}
{"type":"ai-title","sessionId":"abc","cwd":"C:\\repo","title":"Short title"}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text.trim(), "hello");
        assert_eq!(session.blocks[1].text.trim(), "done");
    }

    #[test]
    fn ignores_claude_non_dialogue_events_and_keeps_custom_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"custom-title","sessionId":"abc","customTitle":"Named session"}
{"type":"permission-mode","sessionId":"abc","permissionMode":"default"}
{"type":"user","sessionId":"abc","cwd":"C:\\repo","message":{"role":"user","content":"hello"}}
{"type":"pr-link","sessionId":"abc","prNumber":19,"prUrl":"https://github.com/Ariestar/sivtr/pull/19","prRepository":"Ariestar/sivtr","timestamp":"2026-05-23T03:25:11.176Z"}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.title.as_deref(), Some("Named session"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text.trim(), "hello");
        assert_eq!(session.blocks[1].text.trim(), "done");
    }

    #[test]
    fn ignores_unknown_claude_content_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"citation","text":"ignored"},{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "done");
    }

    #[test]
    fn parses_claude_embedded_tool_markup_from_text_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"abc","cwd":"C:\\repo","message":{"role":"user","content":"review"}}
{"type":"assistant","sessionId":"abc","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk ls\"}</tool_input>\n</tool_use><tool_result name=\"Bash\">\n  <tool_output>ok</tool_output>\n</tool_result>\nfinal answer"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "hidden");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("Bash"));
        assert_eq!(session.blocks[2].text, "{\"command\":\"rtk ls\"}");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].text, "ok");
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[4].text, "final answer");
    }

    #[test]
    fn parses_truncated_embedded_tool_output_at_next_tool_tag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"before<tool_result name=\"Bash\">\n  <tool_output>partial output\nNeed next.<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk test\"}</tool_input>\n</tool_use>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "before");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert!(session.blocks[2].text.contains("partial output"));
        assert!(!session.blocks[2].text.contains("<tool_result"));
        assert!(!session.blocks[2].text.contains("<tool_output"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[4].text, "after");
    }

    #[test]
    fn splits_embedded_tool_when_next_tool_tag_precedes_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"<tool_result name=\"Bash\">\n  <tool_output>partial output<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk test\"}</tool_input>\n</tool_use></tool_result>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[1].text, "partial output");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].text, "{\"command\":\"rtk test\"}");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "after");
    }

    #[test]
    fn parses_embedded_tool_input_without_child_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"<tool_use name=\"Bash\">\n  <tool_input>{\"command\":\"rtk test\"}\n</tool_use>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].text, "{\"command\":\"rtk test\"}");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "after");
    }

    #[test]
    fn keeps_embedded_tool_without_required_child_as_assistant_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"review"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"before<tool_use name=\"Bash\">plain command</tool_use>after"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "before");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(
            session.blocks[2].text,
            "<tool_use name=\"Bash\">plain command</tool_use>"
        );
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "after");
    }

    #[test]
    fn skips_claude_meta_user_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"hidden command caveat"}}
{"type":"user","message":{"role":"user","content":"real task"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "real task");
        assert_eq!(session.blocks[1].text, "done");
    }

    #[test]
    fn skips_injected_user_envelopes_without_meta_flag() {
        // Background-task notifications arrive as plain user messages with no
        // isMeta flag; only the text identifies them as machine injections.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"<task-notification>\n<task-id>aa05</task-id>\n<output-file>C:\\tmp\\out</output-file>\n</task-notification>"}}
{"type":"user","message":{"role":"user","content":"<bash-input>cargo test</bash-input>\n<bash-stdout>ok</bash-stdout>"}}
{"type":"user","message":{"role":"user","content":"<command-name>/review</command-name>\n<command-message>review</command-message>"}}
{"type":"user","message":{"role":"user","content":"real follow-up"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "real follow-up");
        assert_eq!(session.blocks[1].text, "done");
    }

    #[test]
    fn selects_last_claude_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"first"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"old"}]}}
{"type":"user","message":{"role":"user","content":"second"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"new"}]}}
"#,
        )
        .unwrap();
        let session = ClaudeProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastTurn);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text.trim(), "second");
        assert_eq!(blocks[1].text.trim(), "new");
    }

    #[test]
    fn skips_malformed_unrelated_sessions_during_listing() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let original_claude_home = env::var_os("CLAUDE_HOME");
        let original_data_dir = env::var_os("SIVTR_DATA_DIR");
        env::set_var("CLAUDE_HOME", dir.path());
        env::set_var("SIVTR_DATA_DIR", dir.path().join("data"));

        let projects = dir.path().join("projects").join("workspace");
        std::fs::create_dir_all(&projects).unwrap();

        let valid_path = projects.join("valid.jsonl");
        std::fs::write(
            &valid_path,
            r#"{"type":"user","sessionId":"good","cwd":"C:\\repo","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"good","cwd":"C:\\repo","customTitle":"valid session","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let malformed_path = projects.join("malformed.jsonl");
        std::fs::write(
            &malformed_path,
            r#"{"type":"user","sessionId":"bad","cwd":"C:\\repo","message":{"role":"user","content":"oops"}}
{"type":"assistant","sessionId":"bad","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"text","text":"broken"}]}
"#,
        )
        .unwrap();

        let sessions = ClaudeProvider.list_recent_sessions(None).unwrap();

        match original_claude_home {
            Some(value) => env::set_var("CLAUDE_HOME", value),
            None => env::remove_var("CLAUDE_HOME"),
        }
        match original_data_dir {
            Some(value) => env::set_var("SIVTR_DATA_DIR", value),
            None => env::remove_var("SIVTR_DATA_DIR"),
        }

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("good"));
        assert_eq!(sessions[0].title.as_deref(), Some("valid session"));
    }

    #[test]
    fn explicit_malformed_claude_session_still_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","sessionId":"bad","cwd":"C:\\repo","message":{"role":"user","content":"oops"}}
{"type":"assistant","sessionId":"bad","cwd":"C:\\repo","message":{"role":"assistant","content":[{"type":"text","text":"broken"}]}}{"type":"user","sessionId":"bad","cwd":"C:\\repo","message":{"role":"user","content":"glued"}}
"#,
        )
        .unwrap();

        let error = ClaudeProvider.parse_session_file(&path).unwrap_err();
        let message = format!("{error:#}");

        assert!(message.contains("Failed to parse Claude session line 2 as JSON"));
    }
}
