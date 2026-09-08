use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use sivtr_core::origin::{Entry, Reach};
use sivtr_core::query::{load_workspace_source, LoadMode, NO_RECORD_FOR_SELECTOR};
use sivtr_core::record::{expand_source, WorkPath, WorkRecord, WorkRef};

use crate::commands::memory::filter::{self, Filter};
use crate::commands::memory::records::warn_skipped;
use crate::commands::remote::serve;
use crate::output;

use super::WorkSet;

/// Default deadline for one remote source inside [`query_sources`].
pub const REMOTE_QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Socket-read headroom for one group fan-out inside [`query`]: the daemon
/// dials every member in parallel under its own per-share budget, so this
/// must stay above that budget plus local query time.
const GROUP_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// How one source is scheduled inside [`query_sources`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryTransport {
    /// Disk-local (or named local workspace). Failures abort the batch caller if desired.
    Local,
    /// Mounted remote alias. Failures are isolated when using [`query_sources`].
    Remote,
    /// Group roster fan-out (`team:...`, `team/alice:...`): the daemon dials
    /// every member share in parallel and returns the merged result.
    Group,
}

/// One source to load via the unified query path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuerySource {
    /// Selector accepted by [`query`] (`codex`, `desk:terminal`, …).
    pub selector: String,
    pub transport: QueryTransport,
    /// Local sources load from this root instead of the caller's cwd
    /// (`None` = caller cwd). Lets one batch span many local workspaces.
    pub root: Option<PathBuf>,
    /// Deadline for remote dials and group fan-out; local loads ignore it.
    pub timeout: Duration,
    /// Pre-resolved remote `workspace_key` for `alias:path` sources built
    /// from the registry; `None` means re-resolve on first use (browse-built
    /// sources). Avoids resolving the registry twice for the same query.
    pub workspace_key: Option<String>,
}

impl QuerySource {
    pub fn local(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            transport: QueryTransport::Local,
            root: None,
            timeout: REMOTE_QUERY_TIMEOUT,
            workspace_key: None,
        }
    }

    pub fn local_at(selector: impl Into<String>, root: PathBuf) -> Self {
        Self {
            selector: selector.into(),
            transport: QueryTransport::Local,
            root: Some(root),
            timeout: REMOTE_QUERY_TIMEOUT,
            workspace_key: None,
        }
    }

    pub fn remote(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            transport: QueryTransport::Remote,
            root: None,
            timeout: REMOTE_QUERY_TIMEOUT,
            workspace_key: None,
        }
    }

    pub fn group(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            transport: QueryTransport::Group,
            root: None,
            timeout: GROUP_QUERY_TIMEOUT,
            workspace_key: None,
        }
    }

    /// Build a source from a registry entry: local workspaces load from
    /// their own root, remote mounts keep the `alias:path` selector the
    /// peer-side dispatcher already understands. Adding an origin kind means
    /// one new arm here — `query_sources` then schedules it unchanged.
    pub fn from_entry(entry: &Entry, source: &str) -> Self {
        match &entry.reach {
            Reach::Local { root } => Self::local_at(source.to_string(), PathBuf::from(root)),
            Reach::Remote { workspace_key } => Self {
                selector: format!("{}:{source}", entry.origin.name),
                transport: QueryTransport::Remote,
                root: None,
                timeout: REMOTE_QUERY_TIMEOUT,
                workspace_key: Some(workspace_key.clone()),
            },
        }
    }
}

/// Per-source outcome from [`query_sources`]. Failures never drop other sources.
#[derive(Debug)]
pub enum QuerySourceResult {
    Ok(WorkSet),
    Err(String),
}

