//! Provider adapters for the additional transcript formats covered by the
//! agentsview provider catalog.
//!
//! The registry keeps provider identity and storage roots separate from the
//! parser. Formats that are byte-compatible share a parser (TraeX/Codex),
//! while JSONL, JSON-container, Markdown, directory, and SQLite shapes have
//! one explicit decoding path each.

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::codex::CodexProvider;
use super::{
    extract_content_text, filter_sessions_by_workspace, open_readonly_db, pretty_json_value,
    push_block, push_tool_block, system_time_from_millis, system_time_from_unix_secs,
    AgentBlockKind, AgentProvider, AgentSession, AgentSessionMeta, AgentSessionProvider,
    SessionInfo,
};

#[derive(Debug, Clone, Copy)]
pub struct GenericProvider {
    provider: AgentProvider,
}

impl GenericProvider {
    pub const fn new(provider: AgentProvider) -> Self {
        Self { provider }
    }

    fn roots(self) -> Result<Vec<PathBuf>> {
        if let Some(name) = root_env(self.provider) {
            if let Some(value) = std::env::var_os(name) {
                if value.is_empty() {
                    anyhow::bail!("{name} must not be empty");
                }
                return Ok(vec![PathBuf::from(value)]);
            }
        }

        if matches!(self.provider, AgentProvider::Aider) {
            return Ok(vec![std::env::current_dir()
                .context("failed to determine current directory for Aider")?]);
        }

        let home = dirs::home_dir().context("failed to determine home directory")?;
        Ok(default_roots(self.provider)
            .iter()
            .map(|relative| home.join(relative))
            .collect())
    }

    fn candidates(self) -> Result<Vec<PathBuf>> {
        if self.provider == AgentProvider::Aider && std::env::var_os("AIDER_DIR").is_none() {
            let path = std::env::current_dir()
                .context("failed to determine current directory for Aider")?
                .join(".aider.chat.history.md");
            return Ok(path.is_file().then_some(path).into_iter().collect());
        }
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        for root in self.roots()? {
            collect_candidates(self.provider, &root, &mut paths, &mut seen)?;
        }
        paths.sort();
        Ok(paths)
    }
}

impl AgentSessionProvider for GenericProvider {
    fn provider(&self) -> AgentProvider {
        self.provider
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();
        for path in self.candidates()? {
            if self.provider == AgentProvider::Aider {
                sessions.extend(list_aider_sessions(&path)?);
                continue;
            }
            if is_sqlite_provider(self.provider) {
                sessions.extend(list_sqlite_sessions(self.provider, &path)?);
                continue;
            }
            // One unreadable file must not drop the provider's whole listing;
            // warn and keep going (same policy as the jsonl meta reader).
            let parsed = match self.parse_session_file(&path) {
                Ok(parsed) => parsed,
                Err(error) => {
                    crate::diagnostics::warn(format!(
                        "failed to parse {} session {}: {error:#}",
                        self.provider.command_name(),
                        path.display()
                    ));
                    continue;
                }
            };
            if parsed.blocks.is_empty() {
                continue;
            }
            let modified = fs::metadata(&path)
                .with_context(|| format!("failed to stat {}", path.display()))?
                .modified()
                .with_context(|| format!("failed to read mtime {}", path.display()))?;
            let id = parsed.id.clone().or_else(|| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            });
            sessions.push(SessionInfo {
                modified,
                path,
                physical_path: None,
                id,
                cwd: parsed.cwd,
                title: parsed.title,
            });
        }
        sessions.sort_by_key(|session| session.modified);
        sessions.reverse();
        Ok(filter_sessions_by_workspace(sessions, cwd))
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        match self.provider {
            AgentProvider::TraeX => CodexProvider.parse_session_file(path),
            AgentProvider::Aider => parse_aider(path),
            AgentProvider::Kimi | AgentProvider::KimiWork => parse_kimi(path, self.provider),
            AgentProvider::OpenHands => parse_openhands(path),
            AgentProvider::RooCode => parse_roocode(path),
            AgentProvider::Zed
            | AgentProvider::Windsurf
            | AgentProvider::Kilo
            | AgentProvider::Shelley => parse_sqlite(path, self.provider),
            _ => parse_json_document(path, self.provider),
        }
    }
}

fn root_env(provider: AgentProvider) -> Option<&'static str> {
    Some(match provider {
        AgentProvider::Amp => "AMP_DIR",
        AgentProvider::Aider => "AIDER_DIR",
        AgentProvider::Antigravity => "ANTIGRAVITY_DIR",
        AgentProvider::AntigravityCli => "ANTIGRAVITY_CLI_DIR",
        AgentProvider::CommandCode => "COMMANDCODE_PROJECTS_DIR",
        AgentProvider::Copilot => "COPILOT_DIR",
        AgentProvider::DeepSeekTui => "DEEPSEEK_TUI_SESSIONS_DIR",
        AgentProvider::Forge => "FORGE_DIR",
        AgentProvider::Gptme => "GPTME_DIR",
        AgentProvider::Iflow => "IFLOW_DIR",
        AgentProvider::Kilo => "KILO_DIR",
        AgentProvider::Kimi => "KIMI_DIR",
        AgentProvider::KimiWork => "KIMI_WORK_DIR",
        AgentProvider::Kiro => "KIRO_SESSIONS_DIR",
        AgentProvider::OpenHands => "OPENHANDS_CONVERSATIONS_DIR",
        AgentProvider::Poolside => "POOLSIDE_DIR",
        AgentProvider::PositAssistant => "POSIT_ASSISTANT_DIR",
        AgentProvider::QwenPaw => "QWENPAW_DIR",
        AgentProvider::Reasonix => "REASONIX_DIR",
        AgentProvider::RooCode => "ROOCODE_DIR",
        AgentProvider::Shelley => "SHELLEY_DIR",
        AgentProvider::Trae => "TRAE_DIR",
        AgentProvider::TraeX => "TRAEX_SESSIONS_DIR",
        AgentProvider::Vibe => "VIBE_SESSIONS_DIR",
        AgentProvider::VSCodeCopilot => "VSCODE_COPILOT_DIR",
        AgentProvider::Windsurf => "WINDSURF_DIR",
        AgentProvider::Zed => "ZED_DIR",
        AgentProvider::Zencoder => "ZENCODER_DIR",
        _ => return None,
    })
}

