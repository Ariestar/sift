use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, list_recent_jsonl_sessions,
    open_readonly_db, parse_jsonl_meta, parse_jsonl_session, pretty_json_string, pretty_json_value,
    push_block, system_time_from_unix_secs, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionMeta, AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Hermes";
const STATE_DB_NAME: &str = "state.db";
const SESSION_PATH_PREFIX: &str = "hermes-session-";
const SESSION_PATH_SUFFIX: &str = ".sqlite";

/// Hermes session provider.
///
/// Primary store (current Hermes):
/// `$HERMES_HOME/state.db` tables `sessions` + `messages`.
///
/// Legacy / residual:
/// `$HERMES_HOME/sessions/*.jsonl`
#[derive(Debug, Clone, Copy, Default)]
pub struct HermesProvider;

impl AgentSessionProvider for HermesProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Hermes
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();
        sessions.extend(list_sqlite_sessions()?);
        // JSONL residual: keep sessions not already covered by state.db.
        sessions.extend(list_recent_jsonl_sessions(
            PROVIDER_NAME,
            &hermes_sessions_dir(),
            None,
            parse_session_meta,
        )?);

        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();

        let mut seen_ids = HashSet::new();
        let mut seen_paths = HashSet::new();
        sessions.retain(|session| {
            let id_ok = session
                .id
                .as_deref()
                .map(|id| seen_ids.insert(id.to_string()))
                .unwrap_or(true);
            let path_ok = seen_paths.insert(session.path.clone());
            // Prefer sqlite (listed first) when the same id exists in both stores.
            id_ok && path_ok
        });

        Ok(filter_sessions_by_workspace(sessions, cwd))
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        if let Some(session_id) = session_id_from_sqlite_path(path) {
            return parse_sqlite_session(&hermes_state_db_path(), &session_id, path);
        }

        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "jsonl" || ext.starts_with("jsonl"))
        {
            let mut session = parse_jsonl_session(path, PROVIDER_NAME, apply_event)?;
            if session.id.is_none() {
                session.id = session_id_from_path(path);
            }
            return Ok(session);
        }

        // Allow bare session ids / stems as a convenience for tests and tools.
        if let Some(id) = session_id_from_path(path) {
            let db = hermes_state_db_path();
            if db.exists() {
                return parse_sqlite_session(&db, &id, &sqlite_session_path(&id));
            }
        }

        anyhow::bail!(
            "Hermes session path `{}` is not a recognized sqlite or jsonl transcript",
            path.display()
        )
    }
}

pub fn hermes_home() -> PathBuf {
    if let Ok(path) = std::env::var("HERMES_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    if cfg!(windows) {
        if let Some(local_data) = dirs::data_local_dir() {
            return local_data.join("hermes");
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hermes")
}

pub fn hermes_sessions_dir() -> PathBuf {
    hermes_home().join("sessions")
}

pub fn hermes_state_db_path() -> PathBuf {
    hermes_home().join(STATE_DB_NAME)
}

fn list_sqlite_sessions() -> Result<Vec<SessionInfo>> {
    let db_path = hermes_state_db_path();
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = open_readonly_db(&db_path)?;
    let mut stmt = match conn.prepare(
        "SELECT id, source, model, cwd, title, started_at, ended_at, archived
         FROM sessions
         ORDER BY COALESCE(ended_at, started_at) DESC, id ASC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Ok(Vec::new()),
    };

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, Option<f64>>(6)?,
            row.get::<_, i64>(7).unwrap_or(0),
        ))
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        let (id, source, model, cwd, title, started_at, ended_at, archived) = row?;
        if archived != 0 {
            continue;
        }

        let title = title
            .filter(|value| !value.trim().is_empty())
            .or_else(|| default_title(source.as_deref(), model.as_deref()));

        let modified = system_time_from_unix_secs(ended_at.unwrap_or(started_at));
        sessions.push(SessionInfo {
            path: sqlite_session_path(&id),
            physical_path: Some(db_path.clone()),
            id: Some(id),
            cwd: non_empty_opt(cwd),
            title,
            modified,
        });
    }
    Ok(sessions)
}

