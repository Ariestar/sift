use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, jsonl_files, list_recent_jsonl_sessions, parse_jsonl_meta,
    parse_jsonl_session, pretty_json_string, pretty_json_value, push_block, push_tool_block,
    AgentBlockKind, AgentProvider, AgentSession, AgentSessionMeta, AgentSessionProvider,
    SessionInfo,
};

const PROVIDER_NAME: &str = "Codex";

#[derive(Debug, Clone, Copy, Default)]
pub struct CodexProvider;

impl AgentSessionProvider for CodexProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Codex
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();

        for root in [local_codex_sessions_dir()] {
            // Shared JSONL discovery (stamp-validated listing cache, workspace
            // filter) is applied per root; the merge below re-sorts globally.
            match list_recent_jsonl_sessions(PROVIDER_NAME, &root, cwd, parse_session_meta) {
                Ok(mut root_sessions) => sessions.append(&mut root_sessions),
                Err(error) => {
                    eprintln!(
                        "sivtr: warning: failed to read Codex session dir {}: {error:#}",
                        root.display()
                    );
                }
            }
        }

        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();
        Ok(sessions)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let mut saw_response_item = false;
        let mut event_fallback_indices = Vec::new();
        let mut session = parse_jsonl_session(path, PROVIDER_NAME, |session, value| {
            let timestamp = value
                .get("timestamp")
                .and_then(Value::as_str)
                .map(str::to_string);
            let payload = value.get("payload").unwrap_or(&Value::Null);

            match value.get("type").and_then(Value::as_str) {
                Some("session_meta") => {
                    session.id = payload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    session.cwd = payload
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                Some("response_item") => {
                    saw_response_item = true;
                    apply_response_item(session, payload, timestamp.clone());
                }
                Some("event_msg") => {
                    let start = session.blocks.len();
                    apply_event_msg(session, payload, timestamp);
                    if session.blocks.len() > start {
                        event_fallback_indices.extend(start..session.blocks.len());
                    }
                }
                _ => {}
            }
        })?;

        if saw_response_item && !event_fallback_indices.is_empty() {
            let mut drop_mask = vec![false; session.blocks.len()];
            for index in event_fallback_indices {
                if let Some(slot) = drop_mask.get_mut(index) {
                    *slot = true;
                }
            }
            session.blocks = session
                .blocks
                .into_iter()
                .enumerate()
                .filter_map(|(index, block)| (!drop_mask[index]).then_some(block))
                .collect();
        }

        Ok(session)
    }

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        for path in local_session_files()? {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(id))
            {
                return Ok(Some(path));
            }

            if parse_session_meta(&path)?.id.as_deref() == Some(id) {
                return Ok(Some(path));
            }
        }

        Ok(None)
    }

    fn find_current_session(&self, cwd: &Path) -> Result<Option<PathBuf>> {
        if let Some(path) = find_current_thread_session()? {
            return Ok(Some(path));
        }

        if let Some(session) = list_recent_local_sessions(Some(cwd))?.into_iter().next() {
            return Ok(Some(session.path));
        }

        Ok(list_recent_local_sessions(None)?
            .into_iter()
            .next()
            .map(|session| session.path))
    }
}

pub fn codex_home() -> PathBuf {
    if let Ok(path) = std::env::var("CODEX_HOME") {
        return PathBuf::from(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

pub fn local_codex_sessions_dir() -> PathBuf {
    codex_home().join("sessions")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 1, |meta, value| {
        let payload = value.get("payload").unwrap_or(&Value::Null);
        meta.id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
            meta.add_cwd(cwd);
        }
    })
}

fn apply_response_item(session: &mut AgentSession, payload: &Value, timestamp: Option<String>) {
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            let kind = match payload.get("role").and_then(Value::as_str) {
                Some("user") => AgentBlockKind::User,
                Some("assistant") => {
                    if payload.get("phase").and_then(Value::as_str) == Some("commentary") {
                        return;
                    }
                    AgentBlockKind::Assistant
                }
                _ => return,
            };
            let text = extract_content_text(payload.get("content").unwrap_or(&Value::Null));
            push_block(
                session,
                kind,
                timestamp,
                None,
                clean_codex_message_text(kind, text),
            );
        }
        Some("function_call") => push_tool_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            payload
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            extract_tool_call_text(payload),
            None,
        ),
        Some("function_call_output") => push_tool_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            None,
            payload
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            None,
        ),
        // Modern codex records shell/apply_patch/read calls as custom tool
        // events with a script `input` and an `output` content array.
        Some("custom_tool_call") => push_tool_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            payload
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            payload
                .get("input")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            None,
        ),
        Some("custom_tool_call_output") => push_tool_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            None,
            extract_content_text(payload.get("output").unwrap_or(&Value::Null)),
            None,
        ),
        _ => {}
    }
}

