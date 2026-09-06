//! Archive reads and writes: session upserts and record-blob loads.
//!
//! Records are stored as MessagePack [`WorkRecord`] blobs — the same layout
//! the per-file parse cache used before the archive — so refs, part text,
//! and tool payloads round-trip byte-identically. Upserts resolve an existing
//! row by session id *or* source path first, keeping one row per file even
//! when the derived session id changes between parses.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;

use crate::record::{WorkOutcome, WorkRecord};

/// Virtual source-path prefix for one-shot terminal captures. These rows live
/// in the terminal namespace but are not owned by the terminal log sync.
pub const CAPTURE_SOURCE_PREFIX: &str = "capture://";

/// Which blob column a load materializes: full part text or the stripped
/// metadata view (mirrors [`crate::query::LoadMode`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlobMode {
    Full,
    Light,
}

/// `(mtime secs, mtime nanos, size)` freshness stamp of a source file —
/// the same fingerprint the parse cache used.
pub type Stamp = (u64, u32, u64);

/// One archived session row, as the sync engine and query layer see it.
#[derive(Debug, Clone)]
pub struct ArchiveSession {
    pub row_id: i64,
    pub provider: String,
    pub session_id: String,
    pub source_path: String,
    pub stamp: Stamp,
}

/// Everything needed to store one session's records.
pub struct SessionUpsert<'a> {
    pub provider: &'a str,
    pub session_id: &'a str,
    pub source_path: &'a Path,
    pub cwd: Option<&'a str>,
    pub workspace_key: &'a str,
    pub title: Option<&'a str>,
    pub stamp: Stamp,
    pub records: &'a [WorkRecord],
    pub usage_events: &'a [crate::usage::UsageEvent],
}

/// Insert or replace one session and its records. Returns `true` when a new
/// row was created, `false` when an existing row (matched by session id or
/// source path) was updated in place. The session, records, findings, and
/// usage events commit together so a failed write never stamps partial data.
pub fn upsert_session(conn: &Connection, up: &SessionUpsert) -> Result<bool> {
    let (started_at, ended_at) = session_time_bounds(up.records);
    let cwd_norm = up
        .cwd
        .map(|cwd| crate::agents::normalize_path_for_match(Path::new(cwd)))
        .unwrap_or_default();
    let project = project_from_cwd(up.cwd);

    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin the archive upsert transaction")?;
    let id_row: Option<i64> = tx
        .query_row(
            "SELECT id FROM sessions WHERE provider = ?1 AND session_id = ?2",
            params![up.provider, up.session_id],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to look up archived session by id")?;
    let path_row: Option<i64> = tx
        .query_row(
            "SELECT id FROM sessions WHERE provider = ?1 AND source_path = ?2",
            params![up.provider, up.source_path.to_string_lossy()],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to look up archived session by path")?;

    // The same file re-parsed under a changed id keeps its row (path match,
    // id adopted); a genuine new session inserts. A session id that now
    // belongs to a different file loses that claim: the current file wins
    // the id, keeping one row per source file.
    let existing_row = match (id_row, path_row) {
        (Some(id), Some(path)) if id != path => {
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])
                .context("Failed to release a session id claimed by another file")?;
            Some(path)
        }
        (Some(row), _) | (_, Some(row)) => Some(row),
        (None, None) => None,
    };
    let session_row = match existing_row {
        Some(row) => {
            tx.execute(
                "UPDATE sessions SET session_id = ?2, source_path = ?3, cwd = ?4, cwd_norm = ?5,
                 workspace_key = ?6, title = ?7, started_at = ?8, ended_at = ?9, record_count = ?10,
                 project = ?11, mtime_secs = ?12, mtime_nanos = ?13, size = ?14,
                 synced_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id = ?1",
                params![
                    row,
                    up.session_id,
                    up.source_path.to_string_lossy(),
                    up.cwd,
                    cwd_norm,
                    up.workspace_key,
                    up.title,
                    started_at,
                    ended_at,
                    up.records.len() as i64,
                    project,
                    up.stamp.0 as i64,
                    up.stamp.1 as i64,
                    up.stamp.2 as i64,
                ],
            )
            .with_context(|| format!("Failed to update archived session {}", up.session_id))?;
            // The matched row id, not `last_insert_rowid`: the record
            // replacement below rewrites `records` rows before the rowid is
            // read again, which would re-point it at the last inserted
            // record.
            row
        }
        None => {
            tx.execute(
                "INSERT INTO sessions (provider, session_id, source_path, cwd, cwd_norm,
                 workspace_key, title, started_at, ended_at, record_count, project,
                 mtime_secs, mtime_nanos, size, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![
                    up.provider,
                    up.session_id,
                    up.source_path.to_string_lossy(),
                    up.cwd,
                    cwd_norm,
                    up.workspace_key,
                    up.title,
                    started_at,
                    ended_at,
                    up.records.len() as i64,
                    project,
                    up.stamp.0 as i64,
                    up.stamp.1 as i64,
                    up.stamp.2 as i64,
                ],
            )
            .with_context(|| format!("Failed to insert archived session {}", up.session_id))?;
            tx.last_insert_rowid()
        }
    };
    let inserted = existing_row.is_none();

    replace_records(&tx, session_row, up.records)?;
    replace_secret_findings(&tx, session_row, up.records)?;
    replace_usage_events(&tx, session_row, up.usage_events)?;
    tx.commit().context("Failed to commit archived session")?;
    Ok(inserted)
}