fn parse_sqlite_session(db_path: &Path, session_id: &str, path: &Path) -> Result<AgentSession> {
    let conn = open_readonly_db(db_path)?;
    let (source, model, cwd, title, _started_at): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        f64,
    ) = conn
        .query_row(
            "SELECT source, model, cwd, title, started_at FROM sessions WHERE id = ?1 LIMIT 1",
            [session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .with_context(|| {
            format!(
                "Hermes session `{session_id}` not found in {}",
                db_path.display()
            )
        })?;

    let mut session = AgentSession {
        path: path.to_path_buf(),
        id: Some(session_id.to_string()),
        cwd: non_empty_opt(cwd),
        title: title
            .filter(|value| !value.trim().is_empty())
            .or_else(|| default_title(source.as_deref(), model.as_deref())),
        blocks: Vec::new(),
    };

    let mut stmt = conn
        .prepare(
            "SELECT role, content, tool_call_id, tool_calls, tool_name, timestamp, active
             FROM messages
             WHERE session_id = ?1
             ORDER BY timestamp ASC, id ASC",
        )
        .with_context(|| format!("Failed to query Hermes messages in {}", db_path.display()))?;

    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, i64>(6).unwrap_or(1),
        ))
    })?;

    for row in rows {
        let (role, content, _tool_call_id, tool_calls, tool_name, timestamp, active) = row?;
        if active == 0 {
            continue;
        }
        apply_db_message(
            &mut session,
            &role,
            content.as_deref(),
            tool_calls.as_deref(),
            tool_name.as_deref(),
            Some(timestamp_to_rfc3339(timestamp)),
        );
    }

    Ok(session)
}

fn apply_db_message(
    session: &mut AgentSession,
    role: &str,
    content: Option<&str>,
    tool_calls: Option<&str>,
    tool_name: Option<&str>,
    timestamp: Option<String>,
) {
    match role {
        "session_meta" | "system" => {}
        "user" => {
            if let Some(text) = content {
                push_block(session, AgentBlockKind::User, timestamp, None, text);
            }
        }
        "assistant" => {
            if let Some(text) = content.filter(|text| !text.trim().is_empty()) {
                push_block(
                    session,
                    AgentBlockKind::Assistant,
                    timestamp.clone(),
                    None,
                    text,
                );
            }
            if let Some(tool_calls) = tool_calls {
                push_tool_calls_from_json(session, tool_calls, timestamp);
            }
        }
        "tool" => {
            let text = content.unwrap_or_default();
            push_block(
                session,
                AgentBlockKind::ToolOutput,
                timestamp,
                tool_name.map(str::to_string),
                text,
            );
        }
        _ => {}
    }
}

fn push_tool_calls_from_json(
    session: &mut AgentSession,
    tool_calls: &str,
    timestamp: Option<String>,
) {
    let Ok(value) = serde_json::from_str::<Value>(tool_calls) else {
        push_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            None,
            pretty_json_string(tool_calls),
        );
        return;
    };

    match value {
        Value::Array(items) => {
            for item in items {
                push_one_tool_call(session, &item, timestamp.clone());
            }
        }
        other => push_one_tool_call(session, &other, timestamp),
    }
}

fn push_one_tool_call(session: &mut AgentSession, tool_call: &Value, timestamp: Option<String>) {
    let function = tool_call.get("function").unwrap_or(tool_call);
    let name = function
        .get("name")
        .or_else(|| tool_call.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let arguments = function
        .get("arguments")
        .or_else(|| tool_call.get("arguments"))
        .or_else(|| tool_call.get("input"))
        .map(|value| match value {
            Value::String(text) => pretty_json_string(text),
            other => pretty_json_value(other),
        })
        .unwrap_or_else(|| pretty_json_value(tool_call));
    push_block(
        session,
        AgentBlockKind::ToolCall,
        timestamp,
        name,
        arguments,
    );
}

fn parse_session_meta(path: &Path) -> Result<AgentSessionMeta> {
    let id = session_id_from_path(path);
    let mut meta = parse_jsonl_meta(path, PROVIDER_NAME, 5, update_meta_from_jsonl)?;
    if meta.id.is_none() {
        meta.id = id;
    }
    Ok(meta)
}

fn update_meta_from_jsonl(meta: &mut AgentSessionMeta, value: &Value) {
    if value.get("role").and_then(Value::as_str) != Some("session_meta") {
        // Also accept cwd if a later header-like row carries it.
        if let Some(cwd) = first_cwd(value) {
            meta.add_cwd(cwd);
        }
        return;
    }

    if let Some(cwd) = first_cwd(value) {
        meta.add_cwd(cwd);
    }

    if meta.title.is_none() {
        let platform = value.get("platform").and_then(Value::as_str);
        let model = value.get("model").and_then(Value::as_str);
        meta.title = default_title(platform, model);
    }
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    match value.get("role").and_then(Value::as_str) {
        Some("session_meta") => {
            if session.cwd.is_none() {
                session.cwd = first_cwd(value).map(str::to_string);
            }
            if session.title.is_none() {
                session.title = default_title(
                    value.get("platform").and_then(Value::as_str),
                    value.get("model").and_then(Value::as_str),
                );
            }
        }
        Some("user") => push_user_message(session, value),
        Some("assistant") => push_assistant_message(session, value),
        Some("tool") => push_tool_output(session, value),
        _ => {}
    }
}

fn push_user_message(session: &mut AgentSession, value: &Value) {
    let timestamp = extract_timestamp(value);
    let content = content_text(value);
    push_block(session, AgentBlockKind::User, timestamp, None, content);
}

fn push_assistant_message(session: &mut AgentSession, value: &Value) {
    let timestamp = extract_timestamp(value);
    let content = content_text(value);
    if !content.trim().is_empty() {
        push_block(
            session,
            AgentBlockKind::Assistant,
            timestamp.clone(),
            None,
            content,
        );
    }

    if let Some(tool_calls) = value.get("tool_calls") {
        match tool_calls {
            Value::Array(items) => {
                for tool_call in items {
                    push_one_tool_call(session, tool_call, timestamp.clone());
                }
            }
            Value::String(text) => push_tool_calls_from_json(session, text, timestamp),
            other => push_one_tool_call(session, other, timestamp),
        }
    }
}

fn push_tool_output(session: &mut AgentSession, value: &Value) {
    let timestamp = extract_timestamp(value);
    let content = content_text(value);
    let label = value
        .get("tool_name")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    push_block(
        session,
        AgentBlockKind::ToolOutput,
        timestamp,
        label,
        content,
    );
}

fn content_text(value: &Value) -> String {
    value
        .get("content")
        .map(extract_content_text)
        .unwrap_or_default()
}

fn extract_timestamp(value: &Value) -> Option<String> {
    match value.get("timestamp") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(num)) => num.as_f64().map(timestamp_to_rfc3339),
        _ => None,
    }
}

