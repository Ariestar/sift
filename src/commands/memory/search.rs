use anyhow::{Context, Result};

use crate::cli::SearchArgs;
use crate::commands::memory::workset::WorkSet;
use crate::commands::memory::{filter, show, workset};

pub fn execute(args: &SearchArgs) -> Result<()> {
    let mut workset = run(args)?;
    show::print_workset(
        &mut workset,
        show::resolve_output_format(args.format, false, args.refs, args.json),
    )
}

/// Unified query for search: local and remote both run load+filter at the data owner.
pub fn run(args: &SearchArgs) -> Result<WorkSet> {
    let mut set = if args.semantic || args.hybrid {
        super::semantic::run(args)?
    } else {
        workset::query(
            &args.source,
            filter::from_search_args(args)?,
            args.cwd.as_deref(),
        )?
    };
    workset::persist(&mut set, args.save.as_deref()).context("persist search WorkSet")?;
    Ok(set)
}
