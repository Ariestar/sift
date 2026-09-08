use std::path::Path;

use anyhow::Result;
use sivtr_core::agents::AgentProvider;
use sivtr_core::query::{load_workspace_records, LoadMode};
use sivtr_core::record::WorkRecordIndex;
use sivtr_core::session_source::workspace_sources;

use crate::output;

/// Build the record index for the current workspace.
///
/// Thin wrapper over [`sivtr_core::query::load_workspace_records`] that keeps
/// the CLI behavior of warning about session files that failed to parse.
/// Loads metadata only (no part text); full-record callers use
/// [`crate::commands::memory::workset::query`] instead.
pub(crate) fn current_work_record_index(
    providers: &[AgentProvider],
    cwd: &Path,
    recent_sessions: Option<usize>,
) -> Result<WorkRecordIndex> {
    let sources = workspace_sources(providers);
    let result = load_workspace_records(&sources, cwd, recent_sessions, LoadMode::Light)?;
    warn_skipped(&result.skipped);
    Ok(result.into_index())
}

pub(crate) fn warn_skipped(skipped_sessions: &[sivtr_core::query::SkippedSession]) {
    for skipped in skipped_sessions {
        output::warning(format!(
            "failed to parse {} session {}: {:#}",
            skipped.namespace,
            skipped.path.display(),
            skipped.error,
        ));
    }
}
