//! Goose (Block) agent sessions.
//!
//! Store (Goose >= 1.0): a single SQLite database.
//! ```text
//! <data-dir>/sessions/sessions.db    tables: sessions + messages
//! ```
//! The data dir follows `etcetera`'s app layout for `Block/goose`:
//! Windows `%APPDATA%\Block\goose\data`; macOS and Linux use
//! `$XDG_DATA_HOME/goose` (default `~/.local/share/goose`).
//! `GOOSE_PATH_ROOT` (absolute) overrides the root; sessions then live under
//! `<root>/data/sessions/`.
//!
//! `sessions` holds `id`, `working_dir`, `name`, timestamps; `messages`
//! holds `role` (`user`/`assistant`), `content_json` (a JSON array of
//! `{type: text|thinking|toolRequest|toolResponse|…}` blocks) and
//! `created_timestamp` (epoch seconds or millis).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    extract_content_text, filter_sessions_by_workspace, open_readonly_db, pretty_json_value,
    push_block, push_tool_block, system_time_from_unix_secs, AgentBlockKind, AgentProvider,
    AgentSession, AgentSessionProvider, SessionInfo,
};

const SESSION_PATH_PREFIX: &str = "goose-session-";
const SESSION_PATH_SUFFIX: &str = ".sqlite";

#[derive(Debug, Clone, Copy, Default)]
pub struct GooseProvider;

impl AgentSessionProvider for GooseProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Goose
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let db_path = goose_db_path();
        if !db_path.exists() {
            return Ok(Vec::new());
        }

        let conn = open_readonly_db(&db_path)?;
        let mut stmt = conn.prepare(
            "select id, working_dir, name, coalesce(cast(strftime('%s', updated_at) as integer), 0) \
             from sessions order by updated_at desc, id desc",
        )?;
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
            let (id, working_dir, name, updated_secs) = row?;
            sessions.push(SessionInfo {
                modified: system_time_from_unix_secs(updated_secs as f64),
                path: goose_session_path(&id),
                physical_path: Some(db_path.clone()),
                id: Some(id),
                cwd: Some(working_dir).filter(|value| !value.trim().is_empty()),
                title: Some(name).filter(|name| !name.trim().is_empty()),
            });
        }

        Ok(filter_sessions_by_workspace(sessions, cwd))
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let session_id = session_id_from_path(path).with_context(|| {
            format!(
                "Goose session path `{}` does not contain a session id",
                path.display()
            )
        })?;
        parse_session_by_id(&session_id)
    }

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        let db_path = goose_db_path();
        if !db_path.exists() {
            return Ok(None);
        }

        let conn = open_readonly_db(&db_path)?;
        let exact = conn
            .query_row(
                "select id from sessions where id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(id) = exact {
            return Ok(Some(goose_session_path(&id)));
        }

        let prefix = format!("{id}%");
        let prefix_match = conn
            .query_row(
                "select id from sessions where id like ?1 order by updated_at desc limit 1",
                params![prefix],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(prefix_match.map(|id| goose_session_path(&id)))
    }
}

pub fn goose_db_path() -> PathBuf {
    if let Some(root) = goose_path_root() {
        return root.join("data").join("sessions").join("sessions.db");
    }

    if cfg!(windows) {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Block")
            .join("goose")
            .join("data")
            .join("sessions")
            .join("sessions.db")
    } else {
        xdg_data_dir()
            .join("goose")
            .join("sessions")
            .join("sessions.db")
    }
}

fn xdg_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
        })
}

fn goose_path_root() -> Option<PathBuf> {
    std::env::var_os("GOOSE_PATH_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn goose_session_path(id: &str) -> PathBuf {
    PathBuf::from(format!("{SESSION_PATH_PREFIX}{id}{SESSION_PATH_SUFFIX}"))
}

fn session_id_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix(SESSION_PATH_PREFIX)?
        .strip_suffix(SESSION_PATH_SUFFIX)
        .map(str::to_string)
}

