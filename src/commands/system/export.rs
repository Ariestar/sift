//! Portable exports from the unified archive.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use sivtr_core::archive::store::{self, BlobMode, SessionMeta};
use sivtr_core::record::WorkRecord;

use crate::cli::{ExportCommand, ExportFormat, ExportSessionsArgs};

pub fn execute(command: &ExportCommand) -> Result<()> {
    match &command.action {
        crate::cli::ExportAction::Sessions(args) => sessions(args),
    }
}

#[derive(Debug, Serialize)]
struct ExportSession {
    session: SessionMeta,
    records: Vec<WorkRecord>,
}

fn sessions(args: &ExportSessionsArgs) -> Result<()> {
    let (provider, session_id) = match args.source.as_deref() {
        Some(source) => {
            let (provider, session_id) = super::parse_provider_session_address(source)?;
            (Some(provider.to_string()), Some(session_id.to_string()))
        }
        None => (None, None),
    };
    let conn = sivtr_core::archive::open()?;
    sivtr_core::archive::sync::ensure_fresh_with_conn(&conn)?;
    let rows = store::list_session_rows(
        &conn,
        provider.as_deref(),
        session_id.as_deref(),
        args.starred.then_some(true),
    )?;
    if args.source.is_some() && rows.is_empty() {
        bail!(
            "no archived session matched `{}`",
            args.source.as_deref().unwrap_or_default()
        );
    }

    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows {
        let session = store::session_meta_by_key(&conn, &row.provider, &row.session_id)?
            .with_context(|| {
                format!(
                    "archived session `{}/{}` disappeared",
                    row.provider, row.session_id
                )
            })?;
        let records = store::load_records_by_row(&conn, row.row_id, BlobMode::Full)?;
        sessions.push(ExportSession { session, records });
    }

    let format = if args.jsonl {
        ExportFormat::Jsonl
    } else {
        args.format.unwrap_or(ExportFormat::Jsonl)
    };
    let text = render(&sessions, format)?;
    match &args.output {
        Some(path) => std::fs::write(path, text)
            .with_context(|| format!("failed to write export {}", path.display()))?,
        None => print!("{text}"),
    }
    Ok(())
}

fn render(sessions: &[ExportSession], format: ExportFormat) -> Result<String> {
    match format {
        ExportFormat::Json => {
            serde_json::to_string_pretty(sessions).context("failed to serialize JSON export")
        }
        ExportFormat::Jsonl => sessions
            .iter()
            .flat_map(|session| session.records.iter())
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map(|lines| format!("{}\n", lines.join("\n")))
            .context("failed to serialize JSONL export"),
        ExportFormat::Markdown => Ok(render_markdown(sessions)),
        ExportFormat::Html => Ok(render_html(sessions)),
    }
}

fn render_markdown(sessions: &[ExportSession]) -> String {
    let mut output = String::new();
    for session in sessions {
        let title = session
            .session
            .title
            .as_deref()
            .unwrap_or(&session.session.session_id);
        output.push_str(&format!(
            "# {title}\n\n- provider: `{}`\n- session: `{}`\n- project: `{}`\n\n",
            session.session.provider, session.session.session_id, session.session.project
        ));
        for record in &session.records {
            output.push_str(&format!(
                "## {} {}\n\n{}\n\n",
                record.work_ref,
                record.title,
                record.combined_text()
            ));
        }
    }
    output
}

fn render_html(sessions: &[ExportSession]) -> String {
    let mut output = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>sivtr export</title><style>body{max-width:960px;margin:2rem auto;font:15px system-ui;color:#222}article{border:1px solid #ddd;border-radius:8px;margin:1rem 0;padding:1rem}pre{white-space:pre-wrap;background:#f6f7f9;padding:1rem;border-radius:6px}.meta{color:#667}</style></head><body>",
    );
    for session in sessions {
        output.push_str(&format!(
            "<h1>{}</h1><p class=\"meta\">{} / {}</p>",
            html_escape(
                session
                    .session
                    .title
                    .as_deref()
                    .unwrap_or(&session.session.session_id)
            ),
            html_escape(&session.session.provider),
            html_escape(&session.session.session_id),
        ));
        for record in &session.records {
            output.push_str(&format!(
                "<article><h2>{}</h2><p class=\"meta\">{}</p><pre>{}</pre></article>",
                html_escape(&record.title),
                html_escape(&record.work_ref.to_string()),
                html_escape(&record.combined_text()),
            ));
        }
    }
    output.push_str("</body></html>\n");
    output
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escapes_transcript_text() {
        assert_eq!(html_escape("<&>\"'"), "&lt;&amp;&gt;&quot;&#39;");
    }
}
