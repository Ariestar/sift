//! Shared, best-effort privacy helpers.
//!
//! This module deliberately only removes high-signal credential formats.  It
//! is a reduction in accidental disclosure, not a security boundary: callers
//! must still ask the user to review the resulting snapshot before publishing.

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

const REDACTED: &str = "[REDACTED]";

static PATTERNS: LazyLock<Result<Vec<(&'static str, Regex)>, regex::Error>> = LazyLock::new(|| {
    Ok(vec![
        (
            "github_pat",
            Regex::new(r"(?:gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,})")?,
        ),
        ("openai_key", Regex::new(r"sk-[A-Za-z0-9_-]{16,}")?),
        ("sivtr_token", Regex::new(r"s-[A-Za-z0-9]{16,}")?),
        ("slack_token", Regex::new(r"xox[abprs]-[A-Za-z0-9-]{10,}")?),
        ("aws_id", Regex::new(r"AKIA[0-9A-Z]{16}")?),
        (
            "aws_secret",
            Regex::new(r#"(?i)aws_secret_access_key['"\s:=]+[A-Za-z0-9/+=]{40}"#)?,
        ),
        (
            "assigned_secret",
            Regex::new(
                r#"(?i)(api[_-]?key|token|password|secret|bearer)\s*[:=]\s*['"]?[A-Za-z0-9_\-./+=]{12,}['"]?"#,
            )?,
        ),
        (
            "jwt",
            Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b")?,
        ),
        (
            "cookie",
            Regex::new(r"(?i)(?:cookie|set-cookie)\s*[:=]\s*[^\r\n]{12,}")?,
        ),
        ("bearer", Regex::new(r"(?i)bearer\s+[A-Za-z0-9_\-.=]{16,}")?),
        (
            "pem_key",
            Regex::new(
                r"-----BEGIN [A-Z ]+PRIVATE KEY-----[\s\S]*?-----END [A-Z ]+PRIVATE KEY-----",
            )?,
        ),
    ])
});

static WARNING_PATTERNS: LazyLock<Result<Vec<(&'static str, Regex)>, regex::Error>> = LazyLock::new(
    || {
        Ok(vec![
            (
                "absolute_path",
                Regex::new(
                    r#"(?i)(?:[A-Z]:[\\/]|/(?:Users|home|root|tmp|var|etc|opt|srv|usr|mnt|media|data|workspace)/|\\\\)[^\s`]+"#,
                )?,
            ),
            (
                "email",
                Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")?,
            ),
            (
                "internal_url",
                Regex::new(
                    r"(?i)https?://(?:localhost|127\.0\.0\.1|10\.(?:[0-9]{1,3}\.){2}[0-9]{1,3}|192\.168\.(?:[0-9]{1,3}\.)[0-9]{1,3}|[A-Za-z0-9-]+\.local)(?::\d+)?(?:/[^\s]*)?",
                )?,
            ),
        ])
    },
);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextPrivacyReport {
    pub redactions: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivacyReport {
    pub redactions: usize,
    pub warnings: Vec<PrivacyWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivacyWarning {
    pub kind: String,
    pub item_index: usize,
}

/// Redact high-signal credentials and report non-mutating disclosure risks.
pub fn redact_text_with_report(value: &str) -> Result<(String, TextPrivacyReport)> {
    let mut current = value.to_string();
    let mut report = TextPrivacyReport::default();
    let patterns = PATTERNS
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to compile privacy redaction pattern: {error}"))?;
    for (name, regex) in patterns {
        let count = regex.find_iter(&current).count();
        if count > 0 {
            report.redactions += count;
            current = regex.replace_all(&current, REDACTED).into_owned();
            report.warnings.push((*name).to_string());
        }
    }
    let warning_patterns = WARNING_PATTERNS
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to compile privacy warning pattern: {error}"))?;
    for (name, regex) in warning_patterns {
        if regex.is_match(&current) {
            report.warnings.push((*name).to_string());
        }
    }
    report.warnings.sort();
    report.warnings.dedup();
    Ok((current, report))
}

/// Shared redaction entry point used by the existing remote sharing path.
pub fn redact_text(value: &str) -> Result<String> {
    Ok(redact_text_with_report(value)?.0)
}

/// Redact every textual value in a JSON tool payload.  Kept public so future
/// transports can reuse the exact same credential patterns.
pub fn redact_json(value: &mut Value) -> Result<()> {
    match value {
        Value::String(text) => *text = redact_text(text)?,
        Value::Array(items) => {
            for item in items {
                redact_json(item)?;
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                redact_json(item)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

pub fn is_manual_warning(kind: &str) -> bool {
    matches!(kind, "absolute_path" | "email" | "internal_url")
}

/// Count warning-only risks in a public item and attach the item position.
pub fn warnings_for_item(text: &str, item_index: usize) -> Result<Vec<PrivacyWarning>> {
    let (_, report) = redact_text_with_report(text)?;
    Ok(report
        .warnings
        .into_iter()
        .filter(|kind| is_manual_warning(kind))
        .map(|kind| PrivacyWarning { kind, item_index })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_existing_remote_token_shapes() {
        let (text, report) = redact_text_with_report(
            "ghp_aBcDeF0123456789ghij github_pat_11AA22bb33CC44dd55EE66ff77GG88hh99 sk-proj-abcdefghijklmnop",
        )
        .expect("privacy patterns should compile");
        assert_eq!(text, "[REDACTED] [REDACTED] [REDACTED]");
        assert_eq!(report.redactions, 3);
    }

    #[test]
    fn warns_without_changing_paths_and_emails() {
        let (text, report) = redact_text_with_report(r"C:\Users\alice\repo alice@example.com")
            .expect("privacy patterns should compile");
        assert_eq!(text, r"C:\Users\alice\repo alice@example.com");
        assert!(report.warnings.iter().any(|item| item == "absolute_path"));
        assert!(report.warnings.iter().any(|item| item == "email"));
        let (unix, unix_report) =
            redact_text_with_report("/root/.ssh/id_rsa /workspace/company/repo")
                .expect("privacy patterns should compile");
        assert_eq!(unix, "/root/.ssh/id_rsa /workspace/company/repo");
        assert!(unix_report
            .warnings
            .iter()
            .any(|item| item == "absolute_path"));
    }

    #[test]
    fn manual_warning_kinds_are_explicit() {
        assert!(is_manual_warning("absolute_path"));
        assert!(is_manual_warning("email"));
        assert!(is_manual_warning("internal_url"));
        assert!(!is_manual_warning("github_pat"));
    }
}
