//! Archive quality reports that never print secret values.

use anyhow::Result;

use crate::cli::{QualityAction, QualityCommand};
use crate::output;

pub fn execute(command: &QualityCommand) -> Result<()> {
    match &command.action {
        QualityAction::Secrets { json } => secrets(*json),
    }
}

fn secrets(json: bool) -> Result<()> {
    let conn = sivtr_core::archive::open()?;
    let skipped = sivtr_core::archive::sync::ensure_fresh_with_conn(&conn)?;
    let findings = sivtr_core::archive::store::list_secret_findings(&conn)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "findings": findings,
                "warnings": sivtr_core::usage::warnings_from_sync(&skipped),
            }))?
        );
        return Ok(());
    }
    for entry in skipped {
        output::warning(format!(
            "quality skipped {} {}: {}",
            entry.namespace,
            entry.path.display(),
            entry.error
        ));
    }
    if findings.is_empty() {
        output::info("no secret findings in the archive");
        return Ok(());
    }
    for finding in findings {
        println!(
            "{} / {} · {} · {} occurrence(s)",
            finding.provider, finding.session_id, finding.kind, finding.occurrences
        );
    }
    Ok(())
}
