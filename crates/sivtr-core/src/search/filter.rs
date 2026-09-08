//! Core search pipeline: metadata bounds, per-line boolean pattern, and BM25
//! relevance ranking. One `Filter` type serves the CLI, the remote wire
//! protocol, the TUI, and the eval benchmark — no parallel logic sets.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::agents::{AgentProvider, AgentSessionProvider};
use crate::record::{
    WorkAt, WorkOutcome, WorkPart, WorkPartKind, WorkRecord, WorkRecordKind, WorkRef,
};
use crate::time::parse_timestamp;

use super::bm25::{body_text, Bm25Index, SimpleTokenizer, TITLE_WEIGHT};
use super::expand::Prf;
use super::types::{Field, FilterMode, PartKind, Sort};

/// PRF expansion is ON (tuned lambda/max_terms in `expand.rs`; the difficulty
/// gate suppresses it for common-word queries). RRF fusion was tried, measured
/// ndcg@5 0.830 -> 0.813 on the frozen eval, and dropped (see
/// docs/retrieval-literature.md).
const PRF_ENABLED: bool = true;

/// One search hit: the anchor plus its BM25 relevance score when ranking by
/// relevance (None otherwise).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredHit {
    pub anchor: WorkRef,
    pub score: Option<f32>,
}

/// A single per-line regex match inside one record (TUI search results).
#[derive(Debug, Clone, PartialEq)]
pub struct LineMatch {
    pub part_seq: usize,
    pub line: usize,
    pub text: String,
}

/// Unified filter for local and remote query. One type for CLI, wire, eval.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub mode: FilterMode,
    /// Uncompiled boolean pattern; applied case-insensitively per line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_regex: Option<String>,
    /// BM25 query used when `sort` is Relevance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<String>,
    #[serde(default)]
    pub in_field: Field,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<PartKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Client-only; forced false when applied on a remote peer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_current: bool,
    #[serde(default)]
    pub sort: Sort,
    /// Newest-first window applied before the final sort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl Filter {
    /// Keep every loaded anchor (show/nav/zoom).
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether applying this filter reads part text: per-line pattern and
    /// exclude matching scan `parts`, BM25 ranking is built from part text,
    /// parts-mode emits per-part hits, kind and field bounds match on
    /// `part.kind()`, and input/output/command fields inspect part kinds.
    /// Title/session bounds and the default content field with no pattern
    /// stay metadata-only.
    pub fn needs_parts(&self) -> bool {
        self.pattern.is_some()
            || self.exclude_regex.is_some()
            || self.rank.is_some()
            || self.mode == FilterMode::Parts
            || self.kind.is_some()
            || matches!(self.in_field, Field::Input | Field::Output | Field::Command)
    }

    /// Browse session list: newest-first, bounded by `latest` records.
    pub fn browse_session_page(latest: usize) -> Self {
        Self {
            sort: Sort::Newest,
            latest: Some(latest.max(1)),
            ..Self::default()
        }
    }

    /// Benchmark filter for `sivtr eval`: rank the whole field-scoped corpus
    /// under `sort` and keep the top `k`. No pattern match — the eval measures
    /// ranking quality, so the pipeline returns the top-k of a full-corpus
    /// ranking. `pattern` feeds BM25 when `sort` is Relevance.
    pub fn eval(pattern: &str, field: Field, sort: Sort, k: usize) -> Self {
        Self {
            mode: FilterMode::Anchors,
            pattern: None,
            rank: (sort == Sort::Relevance).then(|| pattern.to_string()),
            in_field: field,
            sort,
            limit: Some(k),
            ..Self::default()
        }
    }

    /// Drop client-only flags before applying on a remote peer. Relevance is
    /// computed peer-side over the share's own corpus, so `rank` stays.
    pub fn for_remote_peer(&self) -> Self {
        let mut filter = self.clone();
        filter.exclude_current = false;
        filter
    }

    fn time_range(&self) -> Result<Option<TimeRange>> {
        match (&self.since, &self.until) {
            (None, None) => Ok(None),
            (since, until) => {
                let since = since
                    .as_deref()
                    .map(|value| {
                        DateTime::parse_from_rfc3339(value)
                            .map(|dt| dt.with_timezone(&Utc))
                            .with_context(|| format!("Invalid filter.since: {value}"))
                    })
                    .transpose()?;
                let until = until
                    .as_deref()
                    .map(|value| {
                        DateTime::parse_from_rfc3339(value)
                            .map(|dt| dt.with_timezone(&Utc))
                            .with_context(|| format!("Invalid filter.until: {value}"))
                    })
                    .transpose()?;
                Ok(Some(TimeRange { since, until }))
            }
        }
    }
}

