//! Content blocks: every workpart is a foldable block.
//!
//! A block is the smallest unit the content pane highlights, navigates, and
//! folds: one workpart — an action already carries its whole call→result
//! lifecycle, so one part reads as one operation. Consecutive structure
//! blocks fold into one run block that collapses to a single `kind xN` tag;
//! expanding a run reveals its members below the tag, one call per line,
//! each still folded and expandable in turn — two fold levels. Structure
//! blocks (agent evidence) default to their `<:…:>` tag; body blocks
//! (dialogue, human commands) default to their full text — one fold model,
//! no structure-only special cases.

use sivtr_core::record::{
    MessageRole, Projection, WorkActionStatus, WorkPart, WorkPartBody, WorkRecord, WorkTarget,
};

use crate::tui::content::io::ExpandedBlocks;
use crate::tui::content::tool::{part_body_text, tool_display_name, tool_tag_for_part};

/// Display role of a block's first part: drives the dot-gutter color, the
/// same palette the pane uses for roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockRole {
    User,
    Assistant,
    Reasoning,
    System,
    Shell,
    Tool,
    Agent,
    Failed,
}

impl BlockRole {
    pub(crate) fn of(part: &WorkPart) -> Self {
        match &part.body {
            WorkPartBody::Message { role, .. } => match role {
                MessageRole::User => Self::User,
                MessageRole::Assistant => Self::Assistant,
                MessageRole::Reasoning => Self::Reasoning,
                MessageRole::System => Self::System,
            },
            WorkPartBody::Action { target, status, .. } => {
                if matches!(
                    status,
                    WorkActionStatus::Failed | WorkActionStatus::Cancelled
                ) {
                    return Self::Failed;
                }
                match target {
                    WorkTarget::Shell => Self::Shell,
                    WorkTarget::Agent { .. } => Self::Agent,
                    WorkTarget::Tool { .. } | WorkTarget::Mcp { .. } => Self::Tool,
                }
            }
        }
    }
}

/// A foldable content block: the parts it owns, plus — for runs — the
/// member blocks revealed when the run is expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    /// Stable identity within the dialogue (DFS pre-order over the input
    /// half then the output half), used by the fold state, the marks, and
    /// the content cursor; id 0 is the dialogue's first block.
    pub(crate) id: usize,
    /// Indices into the record's parts, in display order.
    pub(crate) parts: Vec<usize>,
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
    /// Role of the block's first part; drives the dot-gutter color.
    pub(crate) role: BlockRole,
}

impl Block {
    /// A leaf (non-run) block; ids are assigned by `assign_ids` afterwards.
    fn leaf(parts: Vec<usize>) -> Self {
        Block {
            id: 0,
            parts,
            children: Vec::new(),
        }
    }

    /// This block plus every nested member — the ids `assign_ids` consumes.
    fn node_count(&self) -> usize {
        1 + self.children.iter().map(Block::node_count).sum::<usize>()
    }