/// Unified query: local, remote, and group sources share one shape.
///
/// Remote is only transport: same `Filter` is sent, peer runs the same local
/// path on the share root, result comes back. The load mode is derived from
/// the filter: bounds that read part text (pattern, exclude, BM25 ranking)
/// force a full load; pure metadata queries stay light and callers
/// materialize part text on demand.
pub fn query(source: &str, filter: Filter, cwd: Option<&Path>) -> Result<WorkSet> {
    if source == "@" {
        return apply_loaded(read_stdin()?, filter);
    }
    if source.starts_with('@') {
        return apply_loaded(super::load_reference(source)?, filter);
    }

    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);

    let sources = resolve_source(source, &cwd)?;
    let results = query_sources(&sources, filter.clone(), Some(&cwd))?;
    merge_and_apply(results, &cwd, filter)
}

/// Resolve a source expression into the concrete sources it addresses.
/// Every scope shape (bare selector, `local:`, `all:`, registry aliases,
/// groups) collapses into a [`QuerySource`] list here; [`query_sources`] then
/// schedules them unchanged.
fn resolve_source(source: &str, cwd: &Path) -> Result<Vec<QuerySource>> {
    let source = expand_source(source)?;

    // Bare selector: the current workspace's local source.
    let Some((scope, path)) = source.split_once(':') else {
        return Ok(vec![QuerySource::local_at(source, cwd.to_path_buf())]);
    };
    if path.is_empty() {
        anyhow::bail!("source `{source}` is missing a selector after `:`");
    }
    if path.starts_with('/') {
        anyhow::bail!(
            "Invalid source `{source}`; use `scope:path` (for example `desk:terminal`), not `://`"
        );
    }

    let scope = scope.to_ascii_lowercase();
    match scope.as_str() {
        "local" => Ok(vec![QuerySource::local_at(
            path.to_string(),
            cwd.to_path_buf(),
        )]),
        "all" => {
            // Passive enumeration, same as `ws list`: mounts are listed only
            // while the daemon is already running. `all:` never starts the
            // daemon, so a pure-local query is not blocked on daemon startup.
            let registry = crate::origins::collect(cwd)?;
            Ok(registry
                .entries()
                .map(|entry| QuerySource::from_entry(entry, path))
                .collect())
        }
        // Everything else is a named origin (local workspace or remote
        // mount) or a group. The registry is the single alias table;
        // resolution applies kind precedence on name collisions.
        scope => {
            let mut registry = crate::origins::collect(cwd)?;
            // The registry lists mounts only while the daemon is already
            // running, so a passive miss is retried once with the daemon up
            // — a cold `desk:terminal` query must still resolve its mount.
            if registry.resolve(scope)?.is_none() {
                serve::ensure_running()?;
                registry = crate::origins::collect(cwd)?;
            }
            match registry.resolve(scope)? {
                Some(entry) => Ok(vec![QuerySource::from_entry(entry, path)]),
                None if split_group_scope(scope).is_some() => {
                    Ok(vec![QuerySource::group(&source)])
                }
                None => anyhow::bail!(
                    "unknown scope `{scope}`; use `sivtr ws list` for local workspaces, `sivtr remote list` for remotes, or `sivtr group list` for groups"
                ),
            }
        }
    }
}

/// Merge per-source outcomes into one corpus, then apply the filter once
/// across the merged records. A failed source drops without aborting the
/// batch; when every source failed, the first error is what the caller sees.
/// Anchors merge with the records: a source that resolved part anchors keeps
/// them, and one that carried none contributes its records' whole refs.
fn merge_and_apply(results: Vec<QuerySourceResult>, cwd: &Path, filter: Filter) -> Result<WorkSet> {
    let mut records: Vec<WorkRecord> = Vec::new();
    let mut anchors: Vec<WorkRef> = Vec::new();
    let mut seen: HashSet<WorkRef> = HashSet::new();
    let mut errors: Vec<String> = Vec::new();
    let mut any_ok = false;
    for result in results {
        match result {
            QuerySourceResult::Ok(set) => {
                any_ok = true;
                let (set_records, set_anchors) = set.into_parts();
                anchors.extend(set_anchors);
                for record in set_records {
                    if seen.insert(record.work_ref.whole()) {
                        records.push(record);
                    }
                }
            }
            QuerySourceResult::Err(message) => errors.push(message),
        }
    }
    if !any_ok {
        if let Some(first) = errors.first() {
            anyhow::bail!("{first}");
        }
        return apply_loaded(WorkSet::new(cwd.display().to_string(), Vec::new()), filter);
    }
    for error in &errors {
        output::warning(format!("skipped an origin: {error}"));
    }
    // `from_parts` normalizes the anchors (dedup + Whole canonical form), so
    // the merged per-source anchors need no pre-dedup here.
    apply_loaded(
        WorkSet::from_parts(cwd.display().to_string(), records, anchors),
        filter,
    )
}