fn default_roots(provider: AgentProvider) -> &'static [&'static str] {
    match provider {
        AgentProvider::Amp => &[".local/share/amp/threads"],
        AgentProvider::Antigravity | AgentProvider::AntigravityCli => &[".gemini/antigravity"],
        AgentProvider::CommandCode => &[".commandcode/projects"],
        AgentProvider::Copilot => &[".copilot"],
        AgentProvider::DeepSeekTui => &[".codewhale/sessions", ".deepseek/sessions"],
        AgentProvider::Forge => &[".forge"],
        AgentProvider::Gptme => &[".local/share/gptme/logs"],
        AgentProvider::Iflow => &[".iflow/projects"],
        AgentProvider::Kilo => &[".local/share/kilo"],
        AgentProvider::Kimi => &[".kimi/sessions", ".kimi-code/sessions"],
        AgentProvider::KimiWork => &[
            "Library/Application Support/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
            ".config/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
            ".local/share/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
            "AppData/Roaming/kimi-desktop/daimon-share/daimon/runtime/kimi-code/home/sessions",
        ],
        AgentProvider::Kiro => &[".kiro/sessions", ".local/share/kiro-cli"],
        AgentProvider::OpenHands => &[".openhands/conversations"],
        AgentProvider::Poolside => &[
            "Library/Application Support/poolside",
            ".local/state/poolside",
            "AppData/Roaming/poolside",
        ],
        AgentProvider::PositAssistant => &[".posit/assistant/workspaces"],
        AgentProvider::QwenPaw => &[".copaw/workspaces"],
        AgentProvider::Reasonix => &[".reasonix", "AppData/Roaming/reasonix"],
        AgentProvider::RooCode => &[
            "Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline",
            ".config/Code/User/globalStorage/rooveterinaryinc.roo-cline",
            "AppData/Roaming/Code/User/globalStorage/rooveterinaryinc.roo-cline",
        ],
        AgentProvider::Shelley => &[".config/shelley"],
        AgentProvider::Trae => &[
            "AppData/Roaming/Trae/User",
            "AppData/Roaming/Trae CN/User",
            "Library/Application Support/Trae/User",
            ".config/Trae/User",
        ],
        AgentProvider::TraeX => &[".trae/cli/sessions", ".trae/cli/archived_sessions"],
        AgentProvider::Vibe => &[".vibe/logs/session"],
        AgentProvider::VSCodeCopilot => &[
            "AppData/Roaming/Code/User",
            "AppData/Roaming/Code - Insiders/User",
            "Library/Application Support/Code/User",
            ".config/Code/User",
        ],
        AgentProvider::Windsurf => &[
            "AppData/Roaming/Windsurf/User",
            "Library/Application Support/Windsurf/User",
            ".config/Windsurf/User",
        ],
        AgentProvider::Zed => &[
            "Library/Application Support/Zed",
            ".config/zed",
            "AppData/Roaming/Zed",
        ],
        AgentProvider::Zencoder => &[".zencoder/sessions"],
        _ => &[],
    }
}

fn collect_candidates(
    provider: AgentProvider,
    path: &Path,
    output: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()))
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        if is_openhands_dir(provider, path) || is_roocode_dir(provider, path) {
            if seen.insert(path.to_path_buf()) {
                output.push(path.to_path_buf());
            }
            return Ok(());
        }
        for entry in
            fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
        {
            collect_candidates(provider, &entry?.path(), output, seen)?;
        }
        return Ok(());
    }
    if is_candidate_file(provider, path) && seen.insert(path.to_path_buf()) {
        output.push(path.to_path_buf());
    }
    Ok(())
}

fn is_openhands_dir(provider: AgentProvider, path: &Path) -> bool {
    provider == AgentProvider::OpenHands
        && path.join("base_state.json").is_file()
        && path.join("events").is_dir()
}

fn is_roocode_dir(provider: AgentProvider, path: &Path) -> bool {
    provider == AgentProvider::RooCode && path.join("history_item.json").is_file()
}

fn is_candidate_file(provider: AgentProvider, path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    match provider {
        AgentProvider::Aider => name == ".aider.chat.history.md",
        AgentProvider::Amp => extension.eq_ignore_ascii_case("json"),
        AgentProvider::DeepSeekTui => {
            extension.eq_ignore_ascii_case("json") && lower.contains("session")
        }
        AgentProvider::Gptme => lower == "conversation.jsonl",
        AgentProvider::Iflow => lower.starts_with("session-") && lower.ends_with(".jsonl"),
        AgentProvider::Kimi | AgentProvider::KimiWork => {
            lower == "wire.jsonl"
                && (provider == AgentProvider::Kimi
                    || path.components().any(|component| {
                        component.as_os_str().to_string_lossy().starts_with("conv-")
                    }))
        }
        AgentProvider::Kiro => extension.eq_ignore_ascii_case("jsonl"),
        AgentProvider::Poolside => extension.eq_ignore_ascii_case("ndjson"),
        AgentProvider::TraeX => extension.eq_ignore_ascii_case("jsonl"),
        AgentProvider::Vibe => lower == "messages.jsonl",
        AgentProvider::VSCodeCopilot | AgentProvider::Trae => {
            (extension.eq_ignore_ascii_case("json") || extension.eq_ignore_ascii_case("jsonl"))
                && path.components().any(|component| {
                    matches!(
                        component
                            .as_os_str()
                            .to_string_lossy()
                            .to_ascii_lowercase()
                            .as_str(),
                        "chatsessions" | "session-state" | "globalstorage"
                    )
                })
        }
        AgentProvider::Windsurf => lower == "state.vscdb",
        AgentProvider::Zed => lower == "threads.db",
        AgentProvider::Kilo | AgentProvider::Shelley => {
            extension.eq_ignore_ascii_case("db") || lower.ends_with(".sqlite")
        }
        AgentProvider::RooCode | AgentProvider::OpenHands => false,
        _ => {
            extension.eq_ignore_ascii_case("jsonl")
                || extension.eq_ignore_ascii_case("ndjson")
                || extension.eq_ignore_ascii_case("json")
        }
    }
}

fn is_sqlite_provider(provider: AgentProvider) -> bool {
    matches!(
        provider,
        AgentProvider::Zed | AgentProvider::Windsurf | AgentProvider::Kilo | AgentProvider::Shelley
    )
}

