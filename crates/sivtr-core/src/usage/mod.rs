//! Token usage and cost tracking over the unified archive.
//!
//! Providers whose transcripts carry per-message usage (Claude, Codex)
//! are walked at sync time and folded into the archive's `usage_events`
//! table (raw token counts only — costs are computed at read time from
//! the embedded pricing snapshot, so a pricing refresh re-prices history
//! without touching stored events).

pub mod extract;
pub mod pricing;

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::Connection;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

/// Integer cost in microdollars (µ$): 1_000_000 µ$ = $1. All arithmetic is
/// exact integer math on µ$/token rates, so totals never accumulate float
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct Money {
    pub microdollars: i64,
}

impl Money {
    pub fn zero() -> Self {
        Self { microdollars: 0 }
    }

    pub fn from_microdollars(microdollars: i64) -> Self {
        Self { microdollars }
    }

    pub fn add(&mut self, other: Money) {
        self.microdollars = self.microdollars.saturating_add(other.microdollars);
    }

    /// Whole dollars portion (`$3.25` -> `3`).
    pub fn dollars(self) -> i64 {
        self.microdollars.div_euclid(1_000_000)
    }

    /// Fractional remainder in hundredths of a dollar (`$3.25` -> `25`).
    pub fn cents(self) -> i64 {
        self.microdollars.rem_euclid(1_000_000) / 10_000
    }

    /// `$3.25`, `$0.42`, `$1,204.00`-style formatting (no grouping here).
    pub fn format(self) -> String {
        let sign = if self.microdollars < 0 { "-" } else { "" };
        let abs = self.microdollars.unsigned_abs();
        format!(
            "${sign}{}.{:02}",
            abs / 1_000_000,
            (abs % 1_000_000) / 10_000
        )
    }
}

/// Raw per-call token usage as recorded in a provider transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEvent {
    /// Model that served the call, when the transcript names it (empty
    /// means unknown — the event still counts tokens, just unpriced).
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    /// Call timestamp from the transcript, when present (RFC3339).
    pub occurred_at: Option<String>,
    /// Stable per-call identity for dedup across re-syncs; providers
    /// without a native id get a content-derived key.
    pub dedup_key: String,
}

impl UsageEvent {
    /// Billable token totals for cost math, normalized so `input_tokens`
    /// excludes cache reads (the cached portion is billed separately).
    pub fn billed_tokens(&self) -> (u64, u64, u64, u64) {
        (
            self.input_tokens,
            self.output_tokens,
            self.cache_read_tokens,
            self.cache_creation_tokens,
        )
    }
}

/// Query bounds shared by the CLI, Web API, and MCP tool.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageQuery {
    pub provider: Option<String>,
    pub session_id: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

impl UsageQuery {
    pub fn validate(&self) -> Result<()> {
        for (flag, value) in [
            ("since", self.since.as_deref()),
            ("until", self.until.as_deref()),
        ] {
            if let Some(value) = value {
                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map_err(|error| anyhow::anyhow!("{flag} must be YYYY-MM-DD: {error}"))?;
            }
        }
        if let (Some(since), Some(until)) = (self.since.as_deref(), self.until.as_deref()) {
            let since = chrono::NaiveDate::parse_from_str(since, "%Y-%m-%d")?;
            let until = chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d")?;
            if since > until {
                anyhow::bail!("since must be on or before until");
            }
        }
        if self
            .provider
            .as_deref()
            .is_some_and(|provider| provider.trim().is_empty())
        {
            anyhow::bail!("provider must not be empty");
        }
        if self
            .session_id
            .as_deref()
            .is_some_and(|session| session.trim().is_empty() || session.contains('/'))
        {
            anyhow::bail!("session_id must be a non-empty session id without `/`");
        }
        Ok(())
    }
}

/// Totals over a set of usage events.
#[derive(Debug, Clone, Default)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub requests: u64,
    pub priced_requests: u64,
    pub unpriced_requests: u64,
    pub unpriced_input_tokens: u64,
    pub unpriced_output_tokens: u64,
    pub unpriced_cache_read_tokens: u64,
    pub unpriced_cache_creation_tokens: u64,
    pub cost: Money,
}

