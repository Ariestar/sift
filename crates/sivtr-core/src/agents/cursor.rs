use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::agents::{
    extract_content_text, jsonl_files, list_recent_jsonl_sessions, parse_jsonl_meta,
    parse_jsonl_session, pretty_json_value, push_block, AgentBlockKind, AgentProvider,
    AgentSession, AgentSessionMeta, AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Cursor";

/// Cursor agent transcript provider.
///
/// Primary evidence source observed in community/runtime layouts:
/// `~/.cursor/projects/<project-id>/agent-transcripts/**/*.jsonl`
///
/// When no transcripts exist, returns empty rather than inventing content.
#[derive(Debug, Clone, Copy, Default)]
pub struct CursorProvider;

impl AgentSessionProvider for CursorProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Cursor
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let root = cursor_transcripts_root();
        if !root.exists() {
            return Ok(Vec::new());
        }

        // Prefer structured jsonl listing with metadata when available.
        let mut sessions =
            list_recent_jsonl_sessions(PROVIDER_NAME, &root, cwd, parse_cursor_meta)?;
        if !sessions.is_empty() {
            return Ok(sessions);
        }

        // Fallback: path-only discovery when transcripts have no parseable meta.
        for path in jsonl_files(&root)? {
            let modified = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH);
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string);
            sessions.push(SessionInfo {
                path,
                id,
                cwd: None,
                title: None,
                modified,
            });
        }
        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();
        Ok(sessions)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        // Try shared JSONL pipeline first.
        let session = parse_jsonl_session(path, PROVIDER_NAME, apply_event)?;
        if !session.blocks.is_empty() {
            return Ok(session);
        }

        // Fallback for less structured rows.
        parse_cursor_jsonl_fallback(path)
    }
}

pub fn cursor_home() -> PathBuf {
    if let Ok(path) = std::env::var("CURSOR_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cursor")
}

fn cursor_transcripts_root() -> PathBuf {
    cursor_home().join("projects")
}

fn parse_cursor_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 80, update_meta)
}

