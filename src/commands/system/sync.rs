//! `sivtr sync`: bring the unified archive up to date from every source.

use anyhow::{Context, Result};

use crate::cli::SyncArgs;
use crate::output;
use sivtr_core::archive::sync::{sync_all, SyncReport};

pub fn execute(args: &SyncArgs) -> Result<()> {
    let report = sync_all(args.full)?;
    if args.json {
        let json =
            serde_json::to_string_pretty(&report).context("Failed to serialize sync report")?;
        println!("{json}");
        return Ok(());
    }
    print_report(&report);
    Ok(())
}

fn print_report(report: &SyncReport) {
    for source in &report.sources {
        let counts = &source.counts;
        let mut line = format!(
            "{}: {} added, {} updated, {} unchanged",
            source.source, counts.added, counts.updated, counts.unchanged
        );
        if counts.failed > 0 {
            line.push_str(&format!(", {} failed", counts.failed));
        }
        output::info(line);
        if let Some(error) = &source.error {
            output::warning(format!("{} listing failed: {error}", source.source));
        }
        for (path, error) in source.failures.iter().take(MAX_REPORTED_FAILURES) {
            output::warning(format!("{}: {}: {error}", source.source, path.display()));
        }
        let hidden = source.failures.len().saturating_sub(MAX_REPORTED_FAILURES);
        if hidden > 0 {
            output::info(format!(
                "{}: {} more failures hidden (use `sivtr sync --json` for the full report)",
                source.source, hidden
            ));
        }
    }
    output::info(format!(
        "archive synced: {} changed in {} ms",
        report.changed(),
        report.duration_ms
    ));
}

const MAX_REPORTED_FAILURES: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_changed_and_failed_aggregate_sources() {
        let report = SyncReport {
            sources: vec![report("codex", 1, 0, 5, 1), report("terminal", 2, 1, 9, 0)],
            duration_ms: 12,
        };
        assert_eq!(report.changed(), 4);
        assert_eq!(report.failed(), 1);
        assert_eq!(report.errors().len(), 0);
    }

    fn report(
        source: &str,
        added: usize,
        updated: usize,
        unchanged: usize,
        failed: usize,
    ) -> sivtr_core::archive::sync::SourceSyncReport {
        sivtr_core::archive::sync::SourceSyncReport {
            source: source.to_string(),
            counts: sivtr_core::archive::sync::SyncCounts {
                added,
                updated,
                unchanged,
                failed,
            },
            error: None,
            failures: Vec::new(),
        }
    }
}
