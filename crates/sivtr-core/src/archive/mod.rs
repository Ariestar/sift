//! Unified local archive: one SQLite store for every conversation source.
//!
//! Terminal captures and agent provider sessions are synced into
//! `archive.db` (sessions + per-record rows holding MessagePack
//! [`WorkRecord`] blobs), and every query surface — search, show, TUI,
//! MCP, remote share — reads records from this store instead of re-parsing
//! native session files. Native stores remain the source of truth for
//! capture; the archive is derived state that [`sync`] keeps fresh by
//! stat-stamp comparison, so record refs and ordering stay stable across
//! incremental re-syncs.

pub mod schema;
pub mod store;
pub mod sync;

pub use schema::{db_path, open};
