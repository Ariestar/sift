use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, jsonl_files, open_readonly_db,
    pretty_json_value, push_block, system_time_from_millis, AgentBlockKind, AgentProvider,
    AgentSession, AgentSessionProvider, SessionInfo,
};

const SESSION_PATH_PREFIX: &str = "openclaw-session-";
const SESSION_PATH_SUFFIX: &str = ".json";
const AGENT_DB_NAME: &str = "openclaw-agent.sqlite";

/// OpenClaw session provider.
///
/// Primary store (current):
/// `~/.openclaw/agents/<agentId>/agent/openclaw-agent.sqlite`
/// with tables `session_entries` + `transcript_events`.
///
/// Legacy/archive:
/// `~/.openclaw/agents/<agentId>/sessions/*.jsonl` and `sessions.json`.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenClawProvider;

impl AgentSessionProvider for OpenClawProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::OpenClaw
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();
        sessions.extend(list_sqlite_sessions()?);
        sessions.extend(list_legacy_jsonl_sessions()?);

        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();
        // Dedup by path (sqlite wins over legacy if both exist for same id).
        let mut seen = std::collections::HashSet::new();
        sessions.retain(|session| seen.insert(session.path.clone()));
        Ok(filter_sessions_by_workspace(sessions, cwd))
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        if let Some((agent_id, session_id)) = session_ids_from_path(path) {
            if let Some(db_path) = agent_db_path(&agent_id) {
                if db_path.exists() {
                    return parse_sqlite_session(&db_path, &agent_id, &session_id);
                }
            }
        }

        // Legacy / archive transcript files.
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "jsonl" || ext.starts_with("jsonl"))
        {
            return parse_legacy_jsonl_session(path);
        }

        anyhow::bail!(
            "OpenClaw session path `{}` is not a recognized sqlite or jsonl transcript",
            path.display()
        )
    }
}

pub fn openclaw_home() -> PathBuf {
    if let Ok(path) =
        std::env::var("OPENCLAW_STATE_DIR").or_else(|_| std::env::var("OPENCLAW_HOME"))
    {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openclaw")
}

pub fn openclaw_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCLAW_CONFIG_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    openclaw_home().join("openclaw.json")
}

fn agents_dir() -> PathBuf {
    openclaw_home().join("agents")
}

fn agent_db_path(agent_id: &str) -> Option<PathBuf> {
    if agent_id.trim().is_empty() {
        return None;
    }
    Some(
        agents_dir()
            .join(agent_id)
            .join("agent")
            .join(AGENT_DB_NAME),
    )
}

fn list_sqlite_sessions() -> Result<Vec<SessionInfo>> {
    let root = agents_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for agent_entry in fs::read_dir(&root)
        .with_context(|| format!("Failed to read OpenClaw agents dir {}", root.display()))?
    {
        let agent_entry = agent_entry?;
        if !agent_entry.file_type()?.is_dir() {
            continue;
        }
        let agent_id = agent_entry.file_name().to_string_lossy().to_string();
        let db_path = agent_entry.path().join("agent").join(AGENT_DB_NAME);
        if !db_path.exists() {
            continue;
        }
        out.extend(list_sessions_from_db(&db_path, &agent_id)?);
    }
    Ok(out)
}

fn list_sessions_from_db(db_path: &Path, agent_id: &str) -> Result<Vec<SessionInfo>> {
    let conn = open_readonly_db(db_path)?;
    // Current OpenClaw schema: session_entries(session_key, session_id, entry_json, updated_at, ...)
    let mut stmt = match conn.prepare(
        "select session_key, session_id, entry_json, updated_at from session_entries order by updated_at desc, session_key asc",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Ok(Vec::new()), // schema not present / empty stub db
    };

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        let (session_key, session_id, entry_json, updated_at) = row?;
        if session_key.starts_with("__") {
            continue;
        }
        let entry: Value = serde_json::from_str(&entry_json).unwrap_or(Value::Null);
        if entry
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| {
                status.eq_ignore_ascii_case("archived") || status.eq_ignore_ascii_case("deleted")
            })
        {
            continue;
        }
        let cwd = entry
            .get("cwd")
            .or_else(|| entry.pointer("/origin/cwd"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let title = entry
            .get("displayName")
            .or_else(|| entry.get("label"))
            .or_else(|| entry.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(session_key.clone()));

        sessions.push(SessionInfo {
            path: openclaw_session_path(agent_id, &session_id),
            physical_path: Some(db_path.to_path_buf()),
            id: Some(session_id),
            cwd,
            title,
            modified: system_time_from_millis(updated_at),
        });
    }
    Ok(sessions)
}

