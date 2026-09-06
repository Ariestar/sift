//! Shared secret redaction for the live remote-sharing transport.
//!
//! The implementation lives in `sivtr-core::privacy` so public publication
//! and device-to-device sharing cannot silently drift apart.

use anyhow::Result;
use sivtr_core::privacy;
use sivtr_core::record::{WorkContent, WorkPart, WorkPartBody, WorkRecord, WorkTarget};

pub fn redact_record(record: &WorkRecord) -> Result<WorkRecord> {
    let mut out = record.clone();
    out.title = privacy::redact_text(&out.title)?;
    out.parts = out
        .parts
        .into_iter()
        .map(redact_part)
        .collect::<Result<Vec<_>>>()?;
    Ok(out)
}

pub fn redact_part(mut part: WorkPart) -> Result<WorkPart> {
    match &mut part.body {
        WorkPartBody::Message { label, content, .. } => {
            if let Some(label) = label {
                *label = privacy::redact_text(label)?;
            }
            redact_content(content)?;
        }
        WorkPartBody::Action {
            title,
            target,
            input,
            output,
            ..
        } => {
            if let Some(title) = title {
                *title = privacy::redact_text(title)?;
            }
            redact_target(target)?;
            if let Some(input) = input {
                redact_content(input)?;
            }
            for block in output {
                redact_content(&mut block.content)?;
            }
        }
    }
    Ok(part)
}

fn redact_target(target: &mut WorkTarget) -> Result<()> {
    match target {
        WorkTarget::Shell => {}
        WorkTarget::Tool { name } => {
            if let Some(name) = name {
                *name = privacy::redact_text(name)?;
            }
        }
        WorkTarget::Agent { name } => {
            *name = privacy::redact_text(name)?;
        }
        WorkTarget::Mcp { server, tool } => {
            *server = privacy::redact_text(server)?;
            *tool = privacy::redact_text(tool)?;
        }
    }
    Ok(())
}

fn redact_content(content: &mut WorkContent) -> Result<()> {
    match content {
        WorkContent::Text { content, ansi } => {
            *content = privacy::redact_text(content)?;
            if let Some(ansi) = ansi {
                *ansi = privacy::redact_text(ansi)?;
            }
        }
        WorkContent::Json(value) => privacy::redact_json(value)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_token_formats() {
        assert_eq!(
            privacy::redact_text("token ghp_aBcDeF0123456789ghij")
                .expect("privacy patterns should compile"),
            "token [REDACTED]"
        );
        assert_eq!(
            privacy::redact_text("key=sk-abcd1234efgh5678ijkl")
                .expect("privacy patterns should compile"),
            "key=[REDACTED]"
        );
        assert_eq!(
            privacy::redact_text("Authorization: Bearer abcdef1234567890XYZ")
                .expect("privacy patterns should compile"),
            "Authorization: [REDACTED]"
        );
    }

    #[test]
    fn redacts_pem_private_key_blocks() {
        let input = "before -----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA1234567890\n-----END RSA PRIVATE KEY----- after";
        let out = privacy::redact_text(input).expect("privacy patterns should compile");
        assert!(!out.contains("MIIEpAIBAA"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn does_not_redact_plain_text() {
        assert_eq!(
            privacy::redact_text("the build succeeded with 42 warnings")
                .expect("privacy patterns should compile"),
            "the build succeeded with 42 warnings"
        );
    }
}