struct TimeRange {
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
}

impl TimeRange {
    fn contains(&self, timestamp: Option<&str>) -> bool {
        let Some(timestamp) = timestamp.and_then(parse_timestamp) else {
            return false;
        };
        if self.since.is_some_and(|since| timestamp < since) {
            return false;
        }
        if self.until.is_some_and(|until| timestamp > until) {
            return false;
        }
        true
    }
}

/// Where a searcher gets its BM25 index: lazily built and owned, or shared
/// from a caller-owned cache that outlives individual searches.
enum IndexSource<'a> {
    Lazy(Box<RefCell<Option<Bm25Index>>>),
    Shared(&'a Bm25Index),
}

/// Search engine over a record corpus. The BM25 index is built lazily on the
/// first relevance-ordered search and reused by every later one; callers that
/// rebuild the corpus rarely (TUI browsing) can own the index themselves via
/// [`Searcher::with_index`] and skip rebuilding it per search.
pub struct Searcher<'a> {
    records: &'a [WorkRecord],
    bm25: IndexSource<'a>,
}

impl<'a> Searcher<'a> {
    pub fn new(records: &'a [WorkRecord]) -> Self {
        Self {
            records,
            bm25: IndexSource::Lazy(Box::new(RefCell::new(None))),
        }
    }

    /// Search over `records` using a caller-owned, already-built index.
    pub fn with_index(records: &'a [WorkRecord], bm25: &'a Bm25Index) -> Self {
        Self {
            records,
            bm25: IndexSource::Shared(bm25),
        }
    }

    /// Run the filter pipeline over `anchors`: metadata bounds → per-line
    /// pattern → exclude → dedup → `latest` window → final sort → `limit`.
    pub fn search(
        &self,
        filter: &Filter,
        anchors: &[WorkRef],
        cwd: &Path,
    ) -> Result<Vec<ScoredHit>> {
        let pattern = compile_regex(filter.pattern.as_deref())?;
        let exclude = compile_regex(filter.exclude_regex.as_deref())?;
        let time_range = filter.time_range()?;
        let excluded_sessions = if filter.exclude_current {
            current_agent_session_paths(&providers_for_records(self.records), cwd)?
        } else {
            HashSet::new()
        };

        let index = RecordIndex::new(self.records);
        let mut hits = Vec::new();
        for anchor in anchors {
            let Some(record) = index.resolve(anchor) else {
                continue;
            };
            if !record_matches_metadata(record, filter, time_range.as_ref(), &excluded_sessions) {
                continue;
            }
            for hit in matching_hits(record, anchor, filter, pattern.as_ref()) {
                if !match_excluded(record, &hit.anchor, exclude.as_ref()) {
                    hits.push(hit);
                }
            }
        }
        let mut hits = dedup_hits(hits);

        if let Some(latest) = filter.latest {
            hits.sort_by(|a, b| compare_time_desc(&index, &a.anchor, &b.anchor));
            hits.truncate(latest);
        }

        let scores = if filter.sort == Sort::Relevance {
            filter
                .rank
                .as_deref()
                .map(|query| self.relevance_scores(query))
        } else {
            None
        };
        if let Some(scores) = &scores {
            for hit in &mut hits {
                hit.score = scores.get(&hit.anchor).copied();
            }
        }
        sort_hits(&mut hits, &index, filter.sort, scores.as_ref());

        if let Some(limit) = filter.limit {
            hits.truncate(limit);
        }
        Ok(hits)
    }

