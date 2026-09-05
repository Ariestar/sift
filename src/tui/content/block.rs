//! Content blocks: every workpart is a foldable block.
//!
//! A block is the smallest unit the content pane highlights, navigates, and
//! folds: one workpart, or a ToolCall + ToolResult pair with the same call id
//! (they read as one tool invocation). Consecutive structure blocks fold
//! into one run block that collapses to a single `kind xN` tag; expanding a
//! run reveals its members below the tag, one call per line, each still
//! folded and expandable in turn — two fold levels. Structure blocks default
//! to their `<:…:>` tag; body blocks default to their full text — one fold
//! model, no structure-only special cases.

use sivtr_core::record::{work_atoms, WorkPart, WorkPartData, WorkPartKind, WorkRecord};

use crate::tui::content::io::ExpandedBlocks;
use crate::tui::content::tool::{part_body_text, tool_display_name, tool_tag_for_part};

/// A foldable content block: the parts it owns, the kind that drives its
/// fold default and collapsed tag (the first part's kind), and — for runs —
/// the member blocks revealed when the run is expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    /// Stable identity within the dialogue (DFS pre-order over the input
    /// half then the output half), used by the fold state, the marks, and
    /// the content cursor; id 0 is the dialogue's first block.
    pub(crate) id: usize,
    /// Indices into the record's parts, in display order.
    pub(crate) parts: Vec<usize>,
    pub(crate) kind: WorkPartKind,
    /// Member blocks of a run; empty for leaves.
    pub(crate) children: Vec<Block>,
}

/// One rendered segment of a half's display text: a block's collapsed tag
/// or full body. `tight` joins the next segment with a single newline
/// instead of a blank line — members of one run read as a single series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BlockText {
    pub(crate) id: usize,
    pub(crate) text: String,
    pub(crate) tight: bool,
    /// Kind of the block's first part; drives the dot-gutter color.
    pub(crate) kind: WorkPartKind,
}

impl Block {
    /// A leaf (non-run) block; ids are assigned by `assign_ids` afterwards.
    fn leaf(parts: Vec<usize>, kind: WorkPartKind) -> Self {
        Block {
            id: 0,
            parts,
            kind,
            children: Vec::new(),
        }
    }

    /// This block plus every nested member — the ids `assign_ids` consumes.
    fn node_count(&self) -> usize {
        1 + self.children.iter().map(Block::node_count).sum::<usize>()
    }