fn list_sqlite_sessions(_provider: AgentProvider, path: &Path) -> Result<Vec<SessionInfo>> {
    let conn = open_readonly_db(path)?;
    let tables = table_names(&conn)?;
    let modified = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime {}", path.display()))?;
    let mut sessions = Vec::new();
    let mut seen = HashSet::new();

    if tables.contains("threads") {
        let columns = table_columns(&conn, "threads")?;
        if columns.contains("id") {
            let id = column_or_null(&columns, "id");
            let summary = column_or_null(&columns, "summary");
            let updated = if columns.contains("updated_at") {
                "CAST(updated_at AS TEXT)"
            } else if columns.contains("last_message_at") {
                "CAST(last_message_at AS TEXT)"
            } else {
                "NULL"
            };
            let cwd = if columns.contains("folder_paths") {
                "CAST(folder_paths AS TEXT)"
            } else if columns.contains("cwd") {
                "CAST(cwd AS TEXT)"
            } else {
                "NULL"
            };
            let sql = format!(
                "SELECT CAST({id} AS TEXT), CAST({summary} AS TEXT), {updated}, {cwd} FROM threads ORDER BY rowid DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (id, title, updated, cwd) = row?;
                if let Some(info) = sqlite_session_info(
                    path,
                    id.as_deref(),
                    title.as_deref(),
                    cwd.as_deref(),
                    updated.as_deref(),
                    modified,
                    &mut seen,
                ) {
                    sessions.push(info);
                }
            }
        }
    }

    if tables.contains("session") {
        let columns = table_columns(&conn, "session")?;
        if columns.contains("id") {
            let id = column_or_null(&columns, "id");
            let title = if columns.contains("title") {
                "CAST(title AS TEXT)"
            } else if columns.contains("name") {
                "CAST(name AS TEXT)"
            } else {
                "NULL"
            };
            let cwd = if columns.contains("directory") {
                "CAST(directory AS TEXT)"
            } else if columns.contains("cwd") {
                "CAST(cwd AS TEXT)"
            } else {
                "NULL"
            };
            let updated = if columns.contains("time_updated") {
                "CAST(time_updated AS TEXT)"
            } else if columns.contains("updated_at") {
                "CAST(updated_at AS TEXT)"
            } else {
                "NULL"
            };
            let sql = format!(
                "SELECT CAST({id} AS TEXT), {title}, {cwd}, {updated} FROM session ORDER BY rowid DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (id, title, cwd, updated) = row?;
                if let Some(info) = sqlite_session_info(
                    path,
                    id.as_deref(),
                    title.as_deref(),
                    cwd.as_deref(),
                    updated.as_deref(),
                    modified,
                    &mut seen,
                ) {
                    sessions.push(info);
                }
            }
        }
    }

    if tables.contains("ItemTable") {
        let columns = table_columns(&conn, "ItemTable")?;
        if columns.contains("key") && columns.contains("value") {
            let mut stmt = conn.prepare(
                "SELECT key, CAST(value AS TEXT) FROM ItemTable WHERE value IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (key, value) = row?;
                if !key.to_ascii_lowercase().contains("chat")
                    && !key.to_ascii_lowercase().contains("session")
                {
                    continue;
                }
                let document: Value = serde_json::from_str(&value)
                    .with_context(|| format!("SQLite ItemTable value `{key}` is not JSON"))?;
                let mut metadata = Vec::new();
                collect_session_metadata(&document, &mut metadata);
                for (id, title, cwd, updated) in metadata {
                    if let Some(info) = sqlite_session_info(
                        path,
                        Some(&id),
                        title.as_deref().or(Some(key.as_str())),
                        cwd.as_deref(),
                        updated.as_deref(),
                        modified,
                        &mut seen,
                    ) {
                        sessions.push(info);
                    }
                }
            }
        }
    }

    for table in ["message", "messages"] {
        if !tables.contains(table) {
            continue;
        }
        let columns = table_columns(&conn, table)?;
        let Some(session_column) = columns
            .iter()
            .find(|name| name.eq_ignore_ascii_case("session_id"))
        else {
            continue;
        };
        let sql = format!(
            "SELECT DISTINCT CAST(\"{}\" AS TEXT) FROM \"{}\" WHERE \"{}\" IS NOT NULL",
            session_column.replace('"', "\"\""),
            table.replace('"', "\"\""),
            session_column.replace('"', "\"\""),
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            if let Some(info) =
                sqlite_session_info(path, Some(&row?), None, None, None, modified, &mut seen)
            {
                sessions.push(info);
            }
        }
    }

    sessions.sort_by_key(|session| session.modified);
    sessions.reverse();
    Ok(sessions)
}

fn sqlite_session_info(
    physical: &Path,
    raw_id: Option<&str>,
    title: Option<&str>,
    cwd: Option<&str>,
    updated: Option<&str>,
    fallback_modified: SystemTime,
    seen: &mut HashSet<String>,
) -> Option<SessionInfo> {
    let id = raw_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(addressable_session_id)?;
    if !seen.insert(id.clone()) {
        return None;
    }
    Some(SessionInfo {
        path: sqlite_virtual_path(physical, &id),
        physical_path: Some(physical.to_path_buf()),
        id: Some(id),
        cwd: parse_sqlite_cwd(cwd),
        title: non_empty(title),
        modified: parse_sqlite_time(updated).unwrap_or(fallback_modified),
    })
}

type SessionMetadata = (String, Option<String>, Option<String>, Option<String>);

fn collect_session_metadata(value: &Value, output: &mut Vec<SessionMetadata>) {
    if let Some(id) = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .or_else(|| value.get("conversationId"))
        .or_else(|| value.get("conversation_id"))
        .and_then(Value::as_str)
    {
        output.push((
            id.to_string(),
            value
                .get("title")
                .or_else(|| value.get("customTitle"))
                .and_then(Value::as_str)
                .map(str::to_string),
            value
                .get("cwd")
                .or_else(|| value.get("workspace"))
                .and_then(Value::as_str)
                .map(str::to_string),
            json_timestamp(value),
        ));
    }
    match value {
        Value::Object(map) => map
            .values()
            .for_each(|value| collect_session_metadata(value, output)),
        Value::Array(items) => items
            .iter()
            .for_each(|value| collect_session_metadata(value, output)),
        _ => {}
    }
}

fn addressable_session_id(raw: &str) -> String {
    if raw
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        raw.to_string()
    } else {
        sanitize_id(raw)
    }
}

fn sqlite_virtual_path(physical: &Path, session_id: &str) -> PathBuf {
    PathBuf::from(format!("{}#{session_id}", physical.display()))
}

fn split_sqlite_virtual_path(path: &Path) -> (PathBuf, Option<String>) {
    let text = path.to_string_lossy();
    let Some((physical, session_id)) = text.rsplit_once('#') else {
        return (path.to_path_buf(), None);
    };
    let physical = PathBuf::from(physical);
    if physical.is_file() {
        (physical, Some(session_id.to_string()))
    } else {
        (path.to_path_buf(), None)
    }
}

fn parse_sqlite_time(value: Option<&str>) -> Option<SystemTime> {
    let value = value?.trim();
    if let Ok(number) = value.parse::<i64>() {
        return Some(if value.len() >= 13 {
            system_time_from_millis(number)
        } else {
            system_time_from_unix_secs(number as f64)
        });
    }
    let timestamp = crate::time::parse_timestamp(value)?;
    let millis = timestamp.timestamp_millis();
    Some(system_time_from_millis(millis))
}

