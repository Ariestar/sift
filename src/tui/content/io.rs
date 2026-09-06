//! Dual Input/Output content panes: text bodies, layout, and active-half access.
//!
//! One place owns empty placeholders, dynamic height split, and search→scroll mapping
//! so picker / render / help don't re-copy the same match arms.

use ratatui::layout::Rect;
use std::collections::HashSet;

use crate::tui::content::block::BlockText;
use crate::tui::content::view::{
    content_view_line_count, layout_content, ContentLayout, ContentViewMode,
};

const EMPTY: &str = "<empty>";
/// Min pane height: top border + 1 content row + bottom border.
const MIN_PANE_H: u16 = 3;

/// Body text for one IO half (no section headers — panes own titles).
/// `input` / `output` are the per-block segments joined with a blank line
/// (single line between members of the same run); the segments stay
/// available so the content pane can map displayed lines back to their
/// owning block (highlight / hit-test / navigation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentIoTexts {
    pub(crate) input: String,
    pub(crate) output: String,
    /// Display segments of the input half (tag or body), in order.
    pub(crate) input_blocks: Vec<BlockText>,
    /// Display segments of the output half (tag or body), in order.
    pub(crate) output_blocks: Vec<BlockText>,
}

impl ContentIoTexts {
    /// Build from per-block segments; the pane text is the segments joined
    /// with a blank line between blocks (the block separator also drives the
    /// content layout's line ownership), single newline within a run.
    pub(crate) fn new(input_blocks: Vec<BlockText>, output_blocks: Vec<BlockText>) -> Self {
        ContentIoTexts {
            input: join_blocks(&input_blocks),
            input_blocks,
            output: join_blocks(&output_blocks),
            output_blocks,
        }
    }

    pub(crate) fn join_displayed(&self) -> String {
        match (self.input_blank(), self.output_blank()) {
            (true, true) => EMPTY.to_string(),
            (false, true) => self.input.clone(),
            (true, false) => self.output.clone(),
            (false, false) => format!("{}\n\n{}", self.input, self.output),
        }
    }

    pub(crate) fn input_blank(&self) -> bool {
        self.input.trim().is_empty()
    }

    pub(crate) fn output_blank(&self) -> bool {
        self.output.trim().is_empty()
    }

    /// Text shown in a half pane (`<empty>` when blank).
    pub(crate) fn display(&self, half: ContentIoFocus) -> &str {
        let raw = match half {
            ContentIoFocus::Input => self.input.as_str(),
            ContentIoFocus::Output => self.output.as_str(),
        };
        if raw.trim().is_empty() {
            EMPTY
        } else {
            raw
        }
    }

    /// One half's block segments, for fold-aware cursor movement.
    pub(crate) fn half_blocks(&self, half: ContentIoFocus) -> &[BlockText] {
        match half {
            ContentIoFocus::Input => &self.input_blocks,
            ContentIoFocus::Output => &self.output_blocks,
        }
    }

    /// Block segments backing the *displayed* text of a half: empty when
    /// the half renders `<empty>`, so the layout's lines and ownership
    /// always derive from the same state and the dot gutter stays aligned.
    pub(crate) fn display_blocks(&self, half: ContentIoFocus) -> &[BlockText] {
        let blocks = self.half_blocks(half);
        match half {
            ContentIoFocus::Input if self.input_blank() => &[],
            ContentIoFocus::Output if self.output_blank() => &[],
            _ => blocks,
        }
    }

    /// The two halves' block segments, for fold-aware cursor movement.
    pub(crate) fn block_slices(&self) -> (&[BlockText], &[BlockText]) {
        (
            self.half_blocks(ContentIoFocus::Input),
            self.half_blocks(ContentIoFocus::Output),
        )
    }
}

