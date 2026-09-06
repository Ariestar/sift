use std::io::IsTerminal;
use std::str::FromStr;

use anyhow::{Context, Result};
use sivtr_core::record::{WorkAt, WorkPart, WorkRecord, WorkRef};

use crate::cli::ShowArgs;
use crate::commands::memory::workset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkSetOutputFormat {
    Full,
    Compact,
    Timeline,
    Md,
    Refs,
    WorkSet,
}

impl FromStr for WorkSetOutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "full" => Ok(Self::Full),
            "compact" => Ok(Self::Compact),
            "timeline" => Ok(Self::Timeline),
            "md" | "markdown" => Ok(Self::Md),
            "refs" => Ok(Self::Refs),
            "workset" | "json" => Ok(Self::WorkSet),
            _ => Err(format!(
                "unknown show output format `{value}`; expected full, timeline, compact, md, refs, or workset"
            )),
        }
    }
}

impl std::fmt::Display for WorkSetOutputFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Timeline => "timeline",
            Self::Md => "md",
            Self::Refs => "refs",
            Self::WorkSet => "workset",
        })
    }
}

struct AnchorItem<'a> {
    anchor: WorkRef,
    record: &'a WorkRecord,
}

struct ShowItem {
    ref_: WorkRef,
    content: String,
}

pub fn default_output_format() -> WorkSetOutputFormat {
    if std::io::stdout().is_terminal() {
        WorkSetOutputFormat::Full
    } else {
        WorkSetOutputFormat::WorkSet
    }
}

pub fn resolve_output_format(
    format: Option<WorkSetOutputFormat>,
    full: bool,
    refs: bool,
    json: bool,
) -> WorkSetOutputFormat {
    if full {
        WorkSetOutputFormat::Full
    } else if refs {
        WorkSetOutputFormat::Refs
    } else if json {
        WorkSetOutputFormat::WorkSet
    } else {
        format.unwrap_or_else(default_output_format)
    }
}

pub fn execute(args: &ShowArgs) -> Result<()> {
    let mut set = run(args)?;
    print_workset(
        &mut set,
        resolve_output_format(args.format, args.full, args.refs, args.json),
    )
}

/// Resolve a show source into a WorkSet without printing.
pub fn run(args: &ShowArgs) -> Result<workset::WorkSet> {
    workset::query(
        &args.source,
        crate::commands::memory::filter::Filter::none(),
        args.cwd.as_deref(),
    )
}

/// Render a WorkSet. Content formats (everything but bare refs) materialize
/// part text first, so a light-loaded set renders fully.
pub fn print_workset(set: &mut workset::WorkSet, format: WorkSetOutputFormat) -> Result<()> {
    if format != WorkSetOutputFormat::Refs {
        set.materialize_parts()?;
    }
    match format {
        WorkSetOutputFormat::Full => print_full(set)?,
        WorkSetOutputFormat::WorkSet => {
            println!("{}", serde_json::to_string_pretty(set)?);
        }
        WorkSetOutputFormat::Compact => print_compact(set)?,
        WorkSetOutputFormat::Timeline => print_timeline(set)?,
        WorkSetOutputFormat::Md => print_markdown(set)?,
        WorkSetOutputFormat::Refs => print_refs(set),
    }
    Ok(())
}

fn anchor_items(set: &workset::WorkSet) -> Result<Vec<AnchorItem<'_>>> {
    set.anchors()
        .iter()
        .map(|anchor| {
            let record = workset::find_record([set.records()], anchor)
                .with_context(|| format!("No record found for anchor `{anchor}`"))?;
            Ok(AnchorItem {
                anchor: anchor.clone(),
                record,
            })
        })
        .collect()
}

fn print_full(set: &workset::WorkSet) -> Result<()> {
    let items = render_full_items(set)?
        .into_iter()
        .map(|(ref_, content)| ShowItem { ref_, content })
        .collect();
    print_show_items(items);
    Ok(())
}

/// Collect full content for each anchor without printing.
pub fn render_full_items(set: &workset::WorkSet) -> Result<Vec<(WorkRef, String)>> {
    anchor_items(set)?
        .into_iter()
        .map(|item| {
            let content = item
                .record
                .content_for_at(item.anchor.at)
                .with_context(|| format!("No content found for ref `{}`", item.anchor))?;
            Ok((item.anchor, content))
        })
        .collect()
}

