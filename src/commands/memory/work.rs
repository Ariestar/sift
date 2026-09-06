use anyhow::{Context, Result};
use serde::Serialize;
use sivtr_core::agents::AgentProvider;
use sivtr_core::record::{WorkPath, WorkRecord};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use crate::cli::{WorkCommand, WorkPartsArgs, WorkRecordsArgs, WorkSessionsArgs};
use crate::commands::memory::filter;
use crate::commands::memory::records::current_work_record_index;
use crate::commands::memory::show;
use crate::commands::memory::work_json::{session_meta, WorkJsonSessionMeta};
use crate::commands::memory::workset;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkSessionSource {
    Terminal,
    Agent(AgentProvider),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkSessionMarker {
    scope: Option<String>,
    source: WorkSessionSource,
    session: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct WorkSessionListItem {
    #[serde(rename = "ref")]
    ref_: String,
    source: String,
    session: String,
    session_meta: WorkJsonSessionMeta,
    timestamp: Option<String>,
    record_count: usize,
    title: String,
}

#[derive(Serialize)]
struct WorkSessionsJsonOutput {
    cwd: String,
    sessions: Vec<WorkSessionListItem>,
}

pub fn execute(command: &WorkCommand) -> Result<()> {
    match &command.action {
        crate::cli::WorkSubcommand::Sessions(args) => execute_sessions(args),
        crate::cli::WorkSubcommand::Records(args) => execute_records(args),
        crate::cli::WorkSubcommand::Parts(args) => execute_parts(args),
    }
}

fn execute_sessions(args: &WorkSessionsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let (display_cwd, records) = if let Some(source) = args.source.as_deref() {
        let set = workset::query(source, filter::Filter::none(), Some(&cwd))?;
        let display_cwd = set.cwd.clone();
        (display_cwd, set.into_records())
    } else {
        let providers = args.provider.providers();
        let records = current_work_record_index(&providers, &cwd, None)?;
        (cwd.display().to_string(), records.records().to_vec())
    };
    let items = build_session_items(&records);

    if args.json {
        let output = WorkSessionsJsonOutput {
            cwd: display_cwd,
            sessions: items,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if items.is_empty() {
        println!("No workspace sessions found");
        return Ok(());
    }

    for item in &items {
        println!("{}", format_session_item(item));
    }
    Ok(())
}

fn execute_records(args: &WorkRecordsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let mut set = workset::query(&args.source, filter::Filter::none(), Some(&cwd))?;
    workset::persist(&mut set, args.save.as_deref()).context("persist record WorkSet")?;
    show::print_workset(
        &mut set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

fn execute_parts(args: &WorkPartsArgs) -> Result<()> {
    let cwd = resolve_cwd(args.cwd.as_deref())?;
    let mut set = workset::query(
        &args.source,
        filter::from_work_parts_args(args)?,
        Some(&cwd),
    )?;
    workset::persist(&mut set, args.save.as_deref()).context("persist part WorkSet")?;
    show::print_workset(
        &mut set,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

fn resolve_cwd(cwd: Option<&Path>) -> Result<std::path::PathBuf> {
    Ok(cwd
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().context("Failed to resolve current directory")?))
}

fn build_session_items(records: &[WorkRecord]) -> Vec<WorkSessionListItem> {
    let mut items: Vec<WorkSessionListItem> = Vec::new();
    let mut positions: HashMap<String, usize> = HashMap::new();

    for record in records {
        let marker = WorkSessionMarker::from_record(record);
        let group_key = session_group_key(record, &marker);
        if let Some(position) = positions.get(&group_key).copied() {
            items[position].record_count += 1;
            continue;
        }
        let display_ref = marker.to_string();
        positions.insert(group_key, items.len());
        items.push(WorkSessionListItem {
            ref_: display_ref,
            source: marker.source_name().to_string(),
            session: marker.session,
            session_meta: session_meta(record),
            timestamp: record.time.primary_at().map(str::to_string),
            record_count: 1,
            title: record.title.clone(),
        });
    }

    items
}

fn session_group_key(record: &WorkRecord, marker: &WorkSessionMarker) -> String {
    let session_id = record
        .session
        .canonical_id
        .as_deref()
        .unwrap_or(marker.session.as_str());
    format!("{marker}/{session_id}")
}

fn format_session_item(item: &WorkSessionListItem) -> String {
    format_marker_line(
        &item.ref_,
        &[
            format!("records {}", item.record_count),
            timestamp_tag(item.timestamp.as_deref()),
        ],
        &item.title,
    )
}

fn format_marker_line(marker: &str, tags: &[String], summary: &str) -> String {
    let mut line = marker.to_string();
    for tag in tags.iter().filter(|tag| !tag.is_empty()) {
        line.push_str("  [");
        line.push_str(tag);
        line.push(']');
    }
    if !summary.trim().is_empty() {
        line.push_str("  ");
        line.push_str(summary);
    }
    line
}

fn timestamp_tag(timestamp: Option<&str>) -> String {
    timestamp.unwrap_or("unknown-time").to_string()
}

impl WorkSessionMarker {
    fn from_record(record: &WorkRecord) -> Self {
        match &record.work_ref.path {
            WorkPath::Terminal { session, .. } => Self {
                scope: record.work_ref.scope_name().map(str::to_string),
                source: WorkSessionSource::Terminal,
                session: session.clone(),
            },
            WorkPath::Agent {
                provider, session, ..
            } => Self {
                scope: record.work_ref.scope_name().map(str::to_string),
                source: WorkSessionSource::Agent(*provider),
                session: session.clone(),
            },
        }
    }

    fn source_name(&self) -> &'static str {
        match self.source {
            WorkSessionSource::Terminal => "terminal",
            WorkSessionSource::Agent(provider) => provider.command_name(),
        }
    }
}

impl fmt::Display for WorkSessionMarker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(scope) = self.scope.as_deref() {
            write!(formatter, "{scope}:")?;
        }
        write!(formatter, "{}/{}", self.source_name(), self.session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::record::{
        MessageRole, WorkChannel, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTime,
    };

    #[test]
    fn session_markers_preserve_scope() {
        let record = test_record(
            "desk:terminal/session_123/1",
            "command",
            Some("2026-05-24T12:00:00Z"),
        );

        assert_eq!(
            build_session_items(&[record])[0].ref_,
            "desk:terminal/session_123"
        );
    }

    #[test]
    fn groups_sessions_in_latest_record_order() {
        let records = vec![
            test_record(
                "codex/alpha/2",
                "latest title",
                Some("2026-05-24T12:00:00Z"),
            ),
            test_record(
                "terminal/shell/1",
                "shell title",
                Some("2026-05-24T11:00:00Z"),
            ),
            test_record("codex/alpha/1", "older title", Some("2026-05-24T10:00:00Z")),
        ];

        let items = build_session_items(&records);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].ref_, "codex/alpha");
        assert_eq!(items[0].record_count, 2);
        assert_eq!(items[0].title, "latest title");
        assert_eq!(items[1].ref_, "terminal/shell");
    }

    #[test]
    fn groups_sessions_with_canonical_session_metadata() {
        let record = test_record("codex/alpha/2", "title", Some("2026-05-24T12:00:00Z"));

        let items = build_session_items(&[record]);

        assert_eq!(items[0].session_meta.display_id, "alpha");
        assert_eq!(
            items[0].session_meta.canonical_id.as_deref(),
            Some("alpha-session-0123456789abcdef")
        );
    }

    fn test_record(reference: &str, title: &str, timestamp: Option<&str>) -> WorkRecord {
        let work_ref: WorkRef = reference.parse().unwrap();
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: work_ref.clone(),
            kind: if matches!(work_ref.path, WorkPath::Terminal { .. }) {
                WorkRecordKind::TerminalCommand
            } else {
                WorkRecordKind::ChatTurn
            },
            source: WorkSource {
                channel: if matches!(work_ref.path, WorkPath::Terminal { .. }) {
                    WorkChannel::Terminal
                } else {
                    WorkChannel::Chat
                },
                provider: work_ref
                    .provider()
                    .map(|provider| provider.command_name().to_string()),
            },
            session: WorkSessionRef {
                id: work_ref.session().to_string(),
                canonical_id: Some(format!("{}-session-0123456789abcdef", work_ref.session())),
                path: None,
            },
            cwd: None,
            time: WorkTime::from_components(timestamp.map(str::to_string), None, None),
            status: None,
            title: title.to_string(),
            parts: vec![
                crate::test_fixtures::message_part(1, MessageRole::User, "user prompt"),
                crate::test_fixtures::message_part(1, MessageRole::Assistant, "assistant reply"),
            ]
            .into_iter()
            .map(|mut part| {
                part.occurred_at = timestamp.map(str::to_string);
                part
            })
            .collect(),
        }
    }
}