/// Join block segments: a blank line between blocks, a single newline
/// between members of the same run (`tight`).
fn join_blocks(blocks: &[BlockText]) -> String {
    let mut text = String::new();
    for (idx, segment) in blocks.iter().enumerate() {
        text.push_str(&segment.text);
        if idx + 1 < blocks.len() {
            text.push_str(if segment.tight { "\n" } else { "\n\n" });
        }
    }
    text
}

/// Which content half keyboard / selection targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentIoFocus {
    #[default]
    Input,
    Output,
}

/// Independent scroll offsets for the dual content panes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentScrolls {
    pub(crate) input: usize,
    pub(crate) output: usize,
}

impl ContentScrolls {
    pub(crate) fn get(self, focus: ContentIoFocus) -> usize {
        match focus {
            ContentIoFocus::Input => self.input,
            ContentIoFocus::Output => self.output,
        }
    }

    pub(crate) fn set(&mut self, focus: ContentIoFocus, value: usize) {
        match focus {
            ContentIoFocus::Input => self.input = value,
            ContentIoFocus::Output => self.output = value,
        }
    }

    pub(crate) fn get_mut(&mut self, focus: ContentIoFocus) -> &mut usize {
        match focus {
            ContentIoFocus::Input => &mut self.input,
            ContentIoFocus::Output => &mut self.output,
        }
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn clamp_to(&mut self, input_lines: usize, output_lines: usize) {
        self.input = self.input.min(input_lines.saturating_sub(1));
        self.output = self.output.min(output_lines.saturating_sub(1));
    }
}

/// Which blocks show their full body instead of their `<:…:>` tag, per
/// dialogue. Structure blocks (tool/skill/thinking, including runs) default
/// to collapsed, body blocks default to expanded; a block in the set flips
/// the kind default (structure expanded, body collapsed). Raw mode always
/// shows full blocks and ignores this state. A block id is the block's
/// stable DFS id within the dialogue — stable because ids come from the
/// block tree, not the rendered segment count, and unique across both IO
/// halves so the fold state spans the input/output boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpandedBlocks(HashSet<usize>);

impl ExpandedBlocks {
    /// Whether `block` shows its full body.
    pub(crate) fn expanded(&self, block: usize, structure: bool) -> bool {
        if structure {
            self.0.contains(&block)
        } else {
            !self.0.contains(&block)
        }
    }

    pub(crate) fn toggle(&mut self, block: usize) {
        if !self.0.insert(block) {
            self.0.remove(&block);
        }
    }

    /// Drop every flip (the shown dialogue changed).
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

/// Geometry of the dual content panes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ContentIoAreas {
    pub(crate) input: Rect,
    pub(crate) output: Rect,
}

impl ContentIoAreas {
    pub(crate) fn area(self, half: ContentIoFocus) -> Rect {
        match half {
            ContentIoFocus::Input => self.input,
            ContentIoFocus::Output => self.output,
        }
    }

    pub(crate) fn hit_test(self, column: u16, row: u16) -> Option<ContentIoFocus> {
        if rect_contains(self.input, column, row) {
            Some(ContentIoFocus::Input)
        } else if rect_contains(self.output, column, row) {
            Some(ContentIoFocus::Output)
        } else {
            None
        }
    }
}

/// Active half: area + display text + scroll slot.
pub(crate) struct ActiveHalf<'a> {
    pub(crate) area: Rect,
    pub(crate) text: &'a str,
    pub(crate) scroll: &'a mut usize,
}

/// Owned view of both halves for one frame: texts, areas, and the cached
/// per-half layouts (rebuilt only when the content changes; scroll reuses
/// them, so wheel events never re-lay-out the text).
#[derive(Clone, Default)]
pub(crate) struct ContentIoFrame {
    pub(crate) texts: ContentIoTexts,
    pub(crate) areas: ContentIoAreas,
    pub(crate) input_layout: ContentLayout,
    pub(crate) output_layout: ContentLayout,
}

