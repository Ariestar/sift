use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::widgets::ListState;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::tui::content::view::{
    content_block_at, content_block_range, content_dot_at, content_link_at, content_position_at,
    content_row_in_text, ContentViewMode,
};
use crate::tui::terminal::read_interaction;
use crate::tui::workspace::{
    active_rows, help_action_for_key, panel_inner_rows, render_workspace, search_match_half,
    selected_index, workspace_help_entries, workspace_hit_test, workspace_layout, ContentIoFocus,
    ContentIoFrame, ContentScrolls, ExpandedBlocks, ListPane, Rows, WorkspaceDialogue,
    WorkspaceFocus, WorkspaceHelpAction, WorkspacePublishView, WorkspaceSearchView,
    WorkspaceSession, WorkspaceSource, WorkspaceView,
};

use super::content::{
    handle_line_filter_key, handle_line_filter_paste, line_filter_spec,
    workspace_picked_content_for_selected_parts,
};
use super::help::{apply_workspace_help_action, set_focus, toggle_list_row, HelpDispatch};
use super::load::{SessionColumn, SessionCtx, SourceLoadState};
use super::nav::{
    dot_gutter_hit, invalidate_after_cursor_move, invalidate_panes_below, move_workspace_cursor,
    open_link_target, row_list_index, source_list_index, ContentBlockCursor,
};
use super::panes::{ContentCtx, ContentPane, DialogueCtx, DialoguePane};
use super::publish_overlay::{self, OverlayFocus, OverlayKey, PublishOverlay};
use super::search::{active_workspace_content_at, workspace_search_target_ref, WorkspaceSearch};
use super::selection::{refresh_next_level, toggle_block_selection, toggle_row_selection};
use super::visual::{
    apply_workspace_mouse_scroll, handle_content_mouse_select, handle_visual_select_key,
    MouseSelectionStart, VisualContentContext, VisualSelectMode, MOUSE_SCROLL_LINES,
};
use super::{PickerMode, PickerResult, PICK_CANCELLED_MESSAGE};
use crate::pane::{Pane, PaneInput, Viewport};

fn session_record_snapshot(
    sessions: &[WorkspaceSession],
    sessions_pane: &SessionColumn,
    search: &WorkspaceSearch,
) -> Vec<Vec<sivtr_core::record::WorkRecord>> {
    sessions
        .iter()
        .map(|session| {
            search.records_for(session).map_or_else(
                || {
                    sessions_pane
                        .body_for(session)
                        .map_or_else(Vec::new, ToOwned::to_owned)
                },
                ToOwned::to_owned,
            )
        })
        .collect()
}