fn parse_sqlite_session(db_path: &Path, agent_id: &str, session_id: &str) -> Result<AgentSession> {
    let conn = open_readonly_db(db_path)?;
    let entry_json: Option<String> = conn
        .query_row(
            "select entry_json from session_entries where session_id = ?1 limit 1",
            [session_id],
            |row| row.get(0),
        )
        .optional()?;

    let entry: Value = entry_json
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null);

    let mut session = AgentSession {
        path: openclaw_session_path(agent_id, session_id),
        id: Some(session_id.to_string()),
        cwd: entry
            .get("cwd")
            .or_else(|| entry.pointer("/origin/cwd"))
            .and_then(Value::as_str)
            .map(str::to_string),
        title: entry
            .get("displayName")
            .or_else(|| entry.get("label"))
            .or_else(|| entry.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string),
        blocks: Vec::new(),
    };

    let mut stmt = conn.prepare(
        "select event_json from transcript_events where session_id = ?1 order by seq asc",
    )?;
    let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
    for row in rows {
        let event_json = row?;
        if let Ok(event) = serde_json::from_str::<Value>(&event_json) {
            apply_transcript_event(&mut session, &event);
        }
    }

    Ok(session)
}

fn list_legacy_jsonl_sessions() -> Result<Vec<SessionInfo>> {
    let root = agents_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for agent_entry in fs::read_dir(&root)? {
        let agent_entry = agent_entry?;
        if !agent_entry.file_type()?.is_dir() {
            continue;
        }
        let sessions_dir = agent_entry.path().join("sessions");
        if !sessions_dir.exists() {
            continue;
        }
        for path in jsonl_files(&sessions_dir)? {
            // Skip compressed archives like *.jsonl.deleted.*.zst
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.ends_with(".jsonl") {
                continue;
            }
            let id = name.trim_end_matches(".jsonl").to_string();
            let modified = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH);
            out.push(SessionInfo {
                path,
                physical_path: None,
                id: Some(id),
                cwd: None,
                title: None,
                modified,
            });
        }
    }
    Ok(out)
}