fn apply_event_msg(session: &mut AgentSession, payload: &Value, timestamp: Option<String>) {
    match payload.get("type").and_then(Value::as_str) {
        Some("user_message") => push_block(
            session,
            AgentBlockKind::User,
            timestamp,
            None,
            extract_content_text(payload.get("message").unwrap_or(&Value::Null)),
        ),
        Some("agent_message") => {
            if payload.get("phase").and_then(Value::as_str) == Some("commentary") {
                return;
            }
            push_block(
                session,
                AgentBlockKind::Assistant,
                timestamp,
                None,
                clean_codex_message_text(
                    AgentBlockKind::Assistant,
                    extract_content_text(payload.get("message").unwrap_or(&Value::Null)),
                ),
            );
        }
        _ => {}
    }
}

fn clean_codex_message_text(kind: AgentBlockKind, text: String) -> String {
    if kind == AgentBlockKind::Assistant {
        strip_trailing_oai_memory_citation(text)
    } else {
        text
    }
}

fn strip_trailing_oai_memory_citation(text: String) -> String {
    let trimmed = text.trim_end();
    if !trimmed.ends_with("</oai-mem-citation>") {
        return text;
    }

    let Some(start) = trimmed.rfind("<oai-mem-citation>") else {
        return text;
    };

    text[..start].trim_end().to_string()
}

fn extract_tool_call_text(payload: &Value) -> String {
    match payload.get("arguments") {
        Some(Value::String(arguments)) => pretty_json_string(arguments),
        Some(arguments) => pretty_json_value(arguments),
        None => pretty_json_value(payload),
    }
}

fn local_session_files() -> Result<Vec<PathBuf>> {
    jsonl_files(&local_codex_sessions_dir())
}

fn list_recent_local_sessions(cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
    list_recent_jsonl_sessions(
        PROVIDER_NAME,
        &local_codex_sessions_dir(),
        cwd,
        parse_session_meta,
    )
}

fn find_current_thread_session() -> Result<Option<PathBuf>> {
    let Ok(thread_id) = std::env::var("CODEX_THREAD_ID") else {
        return Ok(None);
    };

    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Ok(None);
    }

    CodexProvider.find_session_by_id(thread_id)
}

#[cfg(test)]
mod tests {
    use super::CodexProvider;
    use crate::agents::{select_blocks, AgentBlockKind, AgentSelection, AgentSessionProvider};
    use serde_json::json;
    use std::{env, time::Duration};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env_lock()
    }

    #[test]
    fn parses_codex_rollout_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-04-27T00:00:00Z","type":"session_meta","payload":{"id":"abc","cwd":"C:\\repo"}}
{"timestamp":"2026-04-27T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}
{"timestamp":"2026-04-27T00:00:02Z","type":"response_item","payload":{"type":"function_call","name":"shell_command","arguments":"{\"command\":\"echo hi\"}"}}
{"timestamp":"2026-04-27T00:00:03Z","type":"response_item","payload":{"type":"function_call_output","output":"hi"}}
{"timestamp":"2026-04-27T00:00:04Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("C:\\repo"));
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
    }

    #[test]
    fn parses_codex_custom_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-04-27T00:00:00Z","type":"session_meta","payload":{"id":"abc","cwd":"C:\\repo"}}
{"timestamp":"2026-04-27T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"edit it"}]}}
{"timestamp":"2026-04-27T00:00:02Z","type":"response_item","payload":{"type":"custom_tool_call","id":"ctc_1","status":"completed","call_id":"call_1","name":"apply_patch","input":"*** Begin Patch\n*** Update File: a.rs\n@@\n-old\n+new\n"}}
{"timestamp":"2026-04-27T00:00:03Z","type":"response_item","payload":{"type":"custom_tool_call_output","id":"ctco_1","call_id":"call_1","output":[{"type":"input_text","text":"Patch applied"}]}}
{"timestamp":"2026-04-27T00:00:04Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("apply_patch"));
        assert_eq!(session.blocks[1].call_id.as_deref(), Some("call_1"));
        assert!(session.blocks[1].text.contains("*** Begin Patch"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[2].call_id.as_deref(), Some("call_1"));
        assert_eq!(session.blocks[2].text, "Patch applied");
    }

    #[test]
    fn selects_last_turn_without_tool_noise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"first"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"old"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"second"}]}}
{"type":"response_item","payload":{"type":"function_call_output","output":"debug"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"new"}]}}
"#,
        )
        .unwrap();
        let session = CodexProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastTurn);

        // Last turn keeps structural blocks (tool output between user and assistant).
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].text, "second");
        assert_eq!(blocks[0].kind, AgentBlockKind::User);
        assert_eq!(blocks[1].kind, AgentBlockKind::ToolOutput);
        assert_eq!(blocks[2].text, "new");
        assert_eq!(blocks[2].kind, AgentBlockKind::Assistant);
        // The tool output block keeps its payload; "unknown result" is the
        // default label the structure marker derives from a bare output.
        assert!(blocks[1].text.contains("debug"));
    }

    #[test]
    fn skips_commentary_assistant_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"copy the answer"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"text":"working update"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"real answer"}]}}