/// Load many sources in parallel — local, remote, and group share one
/// scheduler. A local source may carry its own root ([`QuerySource::root`])
/// so one batch spans every local workspace; remotes run with a per-source
/// timeout and out-of-order arrival, and a source's failure never drops the
/// others. Order of `results` matches `sources`.
pub fn query_sources(
    sources: &[QuerySource],
    filter: Filter,
    cwd: Option<&Path>,
) -> Result<Vec<QuerySourceResult>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let cwd = cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?);
    let cwd = cwd.as_path();

    // Bounds that read part text (pattern, exclude, BM25 ranking) need a full
    // load; metadata-only filters stay light and callers materialize part
    // text on demand.
    let mode = if filter.needs_parts() {
        LoadMode::Full
    } else {
        LoadMode::Light
    };

    // Scoped threads borrow `sources` and `cwd` instead of cloning per
    // worker; only the filter (consumed by each load path) is cloned. Each
    // worker reports `(idx, outcome)`, joined in list order so a slow source
    // never stalls the others' completion.
    let outcomes = std::thread::scope(|scope| {
        let handles: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(idx, source)| {
                let filter = filter.clone();
                scope.spawn(move || {
                    let result = match source.transport {
                        QueryTransport::Local => {
                            let root = source.root.as_deref().unwrap_or(cwd);
                            run_local(&source.selector, root, filter, mode)
                        }
                        QueryTransport::Remote => query_remote_bounded(
                            &source.selector,
                            source.workspace_key.as_deref(),
                            filter,
                            cwd,
                            source.timeout,
                        ),
                        QueryTransport::Group => {
                            group_query(&source.selector, filter, cwd, source.timeout)
                        }
                    };
                    (idx, normalize_source_result(result, cwd))
                })
            })
            .collect();

        let mut outcomes: Vec<Option<QuerySourceResult>> = sources.iter().map(|_| None).collect();
        for (slot, handle) in handles.into_iter().enumerate() {
            // A panicked worker must not abort the batch; it becomes that
            // source's error, and the rest still complete.
            outcomes[slot] = Some(match handle.join() {
                Ok((_idx, outcome)) => outcome,
                Err(_) => QuerySourceResult::Err("query worker panicked".to_string()),
            });
        }
        outcomes
    });

    Ok(outcomes
        .into_iter()
        .map(|slot| slot.unwrap_or(QuerySourceResult::Err("load worker exited".to_string())))
        .collect())
}

/// Canonical per-source outcome: an empty selector (no records for the
/// source, e.g. a workspace without terminal logs) is an empty result, not an
/// error — the batch keeps the rest. Timeouts report as `timeout`.
fn normalize_source_result(result: Result<WorkSet>, cwd: &Path) -> QuerySourceResult {
    match result {
        Ok(set) => QuerySourceResult::Ok(set),
        Err(error) => {
            let message = error.to_string();
            if message.starts_with(NO_RECORD_FOR_SELECTOR) {
                QuerySourceResult::Ok(WorkSet::from_parts(
                    cwd.display().to_string(),
                    Vec::new(),
                    Vec::new(),
                ))
            } else if is_timeout_error(&message) {
                QuerySourceResult::Err("timeout".to_string())
            } else {
                QuerySourceResult::Err(format!("{error:#}"))
            }
        }
    }
}

