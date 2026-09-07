//! Token usage and cost reporting over the unified archive.

use std::collections::BTreeMap;

use anyhow::Result;
use sivtr_core::usage::{UsageGroup, UsageQuery, UsageResult, UsageSummary};

use crate::cli::{
    UsageAction, UsageCommand, UsageDailyArgs, UsageSessionArgs, UsageStatuslineArgs,
    UsageWindowArgs,
};
use crate::output;

pub fn execute(command: &UsageCommand) -> Result<()> {
    match &command.action {
        UsageAction::Daily(args) => daily(args),
        UsageAction::Statusline(args) => statusline(args),
        UsageAction::Session(args) => session(args),
    }
}

fn daily(args: &UsageDailyArgs) -> Result<()> {
    let result = read(&args.window, None)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    print_warnings(&result);
    if args.breakdown {
        print_groups(&result.summary);
    } else {
        print_daily_totals(&result.summary);
    }
    print_total(&result.summary);
    Ok(())
}

fn statusline(args: &UsageStatuslineArgs) -> Result<()> {
    let result = read(&args.window, None)?;
    print_warnings(&result);
    let totals = &result.summary.totals;
    let mut line = format!(
        "{} · {} requests · {} input · {} output",
        totals.cost.format(),
        totals.requests,
        compact_tokens(totals.input_tokens),
        compact_tokens(totals.output_tokens),
    );
    if totals.unpriced_requests > 0 {
        line.push_str(&format!(" · {} unpriced", totals.unpriced_requests));
    }
    println!("{line}");
    Ok(())
}

fn session(args: &UsageSessionArgs) -> Result<()> {
    let (provider, session_id) = super::parse_provider_session_address(&args.source)?;
    let result = read(
        &UsageWindowArgs {
            provider: Some(provider.to_string()),
            since: None,
            until: None,
        },
        Some(session_id.to_string()),
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    print_warnings(&result);
    // Echoes the address the user themselves passed on the command line.
    println!("usage for {provider}/{session_id}"); // echoes the CLI argument, not stored secrets
    print_groups(&result.summary);
    print_total(&result.summary);
    Ok(())
}

fn read(window: &UsageWindowArgs, session_id: Option<String>) -> Result<UsageResult> {
    let query = UsageQuery {
        provider: window.provider.clone(),
        session_id,
        since: window.since.clone(),
        until: window.until.clone(),
    };
    query.validate()?;
    let conn = sivtr_core::archive::open()?;
    let skipped = sivtr_core::archive::sync::ensure_fresh_with_conn(&conn)?;
    let summary = sivtr_core::usage::summarize(&conn, &query)?;
    Ok(UsageResult {
        summary,
        warnings: sivtr_core::usage::warnings_from_sync(&skipped),
    })
}

fn print_warnings(result: &UsageResult) {
    for warning in &result.warnings {
        output::warning(format!(
            "usage skipped {} {}: {}",
            warning.provider, warning.path, warning.error
        ));
    }
}

fn print_daily_totals(summary: &UsageSummary) {
    let mut rows: BTreeMap<&str, UsageGroup> = BTreeMap::new();
    for group in &summary.groups {
        let row = rows.entry(&group.day).or_insert_with(|| UsageGroup {
            day: group.day.clone(),
            provider: String::new(),
            model: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            requests: 0,
            priced_requests: 0,
            unpriced_requests: 0,
            cost_microdollars: 0,
            cost: "$0.00".to_string(),
        });
        merge_group(row, group);
    }
    println!(
        "{:<12} {:>10} {:>12} {:>12} {:>12} {:>12}",
        "day", "requests", "input", "output", "cache", "cost"
    );
    for row in rows.values() {
        println!(
            "{:<12} {:>10} {:>12} {:>12} {:>12} {:>12}",
            row.day,
            row.requests,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens
                .saturating_add(row.cache_creation_tokens),
            row.cost
        );
    }
}

fn print_groups(summary: &UsageSummary) {
    println!(
        "{:<12} {:<10} {:<32} {:>9} {:>12} {:>12} {:>12}",
        "day", "provider", "model", "requests", "input", "output", "cost"
    );
    for group in &summary.groups {
        println!(
            "{:<12} {:<10} {:<32} {:>9} {:>12} {:>12} {:>12}",
            group.day,
            group.provider,
            group.model,
            group.requests,
            group.input_tokens,
            group.output_tokens,
            group.cost
        );
    }
}

fn print_total(summary: &UsageSummary) {
    let totals = &summary.totals;
    println!(
        "total: {} · {} requests · {} priced · {} unpriced",
        totals.cost.format(),
        totals.requests,
        totals.priced_requests,
        totals.unpriced_requests
    );
}

fn merge_group(target: &mut UsageGroup, source: &UsageGroup) {
    target.input_tokens = target.input_tokens.saturating_add(source.input_tokens);
    target.output_tokens = target.output_tokens.saturating_add(source.output_tokens);
    target.cache_read_tokens = target
        .cache_read_tokens
        .saturating_add(source.cache_read_tokens);
    target.cache_creation_tokens = target
        .cache_creation_tokens
        .saturating_add(source.cache_creation_tokens);
    target.requests = target.requests.saturating_add(source.requests);
    target.priced_requests = target
        .priced_requests
        .saturating_add(source.priced_requests);
    target.unpriced_requests = target
        .unpriced_requests
        .saturating_add(source.unpriced_requests);
    target.cost_microdollars = target
        .cost_microdollars
        .saturating_add(source.cost_microdollars);
    target.cost = sivtr_core::usage::Money::from_microdollars(target.cost_microdollars).format();
}

fn compact_tokens(tokens: u64) -> String {
    const MILLION: u64 = 1_000_000;
    const THOUSAND: u64 = 1_000;
    if tokens >= MILLION {
        format_decimal(tokens, MILLION, 'M')
    } else if tokens >= THOUSAND {
        format_decimal(tokens, THOUSAND, 'k')
    } else {
        tokens.to_string()
    }
}

fn format_decimal(value: u64, unit: u64, suffix: char) -> String {
    let whole = value / unit;
    let tenth = (value % unit) * 10 / unit;
    if tenth == 0 {
        format!("{whole}{suffix}")
    } else {
        format!("{whole}.{tenth}{suffix}")
    }
}
