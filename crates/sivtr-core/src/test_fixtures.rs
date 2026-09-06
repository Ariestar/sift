//! Shared on-disk git fixtures for tests that need repo/worktree layouts,
//! plus in-memory record fixtures for tests that build WorkRecords.

use std::fs;
use std::path::Path;

use crate::record::{
    MessageRole, WorkActionStatus, WorkActor, WorkContent, WorkContentBlock, WorkOutcome, WorkPart,
    WorkPartBody, WorkRecord, WorkRef, WorkSessionRef, WorkStatus, WorkTarget, WorkTime,
    RECORD_SCHEMA_VERSION,
};

/// Create a normal repo (`root/.git` dir).
pub(crate) fn make_repo(root: &Path) {
    fs::create_dir_all(root.join(".git")).unwrap();
}

/// Create a linked worktree of `main` (mirrors `git worktree add`): the
/// worktree's `.git` is a `gitdir:` pointer to `<main>/.git/worktrees/<name>`,
/// whose `commondir` points back at the main `.git` dir.
pub(crate) fn make_worktree(main: &Path, wt: &Path, name: &str) {
    let gitdir = main.join(".git").join("worktrees").join(name);
    fs::create_dir_all(&gitdir).unwrap();
    fs::write(gitdir.join("commondir"), "../..").unwrap();
    fs::create_dir_all(wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
}

/// A message part of the given role — the shape every record test speaks in.
pub(crate) fn message_part(seq: usize, role: MessageRole, content: &str) -> WorkPart {
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

/// A terminal-style shell action: optional command input and output text.
pub(crate) fn shell_part(seq: usize, command: Option<&str>, output: Option<&str>) -> WorkPart {
    WorkPart {
        seq,
        occurred_at: None,
        body: WorkPartBody::Action {
            id: format!("shell-{seq}"),
            actor: WorkActor::User,
            target: WorkTarget::Shell,
            title: None,
            input: command.map(|command| WorkContent::Text {
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

/// A whole terminal record whose single part carries `text`.
pub(crate) fn terminal_record(session: &str, index: usize, title: &str, text: &str) -> WorkRecord {
    WorkRecord {
        schema_version: RECORD_SCHEMA_VERSION,
        work_ref: WorkRef::terminal(session, index),
        session: WorkSessionRef {
            id: session.to_string(),
            canonical_id: None,
            path: None,
        },
        cwd: None,
        time: WorkTime::default(),
        status: Some(WorkStatus {
            outcome: WorkOutcome::Success,
            exit_code: Some(0),
        }),
        title: title.to_string(),
        parts: vec![shell_part(1, None, Some(text))],
    }
}