fn query_remote_bounded(
    selector: &str,
    workspace_key: Option<&str>,
    filter: Filter,
    cwd: &Path,
    read_timeout: Duration,
) -> Result<WorkSet> {
    // Remote sources are always `alias:path` — resolve_source pins them, and
    // browse builds them from registry origins. Anything else is a bug in
    // the caller, not a selector to re-resolve.
    let Some((scope, path)) = selector.split_once(':') else {
        anyhow::bail!("remote source `{selector}` must be `alias:path`");
    };
    if path.is_empty() || path.starts_with('/') || scope.eq_ignore_ascii_case("local") {
        anyhow::bail!("remote source `{selector}` must be `alias:path`");
    }
    // Pre-resolved sources skip the registry; browse-built ones resolve here.
    let workspace_key = match workspace_key {
        Some(key) => key.to_string(),
        None => {
            serve::ensure_running()?;
            let registry = crate::origins::collect(cwd)?;
            let Some(entry) = registry.resolve(scope)? else {
                anyhow::bail!("unknown remote alias `{scope}`; use `sivtr remote list`");
            };
            let Reach::Remote { workspace_key } = &entry.reach else {
                anyhow::bail!("`{scope}` is not a remote mount");
            };
            workspace_key.clone()
        }
    };
    try_remote_timed(&workspace_key, scope, path, filter, cwd, read_timeout)
        .with_context(|| format!("remote mount `{scope}` unavailable"))
}

fn is_timeout_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("os error 10060")
        || lower.contains("i/o operation")
}

/// Peer-side query on a share root, optional redact. An empty workspace has
/// no sessions and reports an empty result instead of erroring, so one
/// member's empty contribution cannot abort a group fan-out.
///
/// The peer renders the response, so records are always loaded in full.
pub fn run_on_share(
    root: &Path,
    source: &str,
    filter: Filter,
    redact: bool,
) -> Result<(Vec<WorkRecord>, Vec<WorkRef>)> {
    match run_local(source, root, filter.for_remote_peer(), LoadMode::Full) {
        Ok(mut set) => {
            if redact {
                for record in set.records_mut() {
                    *record = crate::remote::redact::redact_record(record)
                        .context("redact shared record")?;
                }
            }
            Ok(set.into_parts())
        }
        Err(error) if error.to_string().starts_with(NO_RECORD_FOR_SELECTOR) => {
            Ok((Vec::new(), Vec::new()))
        }
        Err(error) => Err(error),
    }
}

fn run_local(source: &str, root: &Path, filter: Filter, mode: LoadMode) -> Result<WorkSet> {
    let result = load_workspace_source(root, source, mode)?;
    warn_skipped(&result.skipped);
    apply_loaded(
        WorkSet::from_parts(root.display().to_string(), result.records, result.anchors),
        filter,
    )
}

fn apply_loaded(set: WorkSet, filter: Filter) -> Result<WorkSet> {
    let cwd = PathBuf::from(&set.cwd);
    let (records, anchors) = set.into_parts();
    filter::apply(cwd, records, anchors, filter)
}

fn try_remote_timed(
    workspace_key: &str,
    alias: &str,
    path: &str,
    filter: Filter,
    cwd: &Path,
    read_timeout: Duration,
) -> Result<WorkSet> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    // The registry already confirmed the mount; ensure the daemon is up so a
    // stale socket yields a clear error instead of a confusing one.
    crate::commands::remote::serve::ensure_running()?;
    match ipc::call_with_read_timeout(
        LocalRequest::RemoteQuery {
            workspace_key: workspace_key.to_string(),
            alias: alias.to_ascii_lowercase(),
            source: path.to_string(),
            filter,
        },
        read_timeout,
    )? {
        LocalResponse::Query(response) => Ok(WorkSet::from_parts(
            cwd.display().to_string(),
            response.records,
            response.anchors,
        )),
        response => anyhow::bail!("Unexpected daemon response: {response:?}"),
    }
}

