use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::agents::{
    filter_sessions_by_workspace, open_readonly_db, push_block, system_time_from_millis,
    AgentBlockKind, AgentProvider, AgentSession, AgentSessionProvider, SessionInfo,
};

const SESSION_PATH_PREFIX: &str = "zcode-session-";
const SESSION_PATH_SUFFIX: &str = ".json";
const SESSION_ID_PREFIX: &str = "sess_";

/// ZCode session provider.
///
/// Sessions live in `~/.zcode/cli/db/db.sqlite`:
/// - `session`: one row per session — `id` (`sess_<uuid>`), `title`,
///   `directory`, `time_updated` (epoch ms), `time_archived` (null = active)
/// - `message`: one row per turn — `sequence`, `data.role` is `user` or
///   `assistant`
/// - `part`: transcript content — `sequence`, `data.type` is `text`,
///   `reasoning`, `tool`, `step-start`, or `step-finish`. Tool payloads carry
///   `state.input` (object) plus `state.output` (completed) or `state.error`
///   (failed). Parts with `"synthetic": true` are system reminders injected
///   into the model context, not user dialogue.
#[derive(Debug, Clone, Copy, Default)]
pub struct ZcodeProvider;

impl AgentSessionProvider for ZcodeProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Zcode
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_sessions(cwd)
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let session_id = session_id_from_path(path)?;
        parse_session(&session_id).with_context(|| {
            format!(
                "Failed to load {} session {session_id}",
                AgentProvider::Zcode.name()
            )
        })
    }
}

fn zcode_home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ZCODE_HOME") {
        if path.is_empty() {
            anyhow::bail!("ZCODE_HOME must not be empty");
        }
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::home_dir()
        .context("failed to determine home directory for ZCode")?
        .join(".zcode"))
}

fn zcode_db_path() -> Result<PathBuf> {
    Ok(zcode_home()?.join("cli").join("db").join("db.sqlite"))
}

/// Drop the constant `sess_` prefix so session refs read `zcode/573eff56/1`.
fn display_id(row_id: &str) -> Result<&str> {
    row_id
        .strip_prefix(SESSION_ID_PREFIX)
        .filter(|id| !id.is_empty())
        .context("ZCode session id does not use the expected sess_ prefix")
}

fn list_sessions(cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
    let db_path = zcode_db_path()?;
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = open_readonly_db(&db_path)?;
    let mut stmt = conn
        .prepare(
            "select id, title, directory, time_updated from session \
             where time_archived is null order by time_updated desc",
        )
        .context("Failed to list ZCode sessions")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .context("Failed to read ZCode session rows")?;

    let mut sessions = Vec::new();
    for row in rows {
        let (id, title, directory, time_updated) = row.with_context(|| {
            format!("Failed to read ZCode session row in {}", db_path.display())
        })?;
        sessions.push(SessionInfo {
            path: session_path(&id),
            physical_path: Some(db_path.clone()),
            id: Some(display_id(&id)?.to_string()),
            cwd: Some(directory),
            title: Some(title),
            modified: system_time_from_millis(time_updated),
        });
    }
    Ok(filter_sessions_by_workspace(sessions, cwd))
}

