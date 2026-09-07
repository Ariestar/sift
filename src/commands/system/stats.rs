//! Archive activity and usage analytics.

use anyhow::Result;
use sivtr_core::archive::stats::{StatsQuery, StatsSummary};

use crate::cli::StatsArgs;
use crate::output;

pub fn execute(args: &StatsArgs) -> Result<()> {
    let query = StatsQuery {
        provider: args.window.provider.clone(),
        since: args.window.since.clone(),
        until: args.window.until.clone(),
    };
    query.validate()?;
    let conn = sivtr_core::archive::open()?;
    let skipped = sivtr_core::archive::sync::ensure_fresh_with_conn(&conn)?;
    let report = sivtr_core::archive::stats::compute(&conn, &query)?;
    if args.json {
        let value = serde_json::json!({
            "stats": report,
            "warnings": sivtr_core::usage::warnings_from_sync(&skipped),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    for warning in sivtr_core::usage::warnings_from_sync(&skipped) {
        output::warning(format!(
            "stats skipped {} {}: {}",
            warning.provider, warning.path, warning.error
        ));
    }
    print_report(&report);
    Ok(())
}

fn print_report(report: &StatsSummary) {
    println!(
        "sessions: {} · records: {} · starred: {} · active days: {} · duration: {}",
        report.sessions,
        report.records,
        report.starred_sessions,
        report.active_days,
        format_duration(report.total_duration_ms),
    );
    // A count of findings, never the finding values.
    println!(
        // count only, no secret values
        "outcomes: {} success · {} failure · {} unknown · {} secret findings",
        report.outcomes.success,
        report.outcomes.failure,
        report.outcomes.unknown,
        report.secret_findings,
    );
    println!(
        "usage: {} · {} requests · {} priced · {} unpriced",
        report.usage.totals.cost.format(),
        report.usage.totals.requests,
        report.usage.totals.priced_requests,
        report.usage.totals.unpriced_requests,
    );

    if !report.providers.is_empty() {
        println!("\nproviders");
        for bucket in &report.providers {
            println!(
                "  {:<20} {:>8} sessions {:>8} records",
                bucket.key, bucket.sessions, bucket.records
            );
        }
    }
    if !report.projects.is_empty() {
        println!("\nprojects");
        for bucket in &report.projects {
            println!(
                "  {:<20} {:>8} sessions {:>8} records",
                bucket.key, bucket.sessions, bucket.records
            );
        }
    }
    if !report.days.is_empty() {
        println!("\nactivity");
        for bucket in &report.days {
            println!(
                "  {:<12} {:>8} sessions {:>8} records",
                bucket.key, bucket.sessions, bucket.records
            );
        }
    }
    println!("\nhourly records (UTC)");
    for (hour, count) in report.hourly_records.iter().enumerate() {
        if *count > 0 {
            println!("  {hour:02}:00 {:>8}", count);
        }
    }
}

fn format_duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let hours = seconds / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}
