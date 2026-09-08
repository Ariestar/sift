use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentProvider {
    Claude,
    Codex,
    Cursor,
    Dsh,
    Gemini,
    Goose,
    Grok,
    Hermes,
    OpenClaw,
    OpenCode,
    Pi,
    Qoder,
    QoderCn,
    Qwen,
    Zcode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentBlockKind {
    User,
    Assistant,
    /// Tool invocation (name in `label` when known). MCP tools stay tools.
    ToolCall,
    /// Tool result (name in `label` when known).
    ToolOutput,
    /// Explicit skill invocation / payload (name in `label`).
    Skill,
    /// Model reasoning / thinking channel (not dialogue body).
    Thinking,
}

impl AgentBlockKind {
    pub fn is_dialogue(self) -> bool {
        matches!(self, Self::User | Self::Assistant)
    }

    pub fn is_structure(self) -> bool {
        !self.is_dialogue()
    }

    /// Content-block open tag for structural kinds (`<:tool:bash call:>`, …).
    /// Dialogue returns `None` (plain text, no wrapper).
    pub fn open_marker(self, label: Option<&str>) -> Option<String> {
        let name = normalize_structure_name(label);
        Some(match self {
            Self::User | Self::Assistant => return None,
            Self::ToolCall => format!("<:tool:{name} call:>"),
            Self::ToolOutput => format!("<:tool:{name} result:>"),
            Self::Skill => format!("<:skill:{name}:>"),
            Self::Thinking => "<:thinking:>".to_string(),
        })
    }

    fn close_marker(self, label: Option<&str>) -> Option<String> {
        self.open_marker(label).map(|open| {
            // `<:tag:>` → `<:/tag:>`
            open.replacen("<:", "<:/", 1)
        })
    }

    /// Serialize body for evidence export. Dialogue is plain; structure is marked.
    pub fn format_block(self, label: Option<&str>, text: &str) -> String {
        let text = text.trim();
        match self.open_marker(label) {
            None => text.to_string(),
            Some(open) => {
                let close = self.close_marker(label).unwrap_or_default();
                if text.is_empty() {
                    format!("{open}\n{close}")
                } else {
                    format!("{open}\n{text}\n{close}")
                }
            }
        }
    }
}

fn normalize_structure_name(label: Option<&str>) -> &str {
    label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBlock {
    pub kind: AgentBlockKind,
    pub timestamp: Option<String>,
    pub label: Option<String>,
    pub call_id: Option<String>,
    /// First file line of a read-result body, when the provider numbers its
    /// output (`775→ …`). Generic metadata — providers that number their
    /// read results set it, others leave `None`.
    pub start_line: Option<u64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub path: PathBuf,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub blocks: Vec<AgentBlock>,
}

pub use crate::session_source::SessionInfo;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionMeta {
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub cwd_history: Vec<String>,
    pub title: Option<String>,
}

impl AgentSessionMeta {
    pub fn add_cwd(&mut self, cwd: impl Into<String>) {
        let cwd = cwd.into();
        if cwd.trim().is_empty() {
            return;
        }
        if self.cwd.is_none() {
            self.cwd = Some(cwd.clone());
        }
        if !self.cwd_history.iter().any(|existing| existing == &cwd) {
            self.cwd_history.push(cwd);
        }
    }

    /// Fall back to the first line of the first user message as the title
    /// when no explicit title (summary / user-set name) was captured.
    pub fn fallback_title(&mut self, first_user_text: Option<&str>) {
        if self.title.is_some() {
            return;
        }
        let Some(text) = first_user_text else {
            return;
        };
        let title = text.lines().next().unwrap_or(text).trim();
        if !title.is_empty() {
            self.title = Some(title.to_string());
        }
    }

    pub(crate) fn cwd_candidates(&self) -> impl Iterator<Item = &str> {
        self.cwd_history.iter().map(String::as_str).chain(
            self.cwd
                .as_deref()
                .into_iter()
                .filter(|cwd| !self.cwd_history.iter().any(|existing| existing == cwd)),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSelection {
    LastTurn,
    LastAssistant,
    LastUser,
    LastTool,
    LastBlocks(usize),
    All,
}

impl AgentSelection {
    pub fn label(self) -> &'static str {
        match self {
            Self::LastTurn => "turn",
            Self::LastAssistant => "assistant",
            Self::LastUser => "user",
            Self::LastTool => "tool",
            Self::LastBlocks(_) => "blocks",
            Self::All => "all",
        }
    }
}

pub trait AgentSessionProvider {
    fn provider(&self) -> AgentProvider;

    fn list_recent_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>>;

    fn parse_session_file(&self, path: &Path) -> Result<AgentSession>;

    fn find_session_by_id(&self, id: &str) -> Result<Option<PathBuf>> {
        for session in self.list_recent_sessions(None)? {
            if session
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(id))
                || session.id.as_deref() == Some(id)
            {
                return Ok(Some(session.path));
            }
        }

        Ok(None)
    }

    fn find_current_session(&self, cwd: &Path) -> Result<Option<PathBuf>> {
        if let Some(session) = self.list_recent_sessions(Some(cwd))?.into_iter().next() {
            return Ok(Some(session.path));
        }

        Ok(self
            .list_recent_sessions(None)?
            .into_iter()
            .next()
            .map(|session| session.path))
    }
}

pub fn push_block(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    label: Option<String>,
    text: impl Into<String>,
) {
    let text = text.into().trim().to_string();
    if !text.is_empty() {
        session.blocks.push(AgentBlock {
            kind,
            timestamp,
            label,
            call_id: None,
            start_line: None,
            text,
        });
    }
}

pub fn push_tool_block(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    call_id: Option<String>,
    label: Option<String>,
    text: impl Into<String>,
    start_line: Option<u64>,
) {
    let text = text.into().trim().to_string();
    if !text.is_empty() {
        session.blocks.push(AgentBlock {
            kind,
            timestamp,
            label,
            call_id,
            start_line,
            text,
        });
    }
}

/// Push Gemini-API-style content parts as blocks: `text` parts become the
/// given dialogue kind, `thought` parts become `Thinking`, and
/// `functionCall` / `functionResponse` parts become `ToolCall` / `ToolOutput`.
///
/// Shared by the Gemini CLI and Qwen Code providers, whose transcripts both
/// store content as Gemini API `Part` objects. Plain string content is pushed
/// as a single block of `kind`.
pub fn push_parts_blocks(
    session: &mut AgentSession,
    kind: AgentBlockKind,
    timestamp: Option<String>,
    content: &Value,
) {
    match content {
        Value::String(text) => push_block(session, kind, timestamp, None, text),
        Value::Array(items) => {
            for item in items {
                if item.get("thought").and_then(Value::as_bool) == Some(true) {
                    push_block(
                        session,
                        AgentBlockKind::Thinking,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    );
                } else if let Some(call) = item.get("functionCall") {
                    push_tool_block(
                        session,
                        AgentBlockKind::ToolCall,
                        timestamp.clone(),
                        part_id(item, call),
                        call.get("name").and_then(Value::as_str).map(str::to_string),
                        pretty_json_value(call.get("args").unwrap_or(&Value::Null)),
                        None,
                    );
                } else if let Some(response) = item.get("functionResponse") {
                    let body = response.get("response").unwrap_or(&Value::Null);
                    let text = extract_content_text(body);
                    push_tool_block(
                        session,
                        AgentBlockKind::ToolOutput,
                        timestamp.clone(),
                        part_id(item, response),
                        response
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        text,
                        None,
                    );
                } else {
                    push_block(
                        session,
                        kind,
                        timestamp.clone(),
                        None,
                        extract_content_text(item),
                    );
                }
            }
        }
        _ => {}
    }
}

fn part_id(item: &Value, nested: &Value) -> Option<String> {
    item.get("id")
        .and_then(Value::as_str)
        .or_else(|| nested.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

pub fn extract_content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Object(object) => {
            if let Some(text) = object
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| object.get("input_text").and_then(Value::as_str))
                .or_else(|| object.get("output_text").and_then(Value::as_str))
                .or_else(|| object.get("content").and_then(Value::as_str))
            {
                text.to_string()
            } else if let Some(nested) = object.get("content").or_else(|| object.get("message")) {
                extract_content_text(nested)
            } else {
                String::new()
            }
        }
        Value::Array(items) => items
            .iter()
            .map(extract_content_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

pub fn pretty_json_value(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

pub fn pretty_json_string(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| text.to_string())
}

pub fn normalize_path_for_match(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase()
}

/// Shared workspace filter for agent session lists.
///
/// Policy (all providers):
/// - `cwd == None` → keep every session
/// - session has no cwd metadata → **keep** (unbound / weixin / cron / missing)
/// - session has cwd → keep only when it resolves to the same repository
///   (commondir identity) as the target, or exactly matches the target path
///   when neither side is inside a git checkout
pub fn filter_sessions_by_workspace(
    sessions: Vec<SessionInfo>,
    cwd: Option<&Path>,
) -> Vec<SessionInfo> {
    let Some(cwd) = cwd else {
        return sessions;
    };
    let wanted = WorkspaceMatchTarget::new(cwd);
    sessions
        .into_iter()
        .filter(|session| match session.cwd.as_deref() {
            None => true,
            Some(candidate) => wanted.matches(Path::new(candidate)),
        })
        .collect()
}

/// Whether any cwd candidate matches the workspace target.
/// Empty candidate list means "no metadata" → keep (same policy as unbound sessions).
pub(crate) fn workspace_matches_candidates(
    wanted: &WorkspaceMatchTarget,
    mut candidates: impl Iterator<Item = impl AsRef<Path>>,
) -> bool {
    let mut any = false;
    for candidate in candidates.by_ref() {
        any = true;
        if wanted.matches(candidate.as_ref()) {
            return true;
        }
    }
    !any
}

pub(crate) struct WorkspaceMatchTarget {
    normalized_path: String,
    identity: Option<String>,
    candidate_identities: RefCell<HashMap<String, Option<String>>>,
}

impl WorkspaceMatchTarget {
    pub(crate) fn new(path: &Path) -> Self {
        Self {
            normalized_path: normalize_path_for_match(path),
            identity: crate::workspace::repo_identity(path),
            candidate_identities: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn matches(&self, candidate: &Path) -> bool {
        let normalized_candidate = normalize_path_for_match(candidate);
        if normalized_candidate == self.normalized_path {
            return true;
        }
        let candidate_identity = self.candidate_identity(&normalized_candidate, candidate);
        match (&self.identity, candidate_identity) {
            (Some(target), Some(candidate)) => *target == candidate,
            _ => false,
        }
    }

    fn candidate_identity(&self, key: &str, path: &Path) -> Option<String> {
        if let Some(existing) = self.candidate_identities.borrow().get(key) {
            return existing.clone();
        }
        let identity = crate::workspace::repo_identity(path);
        self.candidate_identities
            .borrow_mut()
            .insert(key.to_string(), identity.clone());
        identity
    }
}

pub fn select_blocks(session: &AgentSession, selection: AgentSelection) -> Vec<AgentBlock> {
    match selection {
        AgentSelection::LastTurn => select_last_turn(&session.blocks),
        AgentSelection::LastAssistant => {
            select_last_kind(&session.blocks, AgentBlockKind::Assistant)
        }
        AgentSelection::LastUser => select_last_kind(&session.blocks, AgentBlockKind::User),
        AgentSelection::LastTool => select_last_kind(&session.blocks, AgentBlockKind::ToolOutput),
        AgentSelection::LastBlocks(count) => {
            let start = session.blocks.len().saturating_sub(count);
            session.blocks[start..].to_vec()
        }
        AgentSelection::All => session.blocks.clone(),
    }
}

fn select_last_kind(blocks: &[AgentBlock], kind: AgentBlockKind) -> Vec<AgentBlock> {
    blocks
        .iter()
        .rev()
        .find(|block| block.kind == kind)
        .cloned()
        .into_iter()
        .collect()
}

fn select_last_turn(blocks: &[AgentBlock]) -> Vec<AgentBlock> {
    let Some(assistant_idx) = blocks
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::Assistant)
    else {
        // User + tools only (interrupted before assistant): keep the trailing structure.
        let Some(user_idx) = blocks
            .iter()
            .rposition(|block| block.kind == AgentBlockKind::User)
        else {
            return Vec::new();
        };
        return blocks[user_idx..].to_vec();
    };
    let user_idx = blocks[..assistant_idx]
        .iter()
        .rposition(|block| block.kind == AgentBlockKind::User)
        .unwrap_or(assistant_idx);

    // Keep the full turn: user, tools/skills/thinking/mcp, assistant — never strip structure.
    blocks[user_idx..=assistant_idx].to_vec()
}

/// Serialize a block for human/machine-readable evidence (not Markdown dialogue headings).
///
/// Dialogue stays plain. Structural channels use content-block markers:
/// `<:tool:bash call:>` … `<:tool:bash result:>`, `<:skill:name:>`, `<:thinking:>`.
pub fn format_structured_block(kind: AgentBlockKind, label: Option<&str>, text: &str) -> String {
    kind.format_block(label, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{make_repo, make_worktree};
    use std::fs;
    use std::time::SystemTime;

    #[test]
    fn extract_content_text_unwraps_nested_message_content_arrays() {
        let value = serde_json::json!({
            "content": [{"type": "text", "text": "hello nested"}]
        });
        assert_eq!(extract_content_text(&value), "hello nested");
    }

    #[test]
    fn cwd_candidates_do_not_duplicate_the_primary_cwd() {
        let mut tracked = AgentSessionMeta::default();
        tracked.add_cwd("/repo");
        tracked.add_cwd("/repo/subdir");
        assert_eq!(
            tracked.cwd_candidates().collect::<Vec<_>>(),
            vec!["/repo", "/repo/subdir"]
        );

        let fallback = AgentSessionMeta {
            cwd: Some("/repo".to_string()),
            ..AgentSessionMeta::default()
        };
        assert_eq!(fallback.cwd_candidates().collect::<Vec<_>>(), vec!["/repo"]);
    }

    #[test]
    fn matches_sessions_across_worktrees_of_same_repo() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("sivtr");
        let worktree = dir.path().join("sivtr-tui-stack");
        make_repo(&main);
        make_worktree(&main, &worktree, "sivtr-tui-stack");

        // A session recorded in the main checkout is visible from the worktree
        // and vice versa: they share one repository identity.
        assert!(WorkspaceMatchTarget::new(&main).matches(&worktree));
        assert!(WorkspaceMatchTarget::new(&worktree).matches(&main));
    }

    #[test]
    fn matches_subdirectories_of_same_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("sivtr");
        make_repo(&repo);
        let subdir = repo.join("crates").join("core");
        fs::create_dir_all(&subdir).unwrap();
        assert!(WorkspaceMatchTarget::new(&repo).matches(&subdir));
    }

    #[test]
    fn does_not_match_different_repos() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("sivtr");
        let second = dir.path().join("md-dragger");
        make_repo(&first);
        make_repo(&second);
        assert!(!WorkspaceMatchTarget::new(&first).matches(&second));
    }

    #[test]
    fn does_not_match_sessions_outside_any_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        make_repo(&repo);
        // A session recorded in a non-repo parent (e.g. `D:\Coding`) must not
        // leak into a repo workspace.
        assert!(!WorkspaceMatchTarget::new(&repo).matches(dir.path()));
        // ...but it still matches when browsing that parent dir itself.
        assert!(WorkspaceMatchTarget::new(dir.path()).matches(dir.path()));
    }

    #[test]
    fn filter_sessions_keeps_unbound_and_matching_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        make_repo(&repo);

        let sessions = vec![
            SessionInfo {
                path: PathBuf::from("unbound"),
                id: Some("u".into()),
                cwd: None,
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
            SessionInfo {
                path: PathBuf::from("match"),
                id: Some("m".into()),
                cwd: Some(repo.to_string_lossy().into_owned()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
            SessionInfo {
                path: PathBuf::from("other"),
                id: Some("o".into()),
                cwd: Some(dir.path().join("other").to_string_lossy().into_owned()),
                title: None,
                modified: SystemTime::UNIX_EPOCH,
            },
        ];

        let filtered = filter_sessions_by_workspace(sessions, Some(&repo));
        let ids: Vec<_> = filtered
            .iter()
            .filter_map(|session| session.id.as_deref())
            .collect();
        assert_eq!(ids, vec!["u", "m"]);
    }

    #[test]
    fn filter_keeps_sessions_recorded_in_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("sivtr");
        let worktree = dir.path().join("sivtr-tui-stack");
        make_repo(&main);
        make_worktree(&main, &worktree, "sivtr-tui-stack");

        // A session recorded in the main checkout shows up when browsing the
        // worktree, and one recorded in the worktree shows up from the main.
        let main_session = vec![SessionInfo {
            path: PathBuf::from("main-session"),
            id: Some("m".into()),
            cwd: Some(main.to_string_lossy().into_owned()),
            title: None,
            modified: SystemTime::UNIX_EPOCH,
        }];
        assert_eq!(
            filter_sessions_by_workspace(main_session, Some(&worktree)).len(),
            1
        );

        let worktree_session = vec![SessionInfo {
            path: PathBuf::from("wt-session"),
            id: Some("w".into()),
            cwd: Some(worktree.to_string_lossy().into_owned()),
            title: None,
            modified: SystemTime::UNIX_EPOCH,
        }];
        assert_eq!(
            filter_sessions_by_workspace(worktree_session, Some(&main)).len(),
            1
        );
    }
}