    /// Per-line regex matches across every part of `record` (TUI search view).
    pub fn content_line_matches(&self, record: &WorkRecord, regex: &Regex) -> Vec<LineMatch> {
        content_line_matches(record, regex)
    }

    fn relevance_scores(&self, query: &str) -> HashMap<WorkRef, f32> {
        let mut lazy_slot;
        let index: &Bm25Index = match &self.bm25 {
            IndexSource::Shared(index) => index,
            IndexSource::Lazy(cell) => {
                lazy_slot = cell.borrow_mut();
                lazy_slot.get_or_insert_with(|| super::index_cache::build_or_load(self.records))
            }
        };

        let query_tokens = SimpleTokenizer.tokenize(query);
        // The command-field boost applies to multi-token queries only: a
        // single-token query like `grok` must not promote unrelated terminal
        // records (title `grok`, few mentions) over provider sessions whose
        // titles never contain the keyword.
        let title_weight = if query_tokens.len() >= 2 {
            TITLE_WEIGHT
        } else {
            0.0
        };
        let mut terms: Vec<(String, f64)> = query_tokens
            .iter()
            .cloned()
            .map(|token| (token, 1.0))
            .collect();

        // Pseudo-relevance feedback: expand rare-term queries with terms
        // harvested from the top-ranked documents. The difficulty gate
        // suppresses expansion for common-word queries, where the
        // pseudo-relevant set is dominated by noise.
        if PRF_ENABLED {
            let prf = Prf::default();
            if prf.gate(&query_tokens, |token| index.df_ratio(token)) {
                let mut docs = Vec::new();
                for (reference, _) in index
                    .rank_terms_with(&terms, title_weight)
                    .into_iter()
                    .take(prf.top_k)
                {
                    if let Some(record) = self.records.iter().find(|r| r.work_ref == reference) {
                        docs.push(SimpleTokenizer.tokenize(&body_text(record)));
                    }
                }
                for term in prf.select_terms(&query_tokens, &docs) {
                    if !terms.iter().any(|(existing, _)| *existing == term) {
                        terms.push((term, prf.lambda));
                    }
                }
            }
        }

        let ranked = index.rank_terms_with(&terms, title_weight);
        ranked.into_iter().collect()
    }
}