/// Store output from `sivtr run` or `sivtr pipe` in the terminal archive.
pub fn insert_terminal_capture(
    command: Option<&str>,
    output: &str,
    cwd: &Path,
    exit_code: Option<i32>,
) -> Result<String> {
    if output.trim().is_empty() {
        anyhow::bail!("cannot archive empty terminal capture");
    }

    let session_id = format!("capture-{}", uuid::Uuid::new_v4());
    let source_path = PathBuf::from(format!("{CAPTURE_SOURCE_PREFIX}{session_id}.jsonl"));
    let cwd_text = cwd.to_string_lossy().into_owned();
    let workspace_key = crate::workspace::repo_identity(cwd).unwrap_or_default();
    let entry = crate::session::SessionEntry::new("", command.unwrap_or_default(), output)
        .with_metadata(
            Some(cwd_text.clone()),
            Some(chrono::Utc::now().to_rfc3339()),
            None,
            exit_code,
        );
    let record = WorkRecord::terminal(&entry, &source_path, 0)
        .ok_or_else(|| anyhow::anyhow!("terminal capture produced no record"))?;
    let stamp = capture_stamp(output.len())?;
    let conn = super::schema::open()?;
    let usage_events = [];
    upsert_session(
        &conn,
        &SessionUpsert {
            provider: super::sync::TERMINAL_NAMESPACE,
            session_id: &session_id,
            source_path: &source_path,
            cwd: Some(&cwd_text),
            workspace_key: &workspace_key,
            title: command.filter(|command| !command.trim().is_empty()),
            stamp,
            records: std::slice::from_ref(&record),
            usage_events: &usage_events,
        },
    )?;
    Ok(session_id)
}

fn capture_stamp(size: usize) -> Result<Stamp> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    Ok((
        elapsed.as_secs(),
        elapsed.subsec_nanos(),
        u64::try_from(size).context("terminal capture is too large")?,
    ))
}

/// Replace one session's record rows inside a transaction.
fn replace_records(tx: &Transaction<'_>, session_row: i64, records: &[WorkRecord]) -> Result<()> {
    tx.execute("DELETE FROM records WHERE session_row = ?1", [session_row])
        .context("Failed to clear archived records")?;
    for record in records {
        let (outcome, exit_code) = record
            .status
            .as_ref()
            .map(|status| (Some(status.outcome), status.exit_code))
            .unwrap_or((None, None));
        tx.execute(
            "INSERT INTO records (session_row, idx, kind, title, started_at, ended_at,
             outcome, exit_code, blob, blob_light)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                session_row,
                record.work_ref.index() as i64,
                record.kind_label(),
                record.title,
                record.time.started_at,
                record.time.ended_at,
                outcome.map(outcome_label),
                exit_code,
                encode_record(record, false)?,
                encode_record(record, true)?,
            ],
        )
        .context("Failed to insert archived record")?;
    }
    Ok(())
}

fn outcome_label(outcome: WorkOutcome) -> &'static str {
    match outcome {
        WorkOutcome::Success => "success",
        WorkOutcome::Failure => "failure",
        WorkOutcome::Unknown => "unknown",
    }
}

fn encode_record(record: &WorkRecord, light: bool) -> Result<Vec<u8>> {
    let payload = if light {
        without_parts(record)
    } else {
        record.clone()
    };
    let mut serializer = rmp_serde::encode::Serializer::new(Vec::new()).with_struct_map();
    payload
        .serialize(&mut serializer)
        .context("Failed to encode record blob")?;
    Ok(serializer.into_inner())
}

