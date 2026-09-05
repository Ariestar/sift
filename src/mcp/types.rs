use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cli::{FilterArgs, SearchArgs, ShowArgs, ZoomArgs};
use sivtr_core::origin::Origin;
use sivtr_core::record::WorkOutcome;
use sivtr_core::search::{Field, PartKind};

// MCP JSON schema exposes these as strings; serde still uses FromStr aliases.
use crate::commands::memory::show::{self, WorkSetOutputFormat};
use sivtr_core::workset::WorkSet;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Source selector: terminal, agent, pi, desk:terminal, @last, or all:<src>
    /// (e.g. all:agent searches every origin — all local workspaces plus the
    /// current workspace's remote mounts — as one corpus).
    pub source: String,
    /// Plain-text query: BM25 ranks the source by these terms (no regex).
    /// With match_regex present, the regex bounds the set and this query ranks it.
    #[serde(default)]
    pub query: Option<String>,
    /// Use configured embeddings for semantic ranking.
    #[serde(default)]
    pub semantic: Option<bool>,
    /// Fuse BM25 and embedding ranks.
    #[serde(default)]
    pub hybrid: Option<bool>,
    /// Case-insensitive regex content filter (optional refinement)
    #[serde(default)]
    pub match_regex: Option<String>,
    /// Case-insensitive regex exclusion filter
    #[serde(default)]
    pub exclude: Option<String>,
    /// Field to match: content, title, session, input, output, command, all
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub in_field: Option<Field>,
    /// Part kind filter
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub kind: Option<PartKind>,
    /// success | failure | unknown
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub status: Option<WorkOutcome>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub last: Option<String>,
    /// Prefer latest N matches. Defaults to 5 when both latest and limit are omitted (CLI search default).
    #[serde(default)]
    pub latest: Option<usize>,
    /// Cap result anchors (hard ceiling after latest). Prefer latest for "most recent N".
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Save result as @name
    #[serde(default)]
    pub save: Option<String>,
    /// refs | timeline | workset (default timeline)
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub exclude_current: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FilterParams {
    /// Source; defaults to @last (MCP has no stdin pipe)
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub parts: Option<bool>,
    #[serde(default)]
    pub match_regex: Option<String>,
    #[serde(default)]
    pub exclude: Option<String>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub in_field: Option<Field>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub kind: Option<PartKind>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub status: Option<WorkOutcome>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub last: Option<String>,
    #[serde(default)]
    pub latest: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub save: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub exclude_current: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ShowParams {
    /// Ref or WorkSet handle, e.g. terminal/session/3, @last, @failures[1]
    pub source: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// full | timeline | compact | md | refs (default full)
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ZoomParams {
    /// Defaults to @last
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub context: Option<usize>,
    #[serde(default)]
    pub before: Option<usize>,
    #[serde(default)]
    pub after: Option<usize>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub save: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusParams {
    /// Optional workspace directory for mount / origin resolution
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UsageParams {
    /// Optional provider namespace, for example `claude` or `codex`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional exact provider session id.
    #[serde(default)]
    pub session_id: Option<String>,
    /// First UTC day, inclusive (`YYYY-MM-DD`).
    #[serde(default)]
    pub since: Option<String>,
    /// Last UTC day, inclusive (`YYYY-MM-DD`).
    #[serde(default)]
    pub until: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatsParams {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemoryHit {
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    pub time: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemoryResult {
    pub count: usize,
    pub anchors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<MemoryHit>>,
    /// Full WorkSet JSON when detail=workset
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workset: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_as: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ShowResult {
    pub count: usize,
    pub anchors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<MemoryHit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<Vec<ShowContent>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ShowContent {
    #[serde(rename = "ref")]
    pub reference: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StatusResult {
    pub version: String,
    pub config_path: Option<String>,
    pub config_present: bool,
    pub session_dir: Option<String>,
    pub session_dir_present: bool,
    pub shell_hooks_installed: bool,
    pub providers: Vec<ProviderStatus>,
    pub daemon_running: bool,
    pub daemon_node_id: Option<String>,
    /// Every addressable memory source through the unified [`Origin`] shape.
    pub origins: Vec<Origin>,
    pub vars: Vec<VarStatus>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProviderStatus {
    pub name: String,
    pub sessions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VarStatus {
    pub name: String,
    pub items: usize,
    pub created_at: String,
}

pub fn to_search_args(params: &SearchParams) -> Result<SearchArgs, String> {
    Ok(SearchArgs {
        source: params.source.clone(),
        query: params.query.clone(),
        semantic: params.semantic.unwrap_or(false),
        hybrid: params.hybrid.unwrap_or(false),
        match_: params.match_regex.clone(),
        exclude: params.exclude.clone(),
        in_field: params.in_field.unwrap_or_default(),
        kind: params.kind,
        status: params.status,
        exit_code: params.exit_code,
        min_duration: None,
        max_duration: None,
        sort: None,
        cwd: params.cwd.as_deref().map(PathBuf::from),
        since: params.since.clone(),
        until: params.until.clone(),
        last: params.last.clone(),
        latest: params.latest,
        limit: params.limit,
        exclude_current: params.exclude_current.unwrap_or(false),
        format: None,
        json: false,
        refs: false,
        save: params.save.clone(),
    })
}

pub fn to_filter_args(params: &FilterParams) -> Result<FilterArgs, String> {
    Ok(FilterArgs {
        source: params.source.clone().unwrap_or_else(|| "@last".to_string()),
        parts: params.parts.unwrap_or(false),
        match_: params.match_regex.clone(),
        exclude: params.exclude.clone(),
        in_field: params.in_field.unwrap_or_default(),
        kind: params.kind,
        status: params.status,
        exit_code: params.exit_code,
        min_duration: None,
        max_duration: None,
        sort: None,
        cwd: params.cwd.as_deref().map(PathBuf::from),
        since: params.since.clone(),
        until: params.until.clone(),
        last: params.last.clone(),
        latest: params.latest,
        limit: params.limit,
        exclude_current: params.exclude_current.unwrap_or(false),
        format: None,
        json: false,
        refs: false,
        save: params.save.clone(),
    })
}

pub fn to_show_args(params: &ShowParams) -> Result<ShowArgs, String> {
    let mode = params.mode.as_deref().unwrap_or("full");
    let format = match mode {
        "full" => Some(WorkSetOutputFormat::Full),
        "timeline" => Some(WorkSetOutputFormat::Timeline),
        "compact" => Some(WorkSetOutputFormat::Compact),
        "md" | "markdown" => Some(WorkSetOutputFormat::Md),
        "refs" => Some(WorkSetOutputFormat::Refs),
        "workset" | "json" => Some(WorkSetOutputFormat::WorkSet),
        other => {
            return Err(format!(
            "unknown show mode `{other}`; expected full, timeline, compact, md, refs, or workset"
        ))
        }
    };
    Ok(ShowArgs {
        source: params.source.clone(),
        cwd: params.cwd.as_deref().map(PathBuf::from),
        format,
        full: false,
        refs: false,
        json: false,
    })
}

pub fn to_zoom_args(params: &ZoomParams) -> ZoomArgs {
    ZoomArgs {
        source: params.source.clone().unwrap_or_else(|| "@last".to_string()),
        context: params.context,
        before: params.before,
        after: params.after,
        cwd: params.cwd.as_deref().map(PathBuf::from),
        format: None,
        json: false,
        refs: false,
        save: params.save.clone(),
    }
}

pub fn memory_result(
    set: &WorkSet,
    detail: Option<&str>,
    saved_as: Option<String>,
) -> anyhow::Result<MemoryResult> {
    let detail = detail.unwrap_or("timeline");
    let anchors = set
        .anchors()
        .iter()
        .map(|anchor| anchor.to_string())
        .collect::<Vec<_>>();
    let count = anchors.len();

    let (items, workset) = match detail {
        "refs" => (None, None),
        "workset" | "json" => (
            None,
            Some(serde_json::to_value(set).map_err(|error| anyhow::anyhow!(error))?),
        ),
        _ => {
            let items = show::render_summary_items(set)?
                .into_iter()
                .map(|item| MemoryHit {
                    reference: item.reference,
                    title: item.title,
                    time: item.time,
                    source: item.source,
                    status: item.status,
                    snippet: item.snippet,
                })
                .collect();
            (Some(items), None)
        }
    };

    Ok(MemoryResult {
        count,
        anchors,
        items,
        workset,
        saved_as,
    })
}

pub fn show_result(set: &WorkSet, mode: Option<&str>) -> anyhow::Result<ShowResult> {
    let mode = mode.unwrap_or("full");
    let anchors = set
        .anchors()
        .iter()
        .map(|anchor| anchor.to_string())
        .collect::<Vec<_>>();
    let count = anchors.len();

    match mode {
        "refs" => Ok(ShowResult {
            count,
            anchors,
            items: None,
            contents: None,
        }),
        "timeline" | "compact" | "md" => {
            let items = show::render_summary_items(set)?
                .into_iter()
                .map(|item| MemoryHit {
                    reference: item.reference,
                    title: item.title,
                    time: item.time,
                    source: item.source,
                    status: item.status,
                    snippet: item.snippet,
                })
                .collect();
            Ok(ShowResult {
                count,
                anchors,
                items: Some(items),
                contents: None,
            })
        }
        _ => {
            let contents = show::render_full_items(set)?
                .into_iter()
                .map(|(reference, content)| ShowContent {
                    reference: reference.to_string(),
                    content,
                })
                .collect();
            Ok(ShowResult {
                count,
                anchors,
                items: None,
                contents: Some(contents),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_search_params() {
        let args = to_search_args(&SearchParams {
            source: "terminal".into(),
            query: Some("panic".into()),
            semantic: None,
            hybrid: None,
            match_regex: Some("panic".into()),
            exclude: None,
            in_field: Some(Field::Content),
            kind: None,
            status: Some(WorkOutcome::Failure),
            exit_code: Some(1),
            since: None,
            until: None,
            last: Some("2h".into()),
            latest: Some(5),
            limit: None,
            cwd: None,
            save: Some("failures".into()),
            detail: Some("timeline".into()),
            exclude_current: Some(true),
        })
        .expect("map");
        assert_eq!(args.source, "terminal");
        assert_eq!(args.match_.as_deref(), Some("panic"));
        assert_eq!(args.status, Some(WorkOutcome::Failure));
        assert_eq!(args.latest, Some(5));
        assert!(args.exclude_current);
        assert_eq!(args.save.as_deref(), Some("failures"));
    }

    #[test]
    fn search_passes_unbounded_params_to_cli_defaults() {
        // CLI search applies latest=5 when both are None; MCP just forwards.
        let args = to_search_args(&SearchParams {
            source: "terminal".into(),
            query: None,
            semantic: None,
            hybrid: None,
            match_regex: None,
            exclude: None,
            in_field: None,
            kind: None,
            status: None,
            exit_code: None,
            since: None,
            until: None,
            last: None,
            latest: None,
            limit: None,
            cwd: None,
            save: None,
            detail: None,
            exclude_current: None,
        })
        .expect("map");
        assert_eq!(args.latest, None);
        assert_eq!(args.limit, None);
    }

    #[test]
    fn filter_defaults_to_last() {
        let args = to_filter_args(&FilterParams {
            source: None,
            parts: None,
            match_regex: None,
            exclude: None,
            in_field: None,
            kind: None,
            status: None,
            exit_code: None,
            since: None,
            until: None,
            last: None,
            latest: None,
            limit: None,
            cwd: None,
            save: None,
            detail: None,
            exclude_current: None,
        })
        .expect("map");
        assert_eq!(args.source, "@last");
    }
}
