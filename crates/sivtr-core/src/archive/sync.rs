//! Archive sync: keep the unified store fresh from every conversation source.
//!
//! One incremental pass lists every agent provider and every workspace's
//! terminal logs, compares each source file's `(mtime, size)` stamp against
//! the archive, and re-parses only changed files. Sync failures are
//! per-source and reported — one broken provider never hides the rest.
//! Query paths call [`ensure_fresh`], which rate-limits passes with the
//! `[sync] max_age_secs` config so rapid successive searches skip the sweep.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::agents::AgentProvider;
use crate::config::SivtrConfig;
use crate::query::SkippedSession;
use crate::record::WorkRecord;
use crate::session_source::{SessionSource, TerminalSource};
use crate::workspace;

use super::schema;
use super::store::{self, SessionUpsert, Stamp};

/// The archive namespace for terminal captures.
pub const TERMINAL_NAMESPACE: &str = "terminal";

/// Per-source sync counts.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncCounts {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub failed: usize,
}

impl SyncCounts {
    pub fn changed(&self) -> usize {
        self.added + self.updated + self.removed
    }
}

/// Outcome for one sync source (a provider, or the terminal aggregate).
#[derive(Debug, Clone, Serialize)]
pub struct SourceSyncReport {
    pub source: String,
    pub counts: SyncCounts,
    /// Source-level failure (listing, workspace scan) that aborted the whole
    /// source.
    pub error: Option<String>,
    /// Per-file parse failures: `(path, reason)`.
    pub failures: Vec<(PathBuf, String)>,
}

/// Full outcome of one sync pass.
#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub sources: Vec<SourceSyncReport>,
    pub duration_ms: u64,
}

impl SyncReport {
    pub fn changed(&self) -> usize {
        self.sources
            .iter()
            .map(|source| source.counts.changed())
            .sum()
    }

    pub fn failed(&self) -> usize {
        self.sources.iter().map(|source| source.counts.failed).sum()
    }

    /// Sources whose listing or scan failed outright.
    pub fn errors(&self) -> Vec<&SourceSyncReport> {
        self.sources
            .iter()
            .filter(|source| source.error.is_some())
            .collect()
    }
}

/// Sync every source into the archive. `full` re-parses every session and
/// ignores cached stamps (a schema bump or a suspicious archive rebuilds
/// this way).
pub fn sync_all(full: bool) -> Result<SyncReport> {
    let conn = schema::open()?;
    sync_all_with_conn(&conn, full)
}

/// [`sync_all`] on a caller-owned connection, so one process pass reuses a
/// single handle (the query path syncs and reads on the same connection).
pub fn sync_all_with_conn(conn: &Connection, full: bool) -> Result<SyncReport> {
    let providers: Vec<_> = AgentProvider::all()
        .iter()
        .filter(|spec| spec.provider.has_native_source())
        .map(|spec| spec.provider)
        .collect();
    sync_sources(conn, full, &providers, true)
}

/// Provider session totals after the same archive freshness pass used by all
/// query surfaces. A failed provider is reported without exposing a second
/// native listing path to callers.
#[derive(Debug, Clone)]
pub struct ProviderArchiveStatus {
    pub name: String,
    pub sessions: usize,
    pub error: Option<String>,
}

pub fn provider_status() -> Result<Vec<ProviderArchiveStatus>> {
    let conn = schema::open()?;
    provider_status_with_conn(&conn)
}

pub fn provider_status_with_conn(conn: &Connection) -> Result<Vec<ProviderArchiveStatus>> {
    let skipped = ensure_fresh_with_conn(conn)?;
    let counts = store::provider_counts(conn)?;
    let counts: HashMap<String, i64> = counts
        .into_iter()
        .map(|count| (count.provider, count.sessions))
        .collect();

    AgentProvider::all()
        .iter()
        .filter(|spec| spec.provider.has_native_source())
        .map(|spec| {
            let provider = spec.provider.command_name();
            let errors: Vec<String> = skipped
                .iter()
                .filter(|entry| entry.namespace == provider)
                .map(|entry| format!("{}: {}", entry.path.display(), entry.error))
                .collect();
            let error = (!errors.is_empty()).then(|| errors.join("; "));
            let sessions = usize::try_from(counts.get(provider).copied().unwrap_or_default())
                .context("archived provider session count is negative")?;
            Ok(ProviderArchiveStatus {
                name: spec.provider.name().to_string(),
                sessions,
                error,
            })
        })
        .collect()
}

fn sync_sources(
    conn: &Connection,
    full: bool,
    providers: &[AgentProvider],
    include_terminals: bool,
) -> Result<SyncReport> {
    let started = std::time::Instant::now();

    let mut sources = Vec::new();
    for provider in providers {
        sources.push(sync_provider(conn, *provider, full));
    }
    if include_terminals {
        sources.push(sync_terminals(conn, full));
    }

    let report = SyncReport {
        sources,
        duration_ms: started.elapsed().as_millis() as u64,
    };
    if report
        .sources
        .iter()
        .all(|source| source.error.is_none() && source.counts.failed == 0)
    {
        store::meta_set(
            conn,
            "last_sync_at",
            &Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        )?;
    }
    Ok(report)
}

