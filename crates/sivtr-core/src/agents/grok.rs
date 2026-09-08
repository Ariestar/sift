use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agents::{
    extract_content_text, list_sessions_matching, parse_jsonl_session, pretty_json_string,
    pretty_json_value, push_block, push_tool_block, AgentBlockKind, AgentProvider, AgentSession,
    AgentSessionMeta, AgentSessionProvider, SessionInfo,
};

const PROVIDER_NAME: &str = "Grok";
const CHAT_HISTORY_FILE: &str = "chat_history.jsonl";
const UPDATES_FILE: &str = "updates.jsonl";
const SUMMARY_FILE: &str = "summary.json";

/// Grok (xAI) coding agent sessions.
///
/// Layout (`GROK_HOME`, default `~/.grok`):
/// ```text
/// sessions/<url-encoded-cwd>/<session-id>/
///   summary.json
///   chat_history.jsonl   ← conversation + tools (rebuilt on compaction)
///   updates.jsonl        ← ACP stream, complete since session start
/// ```
///
/// `chat_history.jsonl` is rebuilt when the session is compacted, so it only
/// covers the recent turns. `updates.jsonl` keeps every stream event (user
/// chunks, thoughts, tool calls, results, turn boundaries) — it is the
/// primary parse source; `chat_history.jsonl` is the fallback for sessions
/// without a stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrokProvider;

impl AgentSessionProvider for GrokProvider {
    fn provider(&self) -> AgentProvider {
        AgentProvider::Grok
    }

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        list_sessions_matching(
            PROVIDER_NAME,
            &grok_sessions_dir(),
            cwd,
            |path, is_dir| {
                is_dir
                    && (path.join(SUMMARY_FILE).exists() || path.join(CHAT_HISTORY_FILE).exists())
            },
            session_dir_meta,
        )
    }

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession> {
        let session_dir = resolve_session_dir(path)?;
        let mut session = if session_dir.join(UPDATES_FILE).exists() {
            let mut parsed = parse_updates_session(&session_dir)?;
            if parsed.blocks.is_empty() {
                // Noise-only stream (e.g. a session that was never used):
                // fall back to the conversation log.
                if let Ok(history) = parse_chat_history_session(&session_dir) {
                    parsed = history;
                }
            }
            parsed
        } else {
            parse_chat_history_session(&session_dir)?
        };
        // Prefer summary.json for stable id/cwd/title; keep path as the session directory.
        session.path = session_dir.clone();
        apply_summary_meta(&mut session, &session_dir)?;
        if session.id.is_none() {
            session.id = session_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string);
        }
        Ok(session)
    }
}

pub fn grok_home() -> PathBuf {
    if let Ok(path) = std::env::var("GROK_HOME") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
}

pub fn grok_sessions_dir() -> PathBuf {
    grok_home().join("sessions")
}

pub fn grok_config_path() -> PathBuf {
    grok_home().join("config.toml")
}

struct SummaryMeta {
    id: Option<String>,
    cwd: Option<String>,
    title: Option<String>,
    modified: SystemTime,
}

/// Listing metadata for one session directory: id/cwd/title from
/// `summary.json` when present, else the directory name. Recency is derived
/// from the session directory stamp by the shared listing cache.
fn session_dir_meta(path: &Path) -> Result<AgentSessionMeta> {
    Ok(match read_summary(&path.join(SUMMARY_FILE))? {
        Some(meta) => AgentSessionMeta {
            id: meta.id,
            cwd: meta.cwd,
            cwd_history: Vec::new(),
            title: meta.title,
        },
        None => AgentSessionMeta {
            id: path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
            ..AgentSessionMeta::default()
        },
    })
}