    /// Full body of a leaf: every part formatted as in the current content
    /// text, members joining on adjacent lines.
    pub(crate) fn body(&self, record: &WorkRecord) -> String {
        self.parts
            .iter()
            .map(|&idx| part_body_text(&record.parts[idx]))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Collapsed tag: `<:kind xN:>` for a run. Members list every kind in
    /// order with its count, repeats collapsed to `kind xN` and singles as
    /// the bare kind — `<:bash, thinking, read:>` — instead of a `+` mashup.
    pub(crate) fn fold_label(&self, record: &WorkRecord) -> String {
        if !self.children.is_empty() {
            let mut kinds: Vec<(String, usize)> = Vec::new();
            for child in &self.children {
                let name = match &record.parts[child.parts[0]].data {
                    WorkPartData::ToolCall { tool, .. } | WorkPartData::ToolResult { tool, .. } => {
                        tool_display_name(tool.as_deref().unwrap_or_default())
                    }
                    _ => kind_name(child.kind).to_string(),
                };
                match kinds.iter_mut().find(|(kind, _)| *kind == name) {
                    Some((_, count)) => *count += 1,
                    None => kinds.push((name, 1)),
                }
            }
            let label = kinds
                .into_iter()
                .map(|(name, count)| {
                    if count > 1 {
                        format!("{name} x{count}")
                    } else {
                        name
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("<:{label}:>")
        } else {
            fold_label_for_part(&record.parts[self.parts[0]])
        }
    }
}

/// Short display name for a kind: body tags use it (`<:user:>`), structure
/// runs use it for the count label (`<:tool x2:>`).
fn kind_name(kind: WorkPartKind) -> &'static str {
    match kind {
        WorkPartKind::Prompt => "prompt",
        WorkPartKind::Command => "command",
        WorkPartKind::User => "user",
        WorkPartKind::Assistant => "assistant",
        WorkPartKind::ToolCall | WorkPartKind::ToolResult => "tool",
        WorkPartKind::Skill => "skill",
        WorkPartKind::Thinking => "thinking",
        WorkPartKind::Output => "output",
        WorkPartKind::Error => "error",
    }
}

/// Partition one IO half's parts into blocks: a ToolCall and the ToolResult
/// carrying the same call id fold into one unit — wherever the result lands,
/// so interleaved parallel calls pair correctly — and consecutive structure
/// units (tool / thinking / skill, mixed kinds allowed) fold into one run;
/// anything else is one part per block. Runs get stable DFS pre-order ids
/// later, over the whole dialogue, so the fold state and cursor survive
/// folds and can move continuously across the input/output boundary.
pub(crate) fn dialogue_blocks(record: &WorkRecord) -> (Vec<Block>, Vec<Block>) {
    let mut input = build_half_units(record, true);
    let mut output = build_half_units(record, false);
    // One global id space per dialogue: input blocks first, output blocks
    // continue after them. The cursor, fold state, and marks key on this
    // single sequence instead of two per-half id spaces.
    let mut next = 0usize;
    for block in &mut input {
        assign_ids(block, &mut next);
    }
    for block in &mut output {
        assign_ids(block, &mut next);
    }
    (input, output)
}

/// One half's blocks in the dialogue-global id space.
#[cfg(test)]
pub(crate) fn half_blocks(record: &WorkRecord, input: bool) -> Vec<Block> {
    let (input_blocks, output_blocks) = dialogue_blocks(record);
    if input {
        input_blocks
    } else {
        output_blocks
    }
}

/// Size of a dialogue's id space: every block of both halves, nested run
/// members included, so a mask of this length holds a mark on any block —
/// even one currently hidden by a fold. Takes the already-built halves
/// ([`dialogue_blocks`]) so callers build the tree once.
pub(crate) fn dialogue_block_count(input: &[Block], output: &[Block]) -> usize {
    input.iter().chain(output).map(Block::node_count).sum()
}

pub(crate) fn dialogue_block_id(record: &WorkRecord, seq: usize) -> Option<usize> {
    let (input, output) = dialogue_blocks(record);
    let part = record.parts.iter().position(|part| part.seq == seq)?;
    input
        .iter()
        .chain(&output)
        .find_map(|block| block_id_for_part(block, part))
}

fn block_id_for_part(block: &Block, part: usize) -> Option<usize> {
    block
        .children
        .iter()
        .find_map(|child| block_id_for_part(child, part))
        .or_else(|| block.parts.contains(&part).then_some(block.id))
}

fn build_half_units(record: &WorkRecord, input: bool) -> Vec<Block> {
    // Core owns semantic atoms; the TUI only groups adjacent structure atoms
    // into a presentation run.
    let index_by_seq = record
        .parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.kind().is_input() == input)
        .map(|(index, part)| (part.seq, index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut blocks: Vec<Block> = Vec::new();
    for atom in work_atoms(record, input) {
        let parts = atom
            .part_seqs
            .into_iter()
            .map(|seq| *index_by_seq.get(&seq).expect("atom part must exist"))
            .collect();
        let unit = Block::leaf(parts, atom.kind);
        let merges =
            unit.kind.is_structure() && blocks.last().is_some_and(|last| last.kind.is_structure());
        if merges {
            let last = blocks.last_mut().expect("run block exists");
            if last.children.is_empty() {
                // Promote the leaf into a run holding itself as first member.
                last.children
                    .push(Block::leaf(last.parts.clone(), last.kind));
            }
            last.parts.extend(unit.parts.iter().copied());
            last.children.push(unit);
        } else {
            blocks.push(unit);
        }
    }

    blocks
}

fn assign_ids(block: &mut Block, next: &mut usize) {
    block.id = *next;
    *next += 1;
    for child in &mut block.children {
        assign_ids(child, next);
    }
}

/// Collapsed tag for one part: the per-tool tag (`<:read: path:>`) for known
/// tools, otherwise the structure marker with the tool description when
/// present, or a plain `<:kind:>` tag for body parts.
pub(crate) fn fold_label_for_part(part: &WorkPart) -> String {
    if let Some(tag) = tool_tag_for_part(part) {
        return tag;
    }
    if part.kind().is_structure() {
        let marker = part
            .kind()
            .as_agent_block_kind()
            .and_then(|kind| kind.open_marker(part.label()))
            .unwrap_or_else(|| "<:structure:>".to_string());
        match tool_description(part) {
            Some(description) => match marker.strip_suffix(":>") {
                Some(base) => format!("{base}: {description}:>"),
                None => marker,
            },
            None => marker,
        }
    } else {
        format!("<:{}:>", kind_name(part.kind()))
    }
}

/// Human description from a tool call's input (`description` field), if any,
/// truncated to fit the tag line.
fn tool_description(part: &WorkPart) -> Option<String> {
    let WorkPartData::ToolCall { input, .. } = &part.data else {
        return None;
    };
    let description = input
        .get("description")
        .and_then(serde_json::Value::as_str)?;
    // Normalize internal whitespace so a multi-line description still folds
    // to a single tag line (block layout assumes one line per tag).
    let description: String = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return None;
    }
    const MAX: usize = 40;
    Some(crate::tui::content::truncate_chars(&description, MAX))
}

/// Render one IO half's blocks to their display segments, in display order:
/// a block's full body when shown, its collapsed tag otherwise. Runs always
/// show the aggregate tag; expanding a run reveals its members below it,
/// each still folded, joined on adjacent lines. `blocks` comes from
/// [`dialogue_blocks`], so ids are dialogue-global.
pub(crate) fn render_half(
    record: &WorkRecord,
    blocks: &[Block],
    reading: bool,
    expanded: &ExpandedBlocks,
) -> Vec<BlockText> {
    let mut out = Vec::new();
    for block in blocks {
        out.extend(render_block(record, block, reading, expanded));
    }
    out
}

fn render_block(
    record: &WorkRecord,
    block: &Block,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> Vec<BlockText> {
    let mut segs = Vec::new();
    if !reading {
        // Raw mode: every block shows its full body; runs expand flat.
        if block.children.is_empty() {
            segs.push(BlockText {
                id: block.id,
                text: block.body(record),
                tight: false,
                kind: block.kind,
            });
        } else {
            for child in &block.children {
                segs.extend(render_block(record, child, reading, expanded));
            }
        }
    } else if block.children.is_empty() {
        // Leaf: body or collapsed tag by the block's fold default.
        let shown = expanded.expanded(block.id, block.kind.is_structure());
        segs.push(BlockText {
            id: block.id,
            text: if shown {
                block.body(record)
            } else {
                block.fold_label(record)
            },
            tight: false,
            kind: block.kind,
        });
    } else {
        // Run: the aggregate tag stays as the group header; expanding the
        // run reveals its members below it, each still folded.
        let shown = expanded.expanded(block.id, true);
        segs.push(BlockText {
            id: block.id,
            text: block.fold_label(record),
            tight: false, // rewritten below: all but the last segment join tight
            kind: block.kind,
        });
        if shown {
            for child in &block.children {
                segs.extend(render_block(record, child, reading, expanded));
            }
        }
    }
    // Members of one run join on adjacent lines; the last segment closes
    // the group with the usual blank line.
    if !block.children.is_empty() {
        let last = segs.len().saturating_sub(1);
        for (i, seg) in segs.iter_mut().enumerate() {
            seg.tight = i < last;
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::content::io::ExpandedBlocks;
    use sivtr_core::agents::AgentProvider;
    use sivtr_core::record::{
        WorkChannel, WorkRecordKind, WorkRef, WorkSessionRef, WorkSource, WorkTime,
        RECORD_SCHEMA_VERSION,
    };

    fn tool_part(seq: usize, tool: &str, input: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: None,
                tool: Some(tool.to_string()),
                input: serde_json::json!({ "command": input }),
            },
        }
    }

    fn tool_result_part(seq: usize, tool: &str, call_id: Option<&str>, output: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: call_id.map(str::to_string),
                tool: Some(tool.to_string()),
                output: serde_json::json!({ "stdout": output }),
                start_line: None,
            },
        }
    }

    fn user_part(seq: usize, content: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::User {
                content: content.to_string(),
            },
        }
    }

    fn thinking_part(seq: usize, content: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::Thinking {
                content: content.to_string(),
            },
        }
    }

