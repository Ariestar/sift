use serde::Serialize;
use sivtr_core::record::WorkRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkJsonSessionMeta {
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub display_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub fn session_meta(record: &WorkRecord) -> WorkJsonSessionMeta {
    WorkJsonSessionMeta {
        ref_: record.work_ref.path.stream_path(),
        provider: record
            .provider()
            .map(|provider| provider.command_name().to_string()),
        display_id: record.session.id.clone(),
        canonical_id: record.session.canonical_id.clone(),
        path: record.session.path.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sivtr_core::agents::AgentProvider;
    use sivtr_core::record::{MessageRole, WorkRef, WorkSessionRef, WorkTime};

    #[test]
    fn builds_session_metadata_with_canonical_id() {
        let record = test_record();
        let metadata = session_meta(&record);

        assert_eq!(metadata.ref_, "codex/shortid");
        assert_eq!(metadata.provider.as_deref(), Some("codex"));
        assert_eq!(metadata.display_id, "shortid");
        assert_eq!(
            metadata.canonical_id.as_deref(),
            Some("session-0123456789abcdef")
        );
        assert_eq!(metadata.path.as_deref(), Some("/tmp/session.jsonl"));
    }

    fn test_record() -> WorkRecord {
        WorkRecord {
            schema_version: sivtr_core::record::RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(AgentProvider::Codex, "shortid", 3),
            session: WorkSessionRef {
                id: "shortid".to_string(),
                canonical_id: Some("session-0123456789abcdef".to_string()),
                path: Some("/tmp/session.jsonl".to_string()),
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "title".to_string(),
            parts: vec![crate::test_fixtures::message_part(
                1,
                MessageRole::Assistant,
                "assistant reply",
            )],
        }
    }
}
