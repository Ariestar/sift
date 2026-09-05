//! Provider-neutral atomic groups inside one work record.

use std::collections::HashMap;

use super::model::{WorkPartData, WorkPartKind, WorkRecord};

/// A selectable semantic atom. Most atoms own one part; a tool invocation owns
/// its call and matching result so callers cannot publish an unusable half.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkAtom {
    /// Stable `WorkPart.seq` values, in transcript order.
    pub part_seqs: Vec<usize>,
    /// Kind of the first part. Tool atoms have `ToolCall` here.
    pub kind: WorkPartKind,
}

impl WorkAtom {
    pub fn is_tool(&self) -> bool {
        self.kind == WorkPartKind::ToolCall
    }
}

/// Build semantic atoms for one record half. Tool calls and matching results
/// are paired by call id (or by the existing id-less same-tool fallback).
/// Consecutive structure runs remain a presentation concern owned by the TUI.
pub fn work_atoms(record: &WorkRecord, input: bool) -> Vec<WorkAtom> {
    let parts: Vec<usize> = record
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.kind().is_input() == input)
        .map(|(index, _)| index)
        .collect();

    let mut atoms = Vec::new();
    let mut open_calls: HashMap<&str, usize> = HashMap::new();
    let mut last_idless_call: Option<(usize, Option<&str>)> = None;

    for part_index in parts {
        let part = &record.parts[part_index];
        match part.kind() {
            WorkPartKind::ToolCall => {
                let atom_index = atoms.len();
                atoms.push(WorkAtom {
                    part_seqs: vec![part.seq],
                    kind: WorkPartKind::ToolCall,
                });
                if let Some(call_id) = part_call_id(part) {
                    open_calls.insert(call_id, atom_index);
                    last_idless_call = None;
                } else {
                    last_idless_call = Some((atom_index, part_tool(part)));
                }
            }
            WorkPartKind::ToolResult => {
                let target = part_call_id(part)
                    .and_then(|id| open_calls.remove(id))
                    .or_else(|| match last_idless_call {
                        Some((atom_index, call_tool))
                            if same_idless_tool(call_tool, part_tool(part)) =>
                        {
                            last_idless_call = None;
                            Some(atom_index)
                        }
                        _ => None,
                    });
                match target {
                    Some(atom_index) => atoms[atom_index].part_seqs.push(part.seq),
                    None => atoms.push(WorkAtom {
                        part_seqs: vec![part.seq],
                        kind: WorkPartKind::ToolResult,
                    }),
                }
            }
            kind => atoms.push(WorkAtom {
                part_seqs: vec![part.seq],
                kind,
            }),
        }
    }

    atoms
}

fn part_call_id(part: &super::model::WorkPart) -> Option<&str> {
    match &part.data {
        WorkPartData::ToolCall { call_id, .. } | WorkPartData::ToolResult { call_id, .. } => {
            call_id.as_deref()
        }
        _ => None,
    }
}

fn part_tool(part: &super::model::WorkPart) -> Option<&str> {
    match &part.data {
        WorkPartData::ToolCall { tool, .. } | WorkPartData::ToolResult { tool, .. } => {
            tool.as_deref()
        }
        _ => None,
    }
}

fn same_idless_tool(call_tool: Option<&str>, result_tool: Option<&str>) -> bool {
    call_tool.is_some() && call_tool == result_tool
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentProvider;
    use crate::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource,
        WorkTime,
    };

    fn record(parts: Vec<WorkPart>) -> WorkRecord {
        WorkRecord {
            schema_version: 3,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".into()),
            },
            session: WorkSessionRef {
                id: "session".into(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "test".into(),
            parts,
        }
    }

    #[test]
    fn pairs_tool_call_and_result_by_call_id() {
        let record = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c1".into()),
                    tool: Some("Bash".into()),
                    input: serde_json::json!({"command":"ls"}),
                },
            },
            WorkPart {
                seq: 2,
                occurred_at: None,
                data: WorkPartData::ToolResult {
                    call_id: Some("c1".into()),
                    tool: Some("Bash".into()),
                    output: serde_json::json!({"stdout":"ok"}),
                    start_line: None,
                },
            },
        ]);
        assert_eq!(work_atoms(&record, false)[0].part_seqs, vec![1, 2]);
    }

    #[test]
    fn keeps_orphan_result_as_its_own_atom() {
        let record = record(vec![WorkPart {
            seq: 1,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: Some("gone".into()),
                tool: Some("Bash".into()),
                output: serde_json::json!("orphan"),
                start_line: None,
            },
        }]);
        assert_eq!(work_atoms(&record, false)[0].part_seqs, vec![1]);
    }
}