/// Metadata view of a record: light fields only, part text emptied.
pub fn without_parts(record: &WorkRecord) -> WorkRecord {
    let mut meta = record.clone();
    meta.parts.clear();
    meta
}

/// Earliest start and latest end across a session's records.
fn session_time_bounds(records: &[WorkRecord]) -> (Option<String>, Option<String>) {
    let mut started: Option<String> = None;
    let mut ended: Option<String> = None;
    for record in records {
        if let Some(at) = &record.time.started_at {
            let newer_start = started.as_ref().is_none_or(|current| at < current);
            if newer_start {
                started = Some(at.clone());
            }
        }
        if let Some(at) = record.time.primary_at() {
            let newer_end = ended.as_ref().is_none_or(|current| at > current.as_str());
            if newer_end {
                ended = Some(at.to_string());
            }
        }
    }
    (started, ended)
}

fn project_from_cwd(cwd: Option<&str>) -> String {
    cwd.and_then(|cwd| cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Look up one archived session by its source file and verify the stamp.
/// `Ok(None)` means the file is missing or stale and must be re-synced.
pub fn fresh_session_row(
    conn: &Connection,
    provider: &str,
    source_path: &Path,
    stamp: Stamp,
) -> Result<Option<ArchiveSession>> {
    let row = conn
        .query_row(
            "SELECT id, provider, session_id, source_path, mtime_secs, mtime_nanos, size
             FROM sessions WHERE provider = ?1 AND source_path = ?2",
            params![provider, source_path.to_string_lossy()],
            |row| {
                Ok(ArchiveSession {
                    row_id: row.get(0)?,
                    provider: row.get(1)?,
                    session_id: row.get(2)?,
                    source_path: row.get(3)?,
                    stamp: (
                        row.get::<_, i64>(4)? as u64,
                        row.get::<_, i64>(5)? as u32,
                        row.get::<_, i64>(6)? as u64,
                    ),
                })
            },
        )
        .optional()
        .context("Failed to query archived session by path")?;
    Ok(row.filter(|session| session.stamp == stamp))
}

/// Load one session's records by source file, requiring the stored stamp to
/// match the file's current stamp. `Ok(None)` when absent or stale.
pub fn load_records_by_path(
    conn: &Connection,
    provider: &str,
    source_path: &Path,
    mode: BlobMode,
) -> Result<Option<Vec<WorkRecord>>> {
    let stamp = match crate::cache::file_stamp(source_path) {
        Some(stamp) => stamp,
        // The source file is gone; the archived copy remains the authoritative
        // retained record for this deleted source.
        None => return load_records_for_deleted_source(conn, provider, source_path, mode),
    };
    let Some(session) = fresh_session_row(conn, provider, source_path, stamp)? else {
        return Ok(None);
    };
    load_records_by_row(conn, session.row_id, mode).map(Some)
}

/// Serve retained records for a source that was deleted after ingestion.
fn load_records_for_deleted_source(
    conn: &Connection,
    provider: &str,
    source_path: &Path,
    mode: BlobMode,
) -> Result<Option<Vec<WorkRecord>>> {
    let row: Option<i64> = conn
        .query_row(
            "SELECT id FROM sessions WHERE provider = ?1 AND source_path = ?2",
            params![provider, source_path.to_string_lossy()],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to query archived session by path")?;
    match row {
        Some(row) => load_records_by_row(conn, row, mode).map(Some),
        None => Ok(None),
    }
}

/// Load one session's records by archive row.
pub fn load_records_by_row(
    conn: &Connection,
    session_row: i64,
    mode: BlobMode,
) -> Result<Vec<WorkRecord>> {
    let column = match mode {
        BlobMode::Full => "blob",
        BlobMode::Light => "blob_light",
    };
    let statement = format!("SELECT {column} FROM records WHERE session_row = ?1 ORDER BY idx ASC");
    let mut stmt = conn
        .prepare(&statement)
        .with_context(|| "Failed to prepare archived record load")?;
    let blobs: Vec<Vec<u8>> = stmt
        .query_map([session_row], |row| row.get(0))
        .context("Failed to read archived record blobs")?
        .collect::<std::result::Result<_, _>>()
        .context("Failed to read archived record blobs")?;

    let mut records = Vec::with_capacity(blobs.len());
    for blob in blobs {
        let record: WorkRecord =
            rmp_serde::from_slice(&blob).context("Failed to decode archived record blob")?;
        records.push(record);
    }
    Ok(records)
}

/// A session row listed for query loading: identity plus its records loaded
/// on demand.
#[derive(Debug, Clone)]
pub struct ListedSession {
    pub row_id: i64,
    pub provider: String,
    pub source_path: String,
}

/// List archived sessions for the given namespaces, workspace-filtered.
///
/// The filter mirrors the shared [`crate::agents::filter_sessions_by_workspace`]
/// policy: unbound sessions (no cwd) stay visible everywhere, an exact cwd
/// match always matches, and a session inside a git checkout matches any
/// browsing directory of the same repository (workspace key from the shared
/// git dir, precomputed at sync time so the query stays an index scan).
///
/// `recent_per_namespace` truncates each namespace to its most recently
/// modified sessions, matching the live listing order.
pub fn list_workspace_sessions(
    conn: &Connection,
    namespaces: &[&str],
    cwd: Option<&Path>,
    recent_per_namespace: Option<usize>,
) -> Result<Vec<ListedSession>> {
    let browsing_key = cwd
        .and_then(crate::workspace::repo_identity)
        .unwrap_or_default();
    let cwd_norm = cwd
        .map(crate::agents::normalize_path_for_match)
        .unwrap_or_default();

    let mut listed = Vec::new();
    for namespace in namespaces {
        let limit = recent_per_namespace.map(|limit| limit as i64).unwrap_or(-1);
        let no_filter = cwd.is_none() as i64;
        let mut stmt = conn.prepare(
            "SELECT id, source_path FROM sessions
             WHERE provider = ?1
               AND (?4 = 1
                    OR cwd IS NULL
                    OR cwd_norm = ?2
                    OR (?3 != '' AND workspace_key = ?3))
             ORDER BY mtime_secs DESC, mtime_nanos DESC, id DESC
             LIMIT ?5",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(
                params![namespace, cwd_norm, browsing_key, no_filter, limit],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<std::result::Result<_, _>>()?;
        listed.extend(rows.into_iter().map(|(row_id, source_path)| ListedSession {
            row_id,
            provider: (*namespace).to_string(),
            source_path,
        }));
    }
    Ok(listed)
}

/// Every archived session of one provider, as the sync engine sees it:
/// source path plus the stamp recorded at last sync.
pub fn provider_stamps(
    conn: &Connection,
    provider: &str,
) -> Result<std::collections::HashMap<String, Stamp>> {
    let mut stmt = conn.prepare(
        "SELECT source_path, mtime_secs, mtime_nanos, size FROM sessions WHERE provider = ?1",
    )?;
    let rows = stmt
        .query_map([provider], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u32,
                    row.get::<_, i64>(3)? as u64,
                ),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().collect())
}

/// Session metadata for listings and API responses.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionMeta {
    pub provider: String,
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub record_count: i64,
    pub project: String,
    pub starred: bool,
}

/// Per-provider archive totals.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderCount {
    pub provider: String,
    pub sessions: i64,
    pub records: i64,
}

/// List archived session metadata, newest-first, optionally filtered by
/// provider and offset-paginated. Used by listings and the web API.
pub fn list_sessions_meta(
    conn: &Connection,
    provider: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<SessionMeta>> {
    let sql = match provider {
        Some(_) => {
            "SELECT provider, session_id, title, cwd, started_at, ended_at, record_count, project, starred
             FROM sessions WHERE provider = ?1
             ORDER BY COALESCE(ended_at, started_at, synced_at) DESC, id DESC
             LIMIT ?2 OFFSET ?3"
        }
        None => {
            "SELECT provider, session_id, title, cwd, started_at, ended_at, record_count, project, starred
             FROM sessions
             ORDER BY COALESCE(ended_at, started_at, synced_at) DESC, id DESC
             LIMIT ?1 OFFSET ?2"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row| {
        Ok(SessionMeta {
            provider: row.get(0)?,
            session_id: row.get(1)?,
            title: row.get(2)?,
            cwd: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            record_count: row.get(6)?,
            project: row.get(7)?,
            starred: row.get::<_, i64>(8)? != 0,
        })
    };
    let rows = match provider {
        Some(provider) => stmt
            .query_map(params![provider, limit, offset], map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?,
        None => stmt
            .query_map(params![limit, offset], map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?,
    };
    Ok(rows)
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub row_id: i64,
    pub provider: String,
    pub session_id: String,
    pub source_path: String,
    pub project: String,
    pub starred: bool,
}

pub fn list_session_rows(
    conn: &Connection,
    provider: Option<&str>,
    session_id: Option<&str>,
    starred: Option<bool>,
) -> Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, provider, session_id, source_path, project, starred
         FROM sessions
         WHERE (?1 IS NULL OR provider = ?1)
           AND (?2 IS NULL OR session_id = ?2)
           AND (?3 IS NULL OR starred = ?3)
         ORDER BY COALESCE(ended_at, started_at, synced_at) DESC, id DESC",
    )?;
    let rows = stmt
        .query_map(
            params![provider, session_id, starred.map(i64::from)],
            |row| {
                Ok(SessionRow {
                    row_id: row.get(0)?,
                    provider: row.get(1)?,
                    session_id: row.get(2)?,
                    source_path: row.get(3)?,
                    project: row.get(4)?,
                    starred: row.get::<_, i64>(5)? != 0,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn remove_missing_sessions(
    conn: &Connection,
    provider: &str,
    source_paths: &[String],
) -> Result<usize> {
    if source_paths.is_empty() {
        return Ok(conn.execute(
            "DELETE FROM sessions
             WHERE provider = ?1 AND source_path NOT LIKE ?2",
            params![provider, format!("{CAPTURE_SOURCE_PREFIX}%")],
        )?);
    }
    let placeholders = std::iter::repeat_n("?", source_paths.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "DELETE FROM sessions
         WHERE provider = ?1 AND source_path NOT LIKE ?2
           AND source_path NOT IN ({placeholders})"
    );
    let capture_prefix = format!("{CAPTURE_SOURCE_PREFIX}%");
    let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(source_paths.len() + 2);
    values.push(&provider);
    values.push(&capture_prefix);
    for path in source_paths {
        values.push(path);
    }
    Ok(conn.execute(&sql, rusqlite::params_from_iter(values))?)
}

pub fn set_session_starred(
    conn: &Connection,
    provider: &str,
    session_id: &str,
    starred: bool,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE sessions SET starred = ?3 WHERE provider = ?1 AND session_id = ?2",
        params![provider, session_id, i64::from(starred)],
    )?;
    if changed == 0 {
        anyhow::bail!("archived session `{provider}/{session_id}` was not found");
    }
    Ok(())
}

fn replace_secret_findings(
    tx: &Transaction<'_>,
    session_row: i64,
    records: &[WorkRecord],
) -> Result<()> {
    let mut counts = std::collections::BTreeMap::<String, i64>::new();
    for record in records {
        let (_, report) = crate::privacy::redact_text_with_report(&record.combined_text())?;
        for kind in report
            .warnings
            .into_iter()
            .filter(|kind| !crate::privacy::is_manual_warning(kind))
        {
            *counts.entry(kind).or_default() += 1;
        }
    }
    tx.execute(
        "DELETE FROM secret_findings WHERE session_row = ?1",
        [session_row],
    )?;
    for (kind, occurrences) in counts {
        tx.execute(
            "INSERT INTO secret_findings (session_row, kind, occurrences) VALUES (?1, ?2, ?3)",
            params![session_row, kind, occurrences],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretFinding {
    pub provider: String,
    pub session_id: String,
    pub project: String,
    pub kind: String,
    pub occurrences: i64,
}

pub fn list_secret_findings(conn: &Connection) -> Result<Vec<SecretFinding>> {
    let mut stmt = conn.prepare(
        "SELECT s.provider, s.session_id, s.project, f.kind, f.occurrences
         FROM secret_findings f JOIN sessions s ON s.id = f.session_row
         ORDER BY s.provider, s.session_id, f.kind",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SecretFinding {
                provider: row.get(0)?,
                session_id: row.get(1)?,
                project: row.get(2)?,
                kind: row.get(3)?,
                occurrences: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Session and record totals per provider, alphabetical by provider name.
pub fn provider_counts(conn: &Connection) -> Result<Vec<ProviderCount>> {
    let mut stmt = conn.prepare(
        "SELECT s.provider, COUNT(DISTINCT s.id), COALESCE(SUM(s.record_count), 0)
         FROM sessions s GROUP BY s.provider ORDER BY s.provider ASC",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ProviderCount {
                provider: row.get(0)?,
                sessions: row.get(1)?,
                records: row.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One session's metadata by its archive key.
pub fn session_meta_by_key(
    conn: &Connection,
    provider: &str,
    session_id: &str,
) -> Result<Option<SessionMeta>> {
    conn.query_row(
        "SELECT provider, session_id, title, cwd, started_at, ended_at, record_count, project, starred
         FROM sessions WHERE provider = ?1 AND session_id = ?2",
        params![provider, session_id],
        |row| {
            Ok(SessionMeta {
                provider: row.get(0)?,
                session_id: row.get(1)?,
                title: row.get(2)?,
                cwd: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            record_count: row.get(6)?,
            project: row.get(7)?,
            starred: row.get::<_, i64>(8)? != 0,
            })
        },
    )
    .optional()
    .context("Failed to read archived session metadata")
}

/// Load one session's records by its archive key (provider + session id).
pub fn load_records_by_key(
    conn: &Connection,
    provider: &str,
    session_id: &str,
    mode: BlobMode,
) -> Result<Option<Vec<WorkRecord>>> {
    let row: Option<i64> = conn
        .query_row(
            "SELECT id FROM sessions WHERE provider = ?1 AND session_id = ?2",
            params![provider, session_id],
            |row| row.get(0),
        )
        .optional()
        .context("Failed to look up archived session")?;
    match row {
        Some(row) => load_records_by_row(conn, row, mode).map(Some),
        None => Ok(None),
    }
}

/// One raw usage event joined with its archive identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUsageEvent {
    pub provider: String,
    pub session_id: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub occurred_at: Option<String>,
    pub dedup_key: String,
}

/// Replace one session's usage events, deduplicating on `dedup_key` so a
/// re-sync of the same transcript never doubles-bills.
fn replace_usage_events(
    tx: &Transaction<'_>,
    session_row: i64,
    events: &[crate::usage::UsageEvent],
) -> Result<()> {
    tx.execute(
        "DELETE FROM usage_events WHERE session_row = ?1",
        [session_row],
    )
    .context("Failed to clear archived usage events")?;
    let mut seen = std::collections::HashSet::new();
    for event in events {
        if !event.dedup_key.is_empty() && !seen.insert(event.dedup_key.as_str()) {
            continue;
        }
        let input_tokens = i64::try_from(event.input_tokens)
            .context("input token count exceeds archive integer range")?;
        let output_tokens = i64::try_from(event.output_tokens)
            .context("output token count exceeds archive integer range")?;
        let cache_read_tokens = i64::try_from(event.cache_read_tokens)
            .context("cache-read token count exceeds archive integer range")?;
        let cache_creation_tokens = i64::try_from(event.cache_creation_tokens)
            .context("cache-creation token count exceeds archive integer range")?;
        tx.execute(
            "INSERT INTO usage_events (session_row, model, input_tokens, output_tokens,
             cache_read_tokens, cache_creation_tokens, occurred_at, dedup_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_row,
                event.model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                event
                    .occurred_at
                    .as_deref()
                    .and_then(crate::time::normalize_timestamp),
                event.dedup_key,
            ],
        )
        .context("Failed to insert archived usage event")?;
    }
    Ok(())
}

/// Read raw usage events over optional provider/session and inclusive day
/// bounds (`YYYY-MM-DD`). Ordering is stable for deterministic reports.
pub fn load_usage_events(
    conn: &Connection,
    provider: Option<&str>,
    session_id: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<Vec<StoredUsageEvent>> {
    let mut stmt = conn.prepare(
        "SELECT s.provider, s.session_id, u.model, u.input_tokens, u.output_tokens,
                u.cache_read_tokens, u.cache_creation_tokens, u.occurred_at, u.dedup_key
         FROM usage_events u JOIN sessions s ON s.id = u.session_row
         WHERE (?1 IS NULL OR s.provider = ?1)
           AND (?2 IS NULL OR s.session_id = ?2)
           AND (?3 IS NULL OR date(u.occurred_at) >= date(?3))
           AND (?4 IS NULL OR date(u.occurred_at) <= date(?4))
         ORDER BY COALESCE(u.occurred_at, '') ASC, s.provider ASC,
                  s.session_id ASC, u.id ASC",
    )?;
    let rows = stmt
        .query_map(params![provider, session_id, since, until], |row| {
            Ok(StoredUsageEvent {
                provider: row.get(0)?,
                session_id: row.get(1)?,
                model: row.get(2)?,
                input_tokens: row.get::<_, i64>(3)? as u64,
                output_tokens: row.get::<_, i64>(4)? as u64,
                cache_read_tokens: row.get::<_, i64>(5)? as u64,
                cache_creation_tokens: row.get::<_, i64>(6)? as u64,
                occurred_at: row.get(7)?,
                dedup_key: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Read/write the archive_meta key/value table (sync bookkeeping).
pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM archive_meta WHERE key = ?1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .context("Failed to read archive meta")
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO archive_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .context("Failed to write archive meta")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        MessageRole, WorkChannel, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
        RECORD_SCHEMA_VERSION,
    };
    use crate::test_fixtures::message_part;

    fn terminal_record(session: &str, index: usize, content: &str) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: format!("terminal/{session}/{index}").parse().unwrap(),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: Some(session.to_string()),
                path: None,
            },
            cwd: Some("/repo".to_string()),
            time: WorkTime {
                started_at: None,
                ended_at: Some("2026-01-01T00:00:00Z".into()),
                duration_ms: None,
            },
            status: None,
            title: format!("record {index}"),
            parts: vec![message_part(1, MessageRole::Assistant, content)],
        }
    }

    fn sample_upsert<'a>(records: &'a [WorkRecord]) -> SessionUpsert<'a> {
        SessionUpsert {
            provider: "terminal",
            session_id: "session_1",
            source_path: Path::new("/repo/terminals/session_1.jsonl"),
            cwd: Some("/repo"),
            workspace_key: "repo-key",
            title: None,
            stamp: (100, 0, 42),
            records,
            usage_events: &[],
        }
    }

    #[test]
    fn upsert_then_load_round_trips_records() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let conn = crate::archive::open().unwrap();
        let records = vec![
            terminal_record("session_1", 1, "first"),
            terminal_record("session_1", 2, "second"),
        ];
        assert!(upsert_session(&conn, &sample_upsert(&records)).unwrap());

        let loaded = load_records_by_path(
            &conn,
            "terminal",
            Path::new("/repo/terminals/session_1.jsonl"),
            BlobMode::Full,
        )
        .unwrap()
        .expect("fresh row loads");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].parts[0].text(), "second");

        let session_row = fresh_session_row(
            &conn,
            "terminal",
            Path::new("/repo/terminals/session_1.jsonl"),
            (100, 0, 42),
        )
        .unwrap()
        .expect("row present")
        .row_id;
        let light = load_records_by_row(&conn, session_row, BlobMode::Light).unwrap();
        assert!(light[0].parts.is_empty());
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[test]
    fn upsert_adopts_new_session_id_and_handles_path_moves() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let conn = crate::archive::open().unwrap();
        let records = vec![terminal_record("session_1", 1, "one")];

        let mut up = sample_upsert(&records);
        assert!(upsert_session(&conn, &up).unwrap(), "first insert");
        // Same path, different derived id: updates in place and adopts the id.
        up.session_id = "renamed";
        assert!(!upsert_session(&conn, &up).unwrap(), "path match updates");
        let saved: String = conn
            .query_row(
                "SELECT session_id FROM sessions WHERE source_path = ?1",
                [up.source_path.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, "renamed");
        // Same id, different path: updates in place (row found by the id).
        let moved = Path::new("/repo/terminals/session_1_moved.jsonl");
        up.source_path = moved;
        // Different record content proves the UPDATE branch rewrites the
        // records of the matched row — not of a rowid left over from the
        // last insert.
        let renamed_records = vec![terminal_record("renamed", 1, "renamed payload")];
        up.records = &renamed_records;
        assert!(!upsert_session(&conn, &up).unwrap(), "id match updates");
        let reloaded = load_records_by_path(&conn, "terminal", moved, BlobMode::Full)
            .unwrap()
            .expect("updated session stays readable");
        assert_eq!(
            reloaded.len(),
            1,
            "updated session keeps exactly one record"
        );
        assert_eq!(reloaded[0].title, "record 1");
        assert_eq!(reloaded[0].parts[0].text(), "renamed payload");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "no duplicate rows");
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[test]
    fn terminal_capture_stays_in_archive_across_terminal_sync_cleanup() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let cwd = dir.path().join("repo");
        std::fs::create_dir(&cwd).unwrap();

        let session_id =
            insert_terminal_capture(Some("echo captured"), "captured", &cwd, Some(0)).unwrap();
        let conn = crate::archive::open().unwrap();
        let records = load_records_by_key(&conn, "terminal", &session_id, BlobMode::Full)
            .unwrap()
            .expect("capture is archived");
        assert_eq!(records[0].session.id, session_id);
        // The whole capture is one shell action; its output block holds the
        // captured text.
        assert_eq!(records[0].parts[0].text(), "captured");

        assert_eq!(remove_missing_sessions(&conn, "terminal", &[]).unwrap(), 0);
        assert!(
            load_records_by_key(&conn, "terminal", &session_id, BlobMode::Full)
                .unwrap()
                .is_some()
        );
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[test]
    fn failed_session_upsert_rolls_back_metadata_and_records() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let conn = crate::archive::open().unwrap();
        let records = vec![terminal_record("session_1", 1, "original")];
        let up = sample_upsert(&records);
        assert!(upsert_session(&conn, &up).unwrap());

        let events = vec![crate::usage::UsageEvent {
            model: "gpt-5".into(),
            input_tokens: u64::MAX,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            occurred_at: None,
            dedup_key: "overflow".into(),
        }];
        let mut replacement = sample_upsert(&records);
        replacement.stamp = (200, 0, 43);
        replacement.usage_events = &events;
        assert!(upsert_session(&conn, &replacement).is_err());

        assert!(fresh_session_row(
            &conn,
            "terminal",
            Path::new("/repo/terminals/session_1.jsonl"),
            (100, 0, 42),
        )
        .unwrap()
        .is_some());
        let rows = load_records_by_path(
            &conn,
            "terminal",
            Path::new("/repo/terminals/session_1.jsonl"),
            BlobMode::Full,
        )
        .unwrap()
        .unwrap();
        assert_eq!(rows[0].parts[0].text(), "original");
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[test]
    fn list_filters_by_workspace_and_exact_cwd() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let conn = crate::archive::open().unwrap();
        let empty: Vec<WorkRecord> = Vec::new();

        // Session bound to repo A.
        // Distinct source paths: the store keys one row per file, and an
        // upsert whose session id *or* path matches updates in place.
        let mut a = sample_upsert(&empty);
        a.session_id = "in-repo";
        a.source_path = Path::new("/repo-a/terminals/session_a.jsonl");
        a.cwd = Some("/repo-a");
        a.workspace_key = "repo-a-key";
        upsert_session(&conn, &a).unwrap();
        // Unbound session (no cwd) — visible everywhere.
        let mut unbound = sample_upsert(&empty);
        unbound.session_id = "unbound";
        unbound.source_path = Path::new("/elsewhere/terminals/unbound.jsonl");
        unbound.cwd = None;
        unbound.workspace_key = "";
        upsert_session(&conn, &unbound).unwrap();
        // Session at a non-repo path.
        let mut loose = sample_upsert(&empty);
        loose.session_id = "loose";
        loose.source_path = Path::new("/scratch/terminals/loose.jsonl");
        loose.cwd = Some("/scratch");
        loose.workspace_key = "";
        upsert_session(&conn, &loose).unwrap();

        let by = |cwd: Option<&Path>| {
            list_workspace_sessions(&conn, &["terminal"], cwd, None)
                .unwrap()
                .len()
        };
        assert_eq!(by(Some(Path::new("/repo-a"))), 2, "repo match + unbound");
        assert_eq!(by(Some(Path::new("/repo-b"))), 1, "only unbound");
        assert_eq!(by(Some(Path::new("/scratch"))), 2, "exact match + unbound");
        assert_eq!(by(None), 3, "no cwd filter lists all");
        std::env::remove_var("SIVTR_DATA_DIR");
    }

    #[test]
    fn usage_events_round_trip_with_provider_and_session_filters() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        let conn = crate::archive::open().unwrap();
        let records: Vec<WorkRecord> = Vec::new();
        let mut up = sample_upsert(&records);
        up.provider = "codex";
        up.session_id = "session-1";
        up.source_path = Path::new("/repo/codex/session-1.jsonl");
        let events = vec![
            crate::usage::UsageEvent {
                model: "gpt-5".into(),
                input_tokens: 2,
                output_tokens: 3,
                cache_read_tokens: 4,
                cache_creation_tokens: 5,
                occurred_at: Some("2026-08-30T00:00:00Z".into()),
                dedup_key: "codex:a".into(),
            },
            crate::usage::UsageEvent {
                model: "gpt-5".into(),
                input_tokens: 6,
                output_tokens: 7,
                cache_read_tokens: 8,
                cache_creation_tokens: 9,
                occurred_at: Some("2026-08-31T00:00:00Z".into()),
                dedup_key: "codex:b".into(),
            },
        ];
        up.usage_events = &events;
        upsert_session(&conn, &up).unwrap();

        let loaded =
            load_usage_events(&conn, Some("codex"), Some("session-1"), None, None).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].input_tokens, 2);
        assert_eq!(loaded[1].dedup_key, "codex:b");
        assert_eq!(
            load_usage_events(&conn, Some("claude"), None, None, None)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            load_usage_events(&conn, Some("codex"), None, Some("2026-08-31"), None)
                .unwrap()
                .len(),
            1
        );
        std::env::remove_var("SIVTR_DATA_DIR");
    }
}
