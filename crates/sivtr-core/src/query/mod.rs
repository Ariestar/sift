//! Workspace query orchestration.
//!
//! Aggregates terminal and agent records for a workspace, deduplicates them,
//! and normalizes session display ids. This is the shared read surface used by
//! both the CLI (`show`/`search`/`copy`/`work`/`nav`/`zoom`) and the server
//! transport (`sivtr serve`). Callers decide how to surface
//! [`QueryResult::skipped`] parse failures — the core does no printing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::agents::AgentProvider;
use crate::record::{WorkRecord, WorkRecordIndex, WorkRef, WorkRefSelector};
use crate::session_source::{workspace_sources, SessionSource};

/// Prefix of the error [`load_workspace_source`] raises when a selector
/// matches no records. An empty source is a normal browse outcome (a
/// workspace with no sessions yet), so callers treat this exact error as an
/// empty result; keep it a named constant so that contract cannot drift.
pub const NO_RECORD_FOR_SELECTOR: &str = "No record found for ref selector";

/// A session file that could not be parsed, retained so callers can warn.
#[derive(Debug, Clone)]
pub struct SkippedSession {
    /// Cache namespace of the source (`"terminal"` or a provider name).
    pub namespace: String,
    pub path: PathBuf,
    /// Rendered error message; `anyhow::Error` is not `Clone`, so the reason is
    /// stored as a string for cheap retention and reporting.
    pub error: String,
}

/// The outcome of building a workspace record index.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Records successfully loaded, ready for `WorkRecordIndex::new`.
    pub records: Vec<WorkRecord>,
    /// Session files that failed to parse, with the reason.
    pub skipped: Vec<SkippedSession>,
}

/// Records and active anchors selected from one workspace source.
#[derive(Debug, Default)]
pub struct SourceQueryResult {
    pub records: Vec<WorkRecord>,
    pub anchors: Vec<WorkRef>,
    pub skipped: Vec<SkippedSession>,
}

impl QueryResult {
    pub fn into_index(self) -> WorkRecordIndex {
        WorkRecordIndex::new(self.records)
    }
}

/// How much of each agent record a load must materialize.
///
/// `Light` loads records with empty `parts`: enough for session lists,
/// metadata filtering, and recency ordering. `Full` also loads part text for
/// rendering, pattern matching, and BM25 ranking (the index is built from
/// part text). Both views share one per-file stamp validation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadMode {
    Light,
    Full,
}

impl From<LoadMode> for crate::archive::store::BlobMode {
    fn from(mode: LoadMode) -> Self {
        match mode {
            LoadMode::Full => Self::Full,
            LoadMode::Light => Self::Light,
        }
    }
}

/// Build the record index for a workspace: read every archived session of
/// the given sources (terminal, agent providers, …), deduplicate records,
/// and sort newest-first.
///
/// The archive is the single read surface: a stamp-gated freshness pass
/// brings it up to date first, then sessions are listed and decoded from
/// `archive.db` — native session files are only parsed by the sync engine.
///
/// `recent_sessions` truncates how many recent sessions each source
/// contributes.
pub fn load_workspace_records(
    sources: &[Box<dyn SessionSource>],
    cwd: &Path,
    recent_sessions: Option<usize>,
    mode: LoadMode,
) -> Result<QueryResult> {
    let namespaces: Vec<&str> = sources.iter().map(|source| source.namespace()).collect();
    let conn = crate::archive::open()?;
    let sync_skipped = crate::archive::sync::ensure_fresh_with_conn(&conn)?;
    let listed = crate::archive::store::list_workspace_sessions(
        &conn,
        &namespaces,
        Some(cwd),
        recent_sessions,
    )?;

    let mut result = QueryResult::default();
    result.records = decode_archived_sessions(&conn, &listed, mode, &mut result.skipped)?;
    result.skipped.extend(sync_skipped);
    dedup_records(&mut result.records);
    normalize_session_display_ids(&mut result.records);
    result
        .records
        .sort_by(|a, b| b.time.primary_at().cmp(&a.time.primary_at()));
    Ok(result)
}