/// Sync one agent provider's sessions.
fn sync_provider(conn: &Connection, provider: AgentProvider, full: bool) -> SourceSyncReport {
    let report = |counts: SyncCounts, error: Option<String>, failures: Vec<(PathBuf, String)>| {
        SourceSyncReport {
            source: provider.command_name().to_string(),
            counts,
            error,
            failures,
        }
    };

    let sessions = match SessionSource::list_sessions(&provider, None) {
        Ok(sessions) => sessions,
        Err(error) => {
            return report(
                SyncCounts::default(),
                Some(format!("{error:#}")),
                Vec::new(),
            )
        }
    };
    let stamps = match store::provider_stamps(conn, provider.command_name()) {
        Ok(stamps) => stamps,
        Err(error) => {
            return report(
                SyncCounts::default(),
                Some(format!("{error:#}")),
                Vec::new(),
            )
        }
    };

    let mut counts = SyncCounts::default();
    let mut failures = Vec::new();
    let mut source_paths = Vec::with_capacity(sessions.len());
    for info in sessions {
        source_paths.push(info.path.to_string_lossy().into_owned());
        let physical_path = info.physical_path.as_deref().unwrap_or(&info.path);
        let Some(stamp) = crate::cache::file_stamp(physical_path) else {
            counts.failed += 1;
            failures.push((
                info.path.clone(),
                format!(
                    "source disappeared before sync: {}",
                    physical_path.display()
                ),
            ));
            continue;
        };
        if !full && stamps.get(info.path.to_string_lossy().as_ref()) == Some(&stamp) {
            counts.unchanged += 1;
            continue;
        }
        match sync_session(
            conn,
            &provider,
            &info.path,
            info.id.as_deref(),
            info.title.as_deref(),
            info.cwd.as_deref(),
            stamp,
        ) {
            Ok(was_new) => {
                if was_new {
                    counts.added += 1;
                } else {
                    counts.updated += 1;
                }
            }
            Err(error) => {
                counts.failed += 1;
                failures.push((info.path.clone(), format!("{error:#}")));
            }
        }
    }
    match store::remove_missing_sessions(conn, provider.command_name(), &source_paths) {
        Ok(removed) => counts.removed = removed,
        Err(error) => {
            counts.failed += 1;
            failures.push((
                PathBuf::from(format!("<{}>", provider.command_name())),
                format!("failed to reconcile removed sessions: {error:#}"),
            ));
        }
    }
    report(counts, None, failures)
}

/// Sync the terminal logs of every known workspace under one report.
fn sync_terminals(conn: &Connection, full: bool) -> SourceSyncReport {
    let mut counts = SyncCounts::default();
    let mut first_error: Option<String> = None;
    let mut failures = Vec::new();

    let workspaces = match workspace::list_workspaces() {
        Ok(workspaces) => workspaces,
        Err(error) => {
            return SourceSyncReport {
                source: TERMINAL_NAMESPACE.to_string(),
                counts,
                error: Some(format!("{error:#}")),
                failures,
            };
        }
    };

    let stamps: HashMap<String, Stamp> = match store::provider_stamps(conn, TERMINAL_NAMESPACE) {
        Ok(stamps) => stamps,
        Err(error) => {
            return SourceSyncReport {
                source: TERMINAL_NAMESPACE.to_string(),
                counts,
                error: Some(format!("{error:#}")),
                failures,
            };
        }
    };
    let mut source_paths = Vec::new();

    for meta in workspaces {
        let root = PathBuf::from(&meta.root);
        let logs = match workspace::terminal_log_paths_for_workspace(&root) {
            Ok(logs) => logs,
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(format!("{error:#}"));
                }
                continue;
            }
        };
        for path in logs {
            source_paths.push(path.to_string_lossy().into_owned());
            let Some(stamp) = crate::cache::file_stamp(&path) else {
                counts.failed += 1;
                failures.push((path.clone(), "source disappeared before sync".to_string()));
                continue;
            };
            if !full && stamps.get(path.to_string_lossy().as_ref()) == Some(&stamp) {
                counts.unchanged += 1;
                continue;
            }
            let session_id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("current")
                .to_string();
            match sync_session(
                conn,
                &TerminalSource,
                &path,
                Some(&session_id),
                None,
                None,
                stamp,
            ) {
                Ok(was_new) => {
                    if was_new {
                        counts.added += 1;
                    } else {
                        counts.updated += 1;
                    }
                }
                Err(error) => {
                    counts.failed += 1;
                    failures.push((path.clone(), format!("{error:#}")));
                }
            }
        }
    }

    if first_error.is_none() {
        match store::remove_missing_sessions(conn, TERMINAL_NAMESPACE, &source_paths) {
            Ok(removed) => counts.removed = removed,
            Err(error) => {
                counts.failed += 1;
                failures.push((
                    PathBuf::from("<terminal>"),
                    format!("failed to reconcile removed sessions: {error:#}"),
                ));
            }
        }
    }

    SourceSyncReport {
        source: TERMINAL_NAMESPACE.to_string(),
        counts,
        error: first_error,
        failures,
    }
}