/// Per-line regex matches across every part of `record`.
pub fn content_line_matches(record: &WorkRecord, regex: &Regex) -> Vec<LineMatch> {
    record
        .parts
        .iter()
        .flat_map(|part| {
            part.text()
                .lines()
                .enumerate()
                .filter(|(_, line)| regex.is_match(line))
                .map(|(line_index, line)| LineMatch {
                    part_seq: part.seq,
                    line: line_index + 1,
                    text: line.to_string(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn compile_regex(value: Option<&str>) -> Result<Option<Regex>> {
    value
        .map(|query| Regex::new(&format!("(?i){query}")))
        .transpose()
        .context("Invalid filter regex")
}

/// O(1) anchor → record resolution for the duration of one search.
///
/// The previous implementation linear-scanned `records` for every anchor,
/// making the pipeline O(anchors × records); with thousands of records that
/// dominated the whole search cost.
struct RecordIndex<'a> {
    records: &'a [WorkRecord],
    by_ref: HashMap<WorkRef, usize>,
}

impl<'a> RecordIndex<'a> {
    fn new(records: &'a [WorkRecord]) -> Self {
        let mut by_ref = HashMap::with_capacity(records.len());
        for (index, record) in records.iter().enumerate() {
            // First occurrence wins, matching the previous linear `find`.
            by_ref.entry(record.work_ref.clone()).or_insert(index);
        }
        Self { records, by_ref }
    }

    fn resolve(&self, anchor: &WorkRef) -> Option<&'a WorkRecord> {
        let key = if anchor.at == WorkAt::Whole {
            anchor
        } else {
            // Part anchors resolve through their whole form (same as the old
            // `anchor.whole()` lookup); avoid the clone for the common case.
            &anchor.whole()
        };
        self.by_ref.get(key).map(|&index| &self.records[index])
    }
}

fn providers_for_records(records: &[WorkRecord]) -> Vec<AgentProvider> {
    let mut providers = Vec::new();
    for record in records {
        if let Some(provider) = record.work_ref.provider() {
            if !providers.contains(&provider) {
                providers.push(provider);
            }
        }
    }
    providers
}

fn record_matches_metadata(
    record: &WorkRecord,
    filter: &Filter,
    time_range: Option<&TimeRange>,
    excluded_sessions: &HashSet<PathBuf>,
) -> bool {
    !excluded_session_matches(record, excluded_sessions)
        && status_matches(
            filter.status,
            record
                .status
                .as_ref()
                .map(|status| status.outcome)
                .unwrap_or(WorkOutcome::Unknown),
        )
        && exit_code_matches(
            filter.exit_code,
            record.status.as_ref().and_then(|status| status.exit_code),
        )
        && duration_matches(
            filter.min_duration_ms,
            filter.max_duration_ms,
            record.time.duration_ms,
        )
        && time_range.is_none_or(|range| range.contains(record.time.primary_at()))
}

fn status_matches(status: Option<WorkOutcome>, outcome: WorkOutcome) -> bool {
    match status {
        Some(expected) => expected == outcome,
        None => true,
    }
}

fn exit_code_matches(expected: Option<i32>, actual: Option<i32>) -> bool {
    expected.is_none_or(|expected| actual == Some(expected))
}

fn duration_matches(min: Option<u64>, max: Option<u64>, actual: Option<u64>) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }
    let Some(actual) = actual else {
        return false;
    };
    min.is_none_or(|min| actual >= min) && max.is_none_or(|max| actual <= max)
}

fn matching_hits(
    record: &WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    pattern: Option<&Regex>,
) -> Vec<ScoredHit> {
    match filter.mode {
        FilterMode::Anchors => match anchor.at {
            WorkAt::Whole => record_anchor_hits(record, anchor, filter, pattern),
            WorkAt::Part(_) => part_anchor_hit(record, anchor, filter, pattern),
        },
        FilterMode::Parts => match anchor.at {
            WorkAt::Part(_) => part_anchor_hit(record, anchor, filter, pattern),
            WorkAt::Whole => record
                .parts
                .iter()
                .filter(|part| part_matches_filters(part, filter, pattern, false))
                .map(|part| hit(record.work_ref.with_part(part.seq)))
                .collect(),
        },
    }
}

fn record_anchor_hits(
    record: &WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    pattern: Option<&Regex>,
) -> Vec<ScoredHit> {
    if matches!(filter.in_field, Field::Title | Field::Session) {
        return (filter.kind.is_none() && meta_matches(record, filter.in_field, pattern))
            .then(|| hit(anchor.clone()))
            .into_iter()
            .collect();
    }

    // A metadata-only record (light view with empty `parts`, as browse
    // session lists load) matches an unrestricted content query — one that
    // reads no part text (no pattern, exclude, rank, or kind bound).
    // Without this, empty parts can never match, so session lists come back
    // empty once the meta cache is warm.
    if record.parts.is_empty() && !filter.needs_parts() && filter.in_field == Field::Content {
        return vec![hit(anchor.clone())];
    }

    let matched_meta = filter.kind.is_none()
        && filter.in_field == Field::All
        && meta_matches(record, Field::All, pattern);
    let matched_part = record
        .parts
        .iter()
        .any(|part| part_matches_filters(part, filter, pattern, false));
    (matched_meta || matched_part)
        .then(|| hit(anchor.clone()))
        .into_iter()
        .collect()
}

fn part_anchor_hit(
    record: &WorkRecord,
    anchor: &WorkRef,
    filter: &Filter,
    pattern: Option<&Regex>,
) -> Vec<ScoredHit> {
    let Some(part) = record.part_for_at(anchor.at) else {
        return Vec::new();
    };
    part_matches_filters(part, filter, pattern, true)
        .then(|| hit(anchor.clone()))
        .into_iter()
        .collect()
}

fn hit(anchor: WorkRef) -> ScoredHit {
    ScoredHit {
        anchor,
        score: None,
    }
}

fn part_matches_filters(
    part: &WorkPart,
    filter: &Filter,
    pattern: Option<&Regex>,
    pinned: bool,
) -> bool {
    if filter.kind.is_some_and(|kind| !kind.matches(part.kind())) {
        return false;
    }
    if !part_field_matches(part, filter.in_field) {
        return false;
    }
    // Default content search covers the same text BM25 ranks: dialogue turns,
    // terminal output, tool results (execution errors), and thinking (error
    // reasoning). Tool-call payloads and skill text stay out as noise; they
    // remain reachable with `--kind` or `--in all`.
    if !pinned
        && matches!(filter.in_field, Field::Content)
        && filter.kind.is_none()
        && matches!(part.kind(), WorkPartKind::ToolCall | WorkPartKind::Skill)
    {
        return false;
    }
    pattern.is_none_or(|pattern| text_has_matching_line(&part.text(), pattern))
}

fn part_field_matches(part: &WorkPart, field: Field) -> bool {
    matches!(field, Field::Content | Field::All)
        || matches!(field, Field::Input) && part.kind().is_input()
        || matches!(field, Field::Output) && part.kind().is_output()
        || matches!(field, Field::Command) && part.kind() == WorkPartKind::Command
}

fn meta_matches(record: &WorkRecord, field: Field, pattern: Option<&Regex>) -> bool {
    match field {
        Field::Title => pattern.is_none_or(|pattern| pattern.is_match(&record.title)),
        Field::Session => pattern.is_none_or(|pattern| pattern.is_match(record.work_ref.session())),
        Field::All => pattern.is_none_or(|pattern| {
            pattern.is_match(&record.title) || pattern.is_match(record.work_ref.session())
        }),
        Field::Content | Field::Input | Field::Output | Field::Command => false,
    }
}

fn text_has_matching_line(text: &str, pattern: &Regex) -> bool {
    text.lines().any(|line| pattern.is_match(line))
}

fn match_excluded(record: &WorkRecord, anchor: &WorkRef, exclude: Option<&Regex>) -> bool {
    let Some(exclude) = exclude else {
        return false;
    };
    match anchor.at {
        WorkAt::Whole => record
            .parts
            .iter()
            .any(|part| text_has_matching_line(&part.text(), exclude)),
        WorkAt::Part(_) => record
            .content_for_at(anchor.at)
            .is_some_and(|text| text_has_matching_line(&text, exclude)),
    }
}

fn dedup_hits(hits: Vec<ScoredHit>) -> Vec<ScoredHit> {
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for hit in hits {
        if seen.insert(hit.anchor.clone()) {
            unique.push(hit);
        }
    }
    unique
}

fn compare_time_desc(index: &RecordIndex, a: &WorkRef, b: &WorkRef) -> std::cmp::Ordering {
    let time = |anchor: &WorkRef| {
        index
            .resolve(anchor)
            .and_then(|record| record.time.primary_at())
    };
    time(b)
        .cmp(&time(a))
        .then_with(|| a.to_string().cmp(&b.to_string()))
}

fn sort_hits(
    hits: &mut [ScoredHit],
    index: &RecordIndex,
    sort: Sort,
    scores: Option<&HashMap<WorkRef, f32>>,
) {
    hits.sort_by(|a, b| {
        let time = |anchor: &WorkRef| {
            index
                .resolve(anchor)
                .and_then(|record| record.time.primary_at())
        };
        match sort {
            Sort::Newest => time(&b.anchor)
                .cmp(&time(&a.anchor))
                .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string())),
            Sort::Oldest => time(&a.anchor)
                .cmp(&time(&b.anchor))
                .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string())),
            Sort::Duration => duration(&b.anchor, index)
                .cmp(&duration(&a.anchor, index))
                .then_with(|| time(&b.anchor).cmp(&time(&a.anchor)))
                .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string())),
            Sort::DurationAsc => duration(&a.anchor, index)
                .cmp(&duration(&b.anchor, index))
                .then_with(|| time(&b.anchor).cmp(&time(&a.anchor)))
                .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string())),
            Sort::ExitCode => exit_code(&b.anchor, index)
                .cmp(&exit_code(&a.anchor, index))
                .then_with(|| time(&b.anchor).cmp(&time(&a.anchor)))
                .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string())),
            Sort::ExitCodeAsc => exit_code(&a.anchor, index)
                .cmp(&exit_code(&b.anchor, index))
                .then_with(|| time(&b.anchor).cmp(&time(&a.anchor)))
                .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string())),
            Sort::Relevance => {
                let score = |anchor: &WorkRef| {
                    scores
                        .and_then(|scores| scores.get(anchor))
                        .copied()
                        .unwrap_or(0.0)
                };
                score(&b.anchor)
                    .total_cmp(&score(&a.anchor))
                    .then_with(|| time(&b.anchor).cmp(&time(&a.anchor)))
                    .then_with(|| a.anchor.to_string().cmp(&b.anchor.to_string()))
            }
        }
    });
}