"#,
        )
        .unwrap();
        let session = CodexProvider.parse_session_file(&path).unwrap();

        let blocks = select_blocks(&session, AgentSelection::LastAssistant);

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(blocks[0].text, "real answer");
    }

    #[test]
    fn strips_trailing_memory_citation_from_codex_assistant_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"real answer\n\n<oai-mem-citation>\n<citation_entries>\nMEMORY.md:1-2|note=[x]\n</citation_entries>\n<rollout_ids>\n019e0390-a9b5-77d3-9a0c-d5993240de06\n</rollout_ids>\n</oai-mem-citation>\n"}]}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 1);
        assert_eq!(session.blocks[0].text, "real answer");
    }

    #[test]
    fn keeps_non_trailing_memory_citation_text_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"example <oai-mem-citation> literal </oai-mem-citation> text"}]}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(
            session.blocks[0].text,
            "example <oai-mem-citation> literal </oai-mem-citation> text"
        );
    }

    #[test]
    fn find_current_session_prefers_codex_thread_id_over_cwd_match() {
        let _guard = env_lock();
        let temp = tempfile::tempdir().unwrap();
        let codex_home = temp.path().join("codex-home");
        let sessions_dir = codex_home
            .join("sessions")
            .join("2026")
            .join("05")
            .join("07");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let cwd_match = temp.path().join("cwd-match");
        let thread_match = temp.path().join("thread-match");
        std::fs::create_dir_all(&cwd_match).unwrap();
        std::fs::create_dir_all(&thread_match).unwrap();

        let cwd_session = sessions_dir.join("rollout-cwd.jsonl");
        std::fs::write(
            &cwd_session,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "type": "session_meta",
                    "payload": { "id": "cwd-session", "cwd": cwd_match }
                }))
                .unwrap()
            ),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(5));

        let thread_session = sessions_dir.join("rollout-thread.jsonl");
        std::fs::write(
            &thread_session,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "type": "session_meta",
                    "payload": { "id": "thread-session", "cwd": thread_match }
                }))
                .unwrap()
            ),
        )
        .unwrap();

        let previous_codex_home = env::var_os("CODEX_HOME");
        let previous_thread_id = env::var_os("CODEX_THREAD_ID");
        let previous_data_dir = env::var_os("SIVTR_DATA_DIR");
        env::set_var("CODEX_HOME", &codex_home);
        env::set_var("CODEX_THREAD_ID", "thread-session");
        env::set_var("SIVTR_DATA_DIR", temp.path().join("data"));

        let resolved = CodexProvider.find_current_session(&cwd_match).unwrap();

        match previous_codex_home {
            Some(value) => env::set_var("CODEX_HOME", value),
            None => env::remove_var("CODEX_HOME"),
        }
        match previous_thread_id {
            Some(value) => env::set_var("CODEX_THREAD_ID", value),
            None => env::remove_var("CODEX_THREAD_ID"),
        }
        match previous_data_dir {
            Some(value) => env::set_var("SIVTR_DATA_DIR", value),
            None => env::remove_var("SIVTR_DATA_DIR"),
        }

        assert_eq!(resolved, Some(thread_session));
    }

    #[test]
    fn parses_session_when_last_json_line_is_still_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp/repo"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"hello"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"done"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"partial"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].text, "done");
    }

    #[test]
    fn falls_back_to_event_messages_when_response_items_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/tmp/repo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"hello from event"}}
{"type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":"working"}}
{"type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"done from event"}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello from event");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "done from event");
    }

    #[test]
    fn response_items_remain_primary_over_event_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"id":"abc"}}
{"type":"event_msg","payload":{"type":"user_message","message":"event user"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"response user"}]}}
{"type":"event_msg","payload":{"type":"agent_message","phase":"final_answer","message":"event answer"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"response answer"}]}}
"#,
        )
        .unwrap();

        let session = CodexProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text, "response user");
        assert_eq!(session.blocks[1].text, "response answer");
    }
}
