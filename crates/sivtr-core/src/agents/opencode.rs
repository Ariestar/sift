use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, open_readonly_db, pretty_json_value,
    push_block, system_time_from_millis, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "OpenCode";
const SESSION_PATH_PREFIX: &str = "opencode-session-";
const SESSION_PATH_SUFFIX: &str = ".json";

#[derive(Debug, Clone, Default)]
pub struct OpenCodeProvider {
    db_path: Option<PathBuf>,
}

impl OpenCodeProvider {
    #[cfg(test)]
    fn with_db_path(path: PathBuf) -> Self {
        Self {
            db_path: Some(path),
        }
    }

    fn db_path(&self) -> PathBuf {
        self.db_path.clone().unwrap_or_else(opencode_db_path)
    }
}

impl AgentSessionProvider for OpenCodeProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::OpenCode
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let db_path = self.db_path();
        if !db_path.exists() {
            return Ok(Vec::new());
        }

        let conn = open_readonly_db(&db_path)?;
        let mut stmt = conn.prepare(
            "select id, directory, time_updated, title from session order by time_updated desc, id desc",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let cwd: String = row.get(1)?;
            let updated: i64 = row.get(2)?;
            let title: String = row.get(3)?;
            Ok((id, cwd, updated, title))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (id, session_cwd, updated, title) = row?;
            sessions.push(SessionInfo {
                modified: system_time_from_millis(updated),
                path: opencode_session_path(&id),
                physical_path: Some(db_path.clone()),
                id: Some(id),
                cwd: Some(session_cwd).filter(|value| !value.trim().is_empty()),
                title: Some(title).filter(|title| !title.trim().is_empty()),
            });
        }

        Ok(filter_sessions_by_workspace(sessions, cwd))
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let session_id = session_id_from_path(path).with_context(|| {
            format!(
                "OpenCode session path `{}` does not contain a session id",
                path.display()
            )
        })?;
        self.parse_session_by_id(&session_id)
    }

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        let db_path = self.db_path();
        if !db_path.exists() {
            return Ok(None);
        }

        let conn = open_readonly_db(&db_path)?;
        let exact = conn
            .query_row("select id from session where id = ?1", params![id], |row| {
                row.get::<_, String>(0)
            })
            .optional()?;
        if let Some(id) = exact {
            return Ok(Some(opencode_session_path(&id)));
        }

        let prefix = format!("{id}%");
        let prefix_match = conn
            .query_row(
                "select id from session where id like ?1 order by time_updated desc limit 1",
                params![prefix],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(prefix_match.map(|id| opencode_session_path(&id)))
    }
}

impl OpenCodeProvider {
    fn parse_session_by_id(&self, session_id: &str) -> Result<AgentSession> {
        let db_path = self.db_path();
        let conn = open_readonly_db(&db_path)?;
        let meta = conn
            .query_row(
                "select id, directory, title from session where id = ?1",
                params![session_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .with_context(|| format!("OpenCode session `{session_id}` was not found"))?;

        let mut session = AgentSession {
            path: opencode_session_path(&meta.0),
            id: Some(meta.0),
            cwd: Some(meta.1),
            title: Some(meta.2).filter(|title| !title.trim().is_empty()),
            blocks: Vec::new(),
        };

        apply_message_rows(&conn, session_id, &mut session)?;
        if session.blocks.is_empty() {
            apply_session_message_rows(&conn, session_id, &mut session)?;
        }

        Ok(session)
    }
}

pub fn opencode_db_path() -> PathBuf {
    if let Ok(path) = std::env::var("OPENCODE_DB_PATH") {
        return PathBuf::from(path);
    }

    if let Ok(path) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(path).join("opencode").join("opencode.db");
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db")
}

fn opencode_session_path(id: &str) -> PathBuf {
    PathBuf::from(format!("{SESSION_PATH_PREFIX}{id}{SESSION_PATH_SUFFIX}"))
}

fn session_id_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix(SESSION_PATH_PREFIX)?
        .strip_suffix(SESSION_PATH_SUFFIX)
        .map(str::to_string)
}

fn apply_message_rows(
    conn: &Connection,
    session_id: &str,
    session: &mut AgentSession,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "select id, time_created, data from message where session_id = ?1 order by time_created, id",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (message_id, timestamp, data) = row?;
        let message = parse_json(&data)?;
        let parts = load_parts(conn, session_id, &message_id)?;
        apply_message(session, &message, timestamp, &parts);
    }

    Ok(())
}

fn load_parts(conn: &Connection, session_id: &str, message_id: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "select data from part where session_id = ?1 and message_id = ?2 order by time_created, id",
    )?;
    let rows = stmt.query_map(params![session_id, message_id], |row| {
        row.get::<_, String>(0)
    })?;

    let mut parts = Vec::new();
    for row in rows {
        parts.push(parse_json(&row?)?);
    }
    Ok(parts)
}

