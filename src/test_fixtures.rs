//! Shared in-memory record fixtures for tests that build WorkRecords.
//! `pub` so sibling module trees (commands, tui) can reach it in tests.

use sivtr_core::record::{
    MessageRole, WorkActionStatus, WorkActor, WorkChannel, WorkContent, WorkContentBlock, WorkPart,
    WorkPartBody, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTarget,
    WorkTime, RECORD_SCHEMA_VERSION,
};

/// A message part of the given role — the shape every record test speaks in.
pub fn message_part(seq: usize, role: MessageRole, content: &str) -> WorkPart {
    WorkPart {
        seq,
        occurred_at: None,
        body: WorkPartBody::Message {
            role,
            label: None,
            content: WorkContent::Text {
                content: content.to_string(),
                ansi: None,
            },
        },
    }
}

/// An agent shell action: command input, optional output text.
pub fn shell_action_part(seq: usize, command: &str, output: Option<&str>) -> WorkPart {
    WorkPart {
        seq,
        occurred_at: None,
        body: WorkPartBody::Action {
            id: format!("shell-{seq}"),
            actor: WorkActor::Agent,
            target: WorkTarget::Shell,
            title: None,
            input: Some(WorkContent::Text {
                content: command.to_string(),
                ansi: None,
            }),
            output: output
                .map(|text| {
                    vec![WorkContentBlock {
                        content: WorkContent::Text {
                            content: text.to_string(),
                            ansi: None,
                        },
                        start_line: None,
                    }]
                })
                .unwrap_or_default(),
            status: WorkActionStatus::Completed,
            exit_code: None,
        },
    }
}

/// A chat-turn record whose parts the caller builds.
pub fn chat_record(index: usize, parts: Vec<WorkPart>) -> WorkRecord {
    WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        work_ref: WorkRef::agent(sivtr_core::agents::AgentProvider::Codex, "test", index),
        kind: WorkRecordKind::ChatTurn,
        source: WorkSource {
            channel: WorkChannel::Chat,
            provider: Some("codex".to_string()),
        },
        session: WorkSessionRef {
            id: "test".to_string(),
            canonical_id: None,
            path: None,
        },
        cwd: None,
        time: WorkTime::default(),
        status: None,
        title: "title".to_string(),
        parts,
    }
}