/// Decode one archived session's records per listed row, collecting decode
/// failures as skips so one corrupt row never hides the corpus.
fn decode_archived_sessions(
    conn: &rusqlite::Connection,
    listed: &[crate::archive::store::ListedSession],
    mode: LoadMode,
    skipped: &mut Vec<SkippedSession>,
) -> Result<Vec<WorkRecord>> {
    let blob_mode = crate::archive::store::BlobMode::from(mode);
    let mut records = Vec::new();
    for session in listed {
        match crate::archive::store::load_records_by_row(conn, session.row_id, blob_mode) {
            Ok(session_records) => records.extend(session_records),
            Err(error) => skipped.push(SkippedSession {
                namespace: session.provider.clone(),
                path: PathBuf::from(&session.source_path),
                error: format!("{error:#}"),
            }),
        }
    }
    Ok(records)
}

/// Load one concrete ref or selector from a workspace.
///
/// `source` is the local-shaped body (`terminal/...`, `agent`, `pi/...`).
/// Remote aliases are attached by the client after the response arrives.
pub fn load_workspace_source(
    cwd: &Path,
    source: &str,
    mode: LoadMode,
) -> Result<SourceQueryResult> {
    if let Ok(reference) = source.parse::<WorkRef>() {
        if !reference.is_local() {
            anyhow::bail!("remote aliases are not valid inside a served source");
        }
        let providers = reference
            .provider()
            .map(|provider| vec![provider])
            .unwrap_or_else(all_agent_providers);
        let sources = workspace_sources(&providers);
        let result = load_workspace_records(&sources, cwd, None, mode)?;
        let index = WorkRecordIndex::new(result.records);
        let record = index
            .resolve(&reference)
            .cloned()
            .with_context(|| format!("No record found for ref `{source}`"))?;
        return Ok(SourceQueryResult {
            records: vec![record],
            anchors: vec![reference],
            skipped: result.skipped,
        });
    }

    let selector: WorkRefSelector = source.parse()?;
    let providers = selector.providers();
    let sources = workspace_sources(&providers);
    let result = load_workspace_records(&sources, cwd, None, mode)?;
    let mut records = Vec::new();
    let mut anchors = Vec::new();

    for record in result.records {
        if !selector.matches_work_ref(&record.work_ref) {
            continue;
        }
        anchors.push(record.work_ref.whole());
        records.push(record);
    }

    if records.is_empty() {
        anyhow::bail!("{NO_RECORD_FOR_SELECTOR} `{source}`");
    }

    Ok(SourceQueryResult {
        records,
        anchors,
        skipped: result.skipped,
    })
}

fn all_agent_providers() -> Vec<AgentProvider> {
    AgentProvider::all()
        .iter()
        .map(|spec| spec.provider)
        .collect()
}

/// Load one session file's records from the archive (either view). A missing
/// or stale archive row is an explicit error: native transcripts are ingested
/// only by the sync engine, never as a hidden query-time fallback.
pub fn load_session_records(
    namespace: &str,
    path: &Path,
    mode: LoadMode,
) -> Result<Vec<WorkRecord>> {
    let conn = crate::archive::open()?;
    crate::archive::store::load_records_by_path(&conn, namespace, path, mode.into())?
        .with_context(|| {
            format!(
                "session `{namespace}:{}` is not present or is stale in the archive; run `sivtr sync`",
                path.display()
            )
        })
}

/// Serial single-source parse path, kept for tests that drive a mocked
/// `SessionSource` (the production path decodes archived sessions).
#[cfg(test)]
fn records_from_source(
    source: &dyn SessionSource,
    cwd: &Path,
    recent_sessions: Option<usize>,
    skipped: &mut Vec<SkippedSession>,
) -> Result<Vec<WorkRecord>> {
    let mut records = Vec::new();
    let mut sessions = source.list_sessions(Some(cwd))?;
    if let Some(limit) = recent_sessions {
        sessions.truncate(limit);
    }

    for info in sessions {
        match source.parse_file(&info.path) {
            Ok(session_records) => records.extend(session_records),
            Err(error) => {
                skipped.push(SkippedSession {
                    namespace: source.namespace().to_string(),
                    path: info.path,
                    error: format!("{error:#}"),
                });
                continue;
            }
        }
    }

    Ok(records)
}

fn dedup_records(records: &mut Vec<WorkRecord>) {
    let mut positions: HashMap<String, usize> = HashMap::new();
    let mut deduped = Vec::with_capacity(records.len());

    for record in records.drain(..) {
        let key = record_identity_key(&record);
        if let Some(position) = positions.get(&key).copied() {
            if record_is_better(&record, &deduped[position]) {
                deduped[position] = record;
            }
            continue;
        }

        positions.insert(key, deduped.len());
        deduped.push(record);
    }

    *records = deduped;
}