fn parse_session_by_id(session_id: &str) -> Result<AgentSession> {
    let db_path = goose_db_path();
    if !db_path.exists() {
        anyhow::bail!("Goose database {} does not exist", db_path.display());
    }

    let conn = open_readonly_db(&db_path)?;
    let meta = conn
        .query_row(
            "select id, working_dir, name from sessions where id = ?1",
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
        .with_context(|| format!("Goose session `{session_id}` was not found"))?;

    let mut session = AgentSession {
        path: goose_session_path(&meta.0),
        id: Some(meta.0),
        cwd: Some(meta.1).filter(|value| !value.trim().is_empty()),
        title: Some(meta.2).filter(|name| !name.trim().is_empty()),
        blocks: Vec::new(),
    };

    apply_message_rows(&conn, session_id, &mut session)?;
    Ok(session)
}

fn apply_message_rows(
    conn: &Connection,
    session_id: &str,
    session: &mut AgentSession,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "select role, content_json, created_timestamp from messages \
         where session_id = ?1 order by created_timestamp, id",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    for row in rows {
        let (role, content_json, created) = row?;
        let timestamp = Some(created.to_string());
        let content = match serde_json::from_str::<Value>(&content_json) {
            Ok(content) => content,
            Err(error) => {
                eprintln!(
                    "warning: failed to parse Goose message content for session {}: {error}",
                    session_id
                );
                continue;
            }
        };
        apply_content(session, &role, timestamp, &content);
    }
    Ok(())
}

fn apply_content(
    session: &mut AgentSession,
    role: &str,
    timestamp: Option<String>,
    content: &Value,
) {
    let dialogue_kind = match role {
        "user" => AgentBlockKind::User,
        "assistant" => AgentBlockKind::Assistant,
        _ => return,
    };

    let Some(items) = content.as_array() else {
        return;
    };
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => push_block(
                session,
                dialogue_kind,
                timestamp.clone(),
                None,
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
            Some("toolRequest") => {
                let id = item.get("id").and_then(Value::as_str).map(str::to_string);
                let tool_call = item.get("toolCall").unwrap_or(&Value::Null);
                let name = tool_call
                    .get("value")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let args = tool_call
                    .get("value")
                    .and_then(|value| value.get("arguments"))
                    .unwrap_or(&Value::Null);
                push_tool_block(
                    session,
                    AgentBlockKind::ToolCall,
                    timestamp.clone(),
                    id,
                    name,
                    pretty_json_value(args),
                    None,
                );
            }
            Some("toolResponse") => {
                let id = item.get("id").and_then(Value::as_str).map(str::to_string);
                let result = item
                    .get("toolResult")
                    .and_then(|result| result.get("value"))
                    .unwrap_or(&Value::Null);
                let text = result
                    .get("content")
                    .map(extract_content_text)
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| pretty_json_value(result));
                push_tool_block(
                    session,
                    AgentBlockKind::ToolOutput,
                    timestamp.clone(),
                    id,
                    None,
                    text,
                    None,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentSessionProvider;
    use rusqlite::Connection;

    #[cfg(not(windows))]
    #[test]
    fn uses_xdg_data_home_for_default_db_path() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let data_home = dir.path().join("data");
        let previous_root = std::env::var_os("GOOSE_PATH_ROOT");
        let previous_data_home = std::env::var_os("XDG_DATA_HOME");
        std::env::remove_var("GOOSE_PATH_ROOT");
        std::env::set_var("XDG_DATA_HOME", &data_home);

        assert_eq!(
            goose_db_path(),
            data_home.join("goose").join("sessions").join("sessions.db")
        );

        match previous_root {
            Some(value) => std::env::set_var("GOOSE_PATH_ROOT", value),
            None => std::env::remove_var("GOOSE_PATH_ROOT"),
        }
        match previous_data_home {
            Some(value) => std::env::set_var("XDG_DATA_HOME", value),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    fn seed_db(db_path: &Path) {
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                session_type TEXT NOT NULL DEFAULT 'user',
                working_dir TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id TEXT,
                session_id TEXT NOT NULL REFERENCES sessions(id),
                role TEXT NOT NULL,
                content_json TEXT NOT NULL,
                created_timestamp INTEGER NOT NULL,
                timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            INSERT INTO sessions (id, name, working_dir) VALUES ('s1', 'goose session', 'D:\repo');
            INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp) VALUES
                ('m1', 's1', 'user', '[{"type":"text","text":"hello"}]', 1700000000),
                ('m2', 's1', 'assistant', '[{"type":"thinking","thinking":"hidden","signature":""},{"type":"text","text":"hi"}]', 1700000001),
                ('m3', 's1', 'assistant', '[{"type":"toolRequest","id":"t1","toolCall":{"status":"success","value":{"name":"bash","arguments":{"command":"ls"}}}}]', 1700000002),
                ('m4', 's1', 'assistant', '[{"type":"toolResponse","id":"t1","toolResult":{"status":"success","value":{"content":[{"type":"text","text":"file.txt"}]}}}]', 1700000003),
                ('m5', 's1', 'assistant', '[{"type":"text","text":"done"}]', 1700000004);
            "#,
        )
        .unwrap();
    }

    #[test]
    fn parses_goose_sqlite_session() {
        let dir = tempfile::tempdir().unwrap();
        seed_db(&dir.path().join("data").join("sessions").join("sessions.db"));

        let _guard = crate::test_env_lock();
        std::env::set_var("GOOSE_PATH_ROOT", dir.path());
        let path = goose_session_path("s1");
        let session = GooseProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some("s1"));
        assert_eq!(session.cwd.as_deref(), Some("D:\\repo"));
        assert_eq!(session.title.as_deref(), Some("goose session"));
        assert_eq!(session.blocks.len(), 6);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "hidden");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "hi");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].label.as_deref(), Some("bash"));
        assert_eq!(session.blocks[3].call_id.as_deref(), Some("t1"));
        assert!(session.blocks[3].text.contains("ls"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].call_id.as_deref(), Some("t1"));
        assert_eq!(session.blocks[4].text, "file.txt");
        assert_eq!(session.blocks[5].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[5].text, "done");
    }

    #[test]
    fn lists_goose_sessions() {
        let dir = tempfile::tempdir().unwrap();
        seed_db(&dir.path().join("data").join("sessions").join("sessions.db"));

        let _guard = crate::test_env_lock();
        std::env::set_var("GOOSE_PATH_ROOT", dir.path());

        let sessions = GooseProvider.list_recent_sessions(None).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("s1"));
        assert_eq!(sessions[0].cwd.as_deref(), Some("D:\\repo"));
    }
}
