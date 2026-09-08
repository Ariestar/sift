//! Dialogue construction, content copy, line filter, and search targeting.

use anyhow::{Context, Result};
use crossterm::event::KeyCode;

use crate::tui::content::block::{dialogue_blocks, Block};
use crate::tui::content::view::{line_count, ContentViewMode};
use crate::tui::workspace::{WorkspaceDialogue, WorkspaceSource};
use sivtr_core::record::{WorkAt, WorkRecord};

use crate::commands::memory::workset::{WorkSelectionAction, WorkSelectionTarget, WorkSet};

use super::text::filter_lines_by_spec;
use super::vim::{VimBlock, VimView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspacePickProjection {
    Whole,
    Input,
    Output,
    Command,
    Parts,
}

#[derive(Clone, Debug)]
pub(crate) enum PickedContent {
    WorkSet {
        source: WorkspaceSource,
        set: WorkSet,
        projection: WorkspacePickProjection,
        line_filter: Option<String>,
    },
    Text {
        source: WorkspaceSource,
        units: Vec<crate::tui::workspace::TextPair>,
    },
}

#[derive(Clone, Copy)]
pub(super) enum WorkspaceCopyShortcut {
    Displayed,
    Input,
    Output,
    Command,
}

/// Source a copy is attributed to: the first picked dialogue's.
pub(super) fn picked_source(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
) -> Option<WorkspaceSource> {
    dialogues
        .get(*picked.first()?)
        .map(|dialogue| dialogue.source.clone())
}

pub(super) fn workspace_picked_content_for_copy(
    dialogues: &[WorkspaceDialogue],
    picked: &[usize],
    shortcut: WorkspaceCopyShortcut,
    line_filter: Option<&str>,
    target: Option<WorkAt>,
    _content_mode: ContentViewMode,
) -> Result<PickedContent> {
    let source = picked_source(dialogues, picked).context("copy needs at least one dialogue")?;
    let display_target = (picked.len() == 1
        && matches!(shortcut, WorkspaceCopyShortcut::Displayed))
    .then_some(target)
    .flatten();
    let mut records = Vec::new();
    let mut complete = true;
    for &idx in picked {
        let Some(dialogue) = dialogues.get(idx) else {
            complete = false;
            continue;
        };
        if let Some(record) = dialogue.record.as_ref() {
            records.push(record.clone());
        } else {
            complete = false;
        }
    }
    let projection = match shortcut {
        WorkspaceCopyShortcut::Displayed if target.is_none() => WorkspacePickProjection::Whole,
        WorkspaceCopyShortcut::Displayed => WorkspacePickProjection::Parts,
        WorkspaceCopyShortcut::Input => WorkspacePickProjection::Input,
        WorkspaceCopyShortcut::Output => WorkspacePickProjection::Output,
        WorkspaceCopyShortcut::Command => WorkspacePickProjection::Command,
    };
    if !complete {
        anyhow::bail!("structured copy needs materialized dialogues");
    }
    if let Some(spec) = line_filter {
        filter_lines_by_spec(&crate::tui::workspace::TextPair::default(), spec)?;
    }
    let cwd = std::env::current_dir().context("copy needs a current directory")?;
    let mut set = WorkSet::from_parts(cwd.display().to_string(), Vec::new(), Vec::new());
    for record in records {
        match display_target {
            Some(WorkAt::Part(seq)) => set.apply_target(
                WorkSelectionAction::Include,
                WorkSelectionTarget::Parts {
                    record,
                    parts: vec![seq],
                },
                [],
            ),
            _ => set.apply_target(
                WorkSelectionAction::Include,
                WorkSelectionTarget::Whole(record),
                [],
            ),
        }
    }
    Ok(PickedContent::WorkSet {
        source,
        set,
        projection,
        line_filter: line_filter.map(str::to_string),
    })
}

/// The Part subset of the canonical selection, attributed to its first
/// materialized dialogue.
pub(super) fn workspace_picked_content_for_selected_parts(
    selection: &WorkSet,
    dialogues: &[WorkspaceDialogue],
) -> Option<PickedContent> {
    let set = selection.parts_only()?;
    let anchor = set.anchors().first()?.clone();
    let source = dialogues.iter().find_map(|dialogue| {
        (dialogue.work_ref.as_ref()?.whole() == anchor.whole()).then(|| dialogue.source.clone())
    })?;
    Some(PickedContent::WorkSet {
        source,
        set,
        projection: WorkspacePickProjection::Parts,
        line_filter: None,
    })
}

/// Copy the block under the content cursor as its part anchors.
pub(super) fn workspace_picked_content_for_cursor_block(
    dialogues: &[WorkspaceDialogue],
    dialogue_idx: usize,
    block_id: usize,
) -> Result<Option<PickedContent>> {
    let Some((source, record, parts)) = workspace_block_parts(dialogues, dialogue_idx, block_id)
    else {
        return Ok(None);
    };
    let cwd = std::env::current_dir().context("copy needs a current directory")?;
    let mut set = WorkSet::from_parts(cwd.display().to_string(), Vec::new(), Vec::new());
    set.apply_target(
        WorkSelectionAction::Include,
        WorkSelectionTarget::Parts { record, parts },
        [],
    );
    Ok(Some(PickedContent::WorkSet {
        source,
        set,
        projection: WorkspacePickProjection::Parts,
        line_filter: None,
    }))
}

pub(super) fn workspace_block_parts(
    dialogues: &[WorkspaceDialogue],
    dialogue_idx: usize,
    block_id: usize,
) -> Option<(WorkspaceSource, WorkRecord, Vec<usize>)> {
    let dialogue = dialogues.get(dialogue_idx)?;
    let record = dialogue.record.as_ref()?;
    let (input_blocks, output_blocks) = dialogue_blocks(record);
    let block = input_blocks
        .iter()
        .chain(&output_blocks)
        .find_map(|block| find_block(block, block_id))?;
    dialogue.work_ref.as_ref()?;
    Some((
        dialogue.source.clone(),
        record.clone(),
        block
            .parts
            .iter()
            .map(|&idx| record.parts[idx].seq)
            .collect(),
    ))
}

/// Depth-first block lookup: run members live nested in `children`, and the
/// content cursor may sit on either a run or one of its members.
fn find_block(block: &Block, id: usize) -> Option<&Block> {
    if block.id == id {
        return Some(block);
    }
    block
        .children
        .iter()
        .find_map(|child| find_block(child, id))
}

pub(super) fn line_filter_spec(line_filter: &str) -> Option<&str> {
    (!line_filter.is_empty()).then_some(line_filter)
}

pub(super) fn handle_line_filter_key(
    key: KeyCode,
    dialogue_count: usize,
    line_filter_input_open: &mut bool,
    line_filter: &mut String,
    line_filter_error: &mut Option<String>,
) -> bool {
    if *line_filter_input_open {
        match key {
            KeyCode::Char(ch) if matches!(ch, '0'..='9' | ':' | ',') => {
                line_filter.push(ch);
                *line_filter_error = None;
                return true;
            }
            KeyCode::Backspace => {
                *line_filter_error = None;
                if line_filter.pop().is_none() {
                    *line_filter_input_open = false;
                }
                return true;
            }
            KeyCode::Esc => {
                *line_filter_input_open = false;
                line_filter.clear();
                *line_filter_error = None;
                return true;
            }
            _ => {}
        }
    }

    match key {
        KeyCode::Char(':') if dialogue_count > 0 => {
            *line_filter_input_open = true;
            *line_filter_error = None;
            true
        }
        KeyCode::Esc if line_filter_error.is_some() => {
            *line_filter_error = None;
            true
        }
        _ => false,
    }
}

/// Apply a bracketed paste to the line filter with the same character policy
/// as typed input (digits, `:`, `,`). Clipboard content is often copied with a
/// trailing newline or other stray characters; appending it verbatim would make
/// the later `filter_lines_by_spec` parse fail and exit the picker. Matching the
/// typed path, the error is cleared once usable characters land.
pub(super) fn handle_line_filter_paste(
    text: &str,
    line_filter: &mut String,
    line_filter_error: &mut Option<String>,
) {
    let filtered: String = text
        .chars()
        .filter(|ch| matches!(ch, '0'..='9' | ':' | ','))
        .collect();
    if !filtered.is_empty() {
        line_filter.push_str(&filtered);
        *line_filter_error = None;
    }
}

#[cfg(test)]
pub(super) fn workspace_dialogue_vim_view(dialogue: &WorkspaceDialogue) -> VimView {
    dialogue_text_vim_view(crate::tui::content::text::workspace_content_text(
        std::slice::from_ref(dialogue),
        0,
        ContentViewMode::Reading,
        None,
    ))
}

pub(super) fn dialogue_text_vim_view(text: String) -> VimView {
    let end = line_count(&text).max(1);
    VimView {
        blocks: vec![VimBlock {
            start: 1,
            end,
            input_start: 1,
            input_end: end,
            output_start: 1,
            output_end: end,
            block_text: text.clone(),
            input_text: text.clone(),
            output_text: text.clone(),
            command_text: String::new(),
        }],
        raw: text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::workspace::WorkspaceSource;
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkRecordKind, WorkRef, WorkSessionRef,
        WorkSource, WorkTime,
    };
    use sivtr_core::workset::{WorkSelectionAction, WorkSelectionTarget, WorkSet};

    fn record(title: &str, tool: &str, command: &str, index: usize) -> WorkRecord {
        let mut record = WorkRecord {
            schema_version: 2,
            work_ref: WorkRef::agent(AgentProvider::Codex, "test", index + 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "test".to_string(),
                canonical_id: Some("test-session-0123456789abcdef".to_string()),
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: title.to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::User {
                    content: "user".to_string(),
                },
            }],
        };
        record.parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: Some("c1".to_string()),
                tool: Some(tool.to_string()),
                input: serde_json::json!({ "command": command }),
            },
        });
        record
    }

    fn dialogue(record: WorkRecord) -> WorkspaceDialogue {
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record),
        }
    }

    #[test]
    fn selected_parts_copy_every_selected_dialogue() {
        let a = dialogue(record("A", "Bash", "ls", 0));
        let b = dialogue(record("B", "Bash", "git status", 1));
        let dialogues = [a, b];
        let mut selection = WorkSet::new(".", Vec::new());
        for dialogue in &dialogues {
            let record = dialogue.record.clone().unwrap();
            selection.apply_target(
                WorkSelectionAction::Include,
                WorkSelectionTarget::Parts {
                    record,
                    parts: vec![2],
                },
                [],
            );
        }
        let picked = workspace_picked_content_for_selected_parts(&selection, &dialogues)
            .expect("selected parts");
        let PickedContent::WorkSet {
            set, projection, ..
        } = picked
        else {
            panic!("marked blocks must remain addressable")
        };
        assert_eq!(projection, WorkspacePickProjection::Parts);
        assert_eq!(set.anchors().len(), 2);
        assert!(set.anchors().iter().all(|anchor| anchor.part().is_some()));
    }

    #[test]
    fn selected_run_parts_are_deduplicated() {
        let mut base = record("A", "Bash", "ls", 0);
        // A second consecutive tool call folds both into one run: block 1 is
        // the run, blocks 2 and 3 its members.
        base.parts.push(WorkPart {
            seq: 3,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: Some("c2".to_string()),
                tool: Some("Bash".to_string()),
                input: serde_json::json!({ "command": "git status" }),
            },
        });
        let dialogues = [dialogue(base)];
        let mut selection = WorkSet::new(".", Vec::new());
        selection.apply_target(
            WorkSelectionAction::Include,
            WorkSelectionTarget::Parts {
                record: dialogues[0].record.clone().unwrap(),
                parts: vec![2, 3, 2],
            },
            [],
        );
        let picked = workspace_picked_content_for_selected_parts(&selection, &dialogues)
            .expect("selected run");
        let PickedContent::WorkSet {
            set, projection, ..
        } = picked
        else {
            panic!("marked blocks must remain addressable")
        };
        assert_eq!(projection, WorkspacePickProjection::Parts);
        assert_eq!(set.anchors().len(), 2, "run members copied twice");
    }
}