fn apply_session_message_rows(
    conn: &Connection,
    session_id: &str,
    session: &mut AgentSession,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "select time_created, data from session_message where session_id = ?1 order by time_created, id",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (timestamp, data) = row?;
        let message = parse_json(&data)?;
        apply_message(session, &message, timestamp, &[]);
    }

    Ok(())
}

fn parse_json(text: &str) -> Result<Value> {
    serde_json::from_str(text).with_context(|| format!("Failed to parse {PROVIDER_NAME} JSON data"))
}

fn apply_message(session: &mut AgentSession, message: &Value, timestamp_ms: i64, parts: &[Value]) {
    let timestamp = Some(timestamp_ms.to_string());
    let Some(kind) = message_kind(message) else {
        return;
    };

    if parts.is_empty() {
        push_message_fallback(session, kind, timestamp, message);
        return;
    }

    for part in parts {
        apply_part(session, kind, timestamp.clone(), part);
    }
}

fn message_kind(message: &Value) -> Option<AgentBlockKind> {
    match message
        .get("role")
        .or_else(|| {
            message
                .get("message")
                .and_then(|message| message.get("role"))
        })
        .and_then(Value::as_str)
    {
        Some("user") => Some(AgentBlockKind::User),
        Some("assistant") => Some(AgentBlockKind::Assistant),
        _ => None,
    }
}

fn apply_part(
    session: &mut AgentSession,
    message_kind: AgentBlockKind,
    timestamp: Option<String>,
    part: &Value,
) {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => push_block(
            session,
            message_kind,
            timestamp,
            None,
            extract_opencode_text(part),
        ),
        Some("tool" | "tool_call" | "tool-call") => push_tool_part(session, timestamp, part),
        Some("tool_result" | "tool-result") => push_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            None,
            extract_opencode_text(part),
        ),
        Some("reasoning") => push_block(
            session,
            AgentBlockKind::Thinking,
            timestamp,
            None,
            extract_opencode_text(part),
        ),
        Some("step-start" | "step_finish" | "snapshot" | "file") => {}
        _ => {}
    }
}

/// OpenCode records MCP tools as `server_tool` (single underscore:
/// `sivtr_sivtr_search`, `playwright_browser_run_code_unsafe`) while every
/// native tool is underscore-free (`bash`, `read`, `edit`, `grep`, ...).
/// Rewrite MCP names to the canonical `mcp__server__tool` form so the TUI
/// renders them as `server: tool`.
fn normalize_opencode_tool(tool: &str) -> String {
    match tool.split_once('_') {
        Some((server, name)) if !server.is_empty() && !name.is_empty() => {
            format!("mcp__{server}__{name}")
        }
        _ => tool.to_string(),
    }
}

fn push_tool_part(session: &mut AgentSession, timestamp: Option<String>, part: &Value) {
    let state = part.get("state").unwrap_or(part);
    let label = part
        .get("tool")
        .or_else(|| part.get("name"))
        .and_then(Value::as_str)
        .map(normalize_opencode_tool);

    if let Some(input) = state.get("input").or_else(|| part.get("input")) {
        push_block(
            session,
            AgentBlockKind::ToolCall,
            timestamp.clone(),
            label.clone(),
            pretty_json_value(input),
        );
    }

    if let Some(output) = state
        .get("output")
        .or_else(|| state.get("error"))
        .or_else(|| part.get("output"))
    {
        push_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            label,
            extract_opencode_text(output),
        );
    }
}

fn push_message_fallback(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    message: &Value,
) {
    let content = message
        .get("content")
        .or_else(|| message.get("text"))
        .or_else(|| message.get("message"))
        .unwrap_or(message);
    push_block(
        session,
        kind,
        timestamp,
        None,
        extract_opencode_text(content),
    );
}

fn extract_opencode_text(value: &Value) -> String {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| value.get("content").and_then(Value::as_str))
        .or_else(|| value.get("output").and_then(Value::as_str));

    text.map(str::to_string)
        .unwrap_or_else(|| extract_content_text(value))
}

#[cfg(test)]
mod tests {
    use super::OpenCodeProvider;
    use crate::agents::{AgentBlockKind, AgentSessionProvider};
    use rusqlite::{params, Connection};