fn parse_sqlite_cwd(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(paths) = serde_json::from_str::<Vec<String>>(value) {
        return paths.into_iter().find(|path| !path.trim().is_empty());
    }
    Some(value.to_string())
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_json_document(path: &Path, provider: AgentProvider) -> Result<AgentSession> {
    let values = read_json_values(path)?;
    let mut session = empty_session(path);
    for value in values {
        match provider {
            AgentProvider::VSCodeCopilot | AgentProvider::Trae => {
                if !apply_vscode_session(&mut session, &value) {
                    apply_generic_value(&mut session, &value, None);
                }
            }
            AgentProvider::Kiro => apply_kiro_value(&mut session, &value),
            AgentProvider::Poolside => apply_poolside_value(&mut session, &value),
            _ => apply_generic_value(&mut session, &value, None),
        }
    }
    finish_session(&mut session, provider, path);
    Ok(session)
}

fn read_json_values(path: &Path) -> Result<Vec<Value>> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("json") {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read JSON transcript {}", path.display()))?;
        return Ok(vec![serde_json::from_str(&text).with_context(|| {
            format!("failed to parse JSON transcript {}", path.display())
        })?]);
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to read transcript {}", path.display()))?;
    let mut values = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line =
            line.with_context(|| format!("failed to read {} line {}", path.display(), index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => values.push(value),
            Err(error) if index > 0 && error.classify() == serde_json::error::Category::Eof => {
                break
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to parse {} line {}", path.display(), index + 1)
                })
            }
        }
    }
    Ok(values)
}

fn empty_session(path: &Path) -> AgentSession {
    AgentSession {
        path: path.to_path_buf(),
        id: None,
        cwd: None,
        title: None,
        blocks: Vec::new(),
    }
}

fn finish_session(session: &mut AgentSession, _provider: AgentProvider, path: &Path) {
    if session.id.is_none() {
        session.id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .map(str::to_string);
    }
    if session.title.is_none() {
        let first_user = session
            .blocks
            .iter()
            .find(|block| block.kind == AgentBlockKind::User)
            .map(|block| block.text.as_str());
        let mut meta = AgentSessionMeta::default();
        meta.fallback_title(first_user);
        session.title = meta.title;
    }
}

fn apply_generic_value(
    session: &mut AgentSession,
    value: &Value,
    inherited_timestamp: Option<String>,
) {
    update_identity(session, value);
    let timestamp = json_timestamp(value).or(inherited_timestamp);

    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        for message in messages {
            apply_generic_value(session, message, timestamp.clone());
        }
        return;
    }
    if let Some(events) = value.get("events").and_then(Value::as_array) {
        for event in events {
            apply_generic_value(session, event, timestamp.clone());
        }
        return;
    }

    if let Some(kind) = value.get("kind").and_then(Value::as_str) {
        if matches!(kind, "MessageEvent" | "ActionEvent" | "ObservationEvent") {
            apply_openhands_value(session, value, timestamp.clone());
            return;
        }
    }

    let payload = value
        .get("message")
        .filter(|message| message.is_object())
        .unwrap_or(value);
    let role = payload
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| value.get("role").and_then(Value::as_str))
        .or_else(|| value.get("source").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str));

    if let Some(role) = role.and_then(role_kind) {
        let content = payload
            .get("content")
            .or_else(|| payload.get("text"))
            .or_else(|| value.get("content"))
            .unwrap_or(&Value::Null);
        push_content(session, role, timestamp.clone(), content);
        push_top_level_tools(session, timestamp, value);
    } else if value.get("thought").and_then(Value::as_bool) == Some(true)
        || value.get("reasoning").is_some()
    {
        let content = value.get("reasoning").or_else(|| value.get("text"));
        if let Some(content) = content {
            push_block(
                session,
                AgentBlockKind::Thinking,
                timestamp,
                None,
                extract_content_text(content),
            );
        }
    }
}

fn update_identity(session: &mut AgentSession, value: &Value) {
    if session.id.is_none() {
        session.id = value
            .get("sessionId")
            .or_else(|| value.get("session_id"))
            .or_else(|| value.get("conversationId"))
            .or_else(|| value.get("conversation_id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string);
    }
    if session.cwd.is_none() {
        session.cwd = value
            .get("cwd")
            .or_else(|| value.get("workspace"))
            .or_else(|| value.get("workspaceRootPath"))
            .or_else(|| value.get("workingDirectory"))
            .or_else(|| value.get("working_dir"))
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
            .map(str::to_string);
    }
    if session.title.is_none() {
        session.title = value
            .get("title")
            .or_else(|| value.get("name"))
            .or_else(|| value.get("summary"))
            .or_else(|| value.get("customTitle"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string);
    }
}

fn role_kind(role: &str) -> Option<AgentBlockKind> {
    match role.to_ascii_lowercase().as_str() {
        "user" | "human" | "prompt" | "session.input" => Some(AgentBlockKind::User),
        "assistant" | "ai" | "model" | "agent" | "say" => Some(AgentBlockKind::Assistant),
        "tool" | "tool_result" | "toolresult" | "tool-response" | "observation" => {
            Some(AgentBlockKind::ToolOutput)
        }
        _ => None,
    }
}

fn push_content(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    value: &Value,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if item.get("functionCall").is_some()
                    || item.get("toolCall").is_some()
                    || item.get("tool_use").is_some()
                {
                    let call = item
                        .get("functionCall")
                        .or_else(|| item.get("toolCall"))
                        .or_else(|| item.get("tool_use"))
                        .unwrap_or(item);
                    push_tool(session, AgentBlockKind::ToolCall, timestamp.clone(), call);
                } else if item.get("functionResponse").is_some()
                    || item.get("toolResult").is_some()
                    || item.get("tool_result").is_some()
                {
                    let result = item
                        .get("functionResponse")
                        .or_else(|| item.get("toolResult"))
                        .or_else(|| item.get("tool_result"))
                        .unwrap_or(item);
                    push_tool(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        result,
                    );
                } else {
                    match item_type {
                        "tool_use" | "tool_call" | "toolCall" | "functionCall" | "tool" => {
                            push_tool(session, AgentBlockKind::ToolCall, timestamp.clone(), item)
                        }
                        "tool_result" | "toolResult" | "functionResponse" => {
                            push_tool(session, AgentBlockKind::ToolOutput, timestamp.clone(), item)
                        }
                        "thinking" | "reasoning" => push_block(
                            session,
                            AgentBlockKind::Thinking,
                            timestamp.clone(),
                            None,
                            extract_content_text(item.get("text").unwrap_or(item)),
                        ),
                        _ => push_block(
                            session,
                            kind,
                            timestamp.clone(),
                            None,
                            extract_content_text(item),
                        ),
                    }
                }
            }
        }
        Value::Object(object) if object.get("type").is_some() => {
            let item_type = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(
                item_type,
                "tool_use" | "tool_call" | "toolCall" | "functionCall" | "tool"
            ) {
                push_tool(session, AgentBlockKind::ToolCall, timestamp, value);
            } else if matches!(item_type, "tool_result" | "toolResult" | "functionResponse") {
                push_tool(session, AgentBlockKind::ToolOutput, timestamp, value);
            } else {
                push_block(session, kind, timestamp, None, extract_content_text(value));
            }
        }
        _ => push_block(session, kind, timestamp, None, extract_content_text(value)),
    }
}