/// Group fan-out: `team:...` (all members), `team/alice:...` (one member), or
/// `team/alice/proj-b:...` (one member, one contributed share). The daemon
/// answers `None` when the group is unknown, which a scheduled [`QuerySource`]
/// treats as a hard error — the scope was already pinned to a group by the
/// caller.
fn group_query(
    selector: &str,
    filter: Filter,
    cwd: &Path,
    read_timeout: Duration,
) -> Result<WorkSet> {
    use crate::remote::ipc;
    use crate::remote::protocol::{LocalRequest, LocalResponse};

    let Some((scope, path)) = selector.split_once(':') else {
        anyhow::bail!("group source `{selector}` is missing a selector after `:`");
    };
    let Some((group, member, share)) = split_group_scope(scope) else {
        anyhow::bail!(
            "`{scope}` is not a group scope; use `team:`, `team/alice:`, or `team/alice/proj-b:`"
        );
    };
    crate::commands::remote::serve::ensure_running()
        .context("failed to start the sivtr daemon for a group query")?;
    // The daemon answers `None` for an unknown group; the fan-out happens
    // inside it (parallel per-member dials), so the socket read gets enough
    // headroom beyond the daemon's per-peer budget.
    match ipc::call_with_read_timeout(
        LocalRequest::GroupQuery {
            group,
            member,
            share,
            source: path.to_string(),
            filter,
        },
        read_timeout,
    )
    .context("group query failed")?
    {
        LocalResponse::GroupQuery(None) => {
            anyhow::bail!("unknown group; use `sivtr group list` to see groups")
        }
        LocalResponse::GroupQuery(Some(response)) => {
            if !response.skipped.is_empty() {
                output::info(format!(
                    "group members offline: {}",
                    response.skipped.join(", ")
                ));
            }
            Ok(WorkSet::from_parts(
                cwd.display().to_string(),
                response.query.records,
                response.query.anchors,
            ))
        }
        response => anyhow::bail!("Unexpected daemon response: {response:?}"),
    }
}

/// Split a group scope: `team` (all members), `team/alice` (one member), or
/// `team/alice/proj-b` (one member, one contributed share). Returns `None` for
/// anything that is not a valid group scope so the caller can fall through to
/// the local-workspace cascade.
fn split_group_scope(scope: &str) -> Option<(String, Option<String>, Option<String>)> {
    let parts: Vec<&str> = scope.split('/').collect();
    match parts.as_slice() {
        [group] if is_identifier(group) => Some((group.to_string(), None, None)),
        [group, member] if is_identifier(group) && is_identifier(member) => {
            Some((group.to_string(), Some(member.to_string()), None))
        }
        [group, member, share]
            if is_identifier(group) && is_identifier(member) && is_identifier(share) =>
        {
            Some((
                group.to_string(),
                Some(member.to_string()),
                Some(share.to_string()),
            ))
        }
        _ => None,
    }
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn read_stdin() -> Result<WorkSet> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("Failed to read WorkSet from stdin")?;
    let set: WorkSet =
        serde_json::from_str(&input).context("Failed to parse WorkSet from stdin")?;
    set.validate().context("Invalid WorkSet from stdin")?;
    Ok(set)
}