fn parse_legacy_jsonl_session(path: &Path) -> Result<AgentSession> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read OpenClaw transcript {}", path.display()))?;
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
        if let Ok(event) = serde_json::from_str::<Value>(line) {
            // Header rows may carry session metadata.
            if event.get("type").and_then(Value::as_str) == Some("session") {
                if session.id.is_none() {
                    session.id = event
                        .get("id")
                        .or_else(|| event.get("sessionId"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                if session.cwd.is_none() {
                    session.cwd = event.get("cwd").and_then(Value::as_str).map(str::to_string);
                }
                continue;
            }
            apply_transcript_event(&mut session, &event);
        }
    }
    Ok(session)
}

fn apply_transcript_event(session: &mut AgentSession, event: &Value) {
    let timestamp = event
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);

    match event.get("type").and_then(Value::as_str) {
        Some("message") => {
            let message = event.get("message").unwrap_or(event);
            apply_message(session, message, timestamp);
        }
        Some("toolCall" | "tool_call" | "tool-call") => {
            let label = event
                .get("name")
                .or_else(|| event.get("tool"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let input = event
                .get("input")
                .or_else(|| event.get("arguments"))
                .unwrap_or(event);
            push_block(
                session,
                AgentBlockKind::ToolCall,
                timestamp,
                label,
                pretty_json_value(input),
            );
        }
        Some("toolResult" | "tool_result" | "tool-result") => {
            let output = event
                .get("output")
                .or_else(|| event.get("result"))
                .or_else(|| event.get("content"))
                .unwrap_or(event);
            push_block(
                session,
                AgentBlockKind::ToolOutput,
                timestamp,
                None,
                extract_content_text(output),
            );
        }
        // compaction / system / custom: ignore for evidence browsing
        _ => {
            // Some legacy rows are bare messages without type.
            if event.get("message").is_some() || event.get("role").is_some() {
                apply_message(session, event.get("message").unwrap_or(event), timestamp);
            }
        }
    }
}

fn apply_message(session: &mut AgentSession, message: &Value, timestamp: Option<String>) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let content = message
        .get("content")
        .or_else(|| message.get("text"))
        .unwrap_or(message);

    match role {
        "user" => push_content(session, AgentBlockKind::User, timestamp, content),
        "assistant" => push_assistant(session, timestamp, content),
        "tool" | "toolResult" => {
            push_content(session, AgentBlockKind::ToolOutput, timestamp, content)
        }
        _ => {
            // content-array style without role
            if content.as_array().is_some() {
                push_assistant(session, timestamp, content);
            }
        }
    }
}

fn push_assistant(session: &mut AgentSession, timestamp: Option<String>, content: &Value) {
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
                    Some("toolCall" | "tool_call" | "tool-call" | "functionCall") => {
                        let label = item
                            .get("name")
                            .or_else(|| item.get("tool"))
                            .or_else(|| item.pointer("/function/name"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let input = item
                            .get("input")
                            .or_else(|| item.get("arguments"))
                            .or_else(|| item.pointer("/function/arguments"))
                            .unwrap_or(item);
                        push_block(
                            session,
                            AgentBlockKind::ToolCall,
                            timestamp.clone(),
                            label,
                            pretty_json_value(input),
                        );
                    }
                    Some("toolResult" | "tool_result" | "tool-result") => push_block(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    ),
                    Some("thinking" | "reasoning") => {}
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
        other => push_content(session, AgentBlockKind::Assistant, timestamp, other),
    }
}

fn push_content(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    let text = extract_content_text(content);
    if !text.trim().is_empty() {
        push_block(session, kind, timestamp, None, text);
    }
}

fn openclaw_session_path(agent_id: &str, session_id: &str) -> PathBuf {
    PathBuf::from(format!(
        "{SESSION_PATH_PREFIX}{agent_id}--{session_id}{SESSION_PATH_SUFFIX}"
    ))
}

fn session_ids_from_path(path: &Path) -> Option<(String, String)> {
    let name = path.file_name()?.to_str()?;
    let rest = name
        .strip_prefix(SESSION_PATH_PREFIX)?
        .strip_suffix(SESSION_PATH_SUFFIX)?;
    let (agent_id, session_id) = rest.split_once("--")?;
    if agent_id.is_empty() || session_id.is_empty() {
        return None;
    }
    Some((agent_id.to_string(), session_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{
        openclaw_config_path, openclaw_home, openclaw_session_path, parse_sqlite_session,
        session_ids_from_path, OpenClawProvider, AGENT_DB_NAME,
    };
    use crate::agents::{AgentBlockKind, AgentProvider, AgentSessionProvider};
    use rusqlite::Connection;
    use std::path::PathBuf;

    #[test]
    fn provider_name_is_openclaw() {
        assert_eq!(AgentProvider::OpenClaw.name(), "OpenClaw");
        assert_eq!(AgentProvider::OpenClaw.command_name(), "openclaw");
    }

    #[test]
    fn config_path_lives_under_home() {
        let home = openclaw_home();
        assert_eq!(openclaw_config_path(), home.join("openclaw.json"));
    }

    #[test]
    fn session_path_round_trips_ids() {
        let path = openclaw_session_path("main", "sess-1");
        assert_eq!(
            session_ids_from_path(&path),
            Some(("main".into(), "sess-1".into()))
        );
    }

    #[test]
    fn parses_sqlite_transcript_events() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(AGENT_DB_NAME);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            create table session_entries (
                session_key text primary key,
                session_id text not null,
                entry_json text not null,
                updated_at integer not null
            );
            create table transcript_events (
                session_id text not null,
                seq integer not null,
                event_json text not null,
                created_at integer not null
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "insert into session_entries (session_key, session_id, entry_json, updated_at) values (?1,?2,?3,?4)",
            rusqlite::params![
                "agent:main:main",
                "sess-1",
                r#"{"sessionId":"sess-1","cwd":"D:/Coding/sivtr","displayName":"Main"}"#,
                1000_i64
            ],
        )
        .unwrap();
        conn.execute(
            "insert into transcript_events (session_id, seq, event_json, created_at) values (?1,?2,?3,?4)",
            rusqlite::params![
                "sess-1",
                1_i64,
                r#"{"type":"message","id":"m1","parentId":null,"timestamp":"t1","message":{"role":"user","content":"hello openclaw"}}"#,
                1001_i64
            ],
        )
        .unwrap();
        conn.execute(
            "insert into transcript_events (session_id, seq, event_json, created_at) values (?1,?2,?3,?4)",
            rusqlite::params![
                "sess-1",
                2_i64,
                r#"{"type":"message","id":"m2","parentId":"m1","timestamp":"t2","message":{"role":"assistant","content":[{"type":"text","text":"hi"},{"type":"toolCall","name":"bash","input":{"cmd":"ls"}}]}}"#,
                1002_i64
            ],
        )
        .unwrap();

        let session = parse_sqlite_session(&db, "main", "sess-1").expect("parse");
        assert_eq!(session.id.as_deref(), Some("sess-1"));
        assert_eq!(session.cwd.as_deref(), Some("D:/Coding/sivtr"));
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert!(session.blocks[0].text.contains("hello openclaw"));
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("bash"));
        assert_eq!(OpenClawProvider.provider(), AgentProvider::OpenClaw);
        let _ = PathBuf::from("noop");
    }
}