fn first_cwd(value: &Value) -> Option<&str> {
    value
        .get("cwd")
        .or_else(|| value.get("working_dir"))
        .or_else(|| value.get("workspace"))
        .or_else(|| value.get("directory"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
}

fn default_title(source_or_platform: Option<&str>, model: Option<&str>) -> Option<String> {
    match (source_or_platform, model) {
        (Some(source), Some(model)) if !source.is_empty() && !model.is_empty() => {
            Some(format!("{source} · {model}"))
        }
        (Some(source), _) if !source.is_empty() => Some(source.to_string()),
        (_, Some(model)) if !model.is_empty() => Some(model.to_string()),
        _ => None,
    }
}

fn non_empty_opt(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn timestamp_to_rfc3339(value: f64) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "1970-01-01T00:00:00Z".to_string();
    }
    let secs = value.trunc() as i64;
    let nanos = ((value.fract()) * 1_000_000_000.0).round() as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos.min(999_999_999))
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from(UNIX_EPOCH))
        .to_rfc3339()
}

fn sqlite_session_path(session_id: &str) -> PathBuf {
    PathBuf::from(format!(
        "{SESSION_PATH_PREFIX}{session_id}{SESSION_PATH_SUFFIX}"
    ))
}

fn session_id_from_sqlite_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix(SESSION_PATH_PREFIX)?
        .strip_suffix(SESSION_PATH_SUFFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn session_id_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentBlockKind, AgentSessionProvider};
    use rusqlite::Connection;

    fn write_state_db(home: &Path) -> PathBuf {
        let db = home.join(STATE_DB_NAME);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                model TEXT,
                cwd TEXT,
                title TEXT,
                started_at REAL NOT NULL,
                ended_at REAL,
                archived INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT,
                tool_call_id TEXT,
                tool_calls TEXT,
                tool_name TEXT,
                timestamp REAL NOT NULL,
                active INTEGER NOT NULL DEFAULT 1
            );
            "#,
        )
        .unwrap();
        db
    }

    #[test]
    fn parses_hermes_messages_and_tools() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("20260426_134409_4f3e2502.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"session_meta","model":"gpt-5.4-mini","platform":"cli","timestamp":"2026-04-26T13:47:42Z"}
{"role":"user","content":"hello","timestamp":"2026-04-26T13:47:43Z"}
{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"terminal","arguments":"{\"command\":\"echo hi\"}"}}],"timestamp":"2026-04-26T13:47:44Z"}
{"role":"tool","content":"{\"output\":\"hi\",\"exit_code\":0}","tool_call_id":"call_1","timestamp":"2026-04-26T13:47:45Z"}
{"role":"assistant","content":"done","timestamp":"2026-04-26T13:47:46Z"}
"#,
        )
        .unwrap();

        let session = HermesProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("20260426_134409_4f3e2502"));
        assert_eq!(session.title.as_deref(), Some("cli · gpt-5.4-mini"));
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("terminal"));
        assert!(session.blocks[1].text.contains("echo hi"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert!(session.blocks[2].text.contains("hi"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "done");
    }

    #[test]
    fn skips_reasoning_and_empty_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"session_meta","model":"gpt-5.4-mini","timestamp":"2026-04-26T00:00:00Z"}
{"role":"user","content":"test","timestamp":"2026-04-26T00:00:01Z"}
{"role":"assistant","content":"answer","reasoning":"hidden thinking","codex_reasoning_items":[{"type":"reasoning","encrypted_content":"xxx"}],"timestamp":"2026-04-26T00:00:02Z"}
"#,
        )
        .unwrap();

        let session = HermesProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "test");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[1].text, "answer");
    }

    #[test]
    fn handles_multiple_tool_calls_in_one_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"role":"session_meta","timestamp":"2026-04-26T00:00:00Z"}
{"role":"user","content":"do both","timestamp":"2026-04-26T00:00:01Z"}
{"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}},{"id":"c2","type":"function","function":{"name":"terminal","arguments":"{\"command\":\"ls\"}"}}],"timestamp":"2026-04-26T00:00:02Z"}
{"role":"tool","content":"file content","tool_call_id":"c1","timestamp":"2026-04-26T00:00:03Z"}
{"role":"tool","content":"dir listing","tool_call_id":"c2","timestamp":"2026-04-26T00:00:04Z"}
{"role":"assistant","content":"all done","timestamp":"2026-04-26T00:00:05Z"}
"#,
        )
        .unwrap();

        let session = HermesProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.blocks.len(), 6);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("read_file"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("terminal"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[5].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[5].text, "all done");
    }

    #[test]
    fn lists_sqlite_sessions_and_keeps_missing_cwd_under_cwd_filter() {
        let home = tempfile::tempdir().unwrap();
        let db = write_state_db(home.path());
        let conn = Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, model, cwd, title, started_at, ended_at, archived)
             VALUES
             ('s_no_cwd', 'weixin', 'm1', NULL, NULL, 100.0, 110.0, 0),
             ('s_match', 'cli', 'm2', '/repo', 'matched', 200.0, 210.0, 0),
             ('s_other', 'cli', 'm3', '/other', 'other', 300.0, 310.0, 0),
             ('s_arch', 'cli', 'm4', NULL, 'gone', 400.0, 410.0, 1)
            ",
            [],
        )
        .unwrap();
        drop(conn);

        let _guard = EnvGuard::set("HERMES_HOME", home.path());
        let sessions = HermesProvider
            .list_recent_sessions(Some(Path::new("/repo")))
            .expect("list");

        let ids: Vec<_> = sessions
            .iter()
            .filter_map(|session| session.id.clone())
            .collect();
        assert!(ids.contains(&"s_no_cwd".to_string()));
        assert!(ids.contains(&"s_match".to_string()));
        assert!(!ids.contains(&"s_other".to_string()));
        assert!(!ids.contains(&"s_arch".to_string()));
    }

    #[test]
    fn parses_sqlite_messages_and_tool_calls() {
        let home = tempfile::tempdir().unwrap();
        let db = write_state_db(home.path());
        let conn = Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, source, model, cwd, title, started_at, ended_at, archived)
             VALUES ('sess1', 'weixin', 'anthropic/claude-opus-4.6', NULL, NULL, 100.0, 120.0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_call_id, tool_calls, tool_name, timestamp, active)
             VALUES
             ('sess1', 'session_meta', '', NULL, NULL, NULL, 100.0, 1),
             ('sess1', 'user', 'hello', NULL, NULL, NULL, 101.0, 1),
             ('sess1', 'assistant', '', NULL, '[{\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"terminal\",\"arguments\":\"{\\\"command\\\":\\\"echo hi\\\"}\"}}]', NULL, 102.0, 1),
             ('sess1', 'tool', '{\"output\":\"hi\"}', 'call_1', NULL, 'terminal', 103.0, 1),
             ('sess1', 'assistant', 'done', NULL, NULL, NULL, 104.0, 1)
            ",
            [],
        )
        .unwrap();
        drop(conn);

        let _guard = EnvGuard::set("HERMES_HOME", home.path());
        let path = sqlite_session_path("sess1");
        let session = HermesProvider.parse_session_file(&path).expect("parse");

        assert_eq!(session.id.as_deref(), Some("sess1"));
        assert_eq!(
            session.title.as_deref(),
            Some("weixin · anthropic/claude-opus-4.6")
        );
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[1].label.as_deref(), Some("terminal"));
        assert!(session.blocks[1].text.contains("echo hi"));
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[2].label.as_deref(), Some("terminal"));
        assert!(session.blocks[2].text.contains("hi"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[3].text, "done");
    }

    #[test]
    fn system_time_from_unix_secs_converts() {
        let t = system_time_from_unix_secs(1.5);
        assert!(t > UNIX_EPOCH);
        let elapsed = t.duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(elapsed.as_secs(), 1);
        assert!(elapsed.subsec_nanos() >= 400_000_000);
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<Path>) -> Self {
            let _lock = crate::test_env_lock();
            let previous = std::env::var_os(key);
            std::env::set_var(key, value.as_ref());
            Self {
                key,
                previous,
                _lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