/// Compact summary rows for agent-facing responses.
pub fn render_summary_items(set: &workset::WorkSet) -> Result<Vec<SummaryItem>> {
    anchor_items(set)?
        .into_iter()
        .map(|item| {
            Ok(SummaryItem {
                reference: item.anchor.to_string(),
                title: item.record.title.clone(),
                time: item
                    .record
                    .time
                    .primary_at()
                    .map(str::to_string)
                    .unwrap_or_default(),
                source: source_label(item.record).to_string(),
                status: item.record.status.as_ref().map(|status| {
                    match status.outcome {
                        sivtr_core::record::WorkOutcome::Success => "success",
                        sivtr_core::record::WorkOutcome::Failure => "failure",
                        sivtr_core::record::WorkOutcome::Unknown => "unknown",
                    }
                    .to_string()
                }),
                snippet: anchor_summary(item.record, &item.anchor),
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct SummaryItem {
    pub reference: String,
    pub title: String,
    pub time: String,
    pub source: String,
    pub status: Option<String>,
    pub snippet: String,
}

fn print_show_items(items: Vec<ShowItem>) {
    let multi = items.len() > 1;
    for item in items {
        if multi {
            println!("--- {} ---", item.ref_);
        }
        print!("{}", item.content);
        if !item.content.ends_with('\n') {
            println!();
        }
    }
}

fn print_compact(set: &workset::WorkSet) -> Result<()> {
    for item in anchor_items(set)? {
        println!(
            "{}  {:<8}  {}",
            short_time(item.record),
            source_label(item.record),
            anchor_summary(item.record, &item.anchor)
        );
    }
    Ok(())
}

fn print_timeline(set: &workset::WorkSet) -> Result<()> {
    let mut previous_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;
    for item in anchor_items(set)? {
        let timestamp =
            anchor_timestamp(item.record, &item.anchor).and_then(sivtr_core::time::parse_timestamp);
        if let (Some(previous), Some(current)) = (previous_timestamp, timestamp) {
            let gap_minutes = (current - previous).num_minutes();
            if gap_minutes >= 15 {
                println!("          -- gap {gap_minutes}m --");
            }
        }
        if timestamp.is_some() {
            previous_timestamp = timestamp;
        }

        println!(
            "{}  {:<8}  {:<28}  {}",
            short_time(item.record),
            source_label(item.record),
            item.anchor,
            anchor_summary(item.record, &item.anchor)
        );
    }
    Ok(())
}

fn print_markdown(set: &workset::WorkSet) -> Result<()> {
    for item in anchor_items(set)? {
        println!(
            "- **{}** `{}` {}",
            short_time(item.record),
            item.anchor,
            escape_markdown_title(&anchor_summary(item.record, &item.anchor))
        );
    }
    Ok(())
}

fn print_refs(set: &workset::WorkSet) {
    for anchor in set.anchors().iter() {
        println!("{anchor}");
    }
}

fn anchor_summary(record: &WorkRecord, anchor: &WorkRef) -> String {
    match anchor.at {
        WorkAt::Whole => record.title.clone(),
        WorkAt::Part(_) => record
            .part_for_at(anchor.at)
            .map(part_summary)
            .unwrap_or_else(|| record.title.clone()),
    }
}

fn part_summary(part: &WorkPart) -> String {
    let label = part
        .label()
        .filter(|label| !label.trim().is_empty())
        .map(|label| format!("{} ", summary_text(label)))
        .unwrap_or_default();
    format!("{}{}", label, summary_text(&part.text()))
}

fn anchor_timestamp<'a>(record: &'a WorkRecord, anchor: &WorkRef) -> Option<&'a str> {
    match anchor.at {
        WorkAt::Part(_) => record
            .part_for_at(anchor.at)
            .and_then(|part| part.occurred_at.as_deref())
            .or_else(|| record.time.primary_at()),
        WorkAt::Whole => record.time.primary_at(),
    }
}

fn summary_text(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    let summary = trimmed.chars().take(96).collect::<String>();
    if trimmed.chars().count() > 96 {
        format!("{summary}...")
    } else {
        summary
    }
}

fn short_time(record: &WorkRecord) -> String {
    record
        .time
        .primary_at()
        .and_then(sivtr_core::time::parse_timestamp)
        .map(|timestamp| {
            timestamp
                .with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "--:--:--".to_string())
}

fn source_label(record: &WorkRecord) -> &'static str {
    record.work_ref.path.namespace()
}

fn escape_markdown_title(title: &str) -> String {
    title.replace('[', "\\[").replace(']', "\\]")
}