fn update_meta(meta: &mut AgentSessionMeta, value: &Value) {
    if meta.id.is_none() {
        meta.id = value
            .get("sessionId")
            .or_else(|| value.get("id"))
            .or_else(|| value.get("conversationId"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if let Some(cwd) = value
        .get("cwd")
        .or_else(|| value.get("workspaceRoot"))
        .or_else(|| value.pointer("/workspace/path"))
        .and_then(Value::as_str)
    {
        meta.add_cwd(cwd);
    }
    if meta.title.is_none() {
        meta.title = value
            .get("title")
            .or_else(|| value.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .or_else(|| value.get("id"))
            .or_else(|| value.get("conversationId"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value
            .get("cwd")
            .or_else(|| value.get("workspaceRoot"))
            .and_then(Value::as_str)
            .map(str::to_string);
    }

    let timestamp = value
        .get("timestamp")
        .or_else(|| value.get("createdAt"))
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        });

    // Common Cursor/Claude-like shapes.
    // Cursor agent transcripts wrap payloads as `{role, message:{content:[...]}}`.
    let payload = cursor_payload(value);
    match value
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| value.get("role").and_then(Value::as_str))
    {
        Some("user") | Some("human") => {
            push_text(session, AgentBlockKind::User, timestamp, payload)
        }
        Some("assistant") | Some("ai") => push_assistant(session, timestamp, payload),
        Some("tool") | Some("tool_result") | Some("toolResult") => {
            push_text(session, AgentBlockKind::ToolOutput, timestamp, value)
        }
        Some("tool_call") | Some("toolCall") => {
            let label = value
                .get("name")
                .or_else(|| value.get("toolName"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let input = value
                .get("input")
                .or_else(|| value.get("args"))
                .or_else(|| value.get("arguments"))
                .unwrap_or(value);
            push_block(
                session,
                AgentBlockKind::ToolCall,
                timestamp,
                label,
                pretty_json_value(input),
            );
        }
        Some("message") => {
            let message = value.get("message").unwrap_or(value);
            match message.get("role").and_then(Value::as_str) {
                Some("user") => push_text(session, AgentBlockKind::User, timestamp, message),
                Some("assistant") => push_assistant(session, timestamp, message),
                Some("tool") => push_text(session, AgentBlockKind::ToolOutput, timestamp, message),
                _ => {}
            }
        }
        _ => {
            // Bubble-like rows sometimes only have text + bubbleType.
            if let Some(kind) = value
                .get("bubbleType")
                .or_else(|| value.get("type"))
                .and_then(Value::as_str)
            {
                match kind {
                    "user" | "human" => push_text(session, AgentBlockKind::User, timestamp, value),
                    "ai" | "assistant" => push_assistant(session, timestamp, value),
                    _ => {}
                }
            }
        }
    }
}

/// Cursor composer rows put the Anthropic-style payload under `message`.
fn cursor_payload(value: &Value) -> &Value {
    match value.get("message") {
        Some(message) if message.is_object() => message,
        _ => value,
    }
}

fn push_assistant(session: &mut AgentSession, timestamp: Option<String>, value: &Value) {
    let content = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("message"))
        .unwrap_or(value);
    match content {
        Value::Array(items) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => push_block(
                        session,
                        AgentBlockKind::Assistant,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    Some("tool_use" | "toolCall" | "tool_call") => {
                        let label = item.get("name").and_then(Value::as_str).map(str::to_string);
                        let input = item
                            .get("input")
                            .or_else(|| item.get("arguments"))
                            .unwrap_or(item);
                        push_block(
                            session,
                            AgentBlockKind::ToolCall,
                            timestamp.clone(),
                            label,
                            pretty_json_value(input),
                        );
                    }
                    Some("tool_result") => push_block(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    _ => {
                        let text = extract_content_text(item);
                        if !text.trim().is_empty() {
                            push_block(
                                session,
                                AgentBlockKind::Assistant,
                                timestamp.clone(),
                                None,
                                text,
                            );
                        }
                    }
                }
            }
        }
        other => push_text(session, AgentBlockKind::Assistant, timestamp, other),
    }
}

fn push_text(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    value: &Value,
) {
    let content = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("message"))
        .unwrap_or(value);
    let text = extract_content_text(content);
    if !text.trim().is_empty() {
        push_block(session, kind, timestamp, None, text);
    }
}

fn parse_cursor_jsonl_fallback(path: &Path) -> Result<AgentSession> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read Cursor transcript {}", path.display()))?;
    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string),
        cwd: None,
        title: None,
        blocks: Vec::new(),
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            apply_event(&mut session, &value);
        }
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::{apply_event, CursorProvider};
    use crate::agents::{AgentBlockKind, AgentProvider, AgentSession, AgentSessionProvider};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn provider_name_is_cursor() {
        assert_eq!(AgentProvider::Cursor.name(), "Cursor");
        assert_eq!(CursorProvider.provider(), AgentProvider::Cursor);
    }

    #[test]
    fn maps_cursor_like_message_events() {
        let mut session = AgentSession {
            path: PathBuf::from("t.jsonl"),
            id: None,
            cwd: None,
            title: None,
            blocks: Vec::new(),
        };
        apply_event(
            &mut session,
            &json!({"type":"user","text":"hello cursor","cwd":"D:/repo"}),
        );
        apply_event(
            &mut session,
            &json!({
                "type":"assistant",
                "content":[
                    {"type":"text","text":"hi"},
                    {"type":"toolCall","name":"read","input":{"path":"a.rs"}}
                ]
            }),
        );
        assert_eq!(session.cwd.as_deref(), Some("D:/repo"));
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
    }

    #[test]
    fn maps_composer_role_message_envelope() {
        let mut session = AgentSession {
            path: PathBuf::from("t.jsonl"),
            id: None,
            cwd: None,
            title: None,
            blocks: Vec::new(),
        };
        apply_event(
            &mut session,
            &json!({
                "role": "user",
                "message": {
                    "content": [{"type": "text", "text": "是否可以考虑把jina模型换成kimi 3"}]
                }
            }),
        );
        apply_event(
            &mut session,
            &json!({
                "role": "assistant",
                "message": {
                    "content": [
                        {"type": "text", "text": "先看 Jina 怎么接。"},
                        {"type": "tool_use", "name": "Read", "input": {"path": "config.py"}}
                    ]
                }
            }),
        );
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert!(session.blocks[0].text.contains("kimi 3"));
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert!(session.blocks[1].text.contains("Jina"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("Read"));
    }
}
