//! Archived session labels and listings.

use anyhow::Result;
use sivtr_core::archive::store;

use crate::cli::{SessionAction, SessionCommand};
use crate::output;

pub fn execute(command: &SessionCommand) -> Result<()> {
    match &command.action {
        SessionAction::Star { source } => set_starred(source, true),
        SessionAction::Unstar { source } => set_starred(source, false),
        SessionAction::List { starred, json } => list(*starred, *json),
    }
}

fn set_starred(source: &str, starred: bool) -> Result<()> {
    let (provider, session_id) = super::parse_provider_session_address(source)?;
    let conn = sivtr_core::archive::open()?;
    sivtr_core::archive::sync::ensure_fresh_with_conn(&conn)?;
    store::set_session_starred(&conn, provider, session_id, starred)?;
    output::success(format!(
        "{} {}",
        if starred { "starred" } else { "unstarred" },
        source
    ));
    Ok(())
}

fn list(starred: bool, json: bool) -> Result<()> {
    let conn = sivtr_core::archive::open()?;
    sivtr_core::archive::sync::ensure_fresh_with_conn(&conn)?;
    let rows = store::list_session_rows(&conn, None, None, starred.then_some(true))?;
    let mut sessions = Vec::new();
    for row in rows {
        if let Some(session) = store::session_meta_by_key(&conn, &row.provider, &row.session_id)? {
            sessions.push(session);
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    for session in sessions {
        println!(
            "{} {} {}{}",
            session.provider,
            session.session_id,
            session.title.as_deref().unwrap_or(""),
            if session.starred { " ★" } else { "" }
        );
    }
    Ok(())
}