fn push_top_level_tools(session: &mut AgentSession, timestamp: Option<String>, value: &Value) {
    for key in [
        "tool_calls",
        "toolCalls",
        "toolCall",
        "functionCall",
        "tool_use",
    ] {
        let Some(tools) = value.get(key) else {
            continue;
        };
        match tools {
            Value::Array(items) => items.iter().for_each(|item| {
                push_tool(session, AgentBlockKind::ToolCall, timestamp.clone(), item)
            }),
            _ => push_tool(session, AgentBlockKind::ToolCall, timestamp.clone(), tools),
        }
    }
    for key in [
        "tool_results",
        "toolResults",
        "toolResult",
        "functionResponse",
    ] {
        let Some(results) = value.get(key) else {
            continue;
        };
        match results {
            Value::Array(items) => items.iter().for_each(|item| {
                push_tool(session, AgentBlockKind::ToolOutput, timestamp.clone(), item)
            }),
            _ => push_tool(
                session,
                AgentBlockKind::ToolOutput,
                timestamp.clone(),
                results,
            ),
        }
    }
}

fn push_tool(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    value: &Value,
) {
    let label = value
        .get("name")
        .or_else(|| value.get("tool"))
        .or_else(|| value.get("toolName"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let call_id = value
        .get("id")
        .or_else(|| value.get("call_id"))
        .or_else(|| value.get("tool_call_id"))
        .or_else(|| value.get("tool_use_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let body = value
        .get("input")
        .or_else(|| value.get("args"))
        .or_else(|| value.get("arguments"))
        .or_else(|| value.get("output"))
        .or_else(|| value.get("content"))
        .or_else(|| value.get("result"))
        .unwrap_or(value);
    push_tool_block(
        session,
        kind,
        timestamp,
        call_id,
        label,
        match body {
            Value::String(text) => text.to_string(),
            _ => pretty_json_value(body),
        },
        None,
    );
}

fn json_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .or_else(|| value.get("createdAt"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("time"))
        .and_then(|timestamp| {
            timestamp
                .as_str()
                .map(str::to_string)
                .or_else(|| timestamp.as_i64().map(|number| number.to_string()))
        })
}

fn apply_openhands_value(session: &mut AgentSession, value: &Value, timestamp: Option<String>) {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "MessageEvent" => {
            let source = value
                .get("source")
                .or_else(|| value.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("assistant");
            let Some(role) = role_kind(source) else {
                return;
            };
            let content = value
                .get("message")
                .or_else(|| value.get("content"))
                .or_else(|| value.get("text"))
                .unwrap_or(&Value::Null);
            push_content(session, role, timestamp, content);
        }
        "ActionEvent" => {
            let action = value
                .get("action")
                .or_else(|| value.get("content"))
                .unwrap_or(value);
            push_tool(session, AgentBlockKind::ToolCall, timestamp, action);
        }
        "ObservationEvent" => {
            let observation = value
                .get("observation")
                .or_else(|| value.get("content"))
                .unwrap_or(value);
            push_tool(session, AgentBlockKind::ToolOutput, timestamp, observation);
        }
        _ => {}
    }
}

fn apply_vscode_session(session: &mut AgentSession, value: &Value) -> bool {
    let Some(requests) = value.get("requests").and_then(Value::as_array) else {
        return false;
    };
    for request in requests {
        let timestamp = json_timestamp(request);
        if let Some(message) = request.get("message") {
            let text = message
                .get("text")
                .or_else(|| message.get("content"))
                .unwrap_or(&Value::Null);
            push_content(session, AgentBlockKind::User, timestamp.clone(), text);
        }
        if let Some(response) = request.get("response") {
            push_vscode_response(session, timestamp, response);
        }
    }
    true
}

fn push_vscode_response(session: &mut AgentSession, timestamp: Option<String>, response: &Value) {
    let Some(items) = response.as_array() else {
        push_content(session, AgentBlockKind::Assistant, timestamp, response);
        return;
    };
    for item in items {
        if let Some(text) = item.get("value").and_then(Value::as_str) {
            push_block(
                session,
                AgentBlockKind::Assistant,
                timestamp.clone(),
                None,
                text,
            );
        } else if item.get("toolSpecificData").is_some() {
            push_tool(session, AgentBlockKind::ToolCall, timestamp.clone(), item);
        } else {
            push_content(session, AgentBlockKind::Assistant, timestamp.clone(), item);
        }
    }
}

fn apply_kiro_value(session: &mut AgentSession, value: &Value) {
    let timestamp = json_timestamp(value);
    match value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "Prompt" => push_content(
            session,
            AgentBlockKind::User,
            timestamp,
            value
                .get("data")
                .or_else(|| value.get("content"))
                .unwrap_or(&Value::Null),
        ),
        "AssistantMessage" => push_content(
            session,
            AgentBlockKind::Assistant,
            timestamp,
            value
                .get("data")
                .or_else(|| value.get("content"))
                .unwrap_or(&Value::Null),
        ),
        "ToolResults" => push_content(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            value
                .get("data")
                .or_else(|| value.get("content"))
                .unwrap_or(&Value::Null),
        ),
        _ => apply_generic_value(session, value, None),
    }
}

fn apply_poolside_value(session: &mut AgentSession, value: &Value) {
    let timestamp = json_timestamp(value);
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "session.input" => push_content(
            session,
            AgentBlockKind::User,
            timestamp,
            value
                .pointer("/session_input/prompt")
                .unwrap_or(&Value::Null),
        ),
        "assistant_message.end" => push_content(
            session,
            AgentBlockKind::Assistant,
            timestamp,
            value
                .pointer("/assistant_message_end/assistant_message")
                .unwrap_or(&Value::Null),
        ),
        "thought.end" => push_block(
            session,
            AgentBlockKind::Thinking,
            timestamp,
            None,
            value
                .pointer("/thought_end/thought")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        "tool_call.parsed" => push_tool(
            session,
            AgentBlockKind::ToolCall,
            timestamp,
            value.get("tool_call_parsed").unwrap_or(value),
        ),
        "tool_call.result" => push_tool(
            session,
            AgentBlockKind::ToolOutput,
            timestamp,
            value.get("tool_call_result").unwrap_or(value),
        ),
        _ => apply_generic_value(session, value, None),
    }
}

fn parse_aider(path: &Path) -> Result<AgentSession> {
    let (physical, selected) = split_aider_virtual_path(path);
    let text = fs::read_to_string(&physical)
        .with_context(|| format!("failed to read Aider history {}", physical.display()))?;
    let runs = split_aider_runs(&text);
    let selected_runs: Vec<_> = match selected {
        Some(index) => vec![runs
            .get(index)
            .with_context(|| format!("Aider run {index} is not present in {}", physical.display()))?
            .clone()],
        None => runs,
    };
    let mut session = empty_session(path);
    session.id = Some(path_id(path));
    session.cwd = physical.parent().map(|parent| parent.display().to_string());
    for run in selected_runs {
        parse_aider_run_body(&mut session, &run.header, &run.body);
    }
    finish_session(&mut session, AgentProvider::Aider, path);
    Ok(session)
}

#[derive(Clone)]
struct AiderRun {
    header: String,
    body: String,
}

fn split_aider_runs(text: &str) -> Vec<AiderRun> {
    let mut runs = Vec::new();
    let mut current: Option<AiderRun> = None;
    for raw in text.lines() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(header) = line.strip_prefix("# aider chat started at ") {
            if let Some(run) = current.take() {
                runs.push(run);
            }
            current = Some(AiderRun {
                header: header.trim().to_string(),
                body: String::new(),
            });
        } else if let Some(run) = current.as_mut() {
            run.body.push_str(line);
            run.body.push('\n');
        }
    }
    if let Some(run) = current {
        runs.push(run);
    }
    runs
}

fn list_aider_sessions(path: &Path) -> Result<Vec<SessionInfo>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Aider history {}", path.display()))?;
    let modified = fs::metadata(path)
        .with_context(|| format!("failed to stat Aider history {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime {}", path.display()))?;
    let mut sessions = Vec::new();
    for (index, run) in split_aider_runs(&text).into_iter().enumerate() {
        let virtual_path = aider_virtual_path(path, index);
        let mut parsed = empty_session(&virtual_path);
        parsed.id = Some(path_id(&virtual_path));
        parsed.cwd = path.parent().map(|parent| parent.display().to_string());
        parse_aider_run_body(&mut parsed, &run.header, &run.body);
        finish_session(&mut parsed, AgentProvider::Aider, &virtual_path);
        if parsed.blocks.is_empty() {
            continue;
        }
        sessions.push(SessionInfo {
            path: virtual_path,
            physical_path: Some(path.to_path_buf()),
            id: parsed.id,
            cwd: parsed.cwd,
            title: parsed.title,
            modified: aider_timestamp(&run.header)
                .and_then(|value| crate::time::parse_timestamp(&value))
                .map(|value| system_time_from_millis(value.timestamp_millis()))
                .unwrap_or(modified),
        });
    }
    Ok(sessions)
}

fn parse_aider_run_body(session: &mut AgentSession, header: &str, body: &str) {
    let mut current: Option<AgentBlockKind> = None;
    let mut content = String::new();
    let timestamp = aider_timestamp(header);
    let flush =
        |session: &mut AgentSession, current: &mut Option<AgentBlockKind>, content: &mut String| {
            if let Some(kind) = *current {
                push_block(session, kind, timestamp.clone(), None, content.as_str());
            }
            *current = None;
            content.clear();
        };

    for line in body.lines() {
        let (kind, line) = if let Some(line) = line.strip_prefix("####") {
            (AgentBlockKind::User, line.trim_start())
        } else if let Some(line) = line.strip_prefix("> ") {
            (AgentBlockKind::ToolOutput, line)
        } else {
            (AgentBlockKind::Assistant, line)
        };
        if current != Some(kind) && !content.trim().is_empty() {
            flush(session, &mut current, &mut content);
        }
        current = Some(kind);
        content.push_str(line);
        content.push('\n');
    }
    flush(session, &mut current, &mut content);
}

fn aider_virtual_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}#{index}", path.display()))
}

