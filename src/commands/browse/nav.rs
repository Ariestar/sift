//! Cursor movement, list clamps, pane resets, and link open.

use anyhow::Result;
use std::process::Command;

use crate::tui::content::block::BlockText;
use crate::tui::workspace::{
    ContentIoFocus, ContentScrolls, Rows, WorkspaceFocus, WorkspaceSource,
};

pub(super) fn open_link_target(target: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(target).spawn()?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(target).spawn()?;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(target).spawn()?;
    }

    Ok(())
}

/// Keyboard/mouse cursor over content blocks, one position per dialogue
/// across both IO halves (block ids are dialogue-global). `follow` asks the
/// picker to keep the cursor block visible on the next redraw; keyboard
/// moves set it, clicks do not (a clicked line is already visible).
#[derive(Default)]
pub(super) struct ContentBlockCursor {
    pub(super) block: Option<usize>,
    pub(super) follow: bool,
    /// Direction of a step that walked off the shown dialogue's blocks and
    /// moved the dialogue cursor instead. The next dialogue's blocks only
    /// exist once its frame is built, so the redraw lands the cursor.
    crossed: Option<bool>,
}

impl ContentBlockCursor {
    pub(super) fn get(&self) -> Option<usize> {
        self.block
    }

    pub(super) fn set(&mut self, block: usize) {
        self.block = Some(block);
    }

    /// Forget the block: its id named the dialogue that just left the screen.
    /// A pending crossing survives — it is *why* that dialogue left.
    pub(super) fn clear(&mut self) {
        self.block = None;
        self.follow = false;
    }

    /// Land on the end of the shown dialogue's visible blocks a crossing
    /// arrived at: the last block when it stepped up, the first when down.
    /// No visible block (an unhydrated dialogue) leaves the cursor unset.
    pub(super) fn land_crossing(&mut self, blocks: (&[BlockText], &[BlockText])) {
        let Some(up) = self.crossed.take() else {
            return;
        };
        let mut ids = blocks.0.iter().chain(blocks.1).map(|block| block.id);
        if let Some(id) = if up { ids.next_back() } else { ids.next() } {
            self.set(id);
            self.follow = true;
        }
    }

    /// `(half, block)` of the cursor, for the view highlight and block
    /// operations. The half is the one whose displayed segments own the
    /// cursor id; a block hidden by a fold has no half and no highlight.
    pub(super) fn focused(
        &self,
        blocks: (&[BlockText], &[BlockText]),
    ) -> Option<(ContentIoFocus, usize)> {
        let block = self.block?;
        let half = if blocks.0.iter().any(|segment| segment.id == block) {
            ContentIoFocus::Input
        } else if blocks.1.iter().any(|segment| segment.id == block) {
            ContentIoFocus::Output
        } else {
            return None;
        };
        Some((half, block))
    }
}

/// Discard whatever the panes right of `focus` derived from its selection,
/// after that selection changed. `true` when the change reaches back to the
/// sources, so the caller must reload sessions.
pub(super) fn invalidate_panes_below(
    focus: WorkspaceFocus,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
) -> bool {
    match focus {
        WorkspaceFocus::Source => {
            rows.sessions.reset(0);
            rows.dialogues.reset(0);
            rows.close_ranges();
            content_scrolls.clear();
            true
        }
        WorkspaceFocus::Sessions => {
            rows.dialogues.reset(0);
            content_scrolls.clear();
            false
        }
        // Dialogues feed the content pane, which rebuilds from the shown
        // dialogue every redraw; Content has nothing to its right.
        WorkspaceFocus::Dialogues | WorkspaceFocus::Content => false,
    }
}

/// Discard whatever the panes right of `focus` derived from the list row the
/// cursor just left. Both ways a cursor lands on a new row share this rule: a
/// j/k step (and the wheel, which steps) and a click's absolute jump.
///
/// Distinct from [`invalidate_panes_below`], which answers the same question
/// after a *selection* change: a selected pane's list spans every marked row,
/// so moving the cursor inside it changes nothing below.
pub(super) fn invalidate_after_cursor_move(
    focus: WorkspaceFocus,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
) {
    match focus {
        WorkspaceFocus::Sessions => {
            // The dialogue list spans every marked session, so it only
            // follows the cursor row while nothing is marked.
            if !rows.sessions.has_scope() {
                rows.dialogues.reset(0);
            }
            content_scrolls.clear();
        }
        // A different dialogue is on screen: its content starts at the top.
        WorkspaceFocus::Dialogues => content_scrolls.clear(),
        // Source feeds sessions through its selection, not its cursor;
        // Content has no list row.
        WorkspaceFocus::Source | WorkspaceFocus::Content => {}
    }
}