fn parse_session(row_id: &str) -> Result<AgentSession> {
    let db_path = zcode_db_path()?;
    let conn = open_readonly_db(&db_path)?;

    let header: Option<(String, String)> = conn
        .query_row(
            "select title, directory from session where id = ?1 limit 1",
            [row_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("Failed to query ZCode session header")?;
    let Some((title, directory)) = header else {
        anyhow::bail!("no ZCode session row for id {row_id}");
    };

    let mut session = AgentSession {
        path: session_path(row_id),
        id: Some(display_id(row_id)?.to_string()),
        cwd: Some(directory),
        title: Some(title),
        blocks: Vec::new(),
    };

    let mut stmt = conn
        .prepare(
            "select m.data, p.data, p.time_created from part p \
             join message m on m.id = p.message_id \
             where p.session_id = ?1 order by m.sequence, p.sequence",
        )
        .context("Failed to query ZCode session parts")?;
    let rows = stmt
        .query_map([row_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .context("Failed to read ZCode session parts")?;

    for row in rows {
        let (message_json, part_json, time_created) =
            row.with_context(|| format!("Failed to read ZCode session {row_id} part row"))?;
        let message: Value = serde_json::from_str(&message_json)
            .with_context(|| format!("Invalid ZCode message JSON in session {row_id}"))?;
        let part: Value = serde_json::from_str(&part_json)
            .with_context(|| format!("Invalid ZCode part JSON in session {row_id}"))?;
        apply_part(&mut session, &message, &part, time_created)?;
    }

    Ok(session)
}

fn apply_part(
    session: &mut AgentSession,
    message: &Value,
    part: &Value,
    time_created: i64,
) -> Result<()> {
    if part.get("synthetic").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let timestamp = Some(time_created.to_string());

    match part.get("type").and_then(Value::as_str) {
        Some("text") => {
            let kind = match message.get("role").and_then(Value::as_str) {
                Some("user") => AgentBlockKind::User,
                Some("assistant") => AgentBlockKind::Assistant,
                _ => return Ok(()),
            };
            let text = part
                .get("text")
                .and_then(Value::as_str)
                .context("ZCode text part has no string text")?;
            push_block(session, kind, timestamp, None, text);
        }
        Some("reasoning") => {
            let text = part
                .get("text")
                .and_then(Value::as_str)
                .context("ZCode reasoning part has no string text")?;
            push_block(session, AgentBlockKind::Thinking, timestamp, None, text);
        }
        Some("tool") => apply_tool_part(session, part, timestamp)?,
        // step-start / step-finish are turn markers, not dialogue.
        _ => {}
    }
    Ok(())
}

fn apply_tool_part(
    session: &mut AgentSession,
    part: &Value,
    timestamp: Option<String>,
) -> Result<()> {
    let label = part.get("tool").and_then(Value::as_str).map(str::to_string);
    let state = part
        .get("state")
        .and_then(Value::as_object)
        .context("ZCode tool part has no state object")?;
    let input = state
        .get("input")
        .and_then(Value::as_object)
        .context("ZCode tool part has no input object")?;

    push_block(
        session,
        AgentBlockKind::ToolCall,
        timestamp.clone(),
        label.clone(),
        serde_json::to_string_pretty(input).context("failed to format ZCode tool input")?,
    );
    if let Some(output) = state.get("output") {
        let text = output
            .as_str()
            .context("ZCode tool output is not a string")?;
        push_block(
            session,
            AgentBlockKind::ToolOutput,
            timestamp.clone(),
            label.clone(),
            text,
        );
    }
    if let Some(error) = state.get("error") {
        let text = error.as_str().context("ZCode tool error is not a string")?;
        push_block(session, AgentBlockKind::ToolOutput, timestamp, label, text);
    }
    Ok(())
}

fn session_path(row_id: &str) -> PathBuf {
    PathBuf::from(format!(
        "{SESSION_PATH_PREFIX}{row_id}{SESSION_PATH_SUFFIX}"
    ))
}

fn session_id_from_path(path: &Path) -> Result<String> {
    let invalid = || format!("Invalid ZCode session path {}", path.display());
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(invalid)?;
    name.strip_prefix(SESSION_PATH_PREFIX)
        .and_then(|rest| rest.strip_suffix(SESSION_PATH_SUFFIX))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .with_context(invalid)
}

#[cfg(test)]
mod tests {
    use super::{session_id_from_path, session_path, ZcodeProvider};
    use crate::agents::{AgentBlockKind, AgentProvider, AgentSessionProvider};
    use rusqlite::Connection;
    use std::path::Path;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_env_lock()
    }

    fn write_db(dir: &Path) {
        let db = dir.join("cli").join("db").join("db.sqlite");
        std::fs::create_dir_all(db.parent().expect("db has a parent"))
            .expect("create ZCode db directory");
        let conn = Connection::open(&db).expect("open test ZCode db");
        conn.execute_batch(
            r#"
            create table session (
                id text primary key,
                title text not null,
                directory text not null,
                time_updated integer not null,
                time_archived integer
            );
            create table message (
                id text primary key,
                session_id text not null,
                time_created integer not null,
                data text not null,
                sequence integer
            );
            create table part (
                id text primary key,
                message_id text not null,
                session_id text not null,
                time_created integer not null,
                data text not null,
                sequence integer
            );
            insert into session (id, title, directory, time_updated) values
                ('sess_uuid-1', 'First task', 'D:/Coding/alpha', 2000);
            insert into message (id, session_id, time_created, data, sequence) values
                ('m1', 'sess_uuid-1', 1000, '{"role":"user"}', 0),
                ('m2', 'sess_uuid-1', 2000, '{"role":"assistant"}', 1);
            insert into part (id, message_id, session_id, time_created, data, sequence) values
                ('p1', 'm1', 'sess_uuid-1', 1000, '{"type":"text","text":"hello zcode"}', 0),
                ('p2', 'm2', 'sess_uuid-1', 2000, '{"type":"reasoning","text":"thinking"}', 0),
                ('p3', 'm2', 'sess_uuid-1', 3000, '{"type":"text","text":"working"}', 1),
                ('p4', 'm2', 'sess_uuid-1', 4000, '{"type":"tool","tool":"Bash","state":{"status":"completed","input":{"command":"ls"},"output":"files"}}', 2),
                ('p5', 'm2', 'sess_uuid-1', 5000, '{"type":"tool","tool":"Read","state":{"status":"error","input":{"file_path":"a.rs"},"error":"too large"}}', 3),
                ('p6', 'm2', 'sess_uuid-1', 6000, '{"type":"step-finish","reason":"tool-calls"}', 4),
                ('p7', 'm2', 'sess_uuid-1', 7000, '{"type":"text","text":"system reminder","synthetic":true}', 5);
            "#,
        )
        .expect("seed test ZCode db");
    }

    #[test]
    fn provider_name_is_zcode() {
        assert_eq!(AgentProvider::Zcode.name(), "ZCode");
        assert_eq!(AgentProvider::Zcode.command_name(), "zcode");
        assert_eq!(ZcodeProvider.provider(), AgentProvider::Zcode);
    }

    #[test]
    fn session_path_round_trips_id() {
        let path = session_path("sess_uuid-1");
        assert_eq!(
            session_id_from_path(&path).expect("round-trip a session id"),
            "sess_uuid-1"
        );
    }

    #[test]
    fn lists_and_parses_zcode_sessions() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().expect("create temp dir");
        let original = std::env::var_os("ZCODE_HOME");
        std::env::set_var("ZCODE_HOME", dir.path());
        write_db(dir.path());

        let sessions = ZcodeProvider
            .list_recent_sessions(None)
            .expect("list ZCode sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("uuid-1"));
        assert_eq!(sessions[0].cwd.as_deref(), Some("D:/Coding/alpha"));
        assert_eq!(sessions[0].title.as_deref(), Some("First task"));

        let session = ZcodeProvider
            .parse_session_file(&sessions[0].path)
            .expect("parse ZCode session");
        assert_eq!(session.id.as_deref(), Some("uuid-1"));
        let kinds: Vec<_> = session.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AgentBlockKind::User,
                AgentBlockKind::Thinking,
                AgentBlockKind::Assistant,
                AgentBlockKind::ToolCall,
                AgentBlockKind::ToolOutput,
                AgentBlockKind::ToolCall,
                AgentBlockKind::ToolOutput,
            ]
        );
        assert_eq!(session.blocks[0].text, "hello zcode");
        assert_eq!(session.blocks[0].timestamp.as_deref(), Some("1000"));
        assert_eq!(session.blocks[3].label.as_deref(), Some("Bash"));
        assert_eq!(session.blocks[3].text, "{\n  \"command\": \"ls\"\n}");
        assert_eq!(session.blocks[4].text, "files");
        assert_eq!(session.blocks[6].text, "too large");
        let records = crate::record::WorkRecord::chat_turns(AgentProvider::Zcode, &session);
        assert_eq!(records[0].work_ref.to_string(), "zcode/uuid-1/1");

        match original {
            Some(value) => std::env::set_var("ZCODE_HOME", value),
            None => std::env::remove_var("ZCODE_HOME"),
        }
    }

    #[test]
    fn missing_session_row_errors() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().expect("create temp dir");
        let original = std::env::var_os("ZCODE_HOME");
        std::env::set_var("ZCODE_HOME", dir.path());
        let error = ZcodeProvider
            .parse_session_file(&session_path("sess_missing"))
            .expect_err("missing session row must error");
        assert!(format!("{error:#}").contains("Failed to load"));
        match original {
            Some(value) => std::env::set_var("ZCODE_HOME", value),
            None => std::env::remove_var("ZCODE_HOME"),
        }
    }
}