fn duration(anchor: &WorkRef, index: &RecordIndex) -> Option<u64> {
    index
        .resolve(anchor)
        .and_then(|record| record.time.duration_ms)
}

fn exit_code(anchor: &WorkRef, index: &RecordIndex) -> Option<i32> {
    index
        .resolve(anchor)
        .and_then(|record| record.status.as_ref())
        .and_then(|status| status.exit_code)
}

fn current_agent_session_paths(
    providers: &[AgentProvider],
    cwd: &Path,
) -> Result<HashSet<PathBuf>> {
    let mut paths = HashSet::new();
    for provider in providers {
        let source = provider.session_provider();
        if let Some(path) = current_agent_session_path(source.as_ref(), *provider, cwd)? {
            paths.insert(comparable_path(&path));
        }
    }
    Ok(paths)
}

fn current_agent_session_path(
    source: &dyn AgentSessionProvider,
    provider: AgentProvider,
    cwd: &Path,
) -> Result<Option<PathBuf>> {
    if let Some(path) = current_agent_transcript_path(provider) {
        return Ok(Some(path));
    }
    if let Some(session_id) = current_agent_session_id(provider) {
        if let Some(path) = source.find_session_by_id(&session_id)? {
            return Ok(Some(path));
        }
    }
    source.find_current_session(cwd)
}