fn read_summary(path: &Path) -> Result<Option<SummaryMeta>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("Failed to read Grok summary {}", path.display()))?;
    // A corrupt summary is supplementary metadata: skip it (fall back to the
    // directory name) rather than failing discovery of every other session.
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Ok(None);
    };

    let info = value.get("info").unwrap_or(&value);
    let id = info
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
        });
    let cwd = info
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_string);
    let title = value
        .get("generated_title")
        .or_else(|| value.get("session_summary"))
        .or_else(|| value.get("title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);

    let modified = parse_rfc3339(
        value
            .get("last_active_at")
            .or_else(|| value.get("updated_at"))
            .or_else(|| value.get("created_at"))
            .and_then(Value::as_str),
    )
    .or_else(|| fs::metadata(path).and_then(|meta| meta.modified()).ok())
    .unwrap_or(UNIX_EPOCH);

    Ok(Some(SummaryMeta {
        id,
        cwd,
        title,
        modified,
    }))
}

fn resolve_session_dir(path: &Path) -> Result<PathBuf> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == CHAT_HISTORY_FILE || name == SUMMARY_FILE)
    {
        return path
            .parent()
            .map(Path::to_path_buf)
            .with_context(|| format!("Grok session path `{}` has no parent dir", path.display()));
    }
    // Bare session id under GROK_HOME/sessions/*/<id>
    if let Some(found) = GrokProvider.find_session_by_id(
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    )? {
        return Ok(found);
    }
    anyhow::bail!(
        "Grok session path `{}` is not a session directory or {CHAT_HISTORY_FILE}",
        path.display()
    )
}

fn apply_event(session: &mut AgentSession, value: &Value) {
    match value.get("type").and_then(Value::as_str) {
        Some("system") => {}
        Some("user") => apply_user(session, value),
        Some("assistant") => apply_assistant(session, value),
        Some("tool_result") => apply_tool_result(session, value),
        Some("reasoning") => apply_reasoning(session, value),
        _ => {}
    }
}

fn apply_user(session: &mut AgentSession, value: &Value) {
    // Injected scaffolding (system reminders, project instructions) is not a user turn.
    if value.get("synthetic_reason").is_some() {
        return;
    }

    let text = extract_user_text(value.get("content").unwrap_or(&Value::Null));
    if text.trim().is_empty() {
        return;
    }
    push_block(session, AgentBlockKind::User, None, None, text);
}

fn apply_assistant(session: &mut AgentSession, value: &Value) {
    let content = value.get("content").unwrap_or(&Value::Null);
    let text = extract_content_text(content);
    if !text.trim().is_empty() {
        push_block(session, AgentBlockKind::Assistant, None, None, text);
    }

    if let Some(tool_calls) = value.get("tool_calls") {
        match tool_calls {
            Value::Array(items) => {
                for tool_call in items {
                    push_one_tool_call(session, tool_call);
                }
            }
            Value::String(raw) => {
                if let Ok(Value::Array(items)) = serde_json::from_str(raw) {
                    for tool_call in items {
                        push_one_tool_call(session, &tool_call);
                    }
                } else {
                    push_block(
                        session,
                        AgentBlockKind::ToolCall,
                        None,
                        None,
                        pretty_json_string(raw),
                    );
                }
            }
            other => push_one_tool_call(session, other),
        }
    }
}

fn apply_tool_result(session: &mut AgentSession, value: &Value) {
    let content = value.get("content").unwrap_or(&Value::Null);
    let text = match content {
        Value::String(text) => text.clone(),
        other => extract_content_text(other),
    };
    push_tool_block(
        session,
        AgentBlockKind::ToolOutput,
        None,
        value
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        None,
        text,
        None,
    );
}

fn apply_reasoning(session: &mut AgentSession, value: &Value) {
    let summary = value.get("summary").unwrap_or(&Value::Null);
    let text = match summary {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::String(text) => text.clone(),
        other => extract_content_text(other),
    };
    if text.trim().is_empty() {
        return;
    }
    push_block(session, AgentBlockKind::Thinking, None, None, text);
}

/// Fill id/cwd/title from summary.json and give timestamp-less blocks the
/// session's last activity time so records sort by recency like other
/// providers instead of sinking below timestamped records.
fn apply_summary_meta(session: &mut AgentSession, session_dir: &Path) -> Result<()> {
    if let Some(meta) = read_summary(&session_dir.join(SUMMARY_FILE))? {
        if session.id.is_none() {
            session.id = meta.id;
        }
        if session.cwd.is_none() {
            session.cwd = meta.cwd;
        }
        if session.title.is_none() {
            session.title = meta.title;
        }
        if session.blocks.iter().all(|block| block.timestamp.is_none()) {
            let stamp = rfc3339_from_system_time(meta.modified);
            for block in &mut session.blocks {
                block.timestamp = Some(stamp.clone());
            }
        }
    }
    Ok(())
}

