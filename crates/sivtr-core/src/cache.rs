//! Shared on-disk cache helpers.
//!
//! These caches use a stamp-validated pattern: a payload is stored alongside
//! a source file's `(mtime, size)` fingerprint, and reads only pay the cost
//! of recomputation when the fingerprint no longer matches. The listing
//! cache ([`crate::agents::jsonl`]) applies it to session discovery so
//! listing is a stat sweep, and the BM25 index cache
//! ([`crate::search::index_cache`]) applies it to the search index keyed by
//! the corpus fingerprint. Parsed records themselves live in the unified
//! archive ([`crate::archive`]).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Cache root under the platform data dir (`SIVTR_DATA_DIR` override).
pub fn cache_dir() -> PathBuf {
    crate::workspace::data_dir().join("cache")
}

/// `(mtime secs, mtime nanos, size)` fingerprint of a file; `None` when the
/// file cannot be stamped (missing, unreadable, clock before epoch).
pub fn file_stamp(path: &Path) -> Option<(u64, u32, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let duration = meta
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?;
    Some((duration.as_secs(), duration.subsec_nanos(), meta.len()))
}

/// Cache file for a BM25 index, keyed by the fingerprint of the record corpus
/// it was built from. A changed corpus (new/changed session files) yields a
/// different fingerprint, so reads are safe without re-parsing the index.
pub fn index_cache_path(records_fingerprint: u64) -> PathBuf {
    cache_dir().join(format!("bm25-{:016x}.bin", records_fingerprint))
}

/// Deterministic fingerprint of a record corpus: the ordered sequence of
/// whole work refs. A record's presence, order, or identity change alters the
/// fingerprint; same-ref records with edited body content are represented as
/// new records by the query layer, so refs alone are a sound index key.
pub fn records_fingerprint(records: &[crate::record::WorkRef]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for reference in records {
        reference.hash(&mut hasher);
    }
    hasher.finish()
}

/// Cache file for a session listing, keyed by provider name + root directory.
pub fn listing_cache_path(provider: &str, root: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    provider.hash(&mut hasher);
    root.hash(&mut hasher);
    cache_dir().join(format!("listing-{:016x}.bin", hasher.finish()))
}

/// Best-effort atomic write (tmp + rename, same volume); callers ignore
/// failures because a stale cache only costs a re-parse on the next run.
/// Returns `true` on success, `false` on any failure so expensive-cache
/// callers can diagnose persistent I/O problems.
pub fn write_cache_atomic(target: &Path, bytes: &[u8]) -> bool {
    if std::fs::create_dir_all(cache_dir()).is_err() {
        return false;
    }
    let tmp = target.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        if std::fs::rename(&tmp, target).is_ok() {
            return true;
        }
        // rename failed; clean up the temp file.
        let _ = std::fs::remove_file(&tmp);
    }
    false
}