    fn assistant_part(seq: usize, content: &str) -> WorkPart {
        WorkPart {
            seq,
            occurred_at: None,
            data: WorkPartData::Assistant {
                content: content.to_string(),
            },
        }
    }

    fn record(parts: Vec<WorkPart>) -> WorkRecord {
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: WorkRef::agent(AgentProvider::Codex, "session", 1),
            kind: WorkRecordKind::ChatTurn,
            source: WorkSource {
                channel: WorkChannel::Chat,
                provider: Some("codex".to_string()),
            },
            session: WorkSessionRef {
                id: "session".to_string(),
                canonical_id: None,
                path: None,
            },
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: "cmd".to_string(),
            parts,
        }
    }

    #[test]
    fn tool_call_with_matching_result_folds_into_one_block() {
        let rec = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c1".to_string()),
                    tool: Some("Bash".to_string()),
                    input: serde_json::json!({ "command": "ls" }),
                },
            },
            tool_result_part(2, "Bash", Some("c1"), "ok"),
            tool_part(3, "Read", "file"),
            user_part(4, "question"),
        ]);
        let blocks = half_blocks(&rec, false);
        // The matching pair and the following Read call fold into one tool run.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, WorkPartKind::ToolCall);
        assert_eq!(blocks[0].parts, vec![0, 1, 2]);
        assert_eq!(blocks[0].children.len(), 2);
    }

    #[test]
    fn parallel_calls_pair_results_by_call_id_across_interleaving() {
        // The ACP stream interleaves parallel calls: call 0, call 1, then
        // result 0, result 1. Adjacency pairing would leave all four as
        // separate blocks; call-id pairing folds each call with its result.
        let rec = record(vec![
            WorkPart {
                seq: 1,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c0".to_string()),
                    tool: Some("read_file".to_string()),
                    input: serde_json::json!({ "target_file": "a.rs" }),
                },
            },
            WorkPart {
                seq: 2,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some("c1".to_string()),
                    tool: Some("read_file".to_string()),
                    input: serde_json::json!({ "target_file": "b.rs" }),
                },
            },
            tool_result_part(3, "read_file", Some("c0"), "a body"),
            tool_result_part(4, "read_file", Some("c1"), "b body"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        // Each member owns its call and the matching result, in call order.
        assert_eq!(blocks[0].children[0].parts, vec![0, 2]);
        assert_eq!(blocks[0].children[1].parts, vec![1, 3]);
    }

    #[test]
    fn orphan_results_stay_separate() {
        // A result whose call id never opened, and an id-less result with no
        // preceding id-less call, both stand alone.
        let rec = record(vec![
            tool_result_part(1, "Bash", Some("gone"), "no matching call"),
            tool_result_part(2, "Bash", None, "no call id"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        assert_eq!(blocks[0].children[0].parts, vec![0]);
        assert_eq!(blocks[0].children[1].parts, vec![1]);
    }

    #[test]
    fn idless_result_pairs_with_nearest_idless_call() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_result_part(2, "Bash", None, "ok"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, WorkPartKind::ToolCall);
        assert_eq!(blocks[0].parts, vec![0, 1]);
    }

    #[test]
    fn consecutive_same_kind_structure_parts_fold_to_one_run() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        assert_eq!(blocks[0].fold_label(&rec), "<:bash, read:>");
    }

    #[test]
    fn mixed_structure_kinds_fold_into_one_run() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            thinking_part(2, "reasoning"),
            tool_part(3, "Read", "file"),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 3);
        assert_eq!(blocks[0].fold_label(&rec), "<:bash, thinking, read:>");
        // A body part after the series starts a new block in the same
        // (output) half: the run keeps its members, the body joins after.
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            thinking_part(2, "reasoning"),
            assistant_part(3, "answer"),
        ]);
        assert_eq!(half_blocks(&rec, false).len(), 2);
        assert!(half_blocks(&rec, true).is_empty());
    }

    #[test]
    fn ids_follow_dfs_preorder() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            thinking_part(2, "reasoning"),
            tool_part(3, "Read", "file"),
        ]);
        let blocks = half_blocks(&rec, false);
        // run id 0, members 1..=3.
        assert_eq!(blocks[0].id, 0);
        let ids: Vec<usize> = blocks[0].children.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn body_parts_default_to_full_text_and_structure_to_tag() {
        let rec = record(vec![user_part(1, "question"), tool_part(2, "Bash", "ls")]);
        let expanded = ExpandedBlocks::default();
        let input = render(&rec, true, true, &expanded);
        let output = render(&rec, false, true, &expanded);
        // Body block shows its text; the tool block folds to its tag.
        assert_eq!(texts(input), vec!["question"]);
        assert_eq!(texts(output), vec!["<:bash: ls:>"]);
    }

    #[test]
    fn body_block_folds_to_kind_tag_when_flipped() {
        let rec = record(vec![user_part(1, "question")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        assert_eq!(texts(render(&rec, true, true, &expanded)), vec!["<:user:>"]);
    }

    #[test]
    fn raw_mode_shows_every_block_full() {
        let rec = record(vec![user_part(1, "question"), tool_part(2, "Bash", "ls")]);
        let expanded = ExpandedBlocks::default();
        let input = render(&rec, true, false, &expanded);
        let output = render(&rec, false, false, &expanded);
        assert_eq!(texts(input), vec!["question"]);
        assert_eq!(output[0].text, "$ ls");
    }

    #[test]
    fn body_block_body_uses_plain_text() {
        let rec = record(vec![user_part(1, "hello\nworld")]);
        assert_eq!(half_blocks(&rec, true)[0].body(&rec), "hello\nworld");
    }

    #[test]
    fn expanding_a_run_reveals_members_as_folded_lines() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let mut expanded = ExpandedBlocks::default();
        // Folded: the run collapses to its tag.
        let folded = render(&rec, false, true, &expanded);
        assert_eq!(texts(folded), vec!["<:bash, read:>"]);
        // Expanded: the tag stays as the group header, members below it as
        // folded lines, joined without blank lines (tight).
        expanded.toggle(0);
        let shown = render(&rec, false, true, &expanded);
        assert_eq!(
            texts(shown.clone()),
            vec!["<:bash, read:>", "<:bash: ls:>", "<:tool:Read call:>"]
        );
        // Members join on adjacent lines; only the last segment closes the
        // group with a blank line.
        assert!(shown[0].tight && shown[1].tight && !shown[2].tight);
    }

    #[test]
    fn run_member_expands_to_its_own_body() {
        let rec = record(vec![
            tool_part(1, "Bash", "ls"),
            tool_part(2, "Read", "file"),
        ]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0); // run open
        expanded.toggle(1); // first member open
        let shown = render(&rec, false, true, &expanded);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0].text, "<:bash, read:>");
        assert_eq!(shown[1].text, "$ ls");
        assert_eq!(shown[2].text, "<:tool:Read call:>");
    }

    #[test]
    fn block_count_covers_run_members_so_folded_marks_fit_the_mask() {
        // Input user block + an output run of two tool members: the id space
        // is 4 wide, not 2 — a mask sized by top-level blocks alone would
        // drop a mark on a member the moment the run folds.
        let rec = record(vec![
            user_part(1, "question"),
            tool_part(2, "Bash", "ls"),
            tool_part(3, "Read", "file"),
        ]);
        let (input, output) = dialogue_blocks(&rec);
        assert_eq!(dialogue_block_count(&input, &output), 4);
        assert_eq!(input.len(), 1);
        assert_eq!(output[0].children.len(), 2);
        assert_eq!(output[0].children[1].id, 3);
        assert_eq!(dialogue_block_id(&rec, 3), Some(3));
    }

    /// Render one half through the dialogue-global block ids.
    fn render(
        rec: &WorkRecord,
        input: bool,
        reading: bool,
        expanded: &ExpandedBlocks,
    ) -> Vec<BlockText> {
        render_half(rec, &half_blocks(rec, input), reading, expanded)
    }

    /// Segment texts for compact assertions.
    fn texts(segs: Vec<BlockText>) -> Vec<String> {
        segs.into_iter().map(|seg| seg.text).collect()
    }
}