fn current_agent_transcript_path(provider: AgentProvider) -> Option<PathBuf> {
    let env_name = provider.current_transcript_env()?;
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn current_agent_session_id(provider: AgentProvider) -> Option<String> {
    let env_name = provider.current_session_id_env()?;
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn excluded_session_matches(record: &WorkRecord, excluded_sessions: &HashSet<PathBuf>) -> bool {
    if excluded_sessions.is_empty() || record.kind != WorkRecordKind::ChatTurn {
        return false;
    }
    record
        .session
        .path
        .as_deref()
        .map(Path::new)
        .map(comparable_path)
        .is_some_and(|path| excluded_sessions.contains(&path))
}

fn comparable_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkSessionRef, WorkSource, WorkStatus,
        WorkTime, RECORD_SCHEMA_VERSION,
    };

    fn record(session: &str, index: usize, title: &str, text: &str) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::terminal(session, index),
            kind: WorkRecordKind::TerminalCommand,
            source: WorkSource {
                channel: WorkChannel::Terminal,
                provider: None,
            },
            session: WorkSessionRef {
                id: session.to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: Some(WorkStatus {
                outcome: WorkOutcome::Success,
                exit_code: Some(0),
            }),
            title: title.to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::Output {
                    content: text.to_string(),
                    ansi: None,
                },
            }],
        }
    }

    fn anchors(records: &[WorkRecord]) -> Vec<WorkRef> {
        records
            .iter()
            .map(|record| record.work_ref.whole())
            .collect()
    }

    #[test]
    fn none_keeps_every_anchor() {
        let records = vec![record("s1", 1, "a", "x"), record("s1", 2, "b", "y")];
        let searcher = Searcher::new(&records);
        let hits = searcher
            .search(&Filter::none(), &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn pattern_matches_per_line_case_insensitively() {
        let records = vec![
            record("s1", 1, "build", "Finished dev profile"),
            record("s1", 2, "deploy", "kubectl rollout"),
        ];
        let filter = Filter {
            pattern: Some("KUBECTL".into()),
            ..Filter::none()
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/2");
    }

    #[test]
    fn latest_window_keeps_newest_records() {
        let mut records = vec![record("s1", 1, "a", "x"), record("s1", 2, "b", "y")];
        records[0].time = WorkTime {
            started_at: Some("2026-01-01T00:00:00Z".into()),
            ended_at: Some("2026-01-01T00:00:01Z".into()),
            duration_ms: Some(1_000),
        };
        records[1].time = WorkTime {
            started_at: Some("2026-01-02T00:00:00Z".into()),
            ended_at: Some("2026-01-02T00:00:01Z".into()),
            duration_ms: Some(1_000),
        };
        let filter = Filter {
            latest: Some(1),
            ..Filter::none()
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/2");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn browse_keeps_metadata_only_records_with_empty_parts() {
        // Light session views (browse meta queries) carry empty `parts`; an
        // unrestricted browse query must still match them, or session lists
        // come back empty once the meta cache is warm.
        let mut meta_only = record("s1", 1, "a", "x");
        meta_only.parts.clear();
        let records = vec![meta_only];
        let hits = Searcher::new(&records)
            .search(
                &Filter::browse_session_page(100),
                &anchors(&records),
                Path::new("."),
            )
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/1");

        // A pattern still never matches an empty-part record.
        let filter = Filter {
            pattern: Some("KUBECTL".into()),
            ..Filter::browse_session_page(100)
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert!(hits.is_empty());
    }

    #[test]
    fn relevance_ranks_matching_records_first() {
        let records = vec![
            record("s1", 1, "sqlite schema", "CREATE TABLE records"),
            record("s1", 2, "rollback", "kubectl rollout undo deploy/web"),
            record("s1", 3, "git log", "7227bd8 refactor"),
        ];
        let filter = Filter {
            sort: Sort::Relevance,
            rank: Some("rollback".into()),
            limit: Some(2),
            ..Filter::none()
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/2");
        assert!(hits[0].score.is_some());
    }

    #[test]
    fn status_and_exit_code_bounds() {
        let mut records = vec![record("s1", 1, "ok", "x"), record("s1", 2, "fail", "y")];
        records[1].status = Some(WorkStatus {
            outcome: WorkOutcome::Failure,
            exit_code: Some(1),
        });
        let filter = Filter {
            status: Some(WorkOutcome::Failure),
            ..Filter::none()
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/2");
    }

    #[test]
    fn exclude_drops_records_containing_pattern() {
        let records = vec![
            record("s1", 1, "build", "Finished dev profile"),
            record("s1", 2, "deploy", "kubectl rollout"),
        ];
        let filter = Filter {
            exclude_regex: Some("kubectl".into()),
            ..Filter::none()
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/1");
    }

    #[test]
    fn parts_mode_emits_one_hit_per_part() {
        let mut records = vec![record("s1", 1, "turn", "hello")];
        records[0].parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::Assistant {
                content: "world".into(),
            },
        });
        let filter = Filter {
            mode: FilterMode::Parts,
            pattern: Some("hello".into()),
            ..Filter::none()
        };
        let hits = Searcher::new(&records)
            .search(&filter, &anchors(&records), Path::new("."))
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].anchor.to_string(), "terminal/s1/1/p1");
    }

    #[test]
    fn content_line_matches_reports_part_and_line() {
        let mut records = [record("s1", 1, "turn", "first line\nneedle here")];
        records[0].parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::Output {
                content: "no match".into(),
                ansi: None,
            },
        });
        let regex = Regex::new("needle").expect("regex");
        let matches = content_line_matches(&records[0], &regex);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].part_seq, 1);
        assert_eq!(matches[0].line, 2);
        assert_eq!(matches[0].text, "needle here");
    }

    #[test]
    fn for_remote_peer_only_clears_exclude_current() {
        let filter = Filter {
            exclude_current: true,
            rank: Some("rollback".into()),
            sort: Sort::Relevance,
            ..Filter::none()
        };
        let peer = filter.for_remote_peer();
        assert!(!peer.exclude_current);
        assert_eq!(peer.rank.as_deref(), Some("rollback"));
        assert_eq!(peer.sort, Sort::Relevance);
    }
}
