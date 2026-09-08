//! Archive reads and writes: session upserts and record-blob loads.
//!
//! Records are stored as MessagePack [`WorkRecord`] blobs — the same layout
//! the per-file parse cache used before the archive — so refs, part text,
//! and tool payloads round-trip byte-identically. Upserts resolve an existing
//! row by session id *or* source path first, keeping one row per file even
//! when the derived session id changes between parses.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::record::{WorkOutcome, WorkRecord};

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
}

/// Insert or replace one session and its records. Returns `true` when a new
/// row was created, `false` when an existing row (matched by session id or
/// source path) was updated in place. The matched row adopts the upsert's
/// session id, so a derived-id change on one file can never split into a
/// second row later. The row write and the record replacement commit
/// together: a failure mid-way leaves the previous session intact instead
/// of a half-updated row the freshness stamp would then treat as current.
pub fn upsert_session(conn: &Connection, up: &SessionUpsert) -> Result<bool> {
    let (started_at, ended_at) = session_time_bounds(up.records);
    let cwd_norm = up
        .cwd
        .map(|cwd| crate::agents::normalize_path_for_match(Path::new(cwd)))
        .unwrap_or_default();

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
                 mtime_secs = ?11, mtime_nanos = ?12, size = ?13,
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
                 workspace_key, title, started_at, ended_at, record_count,
                 mtime_secs, mtime_nanos, size, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
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
    tx.commit().context("Failed to commit archived session")?;
    Ok(inserted)
}

/// Replace one session's record rows inside the caller's transaction.
fn replace_records(tx: &Connection, session_row: i64, records: &[WorkRecord]) -> Result<()> {
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
        // The source file is gone; the archived copy stays readable and a
        // missing stamp must not force a parse error.
        None => return load_records_by_row_lookup(conn, provider, source_path, mode),
    };
    let Some(session) = fresh_session_row(conn, provider, source_path, stamp)? else {
        return Ok(None);
    };
    load_records_by_row(conn, session.row_id, mode).map(Some)
}

/// Fallback for vanished source files: serve whatever is archived, so a
/// deleted capture file does not erase the memory it holds.
fn load_records_by_row_lookup(
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
            "SELECT provider, session_id, title, cwd, started_at, ended_at, record_count
             FROM sessions WHERE provider = ?1
             ORDER BY COALESCE(ended_at, started_at, synced_at) DESC, id DESC
             LIMIT ?2 OFFSET ?3"
        }
        None => {
            "SELECT provider, session_id, title, cwd, started_at, ended_at, record_count
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
        "SELECT provider, session_id, title, cwd, started_at, ended_at, record_count
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
    use crate::record::RECORD_SCHEMA_VERSION;
    use crate::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecordKind, WorkSessionRef, WorkSource, WorkTime,
    };

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
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::Output {
                    content: content.to_string(),
                    ansi: None,
                },
            }],
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
        // Same path, different derived id: updates in place and adopts the
        // new id, so a later id+path change still finds this row instead of
        // inserting a duplicate for the same file.
        up.session_id = "renamed";
        assert!(!upsert_session(&conn, &up).unwrap(), "path match updates");
        let saved: String = conn
            .query_row(
                "SELECT session_id FROM sessions WHERE source_path = ?1",
                [up.source_path.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, "renamed", "path match adopts the derived id");
        // The renamed session's file moving keeps one row: id matches.
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
}
