//! Copy = resolve(address, dialogues, projection) → filter → clipboard.
//!
//! Grammar: `sivtr copy [address] [dialogues]` with projection sugar `in|out|cmd`.
//! Address uses the same source/ref language as search/show. Default address is
//! the current terminal session; default dialogues is `1` (newest).

pub(crate) mod export;
pub(crate) mod load;
mod plan;
mod project;

pub use export::export_picked;
pub use plan::{parse_address_dialogues, CopyFilters, CopyPlan, Projection};

use anyhow::{Context, Result};
use sivtr_core::agents::AgentProvider;
use sivtr_core::origin::Reach;
use sivtr_core::record::WorkRecord;

use crate::commands::browse;
use crate::output;
use crate::tui::workspace::{
    WorkspaceFocus, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
};

use export::finish_units;
use load::load_for_plan;
use project::project_record;

/// Single entry: execute a fully built plan.
pub fn execute(plan: CopyPlan) -> Result<()> {
    if plan.pick {
        return execute_pick(&plan);
    }

    let Some(loaded) = load_for_plan(&plan)? else {
        return Ok(());
    };

    let prompt = plan.filters.prompt.as_deref();
    let mut units = Vec::new();
    for record in &loaded.records {
        units.push(
            project_record(record, loaded.projection, prompt).context("project record for copy")?,
        );
    }

    finish_units(&units, &plan.filters, &loaded.label).context("finish copy")
}

fn execute_pick(plan: &CopyPlan) -> Result<()> {
    let picked = match plan.address.as_deref() {
        None => {
            // Full workspace browser (same product surface as bare `sivtr`).
            let providers = AgentProvider::all()
                .iter()
                .map(|spec| spec.provider)
                .collect::<Vec<_>>();
            browse::run(&providers, false, WorkspaceFocus::Sessions)
        }
        Some(address) => {
            let expanded = sivtr_core::record::expand_source(address)?;
            let cwd =
                plan.filters.cwd.clone().unwrap_or(
                    std::env::current_dir().context("Failed to resolve current directory")?,
                );
            let records = load::load_dialogues(&expanded, Some(&cwd))?;
            if records.is_empty() {
                output::warning(format!("no records found for `{address}`"));
                return Ok(());
            }
            let source =
                session_source_from_records(&records).unwrap_or_else(WorkspaceSource::terminal);
            let session = WorkspaceSession {
                source: source.clone(),
                session_id: records
                    .first()
                    .map(|r| r.work_ref.session().to_string())
                    .unwrap_or_else(|| expanded.clone()),
                modified: std::time::SystemTime::now(),
                title: expanded.clone(),
                search_title: expanded,
                records,
                body_loaded: true,
            };
            browse::run_with_sessions(source, vec![session], WorkspaceFocus::Dialogues)
        }
    }?;
    crate::commands::finish_picker(
        picked,
        plan.filters.print,
        plan.filters.regex.as_deref(),
        plan.filters.lines.as_deref(),
        plan.filters.ansi,
    )
}

fn session_source_from_records(records: &[WorkRecord]) -> Option<WorkspaceSource> {
    let record = records.first()?;
    let kind = match record.work_ref.provider() {
        Some(provider) => WorkspaceSourceKind::Agent(provider),
        None => WorkspaceSourceKind::Terminal,
    };
    let Some(scope) = record.work_ref.scope_name() else {
        return Some(WorkspaceSource::local(kind));
    };
    // Only a registry-confirmed remote mount renders with the remote glyph;
    // named local aliases (`docs:`) and groups stay on the local style.
    let cwd = std::env::current_dir().ok()?;
    let registry = crate::origins::collect(&cwd).ok()?;
    let entry = registry.resolve(scope).ok()??;
    Some(if matches!(entry.reach, Reach::Remote { .. }) {
        WorkspaceSource::remote(scope, kind)
    } else {
        WorkspaceSource::local(kind)
    })
}

/// Build plan from CLI pieces (projection sugar + free tokens + flags).
pub fn plan_from_cli(
    projection: Projection,
    free_tokens: &[String],
    pick: bool,
    filters: CopyFilters,
) -> Result<CopyPlan, String> {
    let (address, dialogues) = parse_address_dialogues(free_tokens)?;
    Ok(CopyPlan {
        address,
        dialogues,
        projection,
        pick,
        filters,
    })
}