impl ContentIoFrame {
    pub(crate) fn build(
        area: Rect,
        texts: ContentIoTexts,
        mode: ContentViewMode,
        focus: ContentIoFocus,
    ) -> Self {
        let areas = content_io_layout(area, &texts, mode, focus);
        let input_layout = layout_content(
            areas.input,
            texts.display(ContentIoFocus::Input),
            texts.display_blocks(ContentIoFocus::Input),
            mode,
        );
        let output_layout = layout_content(
            areas.output,
            texts.display(ContentIoFocus::Output),
            texts.display_blocks(ContentIoFocus::Output),
            mode,
        );
        Self {
            texts,
            areas,
            input_layout,
            output_layout,
        }
    }

    pub(crate) fn layout(&self, half: ContentIoFocus) -> &ContentLayout {
        match half {
            ContentIoFocus::Input => &self.input_layout,
            ContentIoFocus::Output => &self.output_layout,
        }
    }

    pub(crate) fn line_count(&self, half: ContentIoFocus) -> usize {
        self.layout(half).lines.len().max(1)
    }

    pub(crate) fn active<'a>(
        &'a self,
        half: ContentIoFocus,
        scrolls: &'a mut ContentScrolls,
    ) -> ActiveHalf<'a> {
        ActiveHalf {
            area: self.areas.area(half),
            text: self.texts.display(half),
            scroll: scrolls.get_mut(half),
        }
    }
}

/// Focus bias: active half weight multiplier (on top of line-count weight).
const FOCUS_WEIGHT: u32 = 3;
/// Focused half always gets at least this share of total height (percent).
const FOCUS_MIN_SHARE_PCT: u16 = 55;

/// Split content column by display-line weight **and** active-half focus bias.
///
/// Provisional 50/50 measures line counts at shared width; final heights use
/// `weight = lines * (FOCUS_WEIGHT if focused else 1)`, then clamp so the
/// focused half is never below `FOCUS_MIN_SHARE_PCT` of total height.
pub(crate) fn content_io_layout(
    area: Rect,
    texts: &ContentIoTexts,
    mode: ContentViewMode,
    focus: ContentIoFocus,
) -> ContentIoAreas {
    if area.height == 0 || area.width == 0 {
        return ContentIoAreas::default();
    }

    let provisional = split_vertical_equal(area);
    let in_lines = content_view_line_count(
        provisional.input,
        texts.display(ContentIoFocus::Input),
        mode,
    )
    .max(1);
    let out_lines = content_view_line_count(
        provisional.output,
        texts.display(ContentIoFocus::Output),
        mode,
    )
    .max(1);

    let top = weighted_top_height(area.height, in_lines, out_lines, focus);
    ContentIoAreas {
        input: split_top(area, top),
        output: split_bottom(area, top),
    }
}

fn half_weight(lines: usize, half: ContentIoFocus, focus: ContentIoFocus) -> u32 {
    let base = lines.max(1) as u32;
    if half == focus {
        base.saturating_mul(FOCUS_WEIGHT)
    } else {
        base
    }
}

fn weighted_top_height(
    total: u16,
    in_lines: usize,
    out_lines: usize,
    focus: ContentIoFocus,
) -> u16 {
    if total == 0 {
        return 0;
    }
    let in_w = half_weight(in_lines, ContentIoFocus::Input, focus);
    let out_w = half_weight(out_lines, ContentIoFocus::Output, focus);
    let sum = in_w.saturating_add(out_w).max(1);

    let mut top = if total < MIN_PANE_H.saturating_mul(2) {
        ((total as u32) * in_w / sum)
            .max(1)
            .min(total.saturating_sub(1).max(1) as u32) as u16
    } else {
        let rem = total.saturating_sub(MIN_PANE_H.saturating_mul(2));
        let extra = (rem as u32) * in_w / sum;
        MIN_PANE_H.saturating_add(extra as u16)
    };

    // Floor: focused half always gets a usable share (Tab to Input never leaves 1 row).
    if total >= MIN_PANE_H.saturating_mul(2) {
        let focus_min = ((total as u32) * (FOCUS_MIN_SHARE_PCT as u32) / 100) as u16;
        let focus_min = focus_min
            .max(MIN_PANE_H)
            .min(total.saturating_sub(MIN_PANE_H));
        top = match focus {
            ContentIoFocus::Input => top.max(focus_min),
            ContentIoFocus::Output => top.min(total.saturating_sub(focus_min)),
        };
    }
    top
}