/// Move the focused pane's cursor one row (`up` or down). Every list pane
/// follows one rule: clamp to the row count, and a move that does not change
/// the row does nothing — so bumping the first or last row never resets the
/// panes below it. Content moves its block cursor instead.
pub(super) fn move_workspace_cursor(
    up: bool,
    focus: WorkspaceFocus,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
    content_cursor: &mut ContentBlockCursor,
    content_blocks: (&[BlockText], &[BlockText]),
) {
    if focus == WorkspaceFocus::Content {
        move_content_cursor(up, rows, content_scrolls, content_cursor, content_blocks);
        return;
    }
    let moved = match focus {
        WorkspaceFocus::Dialogues => rows.dialogues.step(up),
        WorkspaceFocus::Source | WorkspaceFocus::Sessions => {
            rows.scope_pane_mut(focus).is_some_and(|pane| pane.step(up))
        }
        WorkspaceFocus::Content => false,
    };
    if moved {
        invalidate_after_cursor_move(focus, rows, content_scrolls);
    }
}

/// Move the content cursor across the whole dialogue: input blocks then
/// output blocks form one continuous sequence, so j/k flows over the half
/// boundary. The walk follows the *visible* block sequence (the rendered
/// segments), so folds — which change which blocks are shown — resolve the
/// cursor id against the current segments and clamp to the nearest visible
/// block.
///
/// Walking off either end steps the dialogue list instead, and the redraw
/// lands the cursor on the end of the new dialogue it arrived at — so one
/// j/k walk covers every dialogue the list holds, across every marked
/// session. An empty dialogue stops the walk: its blocks may still be
/// hydrating, and a step past it would skip content the user never saw.
fn move_content_cursor(
    up: bool,
    rows: &mut Rows,
    content_scrolls: &mut ContentScrolls,
    cursor: &mut ContentBlockCursor,
    blocks: (&[BlockText], &[BlockText]),
) {
    let (input_blocks, output_blocks) = blocks;
    let input_len = input_blocks.len();
    let total = input_len + output_blocks.len();
    if total == 0 {
        return;
    }
    let ids: Vec<_> = input_blocks
        .iter()
        .chain(output_blocks)
        .map(|block| block.id)
        .collect();
    let current_id = cursor.get();
    let position = current_id.and_then(|id| ids.iter().position(|visible| *visible == id));
    let next = match (current_id, position) {
        (None, _) => Some(0),
        (Some(_), Some(pos)) if up => pos.checked_sub(1),
        (Some(_), Some(pos)) => (pos + 1 < total).then_some(pos + 1),
        (Some(id), None) if up => ids.iter().rposition(|visible| *visible < id),
        (Some(id), None) => ids.iter().position(|visible| *visible > id),
    };
    let Some(next) = next else {
        // Off the end of this dialogue: continue into the next one.
        if rows.dialogues.step(up) {
            cursor.crossed = Some(up);
            invalidate_after_cursor_move(WorkspaceFocus::Dialogues, rows, content_scrolls);
        }
        return;
    };
    cursor.set(ids[next]);
    cursor.follow = true;
}

pub(super) fn row_list_index(
    area: ratatui::layout::Rect,
    row: u16,
    len: usize,
    offset: usize,
) -> Option<usize> {
    let row = row.checked_sub(area.y.saturating_add(1))? as usize;
    let index = row.saturating_add(offset);
    (index < len).then_some(index)
}

/// Is `column` inside a list row's selection-dot gutter (rows render
/// `{dot} ` at `area.x`, two columns wide)?
pub(super) fn dot_gutter_hit(area: ratatui::layout::Rect, column: u16) -> bool {
    column <= area.x.saturating_add(1)
}