pub(crate) fn run(
    terminal: &mut crate::tui::terminal::Tui,
    sources: Vec<WorkspaceSource>,
    source_states: Vec<SourceLoadState>,
    source_scope: Vec<bool>,
    cwd: PathBuf,
    initial_focus: WorkspaceFocus,
    options: &super::PickerOptions,
) -> Result<PickerResult> {
    debug_assert_eq!(sources.len(), source_scope.len());
    debug_assert_eq!(sources.len(), source_states.len());
    // One cursor + mark set + range anchor per list pane. The source presets
    // arrive as a mask, so that pane starts from it.
    let mut rows = Rows::default();
    rows.source = ListPane::from_scope(source_scope);
    let mut help_state = ListState::default();
    help_state.select(Some(0));
    let mut focus = initial_focus;
    // Windowed panes implement crate::pane::Pane: construct + poll/ensure
    // with PaneInput, no special picker branches. The source catalog is the
    // static `sources` slice; content renders one dialogue, so neither is one.
    let mut sessions_pane = SessionColumn::new(sources.clone(), source_states, cwd.clone());
    let mut dialogue_pane = DialoguePane::default();
    let mut content_pane = ContentPane::default();
    let bootstrap = Viewport {
        first: 0,
        visible: 24,
    };
    sessions_pane.kick(rows.source.scope_mask(), bootstrap, true);
    // Meta-only list — dialogue bodies live in SessionColumn, not here.
    let mut all_sessions = sessions_pane.collect(rows.source.scope_mask());
    let mut sessions = all_sessions.clone();
    let mut sessions_dirty = false;
    rows.source.fit(sources.len());
    rows.sessions.fit(sessions.len());
    let mut content_scrolls = ContentScrolls::default();
    let mut content_io_focus = ContentIoFocus::Input;
    let mut content_mode = ContentViewMode::Reading;
    let mut expanded_blocks = ExpandedBlocks::default();
    let mut expanded_key = None;
    // Content frame inputs of the last rebuild; scroll reuses the cached
    // layouts (scroll only slices), so wheel events never re-lay-out text.
    let mut last_content_key = None;
    // Block multi-select lives in the content pane (native pane selection);
    // cleared with the fold state whenever the shown dialogue changes.
    // Block under a pending click; toggled on mouse release unless a drag
    // turned the gesture into a text selection.
    let mut pending_block_toggle: Option<(ContentIoFocus, usize)> = None;
    // Last single click on a block (time + target); a second click on the
    // same block within the window folds it.
    let mut last_block_click: Option<(std::time::Instant, usize)> = None;
    // Block cursor (keyboard j/k + click), highlighted like a list row; one
    // position per dialogue, spanning both IO halves.
    let mut content_cursor = ContentBlockCursor::default();
    // Mouse-down anchor inside content; promoted to a real selection only by
    // the first drag, so a pure click never shows a selection flash.
    let mut mouse_down_select: Option<MouseSelectionStart> = None;
    let mut show_help = false;
    let mut publish_overlay: Option<PublishOverlay> = None;
    let mut publish_error: Option<String> = None;
    let mut search = WorkspaceSearch::new();
    let mut line_filter_input_open = false;
    let mut line_filter = String::new();
    let mut line_filter_error: Option<String> = None;
    let mut fullscreen = None;
    let mut visual_select_mode = None;
    let mut loading_tick = 0u8;
    // Redraw when an event changed state, a background load landed, or the
    // loading spinner ticked. When idle with no load in flight, block on
    // input instead of redrawing the whole frame at a fixed rate.
    let mut redraw = true;
    // Last-rendered content values, reused by event handlers on iterations
    // that skip redrawing (idle with no state change).
    let mut dialogues: Vec<WorkspaceDialogue> = Vec::new();
    let mut content_frame = ContentIoFrame::default();
    // (engine generation, dialogue selection, focused index) for the projection in
    // `dialogues`; unchanged redraws reuse it instead of re-cloning bodies.
    let mut dialogues_key: Option<(u64, Vec<bool>, usize)> = None;

    loop {
        // ── Unified pane poll/ensure ───────────────────────────────────────
        let mut reproject = false;
        if sessions_pane.poll() {
            sessions_dirty = true;
            redraw = true;
        }
        if sessions_dirty {
            all_sessions = sessions_pane.collect(rows.source.scope_mask());
            sessions_dirty = false;
            reproject = true;
        }
        if search.poll(&cwd) {
            redraw = true;
            reproject = true;
        }
        let search_has_query = search.query_active();
        if search_has_query {
            sessions = search.output().sessions.clone();
        } else if reproject {
            sessions = all_sessions.clone();
        }
        // The result set can change shape under the cursor between frames;
        // `fit` keeps the mask and cursor inside it (and drops a `v` range
        // anchored into a list that no longer means the same thing).
        rows.sessions.fit(sessions.len());
        let pending_match = search.pending_match();
        if let Some(matched) = &pending_match {
            // A search jump starts the session list over on the hit row.
            rows.sessions.reset(sessions.len());
            rows.sessions.select(matched.session_index);
            search.finish_pending();
        }
        let session_idx = rows.sessions.cursor();

        let size = terminal.size()?;
        let layout = workspace_layout(
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
            focus,
            fullscreen,
        );

        sessions_pane.ensure(
            SessionCtx {
                source_scope: rows.source.scope_mask(),
                sessions: &sessions,
                session_scope: rows.sessions.scope_mask(),
                search_active: search_has_query,
            },
            &PaneInput::new(
                Viewport::from_panel(rows.sessions.offset(), panel_inner_rows(layout.sessions)),
                session_idx,
            )
            .with_selected(rows.sessions.scope_mask())
            .with_neighbors(1),
        );
        let dialogue_focus_hint = pending_match
            .as_ref()
            .map(|matched| matched.dialogue_index)
            .unwrap_or_else(|| rows.dialogues.cursor());
        let session_scope = rows.sessions.scope_mask().to_vec();
        let records = |s: &crate::tui::workspace::WorkspaceSession| {
            if search_has_query {
                search.records_for(s)
            } else {
                sessions_pane.body_for(s)
            }
        };
        let dialogue_viewport =
            Viewport::from_panel(rows.dialogues.offset(), panel_inner_rows(layout.dialogues));
        rows.dialogues.fit(dialogue_pane.len());
        dialogue_pane.ensure(
            DialogueCtx {
                sessions: &sessions,
                session_idx,
                session_scope: &session_scope,
                records: &records,
            },
            &PaneInput::new(dialogue_viewport, dialogue_focus_hint)
                .with_selected(&[])
                .with_neighbors(1),
        );
        // Growing the list invalidates both mask and focus hint: refit, then
        // re-run the keep policy with a focus the grown list can hold.
        if rows.dialogues.len() != dialogue_pane.len() {
            rows.dialogues.fit(dialogue_pane.len());
            dialogue_pane.ensure(
                DialogueCtx {
                    sessions: &sessions,
                    session_idx,
                    session_scope: &session_scope,
                    records: &records,
                },
                &PaneInput::new(
                    dialogue_viewport,
                    dialogue_focus_hint.min(dialogue_pane.len().saturating_sub(1)),
                )
                .with_selected(&[])
                .with_neighbors(1),
            );
        }

        let dialogue_count = dialogue_pane.len();
        rows.dialogues.select(dialogue_focus_hint);
        let dialogue_idx = rows.dialogues.cursor();
        let dialogue_selection = dialogue_pane.selection_mask(&rows.selection);
        dialogue_pane.ensure(
            DialogueCtx {
                sessions: &sessions,
                session_idx,
                session_scope: &session_scope,
                records: &records,
            },
            &PaneInput::new(dialogue_viewport, dialogue_idx)
                .with_selected(&dialogue_selection)
                .with_neighbors(1),
        );
        if pending_match.is_some() {
            rows.close_ranges();
        }
        let active_content_at =
            active_workspace_content_at(&search, session_idx, &dialogue_selection, dialogue_idx);

        // Materialize the dialogue projection only when the engine, the
        // dialogue selection, or the focused row changed. Content scrolling and
        // other-pane activity reuse the last projection instead of cloning
        // dialogue bodies on every redraw.
        let materialize_key = (
            dialogue_pane.generation(),
            dialogue_selection.clone(),
            dialogue_idx,
        );
        if dialogues_key.as_ref() != Some(&materialize_key) {
            dialogue_pane.materialize_into(&dialogue_selection, dialogue_idx, &mut dialogues);
            dialogues_key = Some(materialize_key.clone());
        }

        if redraw {
            redraw = false;
            // List: title borrows. Content/copy: materialize (body only for focus∪select).
            let dialogue_titles: Vec<&str> = dialogue_pane.titles().collect();

            // Resolve where the pending search match lands *before* building the
            // frame: the frame weights geometry toward the focused half, so a
            // focus switch must be visible to the build or the newly active pane
            // keeps the smaller height until the next redraw.
            let pending_half = pending_match.as_ref().map(|matched| {
                let input = dialogues
                    .get(dialogue_idx)
                    .and_then(|dialogue| dialogue.record.as_ref())
                    .and_then(|record| record.part_for_at(matched.at))
                    .is_none_or(|part| part.kind().is_input());
                search_match_half(input, matched.matched_line)
            });
            if let Some((half, _)) = pending_half {
                content_io_focus = half;
            }
            // Fold state is per-dialogue: reset it when another dialogue — or
            // another targeted part of it — comes on screen.
            let expand_key = (
                dialogues
                    .get(dialogue_idx)
                    .map(|dialogue| (dialogue.source.clone(), dialogue.work_ref.clone())),
                active_content_at,
            );
            if expanded_key.as_ref() != Some(&expand_key) {
                expanded_blocks.clear();
                pending_block_toggle = None;
                last_block_click = None;
                mouse_down_select = None;
                content_cursor.clear();
                // Block ids belong to the shown dialogue: an open block range
                // means nothing once a different dialogue is on screen.
                rows.close_range(WorkspaceFocus::Content);
                expanded_key = Some(expand_key);
            }
            // Rebuild the content frame (texts + cached layouts) only when
            // the shown dialogue, mode, focus, target, expansion, or pane
            // size changed. Scroll events reuse the cached layouts — wheel
            // input never re-lays-out the text, so it stays responsive.
            let content_key = (
                materialize_key.clone(),
                active_content_at,
                content_mode,
                content_io_focus,
                layout.content,
                expanded_blocks.clone(),
            );
            if last_content_key.as_ref() != Some(&content_key) {
                content_frame = content_pane.frame(ContentCtx {
                    dialogues: &dialogues,
                    highlighted_idx: dialogue_idx,
                    mode: content_mode,
                    target: active_content_at,
                    area: layout.content,
                    io_focus: content_io_focus,
                    expanded: &expanded_blocks,
                });
                content_scrolls.clamp_to(
                    content_frame.line_count(ContentIoFocus::Input),
                    content_frame.line_count(ContentIoFocus::Output),
                );
                last_content_key = Some(content_key);
            }
            // A j/k step that walked off this dialogue's blocks moved the
            // dialogue cursor instead; the blocks it arrived at exist now.
            content_cursor.land_crossing(content_frame.texts.block_slices());
            // Cursor block resolved against the frame's displayed segments
            // once: the follow-scroll and the view highlight share it.
            let cursor_focus = content_cursor.focused(content_frame.texts.block_slices());
            // Keyboard moves ask the next redraw to keep the cursor block in
            // view; clicks never set `follow`, so they keep the scroll as-is.
            if content_cursor.follow {
                content_cursor.follow = false;
                if let Some((half, block)) = cursor_focus {
                    let area = content_frame.areas.area(half);
                    if let Some(range) = content_block_range(content_frame.layout(half), block) {
                        let visible = area.height.saturating_sub(2) as usize;
                        let scroll = content_scrolls.get(half);
                        if range.start < scroll {
                            content_scrolls.set(half, range.start);
                        } else if range.start >= scroll.saturating_add(visible) {
                            content_scrolls
                                .set(half, range.start.saturating_add(1).saturating_sub(visible));
                        }
                    }
                }
            }
            if let Some((half, scroll)) = pending_half {
                let total = content_frame.line_count(half);
                content_scrolls.set(half, scroll.min(total.saturating_sub(1)));
            }

            let source_markers = sessions_pane.markers();
            let content_marked =
                ContentPane::block_selection_mask(&dialogues, dialogue_idx, &rows.selection);
            let body_failures: HashSet<(WorkspaceSource, String)> = sessions
                .iter()
                .filter_map(|s| {
                    sessions_pane
                        .body_failure(s)
                        .map(|_| (s.source.clone(), s.session_id.clone()))
                })
                .collect();
            terminal.draw(|frame| {
                render_workspace(
                    frame,
                    WorkspaceView {
                        sources: &sources,
                        source_markers: &source_markers,
                        loading_tick,
                        rows: &rows,
                        sessions: &sessions,
                        body_failures,
                        dialogue_titles: &dialogue_titles,
                        dialogue_selection: &dialogue_selection,
                        dialogues: &dialogues,
                        focus,
                        content_scrolls,
                        content_io_focus,
                        content_mode,
                        content_at: active_content_at,
                        show_help,
                        help_state: &help_state,
                        search: (search.input_open() || search_has_query).then_some(
                            WorkspaceSearchView {
                                query: search.query(),
                                result_count: sessions.len(),
                                current_match: (!search.output().matches.is_empty())
                                    .then_some(search.cursor()),
                                match_count: search.output().matches.len(),
                                current_target: search
                                    .output()
                                    .matches
                                    .get(search.cursor())
                                    .and_then(|matched| {
                                        workspace_search_target_ref(&search, &sessions, matched)
                                    })
                                    .map(|work_ref| work_ref.to_string()),
                                regex: search.regex(),
                                error: search
                                    .is_fetching()
                                    .then_some("loading workspace")
                                    .or_else(|| search.error()),
                                input_open: search.input_open(),
                            },
                        ),
                        line_filter_input_open,
                        line_filter: (!line_filter.is_empty()).then_some(line_filter.as_str()),
                        line_filter_error: line_filter_error.as_deref(),
                        fullscreen,
                        content_selection: visual_select_mode
                            .map(|mode: VisualSelectMode| mode.selection),
                        publish: publish_overlay
                            .as_ref()
                            .map(|overlay| WorkspacePublishView {
                                name: &overlay.name,
                                name_input: overlay.focus == OverlayFocus::Name,
                                selected: overlay.selected,
                                redaction_count: overlay.draft.redaction_count,
                                warning_count: overlay.draft.warning_count(),
                                item_count: overlay.draft.item_count(),
                                schema_version: overlay.draft.snapshot.schema_version(),
                                preview: options.mode == PickerMode::Preview,
                            }),
                        publish_error: publish_error.as_deref(),
                        content_block_cursor: cursor_focus,
                        content_range: rows
                            .range_start(WorkspaceFocus::Content)
                            .zip(content_cursor.get()),
                        content_marked: &content_marked,
                        content_frame: &content_frame,
                    },
                )
            })?;
            // Ratatui reveals the cursor after every frame. Keep it visible
            // only while typing in an overlay or while selecting text — a
            // plain click (no drag) never shows it.
            if search.input_open()
                || line_filter_input_open
                || publish_overlay
                    .as_ref()
                    .is_some_and(|overlay| overlay.focus == OverlayFocus::Name)
                || visual_select_mode
                    .is_some_and(|mode| mode.selection.anchor != mode.selection.cursor)
            {
                terminal.show_cursor()?;
            } else {
                terminal.hide_cursor()?;
            }
        }

        // Wait for input. While a load is in flight, keep a short poll so the
        // spinner animates and results repaint without a keypress. In auto
        // theme mode, poll periodically so a system appearance change swaps
        // the palette while the TUI stays up. When nothing is loading and the
        // theme is fixed, block until an event arrives instead of redrawing
        // the whole frame at a fixed rate.
        let poll_timeout = if sessions_pane.is_fetching() || search.is_fetching() {
            std::time::Duration::from_millis(100)
        } else {
            crate::tui::theme::auto_interval().unwrap_or(std::time::Duration::from_secs(3600))
        };
        if !event::poll(poll_timeout)? {
            if sessions_pane.is_fetching() || search.is_fetching() {
                loading_tick = loading_tick.wrapping_add(1);
                redraw = true;
            } else if crate::tui::theme::refresh_if_changed() {
                // Idle polls already honor `auto_interval`; a loading poll is
                // too fast to re-probe the desktop portal on every tick.
                redraw = true;
            }
            continue;
        }
        redraw = true;
        match read_interaction()? {
            Event::Paste(text) => {
                // Bracketed paste delivers the whole clipboard as one event;
                // route it to the open text input (search or line filter).
                if search.input_open() {
                    // Search terms match single lines: fold every line break
                    // into a space so a trailing newline (common when copying
                    // a line) or a multi-line clipboard searches as one query
                    // instead of silently matching nothing.
                    let pasted = normalize_search_paste(&text);
                    let pasted = pasted.trim_end();
                    search.edit(|query| query.push_str(pasted));
                    invalidate_panes_below(WorkspaceFocus::Source, &mut rows, &mut content_scrolls);
                } else if line_filter_input_open {
                    handle_line_filter_paste(&text, &mut line_filter, &mut line_filter_error);
                } else if publish_overlay
                    .as_ref()
                    .is_some_and(|overlay| overlay.focus == OverlayFocus::Name)
                {
                    if let Some(overlay) = publish_overlay.as_mut() {
                        overlay.name.push_str(&text);
                    }
                }
            }
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    continue;
                }
                // Held keys auto-repeat as Repeat events. Navigation and text
                // input repeat naturally; one-shot toggles (Enter, Esc, v, r,
                // Tab, Space, …) stay press-only so a held key cannot
                // double-fire a commit or toggle.
                if key.kind == KeyEventKind::Repeat
                    && !is_repeat_safe(
                        key.code,
                        key.modifiers,
                        search.input_open()
                            || line_filter_input_open
                            || publish_overlay
                                .as_ref()
                                .is_some_and(|overlay| overlay.focus == OverlayFocus::Name),
                    )
                {
                    continue;
                }

                // Raw mode swallows Ctrl+C; without this the picker is unkillable by terminal
                // control sequences and requires an external kill.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    anyhow::bail!(PICK_CANCELLED_MESSAGE);
                }

                if let Some(overlay) = publish_overlay.as_mut() {
                    let action = publish_overlay::handle_key(
                        key,
                        &mut overlay.selected,
                        &mut overlay.name,
                        &mut overlay.focus,
                    );
                    match action {
                        OverlayKey::Cancel => {
                            publish_overlay = None;
                            publish_error = None;
                        }
                        OverlayKey::Confirm => {
                            let Some(overlay) = publish_overlay.take() else {
                                continue;
                            };
                            let PublishOverlay {
                                selected,
                                name,
                                mut set,
                                draft: preview_draft,
                                ..
                            } = overlay;
                            let expires = publish_overlay::selected_expiry(selected);
                            let title = preview_draft.snapshot.title().to_string();
                            let draft = crate::commands::publish::load_draft_from_set(
                                &mut set,
                                Some(title),
                                expires,
                            )
                            .context("rebuild publish draft from selected WorkSet")?;
                            let save_name = (!name.is_empty()).then_some(name);
                            return Ok(PickerResult::Publish {
                                set,
                                draft: Box::new(draft),
                                expires,
                                save_name,
                            });
                        }
                        OverlayKey::Continue => {}
                    }
                    continue;
                }

                if let Some(mode) = visual_select_mode.as_mut() {
                    if key.code == KeyCode::Char('p') && key.modifiers == KeyModifiers::NONE {
                        publish_error = Some("发布需要完整的区块选择".into());
                        continue;
                    }
                    let active = content_frame.active(content_io_focus, &mut content_scrolls);
                    if let Some(picked) = handle_visual_select_key(
                        key.code,
                        key.modifiers,
                        mode,
                        active.area,
                        active.text,
                        content_mode,
                        active.scroll,
                        &dialogues,
                        rows.dialogues.cursor(),
                    )? {
                        return Ok(PickerResult::Picked(picked));
                    }
                    if matches!(key.code, KeyCode::Esc) {
                        visual_select_mode = None;
                        terminal.hide_cursor()?;
                    }
                    continue;
                }

                if search.input_open() {
                    match key.code {
                        KeyCode::Esc => {
                            search.clear();
                            invalidate_panes_below(
                                WorkspaceFocus::Source,
                                &mut rows,
                                &mut content_scrolls,
                            );
                        }
                        KeyCode::Enter => {
                            search.accept();
                        }
                        KeyCode::Up | KeyCode::Down => {
                            move_workspace_cursor(
                                key.code == KeyCode::Up,
                                focus,
                                &mut rows,
                                &mut content_scrolls,
                                &mut content_cursor,
                                content_frame.texts.block_slices(),
                            );
                        }
                        KeyCode::Backspace => {
                            search.edit(|query| {
                                query.pop();
                            });
                            invalidate_panes_below(
                                WorkspaceFocus::Source,
                                &mut rows,
                                &mut content_scrolls,
                            );
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            search.edit(String::clear);
                            invalidate_panes_below(
                                WorkspaceFocus::Source,
                                &mut rows,
                                &mut content_scrolls,
                            );
                        }
                        KeyCode::Char(ch) => {
                            search.edit(|query| query.push(ch));
                            invalidate_panes_below(
                                WorkspaceFocus::Source,
                                &mut rows,
                                &mut content_scrolls,
                            );
                        }
                        _ => {}
                    }
                    continue;
                }

                let action = if show_help {
                    match key.code {
                        KeyCode::Char('?') | KeyCode::Esc => {
                            show_help = false;
                            continue;
                        }
                        KeyCode::Char('q') => anyhow::bail!(PICK_CANCELLED_MESSAGE),
                        KeyCode::Up | KeyCode::Char('k') => {
                            let next = selected_index(&help_state).saturating_sub(1);
                            help_state.select(Some(next));
                            continue;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let current = selected_index(&help_state);
                            let next =
                                (current + 1).min(workspace_help_entries().len().saturating_sub(1));
                            help_state.select(Some(next));
                            continue;
                        }
                        KeyCode::Enter => {
                            let idx = selected_index(&help_state)
                                .min(workspace_help_entries().len().saturating_sub(1));
                            show_help = false;
                            Some(workspace_help_entries()[idx].action)
                        }
                        _ => continue,
                    }
                } else {
                    if handle_line_filter_key(
                        key.code,
                        dialogue_count,
                        &mut line_filter_input_open,
                        &mut line_filter,
                        &mut line_filter_error,
                    ) {
                        continue;
                    }

                    // Search-result navigation (not in help table — needs match list state).
                    if search_has_query {
                        match key.code {
                            KeyCode::Char('n') if !search.output().matches.is_empty() => {
                                search.next();
                                content_scrolls.clear();
                                continue;
                            }
                            KeyCode::Char('N') if !search.output().matches.is_empty() => {
                                search.previous();
                                content_scrolls.clear();
                                continue;
                            }
                            KeyCode::Esc => {
                                search.clear();
                                invalidate_panes_below(
                                    WorkspaceFocus::Source,
                                    &mut rows,
                                    &mut content_scrolls,
                                );
                                continue;
                            }
                            _ => {}
                        }
                    }

                    help_action_for_key(key.code, key.modifiers, focus)
                };

                if let Some(action) = action {
                    if action == WorkspaceHelpAction::CopyBlock {
                        if let Some(picked) =
                            workspace_picked_content_for_selected_parts(&rows.selection, &dialogues)
                        {
                            return Ok(PickerResult::Picked(picked));
                        }
                    }
                    let selected_dialogues =
                        active_rows(&dialogue_selection, dialogue_idx, dialogue_count);
                    let session_records = if matches!(
                        action,
                        WorkspaceHelpAction::ToggleSelection
                            | WorkspaceHelpAction::ToggleAll
                            | WorkspaceHelpAction::RangeSelect
                    ) {
                        session_record_snapshot(&sessions, &sessions_pane, &search)
                    } else {
                        Vec::new()
                    };
                    // Selection actions must reach every dialogue in range;
                    // the cached projection only materializes bodies for
                    // focus ∪ selected rows, so rebuild it with all rows
                    // before toggling.
                    let mut full_dialogues = Vec::new();
                    let dialogue_slice: &[WorkspaceDialogue] = if matches!(
                        action,
                        WorkspaceHelpAction::ToggleSelection
                            | WorkspaceHelpAction::ToggleAll
                            | WorkspaceHelpAction::RangeSelect
                    ) && focus
                        == WorkspaceFocus::Dialogues
                    {
                        dialogue_pane.materialize_into(
                            &vec![true; dialogues.len()],
                            dialogue_idx,
                            &mut full_dialogues,
                        );
                        &full_dialogues
                    } else {
                        &dialogues
                    };
                    match apply_workspace_help_action(
                        action,
                        &mut focus,
                        &mut fullscreen,
                        &sources,
                        &mut rows,
                        &mut content_scrolls,
                        &mut content_io_focus,
                        &mut content_mode,
                        &mut expanded_blocks,
                        &mut content_cursor,
                        &mut content_pane,
                        content_frame.texts.block_slices(),
                        &mut show_help,
                        active_content_at,
                        line_filter_spec(&line_filter),
                        &sessions,
                        &session_records,
                        dialogue_slice,
                        &selected_dialogues,
                        terminal,
                    )? {
                        HelpDispatch::Continue => {}
                        HelpDispatch::OpenSearch => {
                            search.open(&sources, rows.source.scope_mask(), &cwd);
                        }
                        HelpDispatch::Picked(picked) => return Ok(PickerResult::Picked(picked)),
                        HelpDispatch::Publish => {
                            search.accept();
                            match crate::commands::publish::prepare_picker(
                                &rows.selection,
                                options.publish_title.as_deref(),
                                options.publish_expiry,
                            ) {
                                Ok((set, draft)) => {
                                    publish_error = None;
                                    publish_overlay = Some(publish_overlay::new(
                                        set,
                                        draft,
                                        options.publish_expiry,
                                    ));
                                }
                                Err(error) => publish_error = Some(error.to_string()),
                            }
                        }
                        HelpDispatch::Refresh => {
                            let size = terminal.size()?;
                            let layout = workspace_layout(
                                ratatui::layout::Rect::new(0, 0, size.width, size.height),
                                focus,
                                fullscreen,
                            );
                            let viewport = Viewport::from_panel(
                                rows.sessions.offset(),
                                panel_inner_rows(layout.sessions),
                            );
                            refresh_next_level(
                                focus,
                                &rows,
                                &sessions,
                                &mut sessions_pane,
                                &mut all_sessions,
                                viewport,
                            );
                            sessions_dirty = true;
                            if search.query_active() {
                                search.restart(&sources, rows.source.scope_mask(), &cwd);
                            }
                        }
                    }
                    continue;
                }

                // Focus number keys (0-3) — derived from WorkspaceFocus, not the help table.
                if let KeyCode::Char(ch) = key.code {
                    if ch.is_ascii_digit() {
                        if let Some(next_focus) =
                            WorkspaceFocus::from_number_key(ch, dialogue_count)
                        {
                            set_focus(&mut focus, &mut fullscreen, &mut rows, next_focus);
                        }
                    }
                }
            }
            Event::Mouse(_) if publish_overlay.is_some() => {}
            Event::Mouse(mouse) if show_help && !search.input_open() => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    for _ in 0..MOUSE_SCROLL_LINES {
                        help_state.select(Some(selected_index(&help_state).saturating_sub(1)));
                    }
                }
                MouseEventKind::ScrollDown => {
                    let len = workspace_help_entries().len();
                    for _ in 0..MOUSE_SCROLL_LINES {
                        let next = (selected_index(&help_state) + 1).min(len.saturating_sub(1));
                        help_state.select((len > 0).then_some(next));
                    }
                }
                _ => {}
            },
            Event::Mouse(mouse) if !show_help && !search.input_open() => {
                let size = terminal.size()?;
                let layout = workspace_layout(
                    ratatui::layout::Rect::new(0, 0, size.width, size.height),
                    focus,
                    fullscreen,
                );
                // Content drag-select (free mouse / Ctrl-block) before list hit-tests.
                {
                    let hit_half = content_frame.areas.hit_test(mouse.column, mouse.row);
                    if let Some(half) = hit_half {
                        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                            content_io_focus = half;
                            // Dot-gutter click: toggle the block's mark for
                            // batch copy (y copies every marked block).
                            let active = content_frame.active(half, &mut content_scrolls);
                            if let Some(block) = content_dot_at(
                                active.area,
                                content_frame.layout(half),
                                *active.scroll,
                                mouse.column,
                                mouse.row,
                            ) {
                                // Move focus to the content pane like the
                                // text-click path, so j/k/y act on content
                                // after a dot click, not the previous list.
                                set_focus(
                                    &mut focus,
                                    &mut fullscreen,
                                    &mut rows,
                                    WorkspaceFocus::Content,
                                );
                                content_cursor.set(block);
                                toggle_block_selection(&mut rows, &dialogues, block);
                                continue;
                            }
                        }
                        let active = content_frame.active(half, &mut content_scrolls);
                        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                            && visual_select_mode.is_none()
                        {
                            if let Some(target) = content_link_at(
                                active.area,
                                active.text,
                                *active.scroll,
                                content_mode,
                                mouse.column,
                                mouse.row,
                            ) {
                                let _ = open_link_target(&target);
                                continue;
                            }
                            // Clicking a block highlights it in any mode; in
                            // read mode it also expands/collapses (raw mode
                            // always shows full blocks). Every workpart is a
                            // block, so body text folds too.
                            if let Some(position) = content_position_at(
                                active.area,
                                active.text,
                                *active.scroll,
                                content_mode,
                                mouse.column,
                                mouse.row,
                            ) {
                                // Clicks below the last rendered line clamp
                                // to it in `content_position_at`; ignore
                                // them instead of toggling the last block.
                                if !content_row_in_text(
                                    active.area,
                                    active.text,
                                    content_mode,
                                    *active.scroll,
                                    mouse.row,
                                ) {
                                    continue;
                                }
                                // Record the block under the click and toggle
                                // it on release, so a drag still selects text
                                // instead of collapsing the block.
                                pending_block_toggle =
                                    content_block_at(content_frame.layout(half), position.line)
                                        .map(|block| (half, block));
                            }
                        }
                        // A drag selects text and cancels the pending block toggle.
                        if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) {
                            pending_block_toggle = None;
                        }
                        if handle_content_mouse_select(
                            &mut visual_select_mode,
                            &mut mouse_down_select,
                            mouse.kind,
                            mouse.modifiers,
                            mouse.column,
                            mouse.row,
                            VisualContentContext {
                                area: active.area,
                                text: active.text,
                                mode: content_mode,
                                scroll: *active.scroll,
                            },
                            true,
                        ) {
                            // A click (down) or a live drag focuses content.
                            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                                || visual_select_mode.is_some()
                            {
                                set_focus(
                                    &mut focus,
                                    &mut fullscreen,
                                    &mut rows,
                                    WorkspaceFocus::Content,
                                );
                            }
                            // Pure click (no drag): release highlights the
                            // block; a second click on the same block within
                            // the window folds it (read mode only).
                            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                                if let Some((_, block)) = pending_block_toggle.take() {
                                    content_cursor.set(block);
                                    let now = std::time::Instant::now();
                                    let double_click =
                                        last_block_click.is_some_and(|(at, last_block)| {
                                            last_block == block
                                                && now.duration_since(at)
                                                    < std::time::Duration::from_millis(400)
                                        });
                                    last_block_click = Some((now, block));
                                    if double_click && content_mode == ContentViewMode::Reading {
                                        expanded_blocks.toggle(block);
                                    }
                                    redraw = true;
                                }
                            }
                            continue;
                        }
                    } else if visual_select_mode.is_some() {
                        let active = content_frame.active(content_io_focus, &mut content_scrolls);
                        if handle_content_mouse_select(
                            &mut visual_select_mode,
                            &mut mouse_down_select,
                            mouse.kind,
                            mouse.modifiers,
                            mouse.column,
                            mouse.row,
                            VisualContentContext {
                                area: active.area,
                                text: active.text,
                                mode: content_mode,
                                scroll: *active.scroll,
                            },
                            true,
                        ) {
                            continue;
                        }
                    }
                }
                match mouse.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        if let Some(scroll_focus) =
                            workspace_hit_test(layout, mouse.column, mouse.row)
                        {
                            // The wheel scrolls the content half under the
                            // cursor; anywhere else falls back to the focused
                            // half (hit_test is already None off-content).
                            // A wheel scroll drops any armed mouse selection.
                            mouse_down_select = None;
                            let scroll_half = content_frame
                                .areas
                                .hit_test(mouse.column, mouse.row)
                                .unwrap_or(content_io_focus);
                            apply_workspace_mouse_scroll(
                                scroll_focus,
                                matches!(mouse.kind, MouseEventKind::ScrollUp),
                                &mut rows,
                                &mut content_scrolls,
                                scroll_half,
                                &mut content_cursor,
                                &content_frame,
                            );
                        }
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(clicked_focus) =
                            workspace_hit_test(layout, mouse.column, mouse.row)
                        {
                            // Clicking another pane clears free selection.
                            visual_select_mode = None;
                            set_focus(&mut focus, &mut fullscreen, &mut rows, clicked_focus);
                            match clicked_focus {
                                WorkspaceFocus::Source => {
                                    let vertical = layout.source.height > 3;
                                    if let Some(idx) = source_list_index(
                                        layout.source,
                                        mouse.column,
                                        mouse.row,
                                        &sources,
                                        vertical,
                                        rows.source.offset(),
                                    ) {
                                        rows.source.select(idx);
                                        // Dot-gutter click toggles the row's
                                        // mark, exactly like Space.
                                        if vertical && dot_gutter_hit(layout.source, mouse.column) {
                                            let session_records = session_record_snapshot(
                                                &sessions,
                                                &sessions_pane,
                                                &search,
                                            );
                                            toggle_row_selection(
                                                WorkspaceFocus::Source,
                                                idx,
                                                &mut rows,
                                                &sources,
                                                &sessions,
                                                &session_records,
                                                &dialogues,
                                            );
                                            if toggle_list_row(
                                                WorkspaceFocus::Source,
                                                idx,
                                                &mut rows,
                                                &mut content_scrolls,
                                            ) {
                                                sessions_dirty = true;
                                            }
                                        }
                                    }
                                }
                                WorkspaceFocus::Sessions => {
                                    if let Some(idx) = row_list_index(
                                        layout.sessions,
                                        mouse.row,
                                        sessions.len(),
                                        rows.sessions.offset(),
                                    ) {
                                        rows.sessions.select(idx);
                                        invalidate_after_cursor_move(
                                            WorkspaceFocus::Sessions,
                                            &mut rows,
                                            &mut content_scrolls,
                                        );
                                        if dot_gutter_hit(layout.sessions, mouse.column) {
                                            let session_records = session_record_snapshot(
                                                &sessions,
                                                &sessions_pane,
                                                &search,
                                            );
                                            toggle_row_selection(
                                                WorkspaceFocus::Sessions,
                                                idx,
                                                &mut rows,
                                                &sources,
                                                &sessions,
                                                &session_records,
                                                &dialogues,
                                            );
                                            toggle_list_row(
                                                WorkspaceFocus::Sessions,
                                                idx,
                                                &mut rows,
                                                &mut content_scrolls,
                                            );
                                        }
                                    }
                                }
                                WorkspaceFocus::Dialogues => {
                                    if let Some(idx) = row_list_index(
                                        layout.dialogues,
                                        mouse.row,
                                        dialogue_count,
                                        rows.dialogues.offset(),
                                    ) {
                                        rows.dialogues.select(idx);
                                        invalidate_after_cursor_move(
                                            WorkspaceFocus::Dialogues,
                                            &mut rows,
                                            &mut content_scrolls,
                                        );
                                        if dot_gutter_hit(layout.dialogues, mouse.column) {
                                            toggle_row_selection(
                                                WorkspaceFocus::Dialogues,
                                                idx,
                                                &mut rows,
                                                &sources,
                                                &sessions,
                                                &[],
                                                &dialogues,
                                            );
                                            invalidate_after_cursor_move(
                                                WorkspaceFocus::Dialogues,
                                                &mut rows,
                                                &mut content_scrolls,
                                            );
                                        }
                                    }
                                }
                                WorkspaceFocus::Content => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn normalize_search_paste(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// Keys that safely auto-repeat when held. Navigation and scrolling repeat
/// naturally; toggles and one-shot actions (Enter, Esc, v, r, Tab, Space,
/// focus digits, …) stay press-only so a held key cannot double-fire.
/// Inside a text input every character, Backspace, and Delete repeat so
/// holding a key types continuously.
fn is_repeat_safe(code: KeyCode, modifiers: KeyModifiers, text_input: bool) -> bool {
    if text_input
        && matches!(
            code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
        )
    {
        return true;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        return matches!(code, KeyCode::Char('d') | KeyCode::Char('u'));
    }
    matches!(
        code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Char('j')
            | KeyCode::Char('k')
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Char('n')
            | KeyCode::Char('N')
    )
}

#[cfg(test)]
mod tests {
    fn tool_test_value(text: String) -> serde_json::Value {
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

    use super::super::content::{
        handle_line_filter_key, handle_line_filter_paste, workspace_block_parts,
        workspace_dialogue_vim_view, workspace_picked_content_for_copy,
        workspace_picked_content_for_cursor_block, workspace_picked_content_for_selected_parts,
        PickedContent, WorkspaceCopyShortcut,
    };
    use super::super::nav::move_workspace_cursor;
    use super::super::panes::{ContentCtx, ContentPane, DialogueCtx, DialoguePane};
    use crate::commands::browse::selection::toggle_row_selection;
    use crate::pane::{Pane, PaneInput, Viewport};
    use crate::tui::content::text::workspace_content_text;
    use crate::tui::content::view::ContentViewMode;
    use crate::tui::workspace::{
        ContentIoFocus, ContentScrolls, ListPane, Rows, TextPair, WorkspaceDialogue,
        WorkspaceFocus, WorkspaceSession, WorkspaceSource, WorkspaceSourceKind,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use sivtr_core::ai::AgentProvider;
    use sivtr_core::record::{WorkAt, WorkRef};
    use sivtr_core::record::{
        WorkChannel, WorkPart, WorkPartData, WorkRecord, WorkRecordKind, WorkSessionRef,
        WorkSource, WorkTime, RECORD_SCHEMA_VERSION,
    };
    use std::time::SystemTime;

    fn picked_units(picked: &PickedContent) -> Vec<TextPair> {
        crate::commands::memory::copy::export::picked_units(picked)
            .expect("export picked units")
            .0
    }

    #[test]
    fn repeat_safety_allows_navigation_and_text_input_only() {
        use super::is_repeat_safe;

        // Navigation and scrolling repeat when held.
        assert!(is_repeat_safe(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
            false
        ));
        assert!(is_repeat_safe(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
            false
        ));
        assert!(is_repeat_safe(KeyCode::Down, KeyModifiers::NONE, false));
        assert!(is_repeat_safe(KeyCode::PageDown, KeyModifiers::NONE, false));
        assert!(is_repeat_safe(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
            false
        ));
        assert!(is_repeat_safe(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
            false
        ));
        assert!(is_repeat_safe(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
            false
        ));

        // Commits and toggles stay press-only.
        assert!(!is_repeat_safe(KeyCode::Enter, KeyModifiers::NONE, false));
        assert!(!is_repeat_safe(KeyCode::Esc, KeyModifiers::NONE, false));
        assert!(!is_repeat_safe(KeyCode::Tab, KeyModifiers::NONE, false));
        assert!(!is_repeat_safe(
            KeyCode::Char('v'),
            KeyModifiers::NONE,
            false
        ));
        assert!(!is_repeat_safe(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
            false
        ));
        assert!(!is_repeat_safe(
            KeyCode::Char(' '),
            KeyModifiers::NONE,
            false
        ));

        // Inside a text input, characters and Backspace type continuously;
        // Enter still commits only once.
        assert!(is_repeat_safe(KeyCode::Char('e'), KeyModifiers::NONE, true));
        assert!(is_repeat_safe(KeyCode::Backspace, KeyModifiers::NONE, true));
        assert!(!is_repeat_safe(KeyCode::Enter, KeyModifiers::NONE, true));
    }

    fn dialogues_for_test(
        sessions: &[WorkspaceSession],
        session_idx: usize,
        session_scope: &[bool],
    ) -> Vec<WorkspaceDialogue> {
        let mut pane = DialoguePane::default();
        let records = |s: &WorkspaceSession| {
            sessions
                .iter()
                .find(|x| x.session_id == s.session_id && x.source == s.source)
                .filter(|x| x.body_loaded)
                .map(|x| x.records.as_slice())
        };
        let total: usize = sessions
            .iter()
            .map(|s| s.records.len())
            .sum::<usize>()
            .max(1);
        let vp = Viewport {
            first: 0,
            visible: total.max(40),
        };
        let selected_dialogues = vec![true; total];
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx,
                session_scope,
                records: &records,
            },
            &PaneInput::new(vp, 0)
                .with_selected(&selected_dialogues)
                .with_neighbors(total),
        );
        let n = pane.len();
        let selected_dialogues = vec![true; n];
        pane.ensure(
            DialogueCtx {
                sessions,
                session_idx,
                session_scope,
                records: &records,
            },
            &PaneInput::new(vp, 0)
                .with_selected(&selected_dialogues)
                .with_neighbors(n),
        );
        pane.dialogues()
    }

    #[test]
    fn workspace_dialogues_follow_current_session_without_session_selection() {
        let sessions = vec![
            workspace_test_session("new", WorkspaceSource::agent(AgentProvider::Codex), &["n1"]),
            workspace_test_session(
                "old",
                WorkspaceSource::agent(AgentProvider::Claude),
                &["o1"],
            ),
        ];

        let dialogues = dialogues_for_test(&sessions, 1, &[false, false]);

        assert_eq!(dialogues.len(), 1);
        assert_eq!(
            dialogues[0].record.as_ref().map(|r| r.title.as_str()),
            Some("o1")
        );
        assert!(workspace_content_text(
            std::slice::from_ref(&dialogues[0]),
            0,
            ContentViewMode::Reading,
            None,
        )
        .contains("old:o1"));
        assert_eq!(
            dialogues[0].work_ref.as_ref().unwrap().to_string(),
            "claude/test/1"
        );
    }

    #[test]
    fn workspace_dialogues_aggregate_selected_sessions() {
        let sessions = vec![
            workspace_test_session(
                "codex session",
                WorkspaceSource::agent(AgentProvider::Codex),
                &["c1", "c2"],
            ),
            workspace_test_session(
                "claude session",
                WorkspaceSource::agent(AgentProvider::Claude),
                &["a1"],
            ),
        ];

        let dialogues = dialogues_for_test(&sessions, 0, &[true, true]);

        assert_eq!(dialogues.len(), 3);
        let titles: Vec<_> = dialogues
            .iter()
            .map(|d| d.record.as_ref().map(|r| r.title.as_str()))
            .collect();
        assert_eq!(titles, [Some("c1"), Some("c2"), Some("a1")]);
        let texts: Vec<_> = dialogues
            .iter()
            .map(|dialogue| {
                workspace_content_text(
                    std::slice::from_ref(dialogue),
                    0,
                    ContentViewMode::Reading,
                    None,
                )
            })
            .collect();
        assert!(texts[0].contains("codex session:c1"));
        assert!(texts[1].contains("codex session:c2"));
        assert!(texts[2].contains("claude session:a1"));
        assert_eq!(
            dialogues
                .iter()
                .map(|dialogue| dialogue.work_ref.as_ref().unwrap().to_string())
                .collect::<Vec<_>>(),
            vec!["codex/test/1", "codex/test/2", "claude/test/1"]
        );
    }

    #[test]
    fn session_selection_projects_to_every_dialogue_record() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sessions = vec![workspace_test_session("test", source, &["d1", "d2"])];
        let records: Vec<Vec<_>> = sessions
            .iter()
            .map(|session| session.records.clone())
            .collect();
        let mut rows = Rows::default();
        rows.sessions = ListPane::from_scope(vec![false]);

        toggle_row_selection(
            WorkspaceFocus::Sessions,
            0,
            &mut rows,
            &[],
            &sessions,
            &records,
            &[],
        );

        assert_eq!(rows.selection.anchors().len(), 2);
        assert!(sessions[0]
            .records
            .iter()
            .all(|record| rows.selection.contains(&record.work_ref)));
        let dialogues: Vec<_> = sessions[0]
            .records
            .iter()
            .map(|record| WorkspaceDialogue {
                source: sessions[0].source.clone(),
                work_ref: Some(record.work_ref.clone()),
                record: Some(record.clone()),
            })
            .collect();
        assert_eq!(
            dialogues
                .iter()
                .map(|dialogue| dialogue
                    .work_ref
                    .as_ref()
                    .is_some_and(|reference| rows.selection.contains(reference)))
                .collect::<Vec<_>>(),
            vec![true, true]
        );
        assert!(
            ContentPane::block_selection_mask(&dialogues, 0, &rows.selection)
                .into_iter()
                .all(|marked| marked)
        );
    }

    #[test]
    fn initial_selection_is_empty_until_a_row_is_toggled() {
        let source = WorkspaceSource::agent(AgentProvider::Codex);
        let sessions = vec![workspace_test_session("test", source, &["d1", "d2"])];
        let dialogues = dialogues_for_test(&sessions, 0, &[true]);

        let rows = Rows::default();
        assert!(
            rows.selection.anchors().is_empty(),
            "Rows::default() must start with an empty selection"
        );
        assert!(
            dialogues.iter().all(|dialogue| dialogue
                .work_ref
                .as_ref()
                .is_none_or(|reference| !rows.selection.contains(reference))),
            "nothing should be selected until a row is toggled"
        );
    }

    #[test]
    fn marked_block_copy_with_real_sized_record_keeps_unmarked_blocks_out() {
        // A dialogue-sized record: user, assistant, then a run of three tool
        // pairs (output half). Marking one block must copy only its bodies.
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "cmd",
            "the user question",
            0,
        );
        record.parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::Assistant {
                content: "the assistant answer".to_string(),
            },
        });
        for (i, tool) in ["Bash", "Read", "Grep"].iter().enumerate() {
            record.parts.push(WorkPart {
                seq: 3 + 2 * i,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some(format!("c{i}")),
                    tool: Some(tool.to_string()),
                    input: serde_json::json!({ "command": format!("cmd {i}") }),
                },
            });
            record.parts.push(WorkPart {
                seq: 4 + 2 * i,
                occurred_at: None,
                data: WorkPartData::ToolResult {
                    call_id: Some(format!("c{i}")),
                    tool: Some(tool.to_string()),
                    output: serde_json::json!({ "stdout": format!("out {i}") }),
                    start_line: None,
                },
            });
        }
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record.clone()),
        };
        let dialogues = [dialogue.clone()];
        let mut selection = sivtr_core::workset::WorkSet::new(".", Vec::new());
        selection.apply_target(
            sivtr_core::workset::WorkSelectionAction::Include,
            sivtr_core::workset::WorkSelectionTarget::Parts {
                record: record.clone(),
                parts: record.parts.iter().skip(2).map(|part| part.seq).collect(),
            },
            [],
        );
        let picked = workspace_picked_content_for_selected_parts(&selection, &dialogues)
            .expect("selected parts");
        let text = picked_units(&picked)
            .iter()
            .map(|unit| unit.plain.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("cmd 0") && text.contains("cmd 2"), "{text}");
        assert!(
            !text.contains("the user question"),
            "user block leaked: {text}"
        );
        assert!(
            !text.contains("the assistant answer"),
            "assistant block leaked: {text}"
        );
    }

    #[test]
    fn dot_marked_block_survives_ensure_and_matches_dialogue_block_ids() {
        // Simulate the real click flow: build the frame, hit a block through
        // the layout (as the dot gutter does), toggle it, then run the copy
        // collection — it must find the block, never return None.
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "cmd",
            "the user question",
            0,
        );
        record.parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::Assistant {
                content: "the assistant answer".to_string(),
            },
        });
        for (i, tool) in ["Bash", "Read", "Grep"].iter().enumerate() {
            record.parts.push(WorkPart {
                seq: 3 + 2 * i,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some(format!("c{i}")),
                    tool: Some(tool.to_string()),
                    input: serde_json::json!({ "command": format!("cmd {i}") }),
                },
            });
            record.parts.push(WorkPart {
                seq: 4 + 2 * i,
                occurred_at: None,
                data: WorkPartData::ToolResult {
                    call_id: Some(format!("c{i}")),
                    tool: Some(tool.to_string()),
                    output: serde_json::json!({ "stdout": format!("out {i}") }),
                    start_line: None,
                },
            });
        }
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record.clone()),
        };
        let dialogues = [dialogue.clone()];
        let mut pane = ContentPane::default();
        let area = ratatui::layout::Rect::new(0, 0, 60, 20);
        let frame = pane.frame(ContentCtx {
            dialogues: &dialogues,
            highlighted_idx: 0,
            mode: ContentViewMode::Reading,
            target: None,
            area,
            io_focus: ContentIoFocus::Output,
            expanded: &crate::tui::content::io::ExpandedBlocks::default(),
        });

        // Hit the run block exactly as a dot-gutter click would: the last
        // owned block in the output half is the tool run (assistant's reply
        // owns the first lines).
        let layout = frame.layout(ContentIoFocus::Output);
        let mut hit_id = None;
        for (line, owner) in layout.ownership.iter().enumerate() {
            if owner.is_some() {
                hit_id = crate::tui::content::view::content_block_at(layout, line);
            }
        }
        let hit_id = hit_id.expect("a block is hit in the output half");
        let (_, selected, parts) =
            workspace_block_parts(&dialogues, 0, hit_id).expect("selected block");
        let mut selection = sivtr_core::workset::WorkSet::new(".", Vec::new());
        selection.apply_target(
            sivtr_core::workset::WorkSelectionAction::Include,
            sivtr_core::workset::WorkSelectionTarget::Parts {
                record: selected,
                parts,
            },
            [],
        );
        let picked = workspace_picked_content_for_selected_parts(&selection, &dialogues)
            .expect("selected parts");
        let text = picked_units(&picked)
            .iter()
            .map(|unit| unit.plain.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("cmd 0") && text.contains("cmd 2"), "{text}");
        assert!(
            !text.contains("the user question"),
            "user block leaked: {text}"
        );
        assert!(
            !text.contains("the assistant answer"),
            "assistant block leaked: {text}"
        );
    }

    #[test]
    fn cursor_on_tool_block_after_assistant_copies_the_tool_block() {
        // The user report: with the cursor on a pure tool block (grep) that
        // follows an assistant reply in the output half, y must copy the
        // tool block — it must not stay silent.
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "cmd",
            "user question",
            0,
        );
        record.parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::Assistant {
                content: "assistant reply".to_string(),
            },
        });
        record.parts.push(WorkPart {
            seq: 3,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: Some("c1".to_string()),
                tool: Some("Grep".to_string()),
                input: serde_json::json!({ "pattern": "fn main" }),
            },
        });
        record.parts.push(WorkPart {
            seq: 4,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: Some("c1".to_string()),
                tool: Some("Grep".to_string()),
                output: serde_json::json!({ "matches": [{ "file": "a.rs" }] }),
                start_line: None,
            },
        });
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record.clone()),
        };
        let dialogues = [dialogue.clone()];

        // Dialogue-global ids: user = 0, assistant = 1, grep tool pair = 2.
        // The cursor on the tool block copies only it.
        let picked = workspace_picked_content_for_cursor_block(&dialogues, 0, 2)
            .unwrap()
            .expect("tool block copy must not be None");
        let text = picked_units(&picked)
            .iter()
            .map(|unit| unit.plain.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("fn main"), "tool call missing: {text}");
        assert!(text.contains("\"matches\""), "tool result missing: {text}");
        assert!(
            !text.contains("assistant reply"),
            "assistant block leaked: {text}"
        );
    }

    #[test]
    fn run_member_block_copies_even_though_nested_under_the_run() {
        // Two consecutive tool pairs merge into a run (id 1, after the input
        // user block) with members (id 2, 3). The cursor may sit on a member
        // after expanding the run; y must copy just that member, not fail
        // because the member is not a top-level block.
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "cmd",
            "user question",
            0,
        );
        for (i, tool) in ["Grep", "Read"].iter().enumerate() {
            record.parts.push(WorkPart {
                seq: 2 + 2 * i,
                occurred_at: None,
                data: WorkPartData::ToolCall {
                    call_id: Some(format!("c{i}")),
                    tool: Some(tool.to_string()),
                    input: serde_json::json!({ "pattern": format!("pat {i}") }),
                },
            });
            record.parts.push(WorkPart {
                seq: 3 + 2 * i,
                occurred_at: None,
                data: WorkPartData::ToolResult {
                    call_id: Some(format!("c{i}")),
                    tool: Some(tool.to_string()),
                    output: serde_json::json!({ "matches": format!("out {i}") }),
                    start_line: None,
                },
            });
        }
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record.clone()),
        };
        let dialogues = [dialogue.clone()];

        // Member id 2 = the first tool of the run.
        let picked = workspace_picked_content_for_cursor_block(&dialogues, 0, 2)
            .unwrap()
            .expect("run member copy must not be None");
        let text = &picked_units(&picked)[0].plain;
        assert!(text.contains("pat 0"), "first member missing: {text}");
        assert!(!text.contains("pat 1"), "second member leaked: {text}");

        let (_, selected, parts) =
            workspace_block_parts(&dialogues, 0, 2).expect("selected member");
        let mut selection = sivtr_core::workset::WorkSet::new(".", Vec::new());
        selection.apply_target(
            sivtr_core::workset::WorkSelectionAction::Include,
            sivtr_core::workset::WorkSelectionTarget::Parts {
                record: selected,
                parts,
            },
            [],
        );
        let picked = workspace_picked_content_for_selected_parts(&selection, &dialogues)
            .expect("selected member copy");
        let text = &picked_units(&picked)[0].plain;
        assert!(text.contains("pat 0"), "first member missing: {text}");
        assert!(!text.contains("pat 1"), "second member leaked: {text}");
    }

    #[test]
    fn cursor_block_copy_joins_only_the_focused_block_bodies() {
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "cmd",
            "user text",
            0,
        );
        record.parts.push(WorkPart {
            seq: 2,
            occurred_at: None,
            data: WorkPartData::ToolCall {
                call_id: Some("c1".to_string()),
                tool: Some("Bash".to_string()),
                input: serde_json::json!({ "command": "ls" }),
            },
        });
        record.parts.push(WorkPart {
            seq: 3,
            occurred_at: None,
            data: WorkPartData::ToolResult {
                call_id: Some("c1".to_string()),
                tool: Some("Bash".to_string()),
                output: serde_json::json!({ "stdout": "ok" }),
                start_line: None,
            },
        });
        let dialogue = WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record.clone()),
        };
        let dialogues = [dialogue.clone()];

        // Cursor on the tool block (output half, id 1 after the input user
        // block): call + result bodies only.
        let picked = workspace_picked_content_for_cursor_block(&dialogues, 0, 1)
            .unwrap()
            .expect("cursor block copy");
        let units = picked_units(&picked);
        assert_eq!(units.len(), 2, "call + result bodies");
        let text = units
            .iter()
            .map(|unit| unit.plain.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("ls"), "tool call body missing: {text}");
        assert!(
            text.contains("\"stdout\""),
            "tool result body missing: {text}"
        );
        assert!(
            !text.contains("user text"),
            "other input block leaked: {text}"
        );

        // Cursor on the user block (input half, id 0): just the user text.
        let picked = workspace_picked_content_for_cursor_block(&dialogues, 0, 0)
            .unwrap()
            .expect("cursor block copy");
        assert_eq!(picked_units(&picked)[0].plain, "user text");
    }

    #[test]
    fn move_workspace_cursor_clamps_to_the_row_count_and_skips_no_op_moves() {
        let mut rows = Rows::default();
        let mut content_scrolls = ContentScrolls::default();
        let mut content_cursor = super::super::nav::ContentBlockCursor::default();

        // An empty dialogue list has no row to land on.
        move_workspace_cursor(
            true,
            WorkspaceFocus::Dialogues,
            &mut rows,
            &mut content_scrolls,
            &mut content_cursor,
            (&[], &[]),
        );
        assert_eq!(rows.dialogues.state().selected(), None);

        // A move that cannot change the row leaves the panes below it alone:
        // bumping the last dialogue must not reset the content scroll.
        rows.dialogues.fit(1);
        content_scrolls.set(ContentIoFocus::Output, 7);
        move_workspace_cursor(
            false,
            WorkspaceFocus::Dialogues,
            &mut rows,
            &mut content_scrolls,
            &mut content_cursor,
            (&[], &[]),
        );
        assert_eq!(rows.dialogues.cursor(), 0);
        assert_eq!(content_scrolls.get(ContentIoFocus::Output), 7);
    }

    #[test]
    fn workspace_picked_content_copies_the_picked_rows_in_order() {
        let dialogues = vec![
            workspace_test_dialogue("d1", "text 1", 0),
            workspace_test_dialogue("d2", "text 2", 1),
            workspace_test_dialogue("d3", "text 3", 2),
        ];

        // Which rows those are is [`ListPane::active`]'s answer, tested there.
        let picked = workspace_picked_content_for_copy(
            &dialogues,
            &[1, 2],
            WorkspaceCopyShortcut::Displayed,
            None,
            None,
            ContentViewMode::Reading,
        )
        .unwrap();

        let PickedContent::WorkSet {
            set, projection, ..
        } = picked
        else {
            panic!("dialogue selection must be a WorkSet")
        };
        assert_eq!(
            projection,
            super::super::content::WorkspacePickProjection::Whole
        );
        assert_eq!(set.anchors().len(), 2);
    }

    #[test]
    fn workspace_copy_shortcuts_use_structured_chat_parts_without_headings() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            record: Some(workspace_test_record(
                WorkspaceSource::agent(AgentProvider::Codex),
                "question",
                "question",
                0,
            )),
        }];

        let input = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Input,
            None,
            None,
            ContentViewMode::Reading,
        )
        .unwrap();
        let output = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Output,
            None,
            None,
            ContentViewMode::Reading,
        )
        .unwrap();

        assert!(matches!(
            input,
            PickedContent::WorkSet {
                projection: super::super::content::WorkspacePickProjection::Input,
                ..
            }
        ));
        assert!(matches!(
            output,
            PickedContent::WorkSet {
                projection: super::super::content::WorkspacePickProjection::Output,
                ..
            }
        ));
    }

    #[test]
    fn workspace_line_filter_applies_to_displayed_and_structured_copies() {
        let dialogues = vec![workspace_test_dialogue(
            "question",
            "line 1\nline 2\nline 3",
            0,
        )];
        let displayed = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Displayed,
            Some("2:3"),
            None,
            ContentViewMode::Reading,
        )
        .unwrap();
        let input = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Input,
            Some("1,3"),
            None,
            ContentViewMode::Reading,
        )
        .unwrap();

        // Displayed text is Reading-mode render of parts; filter applies to that text.
        assert!(picked_units(&displayed)[0].plain.lines().count() >= 1);
        assert_eq!(picked_units(&input)[0].plain, "line 1\nline 3");
    }

    #[test]
    fn workspace_line_filter_rejects_invalid_specs() {
        let dialogues = vec![workspace_test_dialogue("d1", "alpha\nbeta\ngamma", 0)];

        let err = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Displayed,
            Some("x"),
            None,
            ContentViewMode::Reading,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("Invalid line number"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn line_filter_key_handler_keeps_colon_inside_active_input() {
        let mut open = false;
        let mut filter = String::new();
        let mut error = None;

        assert!(handle_line_filter_key(
            KeyCode::Char(':'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));
        assert!(open);
        assert_eq!(filter, "");

        assert!(handle_line_filter_key(
            KeyCode::Char('2'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));
        assert!(handle_line_filter_key(
            KeyCode::Char(':'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));
        assert!(handle_line_filter_key(
            KeyCode::Char('3'),
            1,
            &mut open,
            &mut filter,
            &mut error,
        ));

        assert_eq!(filter, "2:3");
        assert!(open);
    }

    #[test]
    fn line_filter_paste_strips_disallowed_characters() {
        let mut filter = String::new();
        let mut error = Some("previous error".into());

        // Clipboard trailing newline and stray text must not leak into the spec.
        handle_line_filter_paste("1,3:5\n", &mut filter, &mut error);
        assert_eq!(filter, "1,3:5");
        assert!(error.is_none());

        // Fully invalid paste is dropped and the previous error is kept.
        let mut error = Some("previous error".into());
        handle_line_filter_paste("alpha beta\n", &mut filter, &mut error);
        assert_eq!(filter, "1,3:5");
        assert!(error.is_some());
    }

    #[test]
    fn search_paste_normalizes_crlf_as_one_space() {
        assert_eq!(super::normalize_search_paste("foo\r\nbar"), "foo bar");
    }

    #[test]
    fn workspace_command_shortcut_uses_terminal_command_without_prompt() {
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::terminal(),
            work_ref: Some(WorkRef::terminal("shell", 1)),
            record: Some(workspace_test_record(
                WorkspaceSource::terminal(),
                "cargo test",
                "cargo test",
                0,
            )),
        }];

        let picked = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Command,
            None,
            None,
            ContentViewMode::Reading,
        )
        .unwrap();

        assert!(matches!(
            picked,
            PickedContent::WorkSet {
                projection: super::super::content::WorkspacePickProjection::Command,
                ..
            }
        ));
    }

    #[test]
    fn workspace_dialogue_vim_view_tracks_exact_dialogue_lines() {
        let dialogue = workspace_test_dialogue("line1", "line1\nline2\nline3\nline4", 0);

        let view = workspace_dialogue_vim_view(&dialogue);
        // Reading mode wraps dialogue with headings/markers — count lines from that render.
        let expected = workspace_content_text(
            std::slice::from_ref(&dialogue),
            0,
            ContentViewMode::Reading,
            None,
        );
        assert_eq!(view.raw, expected);
        assert_eq!(view.blocks.len(), 1);
        assert_eq!(view.blocks[0].start, 1);
        assert_eq!(view.blocks[0].end, expected.lines().count().max(1));
        assert_eq!(view.blocks[0].block_text, view.raw);
        assert_eq!(view.blocks[0].input_text, view.raw);
        assert_eq!(view.blocks[0].output_text, view.raw);
    }

    #[test]
    fn workspace_picked_content_prefers_active_part_target_for_display_copy() {
        let mut record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            "dialogue",
            "visible text",
            0,
        );
        record.parts = vec![sivtr_core::record::WorkPart {
            seq: 1,
            occurred_at: None,
            data: sivtr_core::record::WorkPartData::ToolCall {
                call_id: None,
                tool: Some("tool".to_string()),
                input: tool_test_value("hidden cargo test".to_string()),
            },
        }];
        let dialogues = vec![WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(WorkRef::agent(AgentProvider::Codex, "session", 1)),
            record: Some(record),
        }];

        let picked = workspace_picked_content_for_copy(
            &dialogues,
            &[0],
            WorkspaceCopyShortcut::Displayed,
            None,
            Some(WorkAt::Part(1)),
            ContentViewMode::Reading,
        )
        .unwrap();

        assert!(picked_units(&picked)[0].plain.contains("hidden cargo test"));
        assert!(!picked_units(&picked)[0].plain.contains("codex/"));
    }

    fn workspace_test_session(
        title: &str,
        source: WorkspaceSource,
        dialogue_titles: &[&str],
    ) -> WorkspaceSession {
        WorkspaceSession {
            source: source.clone(),
            session_id: title.to_string(),
            modified: SystemTime::UNIX_EPOCH,
            title: title.to_string(),
            search_title: title.to_string(),
            records: dialogue_titles
                .iter()
                .enumerate()
                .map(|(idx, dialogue_title)| {
                    workspace_test_record(
                        source.clone(),
                        dialogue_title,
                        &format!("{title}:{dialogue_title}"),
                        idx,
                    )
                })
                .collect(),
            body_loaded: true,
        }
    }

    fn workspace_test_record(
        source: WorkspaceSource,
        title: &str,
        plain: &str,
        index: usize,
    ) -> WorkRecord {
        let (channel, provider, kind) = match source.kind {
            WorkspaceSourceKind::Terminal => {
                (WorkChannel::Terminal, None, WorkRecordKind::TerminalCommand)
            }
            WorkspaceSourceKind::Agent(provider) => (
                WorkChannel::Chat,
                Some(provider.command_name().to_string()),
                WorkRecordKind::ChatTurn,
            ),
        };
        let work_ref = match source.kind {
            WorkspaceSourceKind::Terminal => WorkRef::terminal("test", index + 1),
            WorkspaceSourceKind::Agent(provider) => WorkRef::agent(provider, "test", index + 1),
        };
        WorkRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            work_ref: work_ref.clone(),
            source: WorkSource { channel, provider },
            session: WorkSessionRef {
                id: "test".to_string(),
                canonical_id: Some("test-session-0123456789abcdef".to_string()),
                path: None,
            },
            kind,
            cwd: None,
            time: WorkTime::default(),
            status: None,
            title: title.to_string(),
            parts: vec![WorkPart {
                seq: 1,
                occurred_at: None,
                data: sivtr_core::record::WorkPartData::User {
                    content: plain.to_string(),
                },
            }],
        }
    }

    fn workspace_test_dialogue(title: &str, plain: &str, index: usize) -> WorkspaceDialogue {
        let record = workspace_test_record(
            WorkspaceSource::agent(AgentProvider::Codex),
            title,
            plain,
            index,
        );
        WorkspaceDialogue {
            source: WorkspaceSource::agent(AgentProvider::Codex),
            work_ref: Some(record.work_ref.clone()),
            record: Some(record),
        }
    }
}