fn split_aider_virtual_path(path: &Path) -> (PathBuf, Option<usize>) {
    let text = path.to_string_lossy();
    let Some((physical, index)) = text.rsplit_once('#') else {
        return (path.to_path_buf(), None);
    };
    let Ok(index) = index.parse::<usize>() else {
        return (path.to_path_buf(), None);
    };
    let physical = PathBuf::from(physical);
    if physical.is_file() {
        (physical, Some(index))
    } else {
        (path.to_path_buf(), None)
    }
}

fn aider_timestamp(value: &str) -> Option<String> {
    chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|time| {
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(time, chrono::Utc)
                .to_rfc3339()
        })
}

fn parse_kimi(path: &Path, provider: AgentProvider) -> Result<AgentSession> {
    let values = read_json_values(path)?;
    let mut session = empty_session(path);
    session.id = Some(kimi_id(path));
    for value in values {
        update_identity(&mut session, &value);
        let timestamp = json_timestamp(&value);
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "turn.prompt" => push_content(
                &mut session,
                AgentBlockKind::User,
                timestamp,
                value
                    .get("content")
                    .or_else(|| value.get("prompt"))
                    .unwrap_or(&Value::Null),
            ),
            "content.part" | "ContentPart" => push_content(
                &mut session,
                AgentBlockKind::Assistant,
                timestamp,
                value
                    .get("content")
                    .or_else(|| value.get("text"))
                    .unwrap_or(&Value::Null),
            ),
            "tool.call" | "ToolCall" => push_tool(
                &mut session,
                AgentBlockKind::ToolCall,
                timestamp,
                value
                    .get("tool")
                    .or_else(|| value.get("call"))
                    .unwrap_or(&value),
            ),
            "tool.result" | "ToolResult" => push_tool(
                &mut session,
                AgentBlockKind::ToolOutput,
                timestamp,
                value
                    .get("result")
                    .or_else(|| value.get("tool"))
                    .unwrap_or(&value),
            ),
            "context.append_loop_event" => {
                if let Some(event) = value.get("event") {
                    apply_generic_value(&mut session, event, timestamp);
                }
            }
            _ => apply_generic_value(&mut session, &value, timestamp),
        }
    }
    finish_session(&mut session, provider, path);
    Ok(session)
}

fn kimi_id(path: &Path) -> String {
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let grand = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    match (grand, parent) {
        (Some(grand), Some(parent)) => sanitize_id(&format!("{grand}-{parent}")),
        (None, Some(parent)) => sanitize_id(parent),
        _ => path_id(path),
    }
}

