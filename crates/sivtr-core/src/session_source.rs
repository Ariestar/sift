//! Unified conversation sources: terminal logs and agent providers behind one
//! discovery/parse interface.
//!
//! Every source (the terminal, each agent provider, any future conversation
//! store) implements [`SessionSource`]; the query layer consumes only this
//! trait — no terminal/agent branches. Providers keep their internal
//! [`crate::agents::AgentSessionProvider`] contract for parsing;
//! [`AgentProvider`] adapts it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::agents::AgentProvider;
use crate::record::WorkRecord;
use crate::{session, workspace};

/// Metadata for one discovered session file: enough to filter, sort, and
/// address records without parsing the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Logical source path passed to the provider parser. Container providers
    /// use a virtual path so one physical database can expose many sessions.
    pub path: PathBuf,
    /// Physical file or directory used for freshness stamping. `None` means
    /// `path` itself is the physical source.
    pub physical_path: Option<PathBuf>,
    pub id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub modified: SystemTime,
}

/// A conversation source.
pub trait SessionSource {
    /// Cache namespace and selector prefix (`"terminal"`, `"codex"`, …).
    fn namespace(&self) -> &'static str;

    /// Discover session files, workspace-filtered when `cwd` is given.
    fn list_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>>;

    /// Parse one session file into records.
    fn parse_file(&self, path: &Path) -> Result<Vec<WorkRecord>>;
}

/// Terminal session logs. One namespace: users switch shells inside one
/// terminal session, so per-shell identity carries no signal.
pub struct TerminalSource;

impl SessionSource for TerminalSource {
    fn namespace(&self) -> &'static str {
        "terminal"
    }

    fn list_sessions(&self, cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
        let Some(cwd) = cwd else {
            return Ok(Vec::new());
        };
        let mut infos = Vec::new();
        let paths = workspace::terminal_log_paths_for_workspace(cwd).with_context(|| {
            format!(
                "Failed to list terminal sessions for workspace {}",
                cwd.display()
            )
        })?;
        for path in paths {
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .with_context(|| format!("Failed to stamp terminal log {}", path.display()))?;
            infos.push(SessionInfo {
                path,
                physical_path: None,
                id: None,
                cwd: None,
                title: None,
                modified,
            });
        }
        Ok(infos)
    }

    fn parse_file(&self, path: &Path) -> Result<Vec<WorkRecord>> {
        let entries = session::load_entries(path).context("Failed to read session log")?;
        Ok(entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| WorkRecord::terminal(entry, path, idx))
            .collect())
    }
}

/// The source list for a workspace query: the terminal plus the given agent
/// providers.
pub fn workspace_sources(providers: &[AgentProvider]) -> Vec<Box<dyn SessionSource>> {
    let mut sources: Vec<Box<dyn SessionSource>> = vec![Box::new(TerminalSource)];
    sources.extend(
        providers
            .iter()
            .map(|provider| Box::new(*provider) as Box<dyn SessionSource>),
    );
    sources
}

/// Look up a source by its namespace (`"terminal"`, `"codex"`, …).
pub fn source_by_namespace(namespace: &str) -> Option<Box<dyn SessionSource>> {
    if namespace == TerminalSource.namespace() {
        return Some(Box::new(TerminalSource));
    }
    AgentProvider::from_command_name(namespace)
        .map(|provider| Box::new(provider) as Box<dyn SessionSource>)
}