pub(super) fn source_list_index(
    area: ratatui::layout::Rect,
    column: u16,
    row: u16,
    sources: &[WorkspaceSource],
    vertical: bool,
    offset: usize,
) -> Option<usize> {
    if vertical {
        // List panel: one source per row (same as sessions/dialogues).
        return row_list_index(area, row, sources.len(), offset);
    }
    // Compact strip: single content row, labels laid out left→right.
    if row != area.y.saturating_add(1)
        || column <= area.x
        || column >= area.x.saturating_add(area.width)
    {
        return None;
    }
    let mut cursor = area.x.saturating_add(1);
    for (idx, source) in sources.iter().enumerate() {
        if idx > 0 {
            cursor = cursor.saturating_add(2);
        }
        let width = source.label().len() as u16 + 4;
        if column >= cursor && column < cursor.saturating_add(width) {
            return Some(idx);
        }
        cursor = cursor.saturating_add(width);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{move_content_cursor, row_list_index, ContentBlockCursor};
    use crate::tui::content::block::{BlockRole, BlockText};
    use crate::tui::workspace::{ContentScrolls, Rows};
    use ratatui::layout::Rect;

    fn block(id: usize) -> BlockText {
        BlockText {
            id,
            text: String::new(),
            tight: false,
            role: BlockRole::Shell,
        }
    }

    fn blocks(ids: &[usize]) -> Vec<BlockText> {
        ids.iter().copied().map(block).collect()
    }

    /// Two dialogues, one block each: j off the end of the first steps the
    /// dialogue list, and the next redraw's blocks land the cursor.
    #[test]
    fn a_step_off_the_last_block_crosses_into_the_next_dialogue() {
        let mut rows = Rows::default();
        rows.dialogues.fit(2);
        let mut scrolls = ContentScrolls::default();
        let mut cursor = ContentBlockCursor::default();
        let first = blocks(&[0]);
        let second = blocks(&[7, 8]);

        move_content_cursor(false, &mut rows, &mut scrolls, &mut cursor, (&[], &first));
        assert_eq!(cursor.get(), Some(0), "first step lands on the only block");

        move_content_cursor(false, &mut rows, &mut scrolls, &mut cursor, (&[], &first));
        assert_eq!(rows.dialogues.cursor(), 1, "off the end steps the list");
        cursor.clear();
        cursor.land_crossing((&[], &second));
        assert_eq!(cursor.get(), Some(7), "down lands on the first block");

        // Back up: the cursor walks this dialogue's blocks first, then returns
        // to the end of the previous dialogue.
        move_content_cursor(false, &mut rows, &mut scrolls, &mut cursor, (&[], &second));
        assert_eq!(cursor.get(), Some(8), "down inside the second dialogue");
        move_content_cursor(true, &mut rows, &mut scrolls, &mut cursor, (&[], &second));
        assert_eq!(cursor.get(), Some(7));
        assert_eq!(rows.dialogues.cursor(), 1, "still inside the second");
        move_content_cursor(true, &mut rows, &mut scrolls, &mut cursor, (&[], &second));
        assert_eq!(rows.dialogues.cursor(), 0, "off the top steps back");
        cursor.clear();
        cursor.land_crossing((&[], &first));
        assert_eq!(cursor.get(), Some(0), "up lands on the last block");

        // The list ends here: the step is refused and the cursor stays put.
        move_content_cursor(true, &mut rows, &mut scrolls, &mut cursor, (&[], &first));
        assert_eq!(rows.dialogues.cursor(), 0);
        cursor.land_crossing((&[], &first));
        assert_eq!(cursor.get(), Some(0));
    }

    #[test]
    fn row_list_index_includes_scroll_offset() {
        let area = Rect::new(0, 0, 30, 10);
        // Row 1 is the first content row below the panel title.
        assert_eq!(row_list_index(area, 1, 100, 0), Some(0));
        // A scrolled list maps the clicked row onto the offset index.
        assert_eq!(row_list_index(area, 1, 100, 50), Some(50));
        assert_eq!(row_list_index(area, 4, 100, 50), Some(53));
        // Indices beyond the list end are rejected.
        assert_eq!(row_list_index(area, 4, 100, 98), None);
        // The title row is not a selectable row.
        assert_eq!(row_list_index(area, 0, 100, 0), None);
    }

    #[test]
    fn hidden_cursor_moves_to_the_nearest_visible_block() {
        let blocks = [block(0), block(1), block(4)];
        let mut rows = Rows::default();
        let mut scrolls = ContentScrolls::default();
        let mut cursor = ContentBlockCursor::default();
        cursor.set(3);

        move_content_cursor(true, &mut rows, &mut scrolls, &mut cursor, (&[], &blocks));
        assert_eq!(cursor.get(), Some(1));

        cursor.set(3);
        move_content_cursor(false, &mut rows, &mut scrolls, &mut cursor, (&[], &blocks));
        assert_eq!(cursor.get(), Some(4));
    }
}
