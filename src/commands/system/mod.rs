use anyhow::{bail, Result};

pub mod config;
pub mod doctor;
pub mod export;
pub mod hotkey;
pub mod import;
pub mod mcp;
pub mod quality;
pub mod session;
pub mod setup;
pub mod skill;
pub mod stats;
pub mod sync;
pub mod update;
pub mod usage;
pub mod version;

pub(crate) fn parse_provider_session_address(source: &str) -> Result<(&str, &str)> {
    let Some((provider, session_id)) = source.split_once('/') else {
        bail!("session address must be PROVIDER/SESSION");
    };
    if provider.is_empty() || session_id.is_empty() || session_id.contains('/') {
        bail!("session address must be PROVIDER/SESSION");
    }
    Ok((provider, session_id))
}