pub fn load_context_records(
    source_records: &[WorkRecord],
    source_anchors: &[WorkRef],
    cwd: &Path,
) -> Result<Vec<WorkRecord>> {
    let mut sources = Vec::new();
    let mut seen_sources = HashSet::new();
    for anchor in source_anchors {
        let record = super::find_record([source_records], anchor)
            .with_context(|| format!("No record found for ref `{anchor}`"))?;
        let path = match &record.work_ref.path {
            WorkPath::Terminal { session, .. } => format!("terminal/{session}"),
            WorkPath::Agent {
                provider, session, ..
            } => format!("{}/{session}", provider.command_name()),
        };
        let source = match anchor.scope_name() {
            Some(scope) => format!("{scope}:{path}"),
            None => path,
        };
        if seen_sources.insert(source.clone()) {
            sources.push(source);
        }
    }

    let mut records = Vec::new();
    let mut seen_records = HashSet::new();
    for source in sources {
        // Context expansion renders full record bodies, so bypass the
        // filter-driven light mode and force a full load.
        let mut set = query(&source, Filter::none(), Some(cwd))?;
        set.materialize_parts()?;
        for record in set.into_records() {
            let key = record.work_ref.whole().to_string();
            if seen_records.insert(key) {
                records.push(record);
            }
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::{merge_and_apply, resolve_source, split_group_scope, QueryTransport};
    use crate::commands::memory::workset::{QuerySourceResult, WorkSet};
    use sivtr_core::record::{MessageRole, WorkRecord, WorkSessionRef, WorkTime};
    use sivtr_core::search::Filter;

    fn record(index: usize) -> WorkRecord {
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: format!("terminal/session_1/{index}")
                .parse()
                .expect("valid work ref"),
            session: WorkSessionRef {
                id: "session_1".to_string(),
                canonical_id: Some("session_1".to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: format!("record {index}"),
            parts: (1..=2)
                .map(|seq| {
                    crate::test_fixtures::message_part(
                        seq,
                        MessageRole::Assistant,
                        &format!("record {index} part {seq}"),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn merging_sources_keeps_each_ones_anchor_granularity() {
        let first = record(1);
        let second = record(2);
        let part = first.work_ref.with_part(2);
        let results = vec![
            QuerySourceResult::Ok(WorkSet::from_parts(
                "/repo",
                vec![first],
                vec![part.clone()],
            )),
            QuerySourceResult::Ok(WorkSet::new("/repo", vec![second.clone()])),
        ];

        let merged = merge_and_apply(results, std::path::Path::new("/repo"), Filter::none())
            .expect("merge succeeds");
        let mut refs = merged
            .anchors()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        refs.sort();
        assert_eq!(
            refs,
            vec![part.to_string(), second.work_ref.whole().to_string()],
            "a part anchor must survive the merge alongside a record anchor"
        );
    }

    #[test]
    fn group_scope_splits_team_and_member_forms() {
        assert_eq!(
            split_group_scope("team"),
            Some(("team".to_string(), None, None))
        );
        assert_eq!(
            split_group_scope("team/alice"),
            Some(("team".to_string(), Some("alice".to_string()), None))
        );
        assert_eq!(
            split_group_scope("team/alice/proj-b"),
            Some((
                "team".to_string(),
                Some("alice".to_string()),
                Some("proj-b".to_string())
            ))
        );
        // Not group-shaped: falls through to the local-workspace cascade.
        assert_eq!(split_group_scope("team/a/b/c"), None);
        assert_eq!(split_group_scope("team@alice"), None);
        assert_eq!(split_group_scope(""), None);
        assert_eq!(split_group_scope("team/"), None);
        assert_eq!(split_group_scope("a b"), None);
    }

    #[test]
    fn resolve_bare_selector_is_local_with_cwd_root() {
        let cwd = std::path::Path::new("/repo");
        let sources = resolve_source("terminal", cwd).expect("resolve");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].selector, "terminal");
        assert_eq!(sources[0].transport, QueryTransport::Local);
        assert_eq!(sources[0].root.as_deref(), Some(cwd));
    }

    #[test]
    fn resolve_local_prefix_strips_the_scope() {
        let cwd = std::path::Path::new("/repo");
        let sources = resolve_source("local:codex", cwd).expect("resolve");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].selector, "codex");
        assert_eq!(sources[0].transport, QueryTransport::Local);
        assert_eq!(sources[0].root.as_deref(), Some(cwd));
    }

    #[test]
    fn resolve_rejects_empty_or_absolute_paths() {
        let cwd = std::path::Path::new("/repo");
        assert!(resolve_source("desk:", cwd).is_err());
        assert!(resolve_source("desk:/absolute", cwd).is_err());
    }
}