fn parse_openhands(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    let base = path.join("base_state.json");
    if base.is_file() {
        let value: Value = serde_json::from_str(
            &fs::read_to_string(&base)
                .with_context(|| format!("failed to read {}", base.display()))?,
        )
        .with_context(|| format!("failed to parse {}", base.display()))?;
        update_identity(&mut session, &value);
        session.id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(path_id(path)));
    }
    let events = path.join("events");
    let mut files = fs::read_dir(&events)
        .with_context(|| format!("failed to read {}", events.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort();
    for event_path in files {
        if event_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let value: Value = serde_json::from_str(
            &fs::read_to_string(&event_path)
                .with_context(|| format!("failed to read {}", event_path.display()))?,
        )
        .with_context(|| format!("failed to parse {}", event_path.display()))?;
        apply_generic_value(&mut session, &value, None);
    }
    finish_session(&mut session, AgentProvider::OpenHands, path);
    Ok(session)
}

fn parse_roocode(path: &Path) -> Result<AgentSession> {
    let mut session = empty_session(path);
    let history = path.join("history_item.json");
    let metadata: Value = serde_json::from_str(
        &fs::read_to_string(&history)
            .with_context(|| format!("failed to read {}", history.display()))?,
    )
    .with_context(|| format!("failed to parse {}", history.display()))?;
    session.id = metadata
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        });
    session.cwd = metadata
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::to_string);
    session.title = metadata
        .get("task")
        .and_then(Value::as_str)
        .map(str::to_string);
    let messages = path.join("ui_messages.json");
    if !messages.is_file() {
        finish_session(&mut session, AgentProvider::RooCode, path);
        return Ok(session);
    }
    let value: Value = serde_json::from_str(
        &fs::read_to_string(&messages)
            .with_context(|| format!("failed to read {}", messages.display()))?,
    )
    .with_context(|| format!("failed to parse {}", messages.display()))?;
    let items = value
        .as_array()
        .context("RooCode ui_messages.json must be an array")?;
    for item in items {
        let kind = match item.get("type").and_then(Value::as_str) {
            Some("ask") => AgentBlockKind::User,
            Some("say") => AgentBlockKind::Assistant,
            _ => continue,
        };
        let content = item
            .get("text")
            .or_else(|| item.get("ask"))
            .or_else(|| item.get("say"))
            .unwrap_or(&Value::Null);
        push_content(&mut session, kind, json_timestamp(item), content);
        if let Some(reasoning) = item.get("reasoning") {
            push_block(
                &mut session,
                AgentBlockKind::Thinking,
                json_timestamp(item),
                None,
                extract_content_text(reasoning),
            );
        }
    }
    finish_session(&mut session, AgentProvider::RooCode, path);
    Ok(session)
}

fn parse_sqlite(path: &Path, provider: AgentProvider) -> Result<AgentSession> {
    let (physical_path, only_session) = split_sqlite_virtual_path(path);
    let conn = open_readonly_db(&physical_path)?;
    let tables = table_names(&conn)?;
    let mut session = empty_session(path);
    session.id = only_session
        .clone()
        .or_else(|| Some(path_id(&physical_path)));

    if tables.iter().any(|table| table == "threads") {
        parse_thread_rows(&conn, &mut session, only_session.as_deref())?;
    }
    if tables.iter().any(|table| table == "ItemTable") {
        parse_item_table(&conn, &mut session, only_session.as_deref())?;
    }
    if tables
        .iter()
        .any(|table| table == "message" || table == "messages")
    {
        parse_message_rows(
            &conn,
            &mut session,
            if tables.iter().any(|table| table == "message") {
                "message"
            } else {
                "messages"
            },
            only_session.as_deref(),
        )?;
    }
    finish_session(&mut session, provider, path);
    Ok(session)
}

fn table_names(conn: &rusqlite::Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(names)
}

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Result<HashSet<String>> {
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
    let mut stmt = conn.prepare(&sql)?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(columns)
}

fn parse_thread_rows(
    conn: &rusqlite::Connection,
    session: &mut AgentSession,
    only_session: Option<&str>,
) -> Result<()> {
    let columns = table_columns(conn, "threads")?;
    let id = column_or_null(&columns, "id");
    let summary = column_or_null(&columns, "summary");
    let data_type = column_or_null(&columns, "data_type");
    let data = column_or_null(&columns, "data");
    let sql = format!(
        "SELECT {id}, {summary}, {data_type}, CAST({data} AS BLOB) FROM threads WHERE (?1 IS NULL OR {id} = ?1) ORDER BY rowid"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![only_session], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    for row in rows {
        let (_id, summary, data_type, bytes) = row?;
        if session.title.is_none() {
            session.title = summary;
        }
        let bytes = decode_sqlite_payload(&bytes, data_type.as_deref().unwrap_or(""))?;
        let value: Value =
            serde_json::from_slice(&bytes).context("SQLite thread data is not valid JSON")?;
        apply_sqlite_document(session, &value);
    }
    Ok(())
}

fn parse_item_table(
    conn: &rusqlite::Connection,
    session: &mut AgentSession,
    only_session: Option<&str>,
) -> Result<()> {
    let columns = table_columns(conn, "ItemTable")?;
    if !columns.contains("key") || !columns.contains("value") {
        return Ok(());
    }
    let mut stmt =
        conn.prepare("SELECT key, CAST(value AS TEXT) FROM ItemTable WHERE value IS NOT NULL")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        if !key.to_ascii_lowercase().contains("chat")
            && !key.to_ascii_lowercase().contains("session")
        {
            continue;
        }
        let document: Value = serde_json::from_str(&value)
            .with_context(|| format!("SQLite ItemTable value `{key}` is not JSON"))?;
        if let Some(only_session) = only_session {
            let ids = document_session_ids(&document);
            if !ids.is_empty() && !ids.iter().any(|id| id == only_session) {
                continue;
            }
        }
        apply_sqlite_document(session, &document);
    }
    Ok(())
}

fn parse_message_rows(
    conn: &rusqlite::Connection,
    session: &mut AgentSession,
    table: &str,
    only_session: Option<&str>,
) -> Result<()> {
    let columns = table_columns(conn, table)?;
    let role = column_or_null(&columns, "role");
    let content = if columns.contains("content") {
        "content"
    } else if columns.contains("data") {
        "data"
    } else {
        return Ok(());
    };
    let timestamp = if columns.contains("time_created") {
        "time_created"
    } else if columns.contains("created_at") {
        "created_at"
    } else {
        "NULL"
    };
    let session_column = columns
        .iter()
        .find(|name| name.eq_ignore_ascii_case("session_id"))
        .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
        .unwrap_or_else(|| "NULL".to_string());
    let sql = format!(
        "SELECT {role}, CAST({content} AS TEXT), CAST({timestamp} AS TEXT) FROM \"{table}\" WHERE (?1 IS NULL OR {session_column} = ?1) ORDER BY rowid"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![only_session], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (role, content, timestamp) = row?;
        let Some(content) = content else { continue };
        let value = serde_json::from_str::<Value>(&content)
            .with_context(|| format!("SQLite {table} message content is not JSON"))?;
        let Some(kind) = role.as_deref().and_then(role_kind) else {
            continue;
        };
        push_content(session, kind, timestamp, &value);
    }
    Ok(())
}

