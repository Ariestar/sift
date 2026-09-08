//! Archive analytics used by the CLI, Web API, and MCP.

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatsQuery {
    pub provider: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

impl StatsQuery {
    pub fn validate(&self) -> Result<()> {
        crate::usage::UsageQuery {
            provider: self.provider.clone(),
            session_id: None,
            since: self.since.clone(),
            until: self.until.clone(),
        }
        .validate()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsBucket {
    pub key: String,
    pub sessions: u64,
    pub records: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeStats {
    pub success: u64,
    pub failure: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSummary {
    pub sessions: u64,
    pub records: u64,
    pub starred_sessions: u64,
    pub active_days: u64,
    pub total_duration_ms: u64,
    pub providers: Vec<StatsBucket>,
    pub projects: Vec<StatsBucket>,
    pub days: Vec<StatsBucket>,
    pub hourly_records: [u64; 24],
    pub outcomes: OutcomeStats,
    pub secret_findings: u64,
    pub usage: crate::usage::UsageSummary,
}

pub fn compute(conn: &Connection, query: &StatsQuery) -> Result<StatsSummary> {
    query.validate()?;
    let (sessions, records, starred_sessions, active_days) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(record_count), 0),
                COALESCE(SUM(starred), 0),
                COUNT(DISTINCT date(COALESCE(ended_at, started_at)))
         FROM sessions
         WHERE (?1 IS NULL OR provider = ?1)
           AND (?2 IS NULL OR date(COALESCE(ended_at, started_at)) >= date(?2))
           AND (?3 IS NULL OR date(COALESCE(ended_at, started_at)) <= date(?3))",
        params![query.provider, query.since, query.until],
        |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
            ))
        },
    )?;

    let providers = grouped_buckets(conn, query, "provider")?;
    let projects = grouped_buckets(
        conn,
        query,
        "CASE WHEN project = '' THEN 'unassigned' ELSE project END",
    )?;
    let days = grouped_buckets(
        conn,
        query,
        "COALESCE(date(ended_at), date(started_at), 'unknown')",
    )?;

    let mut hourly_records = [0_u64; 24];
    let mut total_duration_ms = 0_u64;
    let mut outcomes = OutcomeStats {
        success: 0,
        failure: 0,
        unknown: 0,
    };
    let mut stmt = conn.prepare(
        "SELECT r.ended_at, r.outcome, r.blob
         FROM records r JOIN sessions s ON s.id = r.session_row
         WHERE (?1 IS NULL OR s.provider = ?1)
           AND (?2 IS NULL OR date(COALESCE(s.ended_at, s.started_at)) >= date(?2))
           AND (?3 IS NULL OR date(COALESCE(s.ended_at, s.started_at)) <= date(?3))",
    )?;
    let rows = stmt.query_map(params![query.provider, query.since, query.until], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for row in rows {
        let (ended_at, outcome, blob) = row?;
        let record: crate::record::WorkRecord = rmp_serde::from_slice(&blob)
            .context("failed to decode record while computing stats")?;
        total_duration_ms = total_duration_ms.saturating_add(record.time.duration_ms.unwrap_or(0));
        match outcome.as_deref() {
            Some("success") => outcomes.success = outcomes.success.saturating_add(1),
            Some("failure") => outcomes.failure = outcomes.failure.saturating_add(1),
            _ => outcomes.unknown = outcomes.unknown.saturating_add(1),
        }
        if let Some(hour) = ended_at
            .as_deref()
            .and_then(crate::time::parse_timestamp)
            .map(|time| time.hour() as usize)
        {
            hourly_records[hour] = hourly_records[hour].saturating_add(1);
        }
    }

    let secret_findings = conn.query_row(
        "SELECT COALESCE(SUM(f.occurrences), 0)
         FROM secret_findings f JOIN sessions s ON s.id = f.session_row
         WHERE (?1 IS NULL OR s.provider = ?1)
           AND (?2 IS NULL OR date(COALESCE(s.ended_at, s.started_at)) >= date(?2))
           AND (?3 IS NULL OR date(COALESCE(s.ended_at, s.started_at)) <= date(?3))",
        params![query.provider, query.since, query.until],
        |row| row.get::<_, i64>(0),
    )? as u64;

    let usage_query = crate::usage::UsageQuery {
        provider: query.provider.clone(),
        session_id: None,
        since: query.since.clone(),
        until: query.until.clone(),
    };
    Ok(StatsSummary {
        sessions,
        records,
        starred_sessions,
        active_days,
        total_duration_ms,
        providers,
        projects,
        days,
        hourly_records,
        outcomes,
        secret_findings,
        usage: crate::usage::summarize(conn, &usage_query)?,
    })
}

fn grouped_buckets(
    conn: &Connection,
    query: &StatsQuery,
    group_expression: &str,
) -> Result<Vec<StatsBucket>> {
    let sql = format!(
        "SELECT {group_expression}, COUNT(*), COALESCE(SUM(record_count), 0)
         FROM sessions
         WHERE (?1 IS NULL OR provider = ?1)
           AND (?2 IS NULL OR date(COALESCE(ended_at, started_at)) >= date(?2))
           AND (?3 IS NULL OR date(COALESCE(ended_at, started_at)) <= date(?3))
         GROUP BY {group_expression} ORDER BY {group_expression}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![query.provider, query.since, query.until], |row| {
            Ok(StatsBucket {
                key: row.get(0)?,
                sessions: row.get::<_, i64>(1)? as u64,
                records: row.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_provider_project_and_day_buckets() {
        let conn = Connection::open_in_memory().unwrap();
        crate::archive::schema::init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions
             (provider, session_id, source_path, project, starred, record_count, started_at, ended_at)
             VALUES ('codex', 'one', 'one.jsonl', 'sivtr', 1, 2, '2026-08-31T01:00:00Z', '2026-08-31T02:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (provider, session_id, source_path, project, starred, record_count, started_at, ended_at)
             VALUES ('claude', 'two', 'two.jsonl', '', 0, 3, '2026-08-30T01:00:00Z', '2026-08-30T02:00:00Z')",
            [],
        )
        .unwrap();

        let report = compute(&conn, &StatsQuery::default()).unwrap();
        assert_eq!(report.sessions, 2);
        assert_eq!(report.records, 5);
        assert_eq!(report.starred_sessions, 1);
        assert_eq!(report.providers.len(), 2);
        assert_eq!(report.projects[0].key, "sivtr");
        assert_eq!(report.projects[1].key, "unassigned");
        assert_eq!(report.days.len(), 2);
    }
}