fn record_identity_key(record: &WorkRecord) -> String {
    match &record.session.canonical_id {
        Some(canonical_id) => format!(
            "{}:{canonical_id}:{}",
            record.work_ref.path.namespace(),
            record.work_ref.path.index()
        ),
        None => record.work_ref.to_string(),
    }
}

fn record_is_better(candidate: &WorkRecord, existing: &WorkRecord) -> bool {
    candidate
        .parts
        .len()
        .cmp(&existing.parts.len())
        .then_with(|| {
            candidate
                .combined_text()
                .len()
                .cmp(&existing.combined_text().len())
        })
        .then_with(|| candidate.time.primary_at().cmp(&existing.time.primary_at()))
        .is_gt()
}

fn normalize_session_display_ids(records: &mut [WorkRecord]) {
    let mut source_sessions: HashMap<String, Vec<String>> = HashMap::new();

    for record in records.iter() {
        let Some(canonical_id) = record.session.canonical_id.as_deref() else {
            continue;
        };
        let source_key = session_source_key(&record.work_ref);
        let sessions = source_sessions.entry(source_key).or_default();
        if !sessions.iter().any(|existing| existing == canonical_id) {
            sessions.push(canonical_id.to_string());
        }
    }

    for record in records.iter_mut() {
        let Some(canonical_id) = record.session.canonical_id.as_deref() else {
            continue;
        };
        let source_key = session_source_key(&record.work_ref);
        let Some(all_sessions) = source_sessions.get(&source_key) else {
            continue;
        };
        let display_id = compact_unique_session_id(canonical_id, all_sessions);
        if record.session.id != display_id {
            rewrite_record_session_display_id(record, &display_id);
        }
    }
}

fn session_source_key(reference: &WorkRef) -> String {
    reference.path.namespace().to_string()
}

fn compact_unique_session_id(canonical_id: &str, all_sessions: &[String]) -> String {
    let canonical_len = canonical_id.chars().count();
    if canonical_len <= 8 {
        return canonical_id.to_string();
    }

    for prefix_len in 8..=canonical_len {
        let candidate = prefix_chars(canonical_id, prefix_len);
        let unique = all_sessions
            .iter()
            .all(|other| other == canonical_id || prefix_chars(other, prefix_len) != candidate);
        if unique {
            return candidate;
        }
    }

    canonical_id.to_string()
}

fn prefix_chars(value: &str, len: usize) -> String {
    value.chars().take(len).collect()
}