fn parse_chat_history_session(session_dir: &Path) -> Result<AgentSession> {
    let history = session_dir.join(CHAT_HISTORY_FILE);
    if !history.exists() {
        anyhow::bail!(
            "Grok session `{}` is missing {CHAT_HISTORY_FILE}",
            session_dir.display()
        );
    }
    parse_jsonl_session(&history, PROVIDER_NAME, apply_event)
}

/// Parse the ACP stream (`updates.jsonl`), the complete session record.
///
/// Text chunks accumulate per kind and flush as blocks at tool events and
/// turn boundaries; `tool_call` / `tool_call_update` events emit tool
/// call/result blocks in stream order. Stream events that carry no record
/// content (`hook_execution`, `plan`, `retry_state`, compaction markers, …)
/// are ignored.
fn parse_updates_session(session_dir: &Path) -> Result<AgentSession> {
    let updates = session_dir.join(UPDATES_FILE);
    let mut session = AgentSession {
        path: session_dir.to_path_buf(),
        id: None,
        cwd: None,
        title: None,
        blocks: Vec::new(),
    };
    let mut parser = UpdatesParser::default();
    for line in fs::read_to_string(&updates)
        .with_context(|| format!("Failed to read Grok updates {}", updates.display()))?
        .lines()
    {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        parser.apply(&mut session, &value);
    }
    parser.flush(&mut session);
    Ok(session)
}

/// Assembler for the ACP stream: pending text segments (in stream order, so
/// interleaved thinking/message stays ordered) plus the toolCallId → tool
/// name map used to label results.
#[derive(Default)]
struct UpdatesParser {
    segments: Vec<(AgentBlockKind, String, Option<String>)>,
    tool_names: HashMap<String, String>,
    timestamp: Option<String>,
}

impl UpdatesParser {
    fn apply(&mut self, session: &mut AgentSession, value: &Value) {
        let Some(update) = value.get("params").and_then(|params| params.get("update")) else {
            return;
        };
        self.timestamp = event_timestamp(value);
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("user_message_chunk") => {
                self.flush(session);
                self.append(AgentBlockKind::User, update);
            }
            Some("agent_message_chunk") => {
                self.flush_user(session);
                self.append(AgentBlockKind::Assistant, update);
            }
            Some("agent_thought_chunk") => {
                self.flush_user(session);
                self.append(AgentBlockKind::Thinking, update);
            }
            Some("tool_call") => {
                self.flush(session);
                self.apply_tool_call(session, update);
            }
            Some("tool_call_update") => self.apply_tool_update(session, update),
            Some("turn_completed") => self.flush(session),
            _ => {}
        }
    }

    fn append(&mut self, kind: AgentBlockKind, update: &Value) {
        let text = chunk_text(update);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match self.segments.last_mut() {
            Some((last_kind, last_text, _)) if *last_kind == kind => {
                last_text.push('\n');
                last_text.push_str(text);
            }
            _ => self
                .segments
                .push((kind, text.to_string(), self.timestamp.clone())),
        }
    }

    /// Flush the pending user segment at the start of the assistant turn.
    fn flush_user(&mut self, session: &mut AgentSession) {
        if self
            .segments
            .first()
            .is_some_and(|(kind, _, _)| *kind == AgentBlockKind::User)
        {
            self.flush(session);
        }
    }

    /// Emit all pending text segments as blocks, in stream order.
    fn flush(&mut self, session: &mut AgentSession) {
        for (kind, text, timestamp) in self.segments.drain(..) {
            push_block(session, kind, timestamp, None, text);
        }
    }

    fn apply_tool_call(&mut self, session: &mut AgentSession, update: &Value) {
        let tool_call_id = update.get("toolCallId").and_then(Value::as_str);
        let name = update
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                update
                    .get("_meta")
                    .and_then(|meta| meta.get("x.ai/tool"))
                    .and_then(|tool| tool.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "tool".to_string());
        if let Some(id) = tool_call_id {
            self.tool_names.insert(id.to_string(), name.clone());
        }
        let arguments = update
            .get("rawInput")
            .map(pretty_json_value)
            .unwrap_or_else(|| pretty_json_value(update));
        push_tool_block(
            session,
            AgentBlockKind::ToolCall,
            self.timestamp.clone(),
            tool_call_id.map(str::to_string),
            Some(name),
            arguments,
            None,
        );
    }

    fn apply_tool_update(&mut self, session: &mut AgentSession, update: &Value) {
        if !matches!(
            update.get("status").and_then(Value::as_str),
            Some("completed" | "failed")
        ) {
            return;
        }
        let text = tool_result_text(update);
        if text.trim().is_empty() {
            return;
        }
        // The provider owns its output shapes: read results number every
        // line (`775→ …`) and grep output rides in a `<workspace_result …>`
        // envelope — both are stripped here so blocks carry clean text, with
        // the read start line kept as explicit metadata.
        let raw_type = update
            .get("rawOutput")
            .and_then(|raw| raw.get("type"))
            .and_then(Value::as_str);
        let (text, start_line) = match raw_type {
            Some("ReadFile") => match strip_read_gutter(&text) {
                Some((start, clean)) => (clean, Some(start)),
                None => (text, None),
            },
            Some("GrepSearch") => (strip_workspace_envelope(&text), None),
            _ => (text, None),
        };
        let tool_call_id = update.get("toolCallId").and_then(Value::as_str);
        let name = tool_call_id.and_then(|id| self.tool_names.get(id)).cloned();
        push_tool_block(
            session,
            AgentBlockKind::ToolOutput,
            self.timestamp.clone(),
            tool_call_id.map(str::to_string),
            name,
            text,
            start_line,
        );
    }
}