fn split_vertical_equal(area: Rect) -> ContentIoAreas {
    let mid = weighted_top_height(area.height, 1, 1, ContentIoFocus::Input);
    ContentIoAreas {
        input: split_top(area, mid),
        output: split_bottom(area, mid),
    }
}

fn split_top(area: Rect, height: u16) -> Rect {
    let h = height.min(area.height);
    Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: h,
    }
}

fn split_bottom(area: Rect, top_height: u16) -> Rect {
    let top = top_height.min(area.height);
    Rect {
        x: area.x,
        y: area.y.saturating_add(top),
        width: area.width,
        height: area.height.saturating_sub(top),
    }
}

/// Map a search hit to (half, 0-based scroll in that half's displayed text).
pub(crate) fn search_match_half(input: bool, matched_line: usize) -> (ContentIoFocus, usize) {
    let half = if input {
        ContentIoFocus::Input
    } else {
        ContentIoFocus::Output
    };
    (half, matched_line.saturating_sub(1))
}

pub(crate) fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::content::block::BlockRole;

    fn seg(text: &str) -> BlockText {
        BlockText {
            id: 0,
            text: text.to_string(),
            tight: false,
            role: BlockRole::Tool,
        }
    }

    #[test]
    fn display_uses_trim_for_empty() {
        let texts = ContentIoTexts::new(vec![seg("  \n")], vec![seg("ok")]);
        assert!(texts.input_blank());
        assert_eq!(texts.display(ContentIoFocus::Input), EMPTY);
        assert_eq!(texts.display(ContentIoFocus::Output), "ok");
    }

    #[test]
    fn join_uses_blank_lines_between_blocks_and_single_within_a_run() {
        let texts = ContentIoTexts::new(
            vec![
                seg("a"),
                BlockText {
                    id: 1,
                    text: "b".to_string(),
                    tight: true,
                    role: BlockRole::Tool,
                },
                BlockText {
                    id: 2,
                    text: "c".to_string(),
                    tight: false,
                    role: BlockRole::Tool,
                },
                seg("d"),
            ],
            Vec::new(),
        );
        assert_eq!(texts.input, "a\n\nb\nc\n\nd");
    }

    #[test]
    fn weighted_height_gives_more_to_heavier_half() {
        let h = weighted_top_height(40, 10, 2, ContentIoFocus::Input);
        assert!(h > 20);
        assert!(h <= 40 - MIN_PANE_H);
    }

    #[test]
    fn focused_half_gets_bias_over_line_weight() {
        // Output has far more lines; Input focused → still ≥ 55% floor.
        let top = weighted_top_height(40, 1, 20, ContentIoFocus::Input);
        assert!(top >= 22); // 55% of 40
                            // Flip focus → Output gets the floor (Input top ≤ 45%).
        let top_out = weighted_top_height(40, 1, 20, ContentIoFocus::Output);
        assert!(top_out <= 18);
        assert!(top_out < top);
    }

    #[test]
    fn search_part_routes_by_kind_direction() {
        let (half, scroll) = search_match_half(false, 2);
        assert_eq!(half, ContentIoFocus::Output);
        assert_eq!(scroll, 1);

        let (half, scroll) = search_match_half(true, 1);
        assert_eq!(half, ContentIoFocus::Input);
        assert_eq!(scroll, 0);
    }
}