/// Parse one source file and store its records. Returns `true` when a new
/// session row was created.
///
/// The session id is derived from the parsed records' canonical session id
/// when available, otherwise from the provider's listing id. Sources without
/// either stable identity are rejected instead of being assigned a guess.
///
/// Usage extraction runs in the same source transaction boundary: providers
/// whose transcripts carry per-call token usage contribute `usage_events`
/// rows, and an extraction/storage error fails this source rather than
/// publishing a session with incomplete accounting.
pub fn sync_session(
    conn: &Connection,
    source: &dyn SessionSource,
    path: &Path,
    listing_id: Option<&str>,
    listing_title: Option<&str>,
    listing_cwd: Option<&str>,
    stamp: Stamp,
) -> Result<bool> {
    let records = SessionSource::parse_file(source, path)?;
    let provider = source.namespace();
    let session_id = derive_session_id(&records, listing_id)?;
    let usage_events = crate::usage::extract::extract(provider, path)?;
    let cwd = records
        .iter()
        .find_map(|record| record.cwd.clone())
        .or_else(|| listing_cwd.map(str::to_string));
    let workspace_key = cwd
        .as_deref()
        .map(Path::new)
        .and_then(workspace::repo_identity)
        .unwrap_or_default();

    let inserted = store::upsert_session(
        conn,
        &SessionUpsert {
            provider,
            session_id: &session_id,
            source_path: path,
            cwd: cwd.as_deref(),
            workspace_key: &workspace_key,
            title: listing_title,
            stamp,
            records: &records,
            usage_events: &usage_events,
        },
    )?;
    Ok(inserted)
}

/// Prefer the canonical session id the records themselves carry, then use the
/// stable id supplied by the provider listing.
fn derive_session_id(records: &[WorkRecord], listing_id: Option<&str>) -> Result<String> {
    records
        .iter()
        .find_map(|record| {
            record
                .session
                .canonical_id
                .clone()
                .filter(|id| !id.trim().is_empty())
        })
        .or_else(|| {
            listing_id
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| anyhow::anyhow!("source session has no stable session id"))
}

/// Rate-limited freshness pass for query paths: re-sync only when the last
/// pass is older than `[sync] max_age_secs`. Returns per-source failures so
/// callers can surface skipped sources instead of silently missing records.
pub fn ensure_fresh() -> Result<Vec<SkippedSession>> {
    let conn = schema::open()?;
    ensure_fresh_with_conn(&conn)
}

/// [`ensure_fresh`] on a caller-owned connection.
pub fn ensure_fresh_with_conn(conn: &Connection) -> Result<Vec<SkippedSession>> {
    let max_age_secs = sync_max_age_secs()?;
    if max_age_secs > 0 {
        if let Some(last) = store::meta_get(conn, "last_sync_at")? {
            if let Ok(last) = DateTime::parse_from_rfc3339(&last) {
                let age = SystemTime::now()
                    .duration_since(last.into())
                    .map(|duration| duration.as_secs())
                    .unwrap_or(u64::MAX);
                if age < max_age_secs {
                    return Ok(Vec::new());
                }
            }
        }
    }

    let report = sync_all_with_conn(conn, false)?;
    let mut skipped = Vec::new();
    for source in &report.sources {
        if let Some(error) = &source.error {
            skipped.push(SkippedSession {
                namespace: source.source.clone(),
                path: PathBuf::from(format!("<{}>", source.source)),
                error: error.clone(),
            });
        }
        for (path, error) in &source.failures {
            skipped.push(SkippedSession {
                namespace: source.source.clone(),
                path: path.clone(),
                error: error.clone(),
            });
        }
    }
    Ok(skipped)
}

/// Read `[sync] max_age_secs`; malformed configuration is an error rather
/// than a reason to silently change query freshness.
fn sync_max_age_secs() -> Result<u64> {
    Ok(SivtrConfig::load()?.sync.max_age_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_stamps_an_empty_source_set() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let conn = schema::open().unwrap();
        let report = sync_sources(&conn, false, &[], false).unwrap();
        assert!(report.sources.is_empty());
        let last = store::meta_get(&conn, "last_sync_at").unwrap();
        assert!(last.is_some(), "sync stamps last_sync_at");
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[test]
    fn sync_session_derives_id_from_canonical_records() {
        use crate::record::{WorkChannel, WorkRecordKind, WorkSessionRef, WorkSource};
        let file = tempfile::NamedTempFile::new().unwrap();
        let record = WorkRecord {
            schema_version: crate::record::RECORD_SCHEMA_VERSION,
            work_ref: "codex/canonical-id/1".parse().unwrap(),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: WorkSessionRef {
                id: "canonical-id".into(),
                canonical_id: Some("canonical-id".into()),
                path: Some(file.path().display().to_string()),
            },
            cwd: None,
            time: Default::default(),
            status: None,
            title: "turn".into(),
            parts: vec![],
        };
        // Records carry the canonical id even when the listing id differs.
        let derived = derive_session_id(&[record], Some("listing-id")).unwrap();
        assert_eq!(derived, "canonical-id");
    }
}