/// Text of a message/thought chunk: `content.text` (or array of text items).
fn chunk_text(update: &Value) -> String {
    match update.get("content").unwrap_or(&Value::Null) {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        other => other
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// Text of a tool result: the raw output body per the tool's own shape
/// (`rawOutput.type` dispatches to the field grok build wrote), byte arrays
/// decoded as UTF-8, unknown shapes kept verbatim so the transcript is
/// preserved. `content[].content.text` only backs up events without a
/// `rawOutput` — on real events it carries the human description, not the
/// result payload.
fn tool_result_text(update: &Value) -> String {
    if let Some(text) = update
        .get("rawOutput")
        .map(tool_raw_output_text)
        .filter(|text| !text.trim().is_empty())
    {
        return text;
    }
    if let Some(items) = update.get("content").and_then(Value::as_array) {
        return items
            .iter()
            .filter_map(|item| {
                item.get("content")
                    .and_then(|content| content.get("text"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

/// Result body of a raw output event, per its declared `type`: known shapes
/// are extracted from the field grok build writes them to, unknown shapes
/// keep the whole event pretty-printed — no recursive guessing.
fn tool_raw_output_text(raw: &Value) -> String {
    let body = match raw.get("type").and_then(Value::as_str) {
        Some("GrepSearch") => raw.get("stdout"),
        Some("Bash") => raw.get("output"),
        Some("ReadFile") => raw.get("FileContent").and_then(|v| v.get("content")),
        Some("ListDir") => raw.get("Content").and_then(|v| v.get("content")),
        Some("TaskOutput") => raw.get("Result").and_then(|v| v.get("output")),
        _ => None,
    };
    if let Some(body) = body {
        return match body {
            Value::String(text) => text.trim().to_string(),
            Value::Array(bytes) => String::from_utf8_lossy(
                &bytes
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|byte| byte as u8)
                    .collect::<Vec<_>>(),
            )
            .trim()
            .to_string(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        };
    }
    serde_json::to_string_pretty(raw).unwrap_or_default()
}

/// grok numbers read results every ten lines (`775→` on lines 1, 10, 20 …),
/// not per line. When the *first* line carries the `N→` gutter, strip the
/// gutter and keep its number as the start line; unmarked lines pass through
/// untouched, so the body is exactly the file content.
fn strip_read_gutter(text: &str) -> Option<(u64, String)> {
    let mut lines = text.lines();
    let first = lines.next()?;
    let (num, rest) = first.split_once('→')?;
    let start = num.trim().parse::<u64>().ok()?;
    let mut out = vec![strip_gutter_space(rest)];
    for line in lines {
        match line.split_once('→') {
            Some((num, rest)) if num.trim().parse::<u64>().is_ok() => {
                out.push(strip_gutter_space(rest));
            }
            _ => out.push(line.to_string()),
        }
    }
    Some((start, out.join("\n")))
}

/// One optional space after the arrow keeps the code's own indentation.
fn strip_gutter_space(rest: &str) -> String {
    rest.strip_prefix(' ').unwrap_or(rest).to_string()
}

/// grok wraps grep output in `<workspace_result …>` — tool plumbing, not a
/// match. The provider owns its shape, so it drops out before the generic
/// grep structure reaches the display layer.
fn strip_workspace_envelope(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("<workspace_result"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Event time from the ACP envelope: precise `agentTimestampMs` when present,
/// else the top-level unix-seconds `timestamp`.
fn event_timestamp(value: &Value) -> Option<String> {
    let millis = value
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get("agentTimestampMs"))
        .and_then(Value::as_i64)
        .or_else(|| {
            value
                .get("timestamp")
                .and_then(Value::as_i64)
                .map(|secs| secs * 1000)
        });
    millis.map(rfc3339_from_millis)
}

fn rfc3339_from_millis(millis: i64) -> String {
    let time = UNIX_EPOCH + Duration::from_millis(millis.max(0) as u64);
    rfc3339_from_system_time(time)
}

fn push_one_tool_call(session: &mut AgentSession, tool_call: &Value) {
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
    push_tool_block(
        session,
        AgentBlockKind::ToolCall,
        None,
        tool_call
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string),
        name,
        arguments,
        None,
    );
}

fn extract_user_text(content: &Value) -> String {
    let raw = extract_content_text(content);
    extract_user_query(&raw).unwrap_or(raw)
}

fn extract_user_query(text: &str) -> Option<String> {
    let start_tag = "<user_query>";
    let end_tag = "</user_query>";
    let start = text.find(start_tag)? + start_tag.len();
    let end = text[start..].find(end_tag)? + start;
    let query = text[start..end].trim();
    if query.is_empty() {
        None
    } else {
        Some(query.to_string())
    }
}

fn parse_rfc3339(value: Option<&str>) -> Option<SystemTime> {
    let value = value?;
    let dt = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    let secs = dt.timestamp();
    let nanos = dt.timestamp_subsec_nanos();
    if secs < 0 {
        return Some(UNIX_EPOCH);
    }
    Some(UNIX_EPOCH + Duration::new(secs as u64, nanos))
}

fn rfc3339_from_system_time(time: SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = time.into();
    dt.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentBlockKind, AgentSessionProvider};

    fn write_session(
        home: &Path,
        cwd_bucket: &str,
        session_id: &str,
        history: &str,
        summary: &str,
    ) {
        let dir = home.join("sessions").join(cwd_bucket).join(session_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(CHAT_HISTORY_FILE), history).unwrap();
        fs::write(dir.join(SUMMARY_FILE), summary).unwrap();
    }

    #[test]
    fn parses_grok_messages_tools_and_reasoning() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "019f6119-df57-7fe1-8e38-e2e41d5a506e";
        write_session(
            home.path(),
            "D%3A%5CCoding%5Crepo",
            session_id,
            r#"{"type":"system","content":"You are Grok"}
{"type":"user","content":[{"type":"text","text":"<user_info>\nWorkspace Path: D:\\repo\n</user_info>"}]}
{"type":"user","content":[{"type":"text","text":"<system-reminder>\nskills\n</system-reminder>"}],"synthetic_reason":"system_reminder"}
{"type":"user","content":[{"type":"text","text":"<user_query>\nfix the ghosting\n</user_query>"}],"prompt_index":0}
{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Inspect gallery CSS"}],"status":"completed"}
{"type":"assistant","content":"Looking at the gallery","tool_calls":[{"id":"call-1","name":"read_file","arguments":"{\"target_file\":\"a.tsx\"}"}],"model_id":"grok-4.5"}
{"type":"tool_result","tool_call_id":"call-1","content":"file body"}
{"type":"assistant","content":"done","model_id":"grok-4.5"}
"#,
            r#"{
  "info": {"id": "019f6119-df57-7fe1-8e38-e2e41d5a506e", "cwd": "D:\\Coding\\repo"},
  "generated_title": "fix gallery ghosting",
  "last_active_at": "2026-07-14T15:19:44.953389100Z"
}"#,
        );

        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let path = home
            .path()
            .join("sessions")
            .join("D%3A%5CCoding%5Crepo")
            .join(session_id);
        let session = GrokProvider.parse_session_file(&path).unwrap();

        assert_eq!(session.id.as_deref(), Some(session_id));
        assert_eq!(session.cwd.as_deref(), Some("D:\\Coding\\repo"));
        assert_eq!(session.title.as_deref(), Some("fix gallery ghosting"));
        assert_eq!(session.blocks.len(), 6);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "fix the ghosting");
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "Inspect gallery CSS");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "Looking at the gallery");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].label.as_deref(), Some("read_file"));
        assert!(session.blocks[3].text.contains("a.tsx"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].text, "file body");
        assert_eq!(session.blocks[5].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[5].text, "done");
    }

    #[test]
    fn parses_updates_stream_with_tool_calls() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "019ff6dd-e8db-7472-b6e8-8b122bc63a3b";
        let dir = home.path().join("sessions").join("bucket").join(session_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(UPDATES_FILE),
            r#"{"timestamp":1000,"params":{"sessionId":"s1","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"fix it"}},"_meta":{"agentTimestampMs":1000000}}}
{"timestamp":1001,"params":{"sessionId":"s1","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Plan: inspect a.rs"}},"_meta":{"agentTimestampMs":1001000}}}
{"timestamp":1001,"params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Looking at the file"}},"_meta":{"agentTimestampMs":1002000}}}
{"timestamp":1002,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"call-1","title":"read_file","rawInput":{"target_file":"a.rs","offset":0,"limit":10}},"_meta":{"agentTimestampMs":1003000}}}
{"timestamp":1002,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"fn main() {}"}}]},"_meta":{"agentTimestampMs":1004000}}}
{"timestamp":1003,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"call-2","title":"use_tool","rawInput":{"tool_name":"sivtr__sivtr_search","tool_input":{"source":"claude"}}},"_meta":{"agentTimestampMs":1005000}}}
{"timestamp":1003,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-2","status":"completed","rawOutput":{"type":"Todo","TodosUpdated":{"summary_for_prompt":"count 3"}}},"_meta":{"agentTimestampMs":1006000}}}
{"timestamp":1004,"params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"done"}},"_meta":{"agentTimestampMs":1007000}}}
{"timestamp":1004,"params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"now"}},"_meta":{"agentTimestampMs":1007001}}}
{"timestamp":1004,"params":{"sessionId":"s1","update":{"sessionUpdate":"turn_completed"},"_meta":{"agentTimestampMs":1008000}}}
{"timestamp":1005,"params":{"sessionId":"s1","update":{"sessionUpdate":"hook_execution"}}}
"#,
        )
        .unwrap();
        fs::write(
            dir.join(SUMMARY_FILE),
            r#"{"info":{"id":"019ff6dd-e8db-7472-b6e8-8b122bc63a3b","cwd":"D:\\Coding\\sivtr-tui-stack"},"generated_title":"tool render","last_active_at":"2026-07-14T15:19:44Z"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let session = GrokProvider.parse_session_file(&dir).unwrap();

        assert_eq!(session.id.as_deref(), Some(session_id));
        assert_eq!(session.blocks.len(), 8);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "fix it");
        assert_eq!(
            session.blocks[0].timestamp.as_deref(),
            Some("1970-01-01T00:16:40+00:00")
        );
        assert_eq!(session.blocks[1].kind, AgentBlockKind::Thinking);
        assert_eq!(session.blocks[1].text, "Plan: inspect a.rs");
        assert_eq!(session.blocks[2].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[2].text, "Looking at the file");
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[3].label.as_deref(), Some("read_file"));
        assert!(session.blocks[3].text.contains("a.rs"));
        assert_eq!(session.blocks[4].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[4].label.as_deref(), Some("read_file"));
        assert_eq!(session.blocks[4].text, "fn main() {}");
        assert_eq!(session.blocks[5].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[5].label.as_deref(), Some("use_tool"));
        assert!(session.blocks[5].text.contains("sivtr__sivtr_search"));
        assert_eq!(session.blocks[6].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[6].label.as_deref(), Some("use_tool"));
        // Unknown rawOutput shapes keep the whole event verbatim.
        assert!(session.blocks[6].text.contains("summary_for_prompt"));
        assert!(session.blocks[6].text.contains("count 3"));
        assert_eq!(session.blocks[7].kind, AgentBlockKind::Assistant);
        assert_eq!(session.blocks[7].text, "done\nnow");
    }

    #[test]
    fn strips_provider_shapes_read_gutter_and_grep_envelope() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "019ff6dd-clean";
        let dir = home.path().join("sessions").join("bucket").join(session_id);
        fs::create_dir_all(&dir).unwrap();
        let _ = fs::write(
            dir.join(UPDATES_FILE),
            r#"{"timestamp":1000,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"r1","title":"read_file","rawInput":{"target_file":"a.rs"}},"_meta":{"agentTimestampMs":1000000}}}
{"timestamp":1001,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"r1","status":"completed","rawOutput":{"type":"ReadFile","FileContent":{"content":"775→ line one\n776→ line two\n"}}},"_meta":{"agentTimestampMs":1001000}}}
{"timestamp":1002,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"g1","title":"GrepSearch","rawInput":{"pattern":"fn"}},"_meta":{"agentTimestampMs":1002000}}}
{"timestamp":1003,"params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"g1","status":"completed","rawOutput":{"type":"GrepSearch","stdout":[60,119,111,114,107,115,112,97,99,101,95,114,101,115,117,108,116,62,10,70,111,117,110,100,32,49,32,109,97,116,99,104,105,110,103,32,108,105,110,101,115,10,97,46,114,115,10,55,58,102,110,32,109,97,105,110,40,41,10]}},"_meta":{"agentTimestampMs":1003000}}}
"#,
        );
        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let path = home.path().join("sessions").join("bucket").join(session_id);
        let session = GrokProvider.parse_session_file(&path).unwrap();

        // Read result: the `N→` gutter is stripped, the start line kept as
        // generic metadata for the display layer.
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolOutput);
        assert_eq!(session.blocks[1].text, "line one\nline two");
        assert_eq!(session.blocks[1].start_line, Some(775));

        // Grep result: the `<workspace_result …>` envelope drops, matches stay.
        assert_eq!(session.blocks[3].kind, AgentBlockKind::ToolOutput);
        assert!(!session.blocks[3].text.contains("<workspace_result"));
        assert!(session.blocks[3].text.contains("Found 1 matching lines"));
        assert!(session.blocks[3].text.contains("7:fn main()"));
    }

    #[test]
    fn read_gutter_strips_when_only_every_tenth_line_is_marked() {
        // Real grok shape: the `N→` gutter appears on the first line and
        // every tenth line, plain lines in between pass through untouched.
        assert_eq!(
            strip_read_gutter("775→ line one\nplain line\n776→ line two"),
            Some((775, "line one\nplain line\nline two".to_string()))
        );
        // A leading `1→` on an empty first line keeps the blank line.
        assert_eq!(
            strip_read_gutter("1→\n--\nname: help\n10→\n\n# Title"),
            Some((1, "\n--\nname: help\n\n\n# Title".to_string()))
        );
        // No gutter at all: verbatim (None keeps the transcript).
        assert_eq!(strip_read_gutter("plain text\nmore"), None);
    }

    #[test]
    fn tool_raw_output_text_extracts_each_shape_verbatim() {
        // GrepSearch: stdout is a UTF-8 byte array.
        let grep = serde_json::json!({
            "status": "completed",
            "rawOutput": {
                "type": "GrepSearch",
                "stdout": [70, 111, 117, 110, 100, 32, 50, 32, 109, 97, 116, 99, 104, 101, 115],
            },
        });
        assert_eq!(tool_result_text(&grep), "Found 2 matches");

        // Bash: output byte array.
        let bash = serde_json::json!({
            "status": "completed",
            "rawOutput": {
                "type": "Bash",
                "output": [111, 107, 10, 119, 97, 114, 110],
            },
        });
        assert_eq!(tool_result_text(&bash), "ok\nwarn");

        // ReadFile / ListDir: nested content strings.
        let read = serde_json::json!({
            "status": "completed",
            "rawOutput": {
                "type": "ReadFile",
                "FileContent": {"content": "fn main() {}\n"},
            },
        });
        assert_eq!(tool_result_text(&read), "fn main() {}");

        // Unknown shape: the whole rawOutput survives, nothing is guessed.
        let unknown = serde_json::json!({
            "status": "completed",
            "rawOutput": {
                "type": "Todo",
                "TodosUpdated": {"summary_for_prompt": "count 3"},
            },
        });
        let text = tool_result_text(&unknown);
        assert!(text.contains("summary_for_prompt"));
        assert!(text.contains("count 3"));

        // content text backs up events without rawOutput.
        let described = serde_json::json!({
            "status": "completed",
            "content": [{"type": "content", "content": {"type": "text", "text": "fn main() {}"}}],
        });
        assert_eq!(tool_result_text(&described), "fn main() {}");
    }

    #[test]
    fn noise_only_updates_stream_falls_back_to_chat_history() {
        let home = tempfile::tempdir().unwrap();
        let session_id = "sess-fallback";
        let dir = home.path().join("sessions").join("bucket").join(session_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(UPDATES_FILE),
            "{\"params\":{\"update\":{\"sessionUpdate\":\"hook_execution\"}}}\n",
        )
        .unwrap();
        fs::write(
            dir.join(CHAT_HISTORY_FILE),
            r#"{"type":"user","content":[{"type":"text","text":"<user_query>\nhello\n</user_query>"}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join(SUMMARY_FILE),
            r#"{"info":{"id":"sess-fallback","cwd":"/tmp"}}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let session = GrokProvider.parse_session_file(&dir).unwrap();

        assert_eq!(session.blocks.len(), 1);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::User);
        assert_eq!(session.blocks[0].text, "hello");
    }

    #[test]
    fn lists_sessions_and_filters_by_cwd() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        write_session(
            home.path(),
            "repo-bucket",
            "sess-match",
            "{\"type\":\"user\",\"content\":\"hi\"}\n",
            &format!(
                r#"{{"info":{{"id":"sess-match","cwd":{}}},"generated_title":"match","last_active_at":"2026-07-14T15:00:00Z"}}"#,
                serde_json::to_string(&repo).unwrap()
            ),
        );
        write_session(
            home.path(),
            "other-bucket",
            "sess-other",
            "{\"type\":\"user\",\"content\":\"yo\"}\n",
            r#"{"info":{"id":"sess-other","cwd":"/other"},"generated_title":"other","last_active_at":"2026-07-14T16:00:00Z"}"#,
        );

        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let listed = GrokProvider
            .list_recent_sessions(Some(&repo))
            .expect("list");
        let ids: Vec<_> = listed
            .iter()
            .filter_map(|session| session.id.clone())
            .collect();
        assert!(ids.contains(&"sess-match".to_string()));
        assert!(!ids.contains(&"sess-other".to_string()));
        assert_eq!(listed[0].title.as_deref(), Some("match"));
    }

    #[test]
    fn parse_accepts_chat_history_path() {
        let home = tempfile::tempdir().unwrap();
        write_session(
            home.path(),
            "bucket",
            "sess1",
            r#"{"type":"user","content":[{"type":"text","text":"hello"}]}"#,
            r#"{"info":{"id":"sess1","cwd":"/tmp"},"session_summary":"title"}"#,
        );
        let _guard = EnvGuard::set("GROK_HOME", home.path());
        let history = home
            .path()
            .join("sessions")
            .join("bucket")
            .join("sess1")
            .join(CHAT_HISTORY_FILE);
        let session = GrokProvider.parse_session_file(&history).unwrap();
        assert_eq!(session.id.as_deref(), Some("sess1"));
        assert_eq!(session.blocks[0].text, "hello");
    }

    #[test]
    fn empty_assistant_with_tools_still_records_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHAT_HISTORY_FILE);
        fs::write(
            &path,
            r#"{"type":"assistant","content":"","tool_calls":[{"id":"c1","name":"bash","arguments":"{\"command\":\"ls\"}"}]}
{"type":"tool_result","tool_call_id":"c1","content":"a.rs"}
"#,
        )
        .unwrap();
        // minimal summary sibling so resolve works when given dir
        fs::write(
            dir.path().join(SUMMARY_FILE),
            r#"{"info":{"id":"x","cwd":"/tmp"}}"#,
        )
        .unwrap();

        let session = GrokProvider.parse_session_file(dir.path()).unwrap();
        assert_eq!(session.blocks.len(), 2);
        assert_eq!(session.blocks[0].kind, AgentBlockKind::ToolCall);
        assert_eq!(session.blocks[0].label.as_deref(), Some("bash"));
        assert_eq!(session.blocks[1].kind, AgentBlockKind::ToolOutput);
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