    fn test_provider() -> (tempfile::TempDir, OpenCodeProvider) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            create table session (
                id text primary key,
                directory text not null,
                title text not null,
                time_updated integer not null
            );
            create table message (
                id text primary key,
                session_id text not null,
                time_created integer not null,
                data text not null
            );
            create table part (
                id text primary key,
                message_id text not null,
                session_id text not null,
                time_created integer not null,
                data text not null
            );
            create table session_message (
                id text primary key,
                session_id text not null,
                type text not null,
                time_created integer not null,
                data text not null
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "insert into session (id, directory, title, time_updated) values (?1, ?2, ?3, ?4)",
            params!["open-session", "D:\\sivtr", "Greeting", 1000_i64],
        )
        .unwrap();
        conn.execute(
            "insert into message (id, session_id, time_created, data) values (?1, ?2, ?3, ?4)",
            params!["m1", "open-session", 1001_i64, r#"{"role":"user"}"#],
        )
        .unwrap();
        conn.execute(
            "insert into part (id, message_id, session_id, time_created, data) values (?1, ?2, ?3, ?4, ?5)",
            params!["p1", "m1", "open-session", 1001_i64, r#"{"type":"text","text":"hello"}"#],
        )
        .unwrap();
        conn.execute(
            "insert into message (id, session_id, time_created, data) values (?1, ?2, ?3, ?4)",
            params!["m2", "open-session", 1002_i64, r#"{"role":"assistant"}"#],
        )
        .unwrap();
        conn.execute(
            "insert into part (id, message_id, session_id, time_created, data) values (?1, ?2, ?3, ?4, ?5)",
            params!["p2", "m2", "open-session", 1002_i64, r#"{"type":"reasoning","text":"hidden"}"#],
        )
        .unwrap();
        conn.execute(
            "insert into part (id, message_id, session_id, time_created, data) values (?1, ?2, ?3, ?4, ?5)",
            params!["p3", "m2", "open-session", 1003_i64, r#"{"type":"tool","tool":"bash","state":{"input":{"command":"echo hi"},"output":"hi"}}"#],
        )
        .unwrap();
        conn.execute(
            "insert into part (id, message_id, session_id, time_created, data) values (?1, ?2, ?3, ?4, ?5)",
            params!["p4", "m2", "open-session", 1004_i64, r#"{"type":"text","text":"done"}"#],
        )
        .unwrap();
        drop(conn);

        let provider = OpenCodeProvider::with_db_path(path);
        (dir, provider)
    }

    #[test]
    fn lists_opencode_sessions_from_database() {
        let (_dir, provider) = test_provider();

        let sessions = provider
            .list_recent_sessions(Some(std::path::Path::new("D:\\sivtr")))
            .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("open-session"));
        assert_eq!(sessions[0].cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(sessions[0].title.as_deref(), Some("Greeting"));
    }

    #[test]
    fn mcp_tools_are_normalized_to_server_tool_form() {
        let dir = tempfile::tempdir().expect("tempdir for opencode test");
        let path = dir.path().join("opencode.db");
        let conn = Connection::open(&path).expect("open opencode test db");
        conn.execute_batch(
            r#"
            create table session (
                id text primary key,
                directory text not null,
                title text not null,
                time_updated integer not null
            );
            create table message (
                id text primary key,
                session_id text not null,
                time_created integer not null,
                data text not null
            );
            create table part (
                id text primary key,
                message_id text not null,
                session_id text not null,
                time_created integer not null,
                data text not null
            );
            create table session_message (
                id text primary key,
                session_id text not null,
                type text not null,
                time_created integer not null,
                data text not null
            );
            "#,
        )
        .expect("create opencode test schema");
        conn.execute(
            "insert into session (id, directory, title, time_updated) values (?1, ?2, ?3, ?4)",
            params!["mcp-session", "D:\\sivtr", "MCP", 1000_i64],
        )
        .expect("insert opencode test session");
        conn.execute(
            "insert into message (id, session_id, time_created, data) values (?1, ?2, ?3, ?4)",
            params!["m1", "mcp-session", 1001_i64, r#"{"role":"assistant"}"#],
        )
        .expect("insert opencode test message");
        conn.execute(
            "insert into part (id, message_id, session_id, time_created, data) values (?1, ?2, ?3, ?4, ?5)",
            params!["p1", "m1", "mcp-session", 1002_i64, r#"{"type":"tool","tool":"sivtr_sivtr_search","state":{"input":{"source":"claude"},"output":"{\"count\": 1}"}}"#],
        )
        .expect("insert opencode test part");
        drop(conn);

        let provider = OpenCodeProvider::with_db_path(path);
        let session = provider
            .parse_session_file(
                &provider
                    .find_session_by_id("mcp-session")
                    .expect("query opencode mcp session")
                    .expect("opencode mcp session exists"),
            )
            .expect("parse opencode mcp session");

        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::ToolCall);
        assert_eq!(
            session.blocks[0].label.as_deref(),
            Some("mcp__sivtr__sivtr_search")
        );
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolOutput);
        assert_eq!(
            session.blocks[1].label.as_deref(),
            Some("mcp__sivtr__sivtr_search")
        );
    }

    #[test]
    fn parses_opencode_messages_and_tools() {
        let (_dir, provider) = test_provider();
        let path = provider
            .find_session_by_id("open-session")
            .unwrap()
            .unwrap();

        let session = provider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("open-session"));
        assert_eq!(session.cwd.as_deref(), Some("D:\\sivtr"));
        assert_eq!(session.title.as_deref(), Some("Greeting"));
        assert_eq!(session.blocks.len(), 5);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "hidden");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[2].label.as_deref(), Some("bash"));
        assert!(session.blocks[2].text.contains("echo hi"));
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[3].text, "hi");
        assert_eq!(session.blocks[4].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[4].text, "done");
    }
}
