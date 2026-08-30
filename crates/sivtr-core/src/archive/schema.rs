//! Archive database location, connection setup, and schema.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Archive schema version. Bump when a release changes the table layout in a
/// way older rows cannot serve; the store then rebuilds from native sources
/// on the next sync (the archive is derived state, so a rebuild is safe).
pub const SCHEMA_VERSION: i64 = 1;

/// Path of the archive database (`<data_dir>/archive.db`).
pub fn db_path() -> PathBuf {
    crate::workspace::data_dir().join("archive.db")
}

/// Open (creating if needed) the archive database with WAL, foreign keys,
/// and a busy timeout — multiple sivtr processes (CLI, MCP server, daemon)
/// share one archive file.
pub fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create archive directory {}", parent.display()))?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open archive database {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("Failed to enable WAL journaling on the archive")?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("Failed to enable foreign keys on the archive")?;
    conn.busy_timeout(std::time::Duration::from_millis(5_000))
        .context("Failed to set archive busy timeout")?;
    init_schema(&conn)?;
    Ok(conn)
}

/// Create tables when missing and verify the schema version. A stored version
/// newer than this build means the archive was written by a newer sivtr —
/// fail with an explicit message instead of misreading unknown columns.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)
        .context("Failed to initialize archive schema")?;
    let stored: Option<i64> = conn
        .query_row(
            "SELECT value FROM archive_meta WHERE key = 'schema_version'",
            [],
            |row| {
                // `value` is a TEXT-affinity column: SQLite converts an
                // INTEGER write back to TEXT on read, so parse here.
                let text: String = row.get(0)?;
                text.parse::<i64>().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        "archive schema version is not an integer".into(),
                    )
                })
            },
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .context("Failed to read archive schema version")?;
    match stored {
        None => {
            conn.execute(
                "INSERT INTO archive_meta (key, value) VALUES ('schema_version', ?1)",
                [SCHEMA_VERSION],
            )
            .context("Failed to stamp archive schema version")?;
        }
        Some(version) if version > SCHEMA_VERSION => {
            anyhow::bail!(
                "archive schema v{version} was written by a newer sivtr (this build supports v{SCHEMA_VERSION}); upgrade sivtr or delete {} to rebuild",
                db_path().display()
            );
        }
        Some(_) => {}
    }
    Ok(())
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS archive_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- One row per archived source file. `provider` is the query namespace
-- (`terminal`, `codex`, ...); `session_id` is the stable identity used by
-- upserts; `source_path` anchors the stat-stamp freshness check.
CREATE TABLE IF NOT EXISTS sessions (
    id            INTEGER PRIMARY KEY,
    provider      TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    source_path   TEXT NOT NULL,
    cwd           TEXT,
    cwd_norm      TEXT NOT NULL DEFAULT '',
    workspace_key TEXT NOT NULL DEFAULT '',
    title         TEXT,
    started_at    TEXT,
    ended_at      TEXT,
    record_count  INTEGER NOT NULL DEFAULT 0,
    mtime_secs    INTEGER NOT NULL DEFAULT 0,
    mtime_nanos   INTEGER NOT NULL DEFAULT 0,
    size          INTEGER NOT NULL DEFAULT 0,
    synced_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(provider, session_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_source_path
    ON sessions(provider, source_path);
CREATE INDEX IF NOT EXISTS idx_sessions_workspace
    ON sessions(workspace_key);
CREATE INDEX IF NOT EXISTS idx_sessions_mtime
    ON sessions(provider, mtime_secs DESC, mtime_nanos DESC);

-- One row per WorkRecord, in ref-index order. `blob` is the full
-- MessagePack record; `blob_light` is the metadata view with part text
-- stripped (the light-load view). Part text lives inside the blob, so a
-- re-sync that produces identical refs simply replaces rows in place.
CREATE TABLE IF NOT EXISTS records (
    id          INTEGER PRIMARY KEY,
    session_row INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    title       TEXT NOT NULL,
    started_at  TEXT,
    ended_at    TEXT,
    outcome     TEXT,
    exit_code   INTEGER,
    blob        BLOB NOT NULL,
    blob_light  BLOB NOT NULL,
    UNIQUE(session_row, idx)
);
CREATE INDEX IF NOT EXISTS idx_records_session ON records(session_row);
CREATE INDEX IF NOT EXISTS idx_records_ended ON records(ended_at);
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_schema_and_is_reopenable() {
        let _guard = crate::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SIVTR_DATA_DIR", dir.path());
        {
            let conn = open().expect("open creates the archive");
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
        let reopened = open().expect("reopen is idempotent");
        let version: i64 = reopened
            .query_row(
                "SELECT value FROM archive_meta WHERE key = 'schema_version'",
                [],
                |row| {
                    let text: String = row.get(0)?;
                    Ok(text.parse::<i64>().unwrap_or(0))
                },
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        std::env::remove_var("SIVTR_DATA_DIR");
    }
}
