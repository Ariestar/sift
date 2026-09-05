use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, parse_jsonl_meta, parse_jsonl_session,
    pretty_json_value, push_block, push_tool_block, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionMeta, AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Qoder";

/// Qoder CLI sessions (global channel, `qodercli`).
///
/// The global CLI shares its home with the Qoder IDE, so one provider covers
/// both. Layout (default `~/.qoder`):
/// ```text
/// projects/<cwd-slug>/<session-uuid>.jsonl
/// ```
/// The cwd slug replaces path separators with `--` (e.g. `D:\Coding\sivtr` → `D--Coding-sivtr`).
#[derive(Debug, Clone, Copy, Default)]
pub struct QoderProvider;

/// Qoder CLI CN sessions (`qoderclicn`).
///
/// Same JSONL schema as the global CLI, separate home (default `~/.qoder-cn`);
/// only the home directory differs.
#[derive(Debug, Clone, Copy, Default)]
pub struct QoderCnProvider;

impl AgentSessionProvider for QoderProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Qoder
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_recent_sessions_in(&qoder_projects_dir(), cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

impl AgentSessionProvider for QoderCnProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::QoderCn
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_recent_sessions_in(&qoder_cn_projects_dir(), cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        parse_jsonl_session(path, PROVIDER_NAME, apply_event)
    }
}

/// Scan `projects/<cwd-slug>/<session>.jsonl` under `root`, newest first.
/// Shared by the global and CN Qoder providers; only `root` differs.
fn list_recent_sessions_in(root: &Path, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for bucket in fs::read_dir(root)
        .with_context(|| format!("Failed to read Qoder projects dir {}", root.display()))?
    {
        let bucket = bucket?;
        let bucket_path = bucket.path();
        if !bucket_path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&bucket_path)
            .with_context(|| format!("Failed to read Qoder bucket {}", bucket_path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            match parse_session_meta(&path) {
                Ok(meta) => {
                    let modified = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    sessions.push(SessionInfo {
                        path,
                        physical_path: None,
                        id: meta.id,
                        cwd: meta.cwd,
                        title: meta.title,
                        modified,
                    });
                }
                Err(_) => continue,
            }
        }
    }

    sessions.sort_by_key(|s| s.modified);
    sessions.reverse();
    Ok(filter_sessions_by_workspace(sessions, cwd))
}

pub fn qoder_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".qoder")
}

pub fn qoder_cn_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".qoder-cn")
}

fn qoder_projects_dir() -> PathBuf {
    qoder_home().join("projects")
}

fn qoder_cn_projects_dir() -> PathBuf {
    qoder_cn_home().join("projects")
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    parse_jsonl_meta(path, PROVIDER_NAME, 20, update_meta)
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
    // workspace-directories event carries the primary cwd
    if meta.cwd.is_none() {
        if let Some(dirs) = value.get("directories").and_then(Value::as_array) {
            if let Some(cwd) = dirs.first().and_then(Value::as_str) {
                meta.add_cwd(cwd);
            }
        }
    }
    if meta.title.is_none() {
        meta.title = value
            .get("aiTitle")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
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
        if let Some(dirs) = value.get("directories").and_then(Value::as_array) {
            session.cwd = dirs.first().and_then(Value::as_str).map(str::to_string);
        }
    }
    if session.cwd.is_none() {
        session.cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
    }
    if let Some(title) = value
        .get("aiTitle")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        session.title = Some(title.to_string());
    }
}

fn apply_message(
    session: &mut AgentSession,
    value: &Value,
    kind: AgentBlockKind,
    timestamp: Option<String>,
) {
    let Some(content) = value.get("message").and_then(|m| m.get("content")) else {
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
                    Some("text") => push_block(
                        session,
                        kind,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    Some("thinking") => {
                        let text = item
                            .get("thinking")
                            .map(extract_content_text)
                            .filter(|t| !t.trim().is_empty())
                            .unwrap_or_else(|| extract_content_text(item));
                        push_block(
                            session,
                            AgentBlockKind::Thinking,
                            timestamp.clone(),
                            None,
                            text,
                        );
                    }
                    Some("tool_use") => {
                        if let Some(input) = item.get("input") {
                            push_tool_block(
                                session,
                                AgentBlockKind::ToolCall,
                                timestamp.clone(),
                                item.get("id").and_then(Value::as_str).map(str::to_string),
                                item.get("name").and_then(Value::as_str).map(str::to_string),
                                pretty_json_value(input),
                                None,
                            );
                        }
                    }
                    Some("tool_result") => {
                        if let Some(result_content) = item.get("content") {
                            push_tool_block(
                                session,
                                AgentBlockKind::ToolOutput,
                                timestamp.clone(),
                                item.get("tool_use_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                None,
                                extract_content_text(result_content),
                                None,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::String(text) => push_block(session, kind, timestamp, None, text.clone()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{QoderCnProvider, QoderProvider};
    use crate::agents::{AgentBlockKind, AgentSessionProvider};

    #[test]
    fn parses_qoder_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"workspace-directories","sessionId":"abc","directories":["D:\\repo"]}
{"type":"user","sessionId":"abc","cwd":"D:\\repo","timestamp":"2026-07-01T00:00:00Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"abc","cwd":"D:\\repo","timestamp":"2026-07-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"I will check."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi"}}]}}
{"type":"user","sessionId":"abc","cwd":"D:\\repo","timestamp":"2026-07-01T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"hi"}]}}
{"type":"assistant","sessionId":"abc","cwd":"D:\\repo","timestamp":"2026-07-01T00:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = QoderProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("abc"));
        assert_eq!(session.cwd.as_deref(), Some("D:\\repo"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("Bash"));
        assert_eq!(session.blocks[2].call_id.as_deref(), Some("toolu_1"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].call_id.as_deref(), Some("toolu_1"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
    }

    #[test]
    fn skips_meta_messages_and_reads_ai_title() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"workspace-directories","sessionId":"abc","directories":["D:\\repo"]}
{"type":"user","sessionId":"abc","isMeta":true,"message":{"role":"user","content":"hidden"}}
{"type":"ai-title","sessionId":"abc","aiTitle":"My Session"}
{"type":"user","sessionId":"abc","cwd":"D:\\repo","message":{"role":"user","content":"real task"}}
{"type":"assistant","sessionId":"abc","cwd":"D:\\repo","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = QoderProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.title.as_deref(), Some("My Session"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text.trim(), "real task");
        assert_eq!(session.blocks[1].text.trim(), "done");
    }

    #[test]
    fn reads_cwd_from_workspace_directories_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"workspace-directories","sessionId":"abc","directories":["D:\\Coding\\sivtr"]}
{"type":"user","sessionId":"abc","message":{"role":"user","content":"hi"}}
{"type":"assistant","sessionId":"abc","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}
"#,
        )
        .unwrap();

        let session = QoderProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.cwd.as_deref(), Some("D:\\Coding\\sivtr"));
    }

    #[test]
    fn cn_provider_parses_identical_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"workspace-directories","sessionId":"cn-1","directories":["D:\\repo"]}
{"type":"user","sessionId":"cn-1","cwd":"D:\\repo","timestamp":"2026-07-01T00:00:00Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","sessionId":"cn-1","cwd":"D:\\repo","timestamp":"2026-07-01T00:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let session = QoderCnProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("cn-1"));
        assert_eq!(session.cwd.as_deref(), Some("D:\\repo"));
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].text.trim(), "hello");
        assert_eq!(session.blocks[1].text.trim(), "done");
    }
}