    /// Full body of a leaf: every part rendered in the projection's slice,
    /// members joining on adjacent lines.
    pub(crate) fn body(&self, record: &WorkRecord, projection: Projection) -> String {
        self.parts
            .iter()
            .map(|&idx| {
                let part = &record.parts[idx];
                part_body_text(part, projection.slice_of(part))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Collapsed tag: `<:kind xN:>` for a run. Members list every kind in
    /// order with its count, repeats collapsed to `kind xN` and singles as
    /// the bare kind — `<:shell, thinking, read:>` — instead of a `+`
    /// mashup.
    pub(crate) fn fold_label(&self, record: &WorkRecord) -> String {
        if !self.children.is_empty() {
            let mut kinds: Vec<(String, usize)> = Vec::new();
            for child in &self.children {
                let name = part_display_name(&record.parts[child.parts[0]]);
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

/// Short display name of one part: actions by target (`shell`, the tool,
/// `server: tool`, the sub-agent), messages by their label or role — the
/// run-tag vocabulary.
fn part_display_name(part: &WorkPart) -> String {
    match &part.body {
        WorkPartBody::Action { target, .. } => match target {
            WorkTarget::Shell => "shell".to_string(),
            WorkTarget::Tool { name } => name
                .as_deref()
                .map(tool_display_name)
                .unwrap_or_else(|| "tool".to_string()),
            WorkTarget::Mcp { server, tool } => format!("{server}: {tool}"),
            WorkTarget::Agent { name } => name.clone(),
        },
        WorkPartBody::Message { role, label, .. } => label
            .clone()
            .unwrap_or_else(|| role_name(*role).to_string()),
    }
}

fn role_name(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
        MessageRole::Reasoning => "thinking",
    }
}

/// Partition one record into the input and output halves' blocks, in
/// display order: the input projection's parts and the output projection's
/// parts, each one part per block with consecutive structure parts folded
/// into one run. Runs get stable DFS pre-order ids later, over the whole
/// dialogue, so the fold state and cursor survive folds and can move
/// continuously across the input/output boundary.
pub(crate) fn dialogue_blocks(record: &WorkRecord) -> (Vec<Block>, Vec<Block>) {
    let mut input = build_half(record, Projection::Input);
    let mut output = build_half(record, Projection::Output);
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

fn build_half(record: &WorkRecord, projection: Projection) -> Vec<Block> {
    let index_by_seq = record
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| (part.seq, index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut blocks: Vec<Block> = Vec::new();
    for part in record.project(projection) {
        let index = index_by_seq[&part.seq];
        let unit = Block::leaf(vec![index]);
        // Consecutive agent-side evidence folds into one presentation run;
        // body content always stands alone.
        let merges = part.is_structure()
            && blocks
                .last()
                .is_some_and(|last| record.parts[last.parts[0]].is_structure());
        if merges {
            let last = blocks.last_mut().expect("run block exists");
            if last.children.is_empty() {
                // Promote the leaf into a run holding itself as first member.
                last.children.push(Block::leaf(last.parts.clone()));
            }
            last.parts.push(index);
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

/// Collapsed tag for one part: the per-tool tag (`<:read: path:>`,
/// `<:shell: ls:>`) when the action's shape is understood, otherwise the
/// action's description or generic marker, otherwise the role tag for
/// messages.
pub(crate) fn fold_label_for_part(part: &WorkPart) -> String {
    if let Some(tag) = tool_tag_for_part(part) {
        return tag;
    }
    match &part.body {
        WorkPartBody::Action { target, output, .. } => {
            let base = match target {
                WorkTarget::Shell => "<:shell:>".to_string(),
                _ => {
                    let kind = if output.is_empty() {
                        sivtr_core::agents::AgentBlockKind::ToolCall
                    } else {
                        sivtr_core::agents::AgentBlockKind::ToolOutput
                    };
                    kind.open_marker(part.label())
                        .unwrap_or_else(|| "<:action:>".to_string())
                }
            };
            match tool_description(part) {
                Some(description) => match base.strip_suffix(":>") {
                    Some(stem) => format!("{stem}: {description}:>"),
                    None => base,
                },
                None => base,
            }
        }
        WorkPartBody::Message { role, .. } => format!("<:{}:>", role_name(*role)),
    }
}

/// Human description from an action's context line, truncated to fit the
/// tag line.
fn tool_description(part: &WorkPart) -> Option<String> {
    let WorkPartBody::Action { title, .. } = &part.body else {
        return None;
    };
    let title = title.as_deref()?;
    // Normalize internal whitespace so a multi-line description still folds
    // to a single tag line (block layout assumes one line per tag).
    let description: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return None;
    }
    const MAX: usize = 40;
    Some(crate::tui::content::truncate_chars(&description, MAX))
}

/// Render one half's blocks to their display segments, in display order:
/// a block's full body when shown, its collapsed tag otherwise. Runs always
/// show the aggregate tag; expanding a run reveals its members below it,
/// each still folded, joined on adjacent lines. `blocks` comes from
/// [`dialogue_blocks`], so ids are dialogue-global.
pub(crate) fn render_half(
    record: &WorkRecord,
    blocks: &[Block],
    projection: Projection,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> Vec<BlockText> {
    let mut out = Vec::new();
    for block in blocks {
        out.extend(render_block(record, block, projection, reading, expanded));
    }
    out
}

fn render_block(
    record: &WorkRecord,
    block: &Block,
    projection: Projection,
    reading: bool,
    expanded: &ExpandedBlocks,
) -> Vec<BlockText> {
    let mut segs = Vec::new();
    if !reading {
        // Raw mode: every block shows its full body; runs expand flat.
        if block.children.is_empty() {
            segs.push(BlockText {
                id: block.id,
                text: block.body(record, projection),
                tight: false,
                role: BlockRole::of(&record.parts[block.parts[0]]),
            });
        } else {
            for child in &block.children {
                segs.extend(render_block(record, child, projection, reading, expanded));
            }
        }
    } else if block.children.is_empty() {
        // Leaf: body or collapsed tag by the block's fold default.
        let shown = expanded.expanded(block.id, record.parts[block.parts[0]].is_structure());
        segs.push(BlockText {
            id: block.id,
            text: if shown {
                block.body(record, projection)
            } else {
                block.fold_label(record)
            },
            tight: false,
            role: BlockRole::of(&record.parts[block.parts[0]]),
        });
    } else {
        // Run: the aggregate tag stays as the group header; expanding the
        // run reveals its members below it, each still folded.
        let shown = expanded.expanded(block.id, true);
        segs.push(BlockText {
            id: block.id,
            text: block.fold_label(record),
            tight: false, // rewritten below: all but the last segment join tight
            role: BlockRole::of(&record.parts[block.parts[0]]),
        });
        if shown {
            for child in &block.children {
                segs.extend(render_block(record, child, projection, reading, expanded));
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

    fn shell_action(seq: usize, command: &str, output: &str, exit: Option<i32>) -> WorkPart {
        let mut part = crate::test_fixtures::shell_action_part(
            seq,
            command,
            (!output.is_empty()).then_some(output),
        );
        if let WorkPartBody::Action {
            id,
            status,
            exit_code,
            ..
        } = &mut part.body
        {
            *id = format!("a{seq}");
            *status = match exit {
                Some(0) | None => WorkActionStatus::Completed,
                Some(_) => WorkActionStatus::Failed,
            };
            *exit_code = exit;
        }
        part
    }

    fn tool_action(seq: usize, tool: &str, path: &str, output: Option<&str>) -> WorkPart {
        crate::test_fixtures::tool_action_part(
            seq,
            &format!("a{seq}"),
            Some(tool),
            Some(serde_json::json!({ "file_path": path })),
            output.map(|text| serde_json::json!({ "stdout": text })),
        )
    }

    fn user_message(seq: usize, content: &str) -> WorkPart {
        crate::test_fixtures::message_part(seq, MessageRole::User, content)
    }

    fn thinking_message(seq: usize, content: &str) -> WorkPart {
        crate::test_fixtures::message_part(seq, MessageRole::Reasoning, content)
    }

    fn assistant_message(seq: usize, content: &str) -> WorkPart {
        crate::test_fixtures::message_part(seq, MessageRole::Assistant, content)
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
    fn shell_call_and_result_render_as_one_block() {
        let rec = record(vec![
            user_message(1, "question"),
            shell_action(2, "ls", "ok", Some(0)),
        ]);
        let blocks = half_blocks(&rec, false);
        // The output half holds the whole shell action; one leaf.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].parts, vec![1]);
        assert!(blocks[0].children.is_empty());
    }

    #[test]
    fn consecutive_agent_actions_fold_to_one_run() {
        let rec = record(vec![
            shell_action(1, "ls", "", None),
            tool_action(2, "Read", "file", None),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 2);
        assert_eq!(blocks[0].fold_label(&rec), "<:shell, read:>");
    }

    #[test]
    fn thinking_folds_into_the_same_run() {
        let rec = record(vec![
            shell_action(1, "ls", "", None),
            thinking_message(2, "reasoning"),
            tool_action(3, "Read", "file", None),
        ]);
        let blocks = half_blocks(&rec, false);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].children.len(), 3);
        assert_eq!(blocks[0].fold_label(&rec), "<:shell, thinking, read:>");
        // A body part after the series starts a new block in the same
        // (output) half: the run keeps its members, the body joins after.
        let rec = record(vec![
            shell_action(1, "ls", "", None),
            thinking_message(2, "reasoning"),
            assistant_message(3, "answer"),
        ]);
        assert_eq!(half_blocks(&rec, false).len(), 2);
        assert!(half_blocks(&rec, true).is_empty());
    }

    #[test]
    fn ids_follow_dfs_preorder() {
        let rec = record(vec![
            shell_action(1, "ls", "", None),
            thinking_message(2, "reasoning"),
            tool_action(3, "Read", "file", None),
        ]);
        let blocks = half_blocks(&rec, false);
        // run id 0, members 1..=3.
        assert_eq!(blocks[0].id, 0);
        let ids: Vec<usize> = blocks[0].children.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn body_parts_default_to_full_text_and_structure_to_tag() {
        let rec = record(vec![
            user_message(1, "question"),
            shell_action(2, "ls", "", None),
        ]);
        let expanded = ExpandedBlocks::default();
        let input = render(&rec, true, true, &expanded);
        let output = render(&rec, false, true, &expanded);
        // Body block shows its text; the agent shell action folds to its tag.
        assert_eq!(texts(input), vec!["question"]);
        assert_eq!(texts(output), vec!["<:shell: ls:>"]);
    }

    #[test]
    fn body_block_folds_to_kind_tag_when_flipped() {
        let rec = record(vec![user_message(1, "question")]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0);
        assert_eq!(texts(render(&rec, true, true, &expanded)), vec!["<:user:>"]);
    }

    #[test]
    fn raw_mode_shows_every_block_full() {
        let rec = record(vec![
            user_message(1, "question"),
            shell_action(2, "ls", "", None),
        ]);
        let expanded = ExpandedBlocks::default();
        let input = render(&rec, true, false, &expanded);
        let output = render(&rec, false, false, &expanded);
        assert_eq!(texts(input), vec!["question"]);
        assert_eq!(output[0].text, "$ ls");
    }

    #[test]
    fn body_block_body_uses_plain_text() {
        let rec = record(vec![user_message(1, "hello\nworld")]);
        assert_eq!(
            half_blocks(&rec, true)[0].body(&rec, Projection::Input),
            "hello\nworld"
        );
    }

    #[test]
    fn expanding_a_run_reveals_members_as_folded_lines() {
        let rec = record(vec![
            shell_action(1, "ls", "", None),
            tool_action(2, "Read", "file", None),
        ]);
        let mut expanded = ExpandedBlocks::default();
        // Folded: the run collapses to its tag.
        let folded = render(&rec, false, true, &expanded);
        assert_eq!(texts(folded), vec!["<:shell, read:>"]);
        // Expanded: the tag stays as the group header, members below it as
        // folded lines, joined without blank lines (tight).
        expanded.toggle(0);
        let shown = render(&rec, false, true, &expanded);
        assert_eq!(
            texts(shown.clone()),
            vec!["<:shell, read:>", "<:shell: ls:>", "<:read: file:>"]
        );
        // Members join on adjacent lines; only the last segment closes the
        // group with a blank line.
        assert!(shown[0].tight && shown[1].tight && !shown[2].tight);
    }

    #[test]
    fn run_member_expands_to_its_own_body() {
        let rec = record(vec![
            shell_action(1, "ls", "ok", Some(0)),
            tool_action(2, "Read", "file", None),
        ]);
        let mut expanded = ExpandedBlocks::default();
        expanded.toggle(0); // run open
        expanded.toggle(1); // first member open
        let shown = render(&rec, false, true, &expanded);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0].text, "<:shell, read:>");
        assert_eq!(shown[1].text, "$ ls\nok");
        assert_eq!(shown[2].text, "<:read: file:>");
    }

    #[test]
    fn block_count_covers_run_members_so_folded_marks_fit_the_mask() {
        // Input user block + an output run of two tool members: the id space
        // is 4 wide, not 2 — a mask sized by top-level blocks alone would
        // drop a mark on a member the moment the run folds.
        let rec = record(vec![
            user_message(1, "question"),
            shell_action(2, "ls", "", None),
            tool_action(3, "Read", "file", None),
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
        let projection = if input {
            Projection::Input
        } else {
            Projection::Output
        };
        render_half(rec, &half_blocks(rec, input), projection, reading, expanded)
    }

    /// Segment texts for compact assertions.
    fn texts(segs: Vec<BlockText>) -> Vec<String> {
        segs.into_iter().map(|seg| seg.text).collect()
    }
}