fn document_session_ids(value: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    collect_ids(value, &mut ids);
    ids
}

fn collect_ids(value: &Value, ids: &mut Vec<String>) {
    if let Some(id) = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .or_else(|| value.get("conversationId"))
        .or_else(|| value.get("conversation_id"))
        .and_then(Value::as_str)
    {
        ids.push(id.to_string());
    }
    match value {
        Value::Object(map) => map.values().for_each(|value| collect_ids(value, ids)),
        Value::Array(items) => items.iter().for_each(|value| collect_ids(value, ids)),
        _ => {}
    }
}

fn apply_sqlite_document(session: &mut AgentSession, value: &Value) {
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        for message in messages {
            if let Some(user) = message.get("User") {
                push_content(
                    session,
                    AgentBlockKind::User,
                    None,
                    user.get("content").unwrap_or(user),
                );
            } else if let Some(agent) = message.get("Agent") {
                push_content(
                    session,
                    AgentBlockKind::Assistant,
                    None,
                    agent.get("content").unwrap_or(agent),
                );
            } else {
                apply_generic_value(session, message, None);
            }
        }
    } else {
        apply_generic_value(session, value, None);
    }
}

fn decode_sqlite_payload(bytes: &[u8], data_type: &str) -> Result<Vec<u8>> {
    if data_type.eq_ignore_ascii_case("zstd") {
        use ruzstd::decoding::StreamingDecoder;
        let mut decoder = StreamingDecoder::new(std::io::Cursor::new(bytes))
            .context("failed to open zstd thread payload")?;
        let mut output = Vec::new();
        decoder
            .read_to_end(&mut output)
            .context("failed to decode zstd thread payload")?;
        return Ok(output);
    }
    Ok(bytes.to_vec())
}

fn column_or_null(columns: &HashSet<String>, name: &str) -> String {
    if columns.contains(name) {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        "NULL".to_string()
    }
}

fn path_id(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sanitize_id(value: &str) -> String {
    let id: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if id.is_empty() {
        "session".to_string()
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generic_gemini_style_messages() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            path.path(),
            r#"{"sessionId":"s1","cwd":"D:/repo","title":"task"}
{"type":"user","timestamp":"2026-08-31T00:00:00Z","content":"hello"}
{"type":"assistant","content":[{"text":"answer"},{"functionCall":{"name":"read","args":{"path":"a.rs"}}}]}
{"type":"tool_result","content":"file"}
"#,
        )
        .unwrap();
        let session = parse_json_document(path.path(), AgentProvider::Iflow).unwrap();
        assert_eq!(session.id.as_deref(), Some("s1"));
        assert_eq!(session.cwd.as_deref(), Some("D:/repo"));
        assert_eq!(session.title.as_deref(), Some("task"));
        assert_eq!(session.blocks.len(), 4);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolCall);
    }

    #[test]
    fn parses_aider_runs_without_collapsing_channels() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            path.path(),
            "# aider chat started at 2026-08-31 12:00:00\n#### fix bug\n\nassistant answer\n> cargo test\n",
        )
        .unwrap();
        let session = parse_aider(path.path()).unwrap();
        assert_eq!(session.blocks.len(), 3);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].kind, AgentBlockKind::ToolOutput);
    }

    #[test]
    fn aider_container_exposes_each_run_with_one_physical_stamp() {
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            path.path(),
            "# aider chat started at 2026-08-31 12:00:00\n#### first\nanswer\n# aider chat started at 2026-08-31 13:00:00\n#### second\nanswer\n",
        )
        .unwrap();
        let sessions = list_aider_sessions(path.path()).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| {
            session.physical_path.as_deref() == Some(path.path())
                && session.path.to_string_lossy().contains('#')
        }));
    }

    #[test]
    fn provider_roots_cover_the_expanded_catalog() {
        assert!(default_roots(AgentProvider::KimiWork).len() >= 3);
        assert_eq!(root_env(AgentProvider::TraeX), Some("TRAEX_SESSIONS_DIR"));
        assert!(is_candidate_file(
            AgentProvider::Kimi,
            Path::new("wire.jsonl")
        ));
    }

    #[test]
    fn sqlite_container_exposes_each_thread_as_a_logical_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("threads.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                summary TEXT,
                data_type TEXT,
                data BLOB,
                updated_at TEXT,
                folder_paths TEXT
            )",
        )
        .unwrap();
        for (id, text) in [("thread-a", "first"), ("thread-b", "second")] {
            let document = serde_json::json!({
                "messages": [
                    {"User": {"content": {"text": format!("prompt {text}")}}},
                    {"Agent": {"content": {"text": format!("answer {text}")}}}
                ]
            });
            conn.execute(
                "INSERT INTO threads (id, summary, data_type, data, updated_at, folder_paths) VALUES (?1, ?2, 'json', ?3, '1770000000000', '[]')",
                rusqlite::params![id, text, document.to_string()],
            )
            .unwrap();
        }
        drop(conn);

        let sessions = list_sqlite_sessions(AgentProvider::Zed, &path).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| {
            session.physical_path.as_deref() == Some(path.as_path())
                && session.path.to_string_lossy().contains('#')
        }));
        for session in sessions {
            let parsed = parse_sqlite(&session.path, AgentProvider::Zed).unwrap();
            assert_eq!(parsed.id, session.id);
            assert_eq!(parsed.blocks.len(), 2);
        }
    }

    #[test]
    fn discovers_i_flow_sessions_from_the_declared_root() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("IFLOW_DIR");
        std::env::set_var("IFLOW_DIR", dir.path());
        let path = dir.path().join("project").join("session-demo.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"sessionId":"demo","cwd":"D:/repo"}
{"type":"user","content":"question"}
{"type":"assistant","content":"answer"}
"#,
        )
        .unwrap();
        let sessions = GenericProvider::new(AgentProvider::Iflow)
            .list_recent_sessions(None)
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_deref(), Some("demo"));
        assert_eq!(sessions[0].physical_path, None);
        match previous {
            Some(value) => std::env::set_var("IFLOW_DIR", value),
            None => std::env::remove_var("IFLOW_DIR"),
        }
    }
}