fn rewrite_record_session_display_id(record: &mut WorkRecord, display_id: &str) {
    record.session.id = display_id.to_string();
    // Preserve scope; only the session id in the path changes.
    record.work_ref = record.work_ref.with_session(display_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{AgentBlock, AgentBlockKind, AgentSession};
    use crate::record::{
        MessageRole, Projection, WorkActionStatus, WorkActor, WorkContent, WorkContentBlock,
        WorkPart, WorkPartBody, WorkSessionRef, WorkTarget, WorkTime,
    };
    use crate::session_source::SessionInfo;
    use crate::test_fixtures::message_part;
    use anyhow::Result;
    use serde::Serialize;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    fn message(seq: usize, role: MessageRole, content: &str) -> WorkPart {
        message_part(seq, role, content)
    }

    #[test]
    fn keeps_short_session_ids_when_already_unique() {
        let mut records = vec![test_record(
            WorkRef::agent(AgentProvider::Codex, "abcdef12", 1),
            "abcdef12",
            Some("abcdef1234567890"),
        )];

        normalize_session_display_ids(&mut records);

        assert_eq!(records[0].session.id, "abcdef12");
        assert_eq!(records[0].work_ref.to_string(), "codex/abcdef12/1");
    }

    #[test]
    fn extends_display_ids_to_break_canonical_prefix_collisions() {
        let mut records = vec![
            test_record(
                WorkRef::agent(AgentProvider::Codex, "abcdef12", 1),
                "abcdef12",
                Some("abcdef1234567890"),
            ),
            test_record(
                WorkRef::agent(AgentProvider::Codex, "abcdef12", 2),
                "abcdef12",
                Some("abcdef1299999999"),
            ),
        ];

        normalize_session_display_ids(&mut records);

        assert_eq!(records[0].session.id, "abcdef123");
        assert_eq!(records[0].work_ref.to_string(), "codex/abcdef123/1");
        assert_eq!(records[1].session.id, "abcdef129");
        assert_eq!(records[1].work_ref.to_string(), "codex/abcdef129/2");
    }

    #[test]
    fn keeps_provider_namespaces_independent_for_compaction() {
        let mut records = vec![
            test_record(
                WorkRef::agent(AgentProvider::Codex, "abcdef12", 1),
                "abcdef12",
                Some("abcdef1234567890"),
            ),
            test_record(
                WorkRef::agent(AgentProvider::Claude, "abcdef12", 1),
                "abcdef12",
                Some("abcdef1299999999"),
            ),
        ];

        normalize_session_display_ids(&mut records);

        assert_eq!(records[0].session.id, "abcdef12");
        assert_eq!(records[1].session.id, "abcdef12");
    }

    #[test]
    fn deduplicates_canonical_records_and_keeps_more_complete_copy() {
        let mut records = vec![
            test_record(
                WorkRef::agent(AgentProvider::Codex, "abcdef12", 1),
                "abcdef12",
                Some("session-0123456789abcdef"),
            ),
            test_record(
                WorkRef::agent(AgentProvider::Codex, "session-01234567", 1),
                "session-01234567",
                Some("session-0123456789abcdef"),
            ),
        ];
        records[1].parts.push(message(
            3,
            MessageRole::Assistant,
            "assistant with more detail",
        ));

        dedup_records(&mut records);

        assert_eq!(records.len(), 1);
        assert!(records[0]
            .parts
            .iter()
            .any(|part| part.text() == "assistant with more detail"));
        assert_eq!(records[0].session.id, "session-01234567");
    }

    #[test]
    fn agent_records_skips_malformed_session_files_and_reports_them() {
        let cwd = PathBuf::from("/repo");
        let source = BrokenAgentSource {
            infos: vec![
                SessionInfo {
                    path: PathBuf::from("broken.jsonl"),
                    physical_path: None,
                    id: Some("broken".to_string()),
                    cwd: Some("/repo".to_string()),
                    title: Some("broken".to_string()),
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                },
                SessionInfo {
                    path: PathBuf::from("good.jsonl"),
                    physical_path: None,
                    id: Some("good".to_string()),
                    cwd: Some("/repo".to_string()),
                    title: Some("good".to_string()),
                    modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                },
            ],
        };
        let mut skipped = Vec::new();
        let records =
            records_from_source(&source, &cwd, Some(10), &mut skipped).expect("load records");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session.id, "good");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].path, PathBuf::from("broken.jsonl"));
        assert_eq!(skipped[0].namespace, "claude");
    }

    struct BrokenAgentSource {
        infos: Vec<SessionInfo>,
    }

    impl SessionSource for BrokenAgentSource {
        fn namespace(&self) -> &'static str {
            "claude"
        }

        fn list_sessions(&self, _cwd: Option<&Path>) -> Result<Vec<SessionInfo>> {
            Ok(self.infos.clone())
        }

        fn parse_file(&self, path: &Path) -> Result<Vec<WorkRecord>> {
            if path == Path::new("broken.jsonl") {
                anyhow::bail!("synthetic parse error")
            }

            let session = AgentSession {
                path: path.to_path_buf(),
                id: Some("good".to_string()),
                cwd: Some("/repo".to_string()),
                title: Some("good".to_string()),
                blocks: vec![
                    AgentBlock {
                        kind: AgentBlockKind::User,
                        timestamp: None,
                        label: None,
                        call_id: None,
                        text: "question".to_string(),
                        start_line: None,
                    },
                    AgentBlock {
                        kind: AgentBlockKind::Assistant,
                        timestamp: None,
                        label: None,
                        call_id: None,
                        text: "assistant".to_string(),
                        start_line: None,
                    },
                ],
            };
            Ok(WorkRecord::chat_turns(AgentProvider::Claude, &session))
        }
    }

    fn test_record(work_ref: WorkRef, display_id: &str, canonical_id: Option<&str>) -> WorkRecord {
        WorkRecord {
            schema_version: 1,
            work_ref: work_ref.clone(),
            session: WorkSessionRef {
                id: display_id.to_string(),
                canonical_id: canonical_id.map(str::to_string),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "title".to_string(),
            parts: vec![
                message(1, MessageRole::User, "user"),
                message(2, MessageRole::Assistant, "assistant"),
            ],
        }
    }

    #[test]
    fn cache_record_round_trips_through_bincode() {
        let mut record = test_record(
            WorkRef::agent(AgentProvider::Codex, "abcdef12", 1),
            "abcdef12",
            Some("abcdef1234567890"),
        );
        record.parts.extend([
            WorkPart {
                seq: 3,
                occurred_at: Some("2026-01-01T00:00:00Z".to_string()),
                body: WorkPartBody::Message {
                    role: MessageRole::System,
                    label: Some("prompt".to_string()),
                    content: WorkContent::Text {
                        content: "prompt".to_string(),
                        ansi: Some("\x1b[31mred\x1b[0m".to_string()),
                    },
                },
            },
            WorkPart {
                seq: 4,
                occurred_at: None,
                body: WorkPartBody::Action {
                    id: "shell-4".to_string(),
                    actor: WorkActor::User,
                    target: WorkTarget::Shell,
                    title: Some("repo>".to_string()),
                    input: Some(WorkContent::Text {
                        content: "ls -la".to_string(),
                        ansi: None,
                    }),
                    output: vec![
                        WorkContentBlock {
                            content: WorkContent::Text {
                                content: "out".to_string(),
                                ansi: None,
                            },
                            start_line: None,
                        },
                        WorkContentBlock {
                            content: WorkContent::Text {
                                content: "err".to_string(),
                                ansi: None,
                            },
                            start_line: None,
                        },
                    ],
                    status: WorkActionStatus::Failed,
                    exit_code: Some(1),
                },
            },
            WorkPart {
                seq: 5,
                occurred_at: None,
                body: WorkPartBody::Action {
                    id: "call-1".to_string(),
                    actor: WorkActor::Agent,
                    target: WorkTarget::Tool {
                        name: Some("Read".to_string()),
                    },
                    title: None,
                    input: Some(WorkContent::Json(serde_json::json!({
                        "command": "ls",
                        "nested": {"list": [1, 2, 3], "flag": true, "none": null},
                        "big": 18446744073709551615u64,
                        "float": 1.5,
                    }))),
                    output: vec![WorkContentBlock {
                        content: WorkContent::Json(serde_json::json!({"stdout": "hi"})),
                        start_line: Some(7),
                    }],
                    status: WorkActionStatus::Completed,
                    exit_code: None,
                },
            },
            WorkPart {
                seq: 6,
                occurred_at: None,
                body: WorkPartBody::Action {
                    id: "mcp-6".to_string(),
                    actor: WorkActor::Agent,
                    target: WorkTarget::Mcp {
                        server: "sivtr".to_string(),
                        tool: "search".to_string(),
                    },
                    title: None,
                    input: Some(WorkContent::Json(serde_json::json!({"query": "x"}))),
                    output: Vec::new(),
                    status: WorkActionStatus::InProgress,
                    exit_code: None,
                },
            },
            WorkPart {
                seq: 7,
                occurred_at: None,
                body: WorkPartBody::Action {
                    id: "agent-7".to_string(),
                    actor: WorkActor::Agent,
                    target: WorkTarget::Agent {
                        name: "Explore".to_string(),
                    },
                    title: None,
                    input: None,
                    output: vec![WorkContentBlock {
                        content: WorkContent::Text {
                            content: "\u{1}binary".to_string(),
                            ansi: None,
                        },
                        start_line: None,
                    }],
                    status: WorkActionStatus::Cancelled,
                    exit_code: None,
                },
            },
            message(8, MessageRole::Reasoning, "think"),
        ]);

        // MessagePack (rmp-serde) is map-driven, so it natively supports the
        // flattened `WorkPart.body`, the internally-tagged `WorkPartBody`, and
        // `serde_json::Value` tool payloads. `with_struct_map` is required:
        // rmp's default struct-as-array encoding breaks `skip_serializing_if`
        // fields (missing trailing fields shift the array layout).
        let mut serializer = rmp_serde::encode::Serializer::new(Vec::new()).with_struct_map();
        record
            .serialize(&mut serializer)
            .expect("rmp serializes WorkRecord");
        let mp_bytes = serializer.into_inner();
        let mp_restored: WorkRecord =
            rmp_serde::from_slice(&mp_bytes).expect("rmp deserializes WorkRecord");
        assert_eq!(
            record, mp_restored,
            "rmp round-trip must preserve WorkRecord"
        );
        assert_eq!(record.project(Projection::Input).len(), 3);
        assert_eq!(record.project(Projection::Commands).len(), 1);
    }
}