impl Serialize for UsageTotals {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("UsageTotals", 13)?;
        state.serialize_field("input_tokens", &self.input_tokens)?;
        state.serialize_field("output_tokens", &self.output_tokens)?;
        state.serialize_field("cache_read_tokens", &self.cache_read_tokens)?;
        state.serialize_field("cache_creation_tokens", &self.cache_creation_tokens)?;
        state.serialize_field("requests", &self.requests)?;
        state.serialize_field("priced_requests", &self.priced_requests)?;
        state.serialize_field("unpriced_requests", &self.unpriced_requests)?;
        state.serialize_field("unpriced_input_tokens", &self.unpriced_input_tokens)?;
        state.serialize_field("unpriced_output_tokens", &self.unpriced_output_tokens)?;
        state.serialize_field(
            "unpriced_cache_read_tokens",
            &self.unpriced_cache_read_tokens,
        )?;
        state.serialize_field(
            "unpriced_cache_creation_tokens",
            &self.unpriced_cache_creation_tokens,
        )?;
        state.serialize_field("cost_microdollars", &self.cost.microdollars)?;
        state.serialize_field("cost", &self.cost.format())?;
        state.end()
    }
}

impl UsageTotals {
    pub fn add_event(&mut self, event: &UsageEvent, cost: Option<Money>) {
        self.input_tokens = self.input_tokens.saturating_add(event.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(event.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(event.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(event.cache_creation_tokens);
        self.requests = self.requests.saturating_add(1);
        match cost {
            Some(cost) => {
                self.priced_requests = self.priced_requests.saturating_add(1);
                self.cost.add(cost);
            }
            None => {
                self.unpriced_requests = self.unpriced_requests.saturating_add(1);
                self.unpriced_input_tokens = self
                    .unpriced_input_tokens
                    .saturating_add(event.input_tokens);
                self.unpriced_output_tokens = self
                    .unpriced_output_tokens
                    .saturating_add(event.output_tokens);
                self.unpriced_cache_read_tokens = self
                    .unpriced_cache_read_tokens
                    .saturating_add(event.cache_read_tokens);
                self.unpriced_cache_creation_tokens = self
                    .unpriced_cache_creation_tokens
                    .saturating_add(event.cache_creation_tokens);
            }
        }
    }
}

/// One deterministic daily/provider/model usage bucket.
#[derive(Debug, Clone, Serialize)]
pub struct UsageGroup {
    /// UTC day of the call (`YYYY-MM-DD`), or `unknown` when no valid
    /// timestamp was present in the source event.
    pub day: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub requests: u64,
    pub priced_requests: u64,
    pub unpriced_requests: u64,
    pub cost_microdollars: i64,
    pub cost: String,
}

/// Complete usage result returned by every interface.
#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub totals: UsageTotals,
    pub groups: Vec<UsageGroup>,
}

/// A source that could not be refreshed before a usage query. Keeping this
/// alongside the result prevents an apparently complete report from hiding
/// missing transcript data.
#[derive(Debug, Clone, Serialize)]
pub struct UsageWarning {
    pub provider: String,
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageResult {
    pub summary: UsageSummary,
    pub warnings: Vec<UsageWarning>,
}

pub fn warnings_from_sync(skipped: &[crate::query::SkippedSession]) -> Vec<UsageWarning> {
    skipped
        .iter()
        .map(|entry| UsageWarning {
            provider: entry.namespace.clone(),
            path: entry.path.display().to_string(),
            error: entry.error.clone(),
        })
        .collect()
}

/// Load and price raw archive events. The archive remains the sole source of
/// token data; the pricing catalog is the sole cost authority.
pub fn summarize(conn: &Connection, query: &UsageQuery) -> Result<UsageSummary> {
    let events = crate::archive::store::load_usage_events(
        conn,
        query.provider.as_deref(),
        query.session_id.as_deref(),
        query.since.as_deref(),
        query.until.as_deref(),
    )?;
    Ok(summarize_events(&events))
}

fn summarize_events(events: &[crate::archive::store::StoredUsageEvent]) -> UsageSummary {
    let mut totals = UsageTotals::default();
    let mut groups: BTreeMap<(String, String, String), UsageGroup> = BTreeMap::new();

    for stored in events {
        let event = UsageEvent {
            model: stored.model.clone(),
            input_tokens: stored.input_tokens,
            output_tokens: stored.output_tokens,
            cache_read_tokens: stored.cache_read_tokens,
            cache_creation_tokens: stored.cache_creation_tokens,
            occurred_at: stored.occurred_at.clone(),
            dedup_key: stored.dedup_key.clone(),
        };
        let cost = pricing::catalog()
            .lookup(&event.model)
            .and_then(|pricing| pricing::cost(&event, pricing.rate()));
        totals.add_event(&event, cost);

        let day = event_day(event.occurred_at.as_deref());
        let key = (day.clone(), stored.provider.clone(), event.model.clone());
        let group = groups.entry(key).or_insert_with(|| UsageGroup {
            day,
            provider: stored.provider.clone(),
            model: event.model.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            requests: 0,
            priced_requests: 0,
            unpriced_requests: 0,
            cost_microdollars: 0,
            cost: Money::zero().format(),
        });
        group.input_tokens = group.input_tokens.saturating_add(event.input_tokens);
        group.output_tokens = group.output_tokens.saturating_add(event.output_tokens);
        group.cache_read_tokens = group
            .cache_read_tokens
            .saturating_add(event.cache_read_tokens);
        group.cache_creation_tokens = group
            .cache_creation_tokens
            .saturating_add(event.cache_creation_tokens);
        group.requests = group.requests.saturating_add(1);
        match cost {
            Some(cost) => {
                group.priced_requests = group.priced_requests.saturating_add(1);
                group.cost_microdollars = group.cost_microdollars.saturating_add(cost.microdollars);
                group.cost = Money::from_microdollars(group.cost_microdollars).format();
            }
            None => group.unpriced_requests = group.unpriced_requests.saturating_add(1),
        }
    }

    UsageSummary {
        totals,
        groups: groups.into_values().collect(),
    }
}

fn event_day(timestamp: Option<&str>) -> String {
    timestamp
        .and_then(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|time| time.with_timezone(&chrono::Utc).date_naive().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_formats_dollars_and_cents() {
        assert_eq!(Money::from_microdollars(0).format(), "$0.00");
        assert_eq!(Money::from_microdollars(3_250_000).format(), "$3.25");
        assert_eq!(Money::from_microdollars(420_000).format(), "$0.42");
        assert_eq!(Money::from_microdollars(1_204_000_000).format(), "$1204.00");
        assert_eq!(Money::from_microdollars(-1_500_000).format(), "$-1.50");
    }

    #[test]
    fn money_add_saturates() {
        let mut total = Money::from_microdollars(i64::MAX - 1);
        total.add(Money::from_microdollars(10));
        assert_eq!(total.microdollars, i64::MAX);
    }

    #[test]
    fn event_without_a_price_is_reported_as_unpriced() {
        let event = UsageEvent {
            model: "missing".into(),
            input_tokens: 4,
            output_tokens: 5,
            cache_read_tokens: 6,
            cache_creation_tokens: 7,
            occurred_at: None,
            dedup_key: "event".into(),
        };
        let mut totals = UsageTotals::default();
        totals.add_event(&event, None);
        assert_eq!(totals.requests, 1);
        assert_eq!(totals.priced_requests, 0);
        assert_eq!(totals.unpriced_requests, 1);
        assert_eq!(totals.unpriced_cache_read_tokens, 6);
    }

    #[test]
    fn summary_groups_by_utc_day_provider_and_model() {
        let events = vec![
            crate::archive::store::StoredUsageEvent {
                provider: "codex".into(),
                session_id: "s1".into(),
                model: "missing-model".into(),
                input_tokens: 2,
                output_tokens: 3,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                occurred_at: Some("2026-08-31T00:30:00+02:00".into()),
                dedup_key: "a".into(),
            },
            crate::archive::store::StoredUsageEvent {
                provider: "codex".into(),
                session_id: "s2".into(),
                model: "missing-model".into(),
                input_tokens: 5,
                output_tokens: 7,
                cache_read_tokens: 11,
                cache_creation_tokens: 13,
                occurred_at: Some("2026-08-31T20:30:00Z".into()),
                dedup_key: "b".into(),
            },
        ];
        let summary = summarize_events(&events);
        assert_eq!(summary.groups.len(), 2);
        assert_eq!(summary.groups[0].day, "2026-08-30");
        assert_eq!(summary.groups[1].day, "2026-08-31");
        assert_eq!(summary.totals.requests, 2);
        assert_eq!(summary.totals.unpriced_requests, 2);
        assert_eq!(summary.totals.unpriced_cache_creation_tokens, 13);
    }

    #[test]
    fn dollars_and_cents_split() {
        let money = Money::from_microdollars(3_250_000);
        assert_eq!(money.dollars(), 3);
        assert_eq!(money.cents(), 25);
    }
}
