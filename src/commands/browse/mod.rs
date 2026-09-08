//! Workspace browser: source catalog, on-demand load, and TUI picker.
//!
//! Product surface for bare `sivtr`, hotkey, and `copy --pick`. Returns a
//! picker result; callers decide how to export or publish it.
//!
//! Pane data capability lives in [`crate::pane`] (`SlidingPane`). This module
//! owns loaders + picker orchestration only. `tui::pane` is chrome only.

mod content;
mod help;
mod load;
mod nav;
mod panes;
#[cfg(feature = "perf-benches")]
pub mod perf;
mod picker;
mod publish_overlay;
mod search;
mod selection;
mod text;
mod vim;
mod visual;

pub(crate) use content::{PickedContent, WorkspacePickProjection};
pub(crate) use load::{workspace_source_catalog, SourceLoadState};
pub(crate) use picker::run as run_picker;
pub(crate) use text::{filter_lines_by_spec, record_text_to_pair, select_lines};

use anyhow::{anyhow, Context, Result};
use sivtr_core::ai::AgentProvider;
use sivtr_core::publication::{PublicationDraft, PublicationExpiry};
use sivtr_core::workset::WorkSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::tui::terminal::{
    finish as finish_tui, init as init_tui, panic_payload_message, restore as restore_tui,
    wait_for_enter,
};
use crate::tui::workspace::{WorkspaceFocus, WorkspaceSource};

pub(crate) enum PickerResult {
    Picked(PickedContent),
    Publish {
        set: WorkSet,
        draft: Box<PublicationDraft>,
        expires: PublicationExpiry,
        save_name: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerMode {
    Browse,
    Preview,
}

struct PickerOptions {
    mode: PickerMode,
    wait_after_panic: bool,
    publish_title: Option<String>,
    publish_expiry: PublicationExpiry,
}

/// Run the workspace browser.
///
/// Catalog = local + mounts. `select_remotes` only sets the initial selection mask.
/// Loads run in the background; the picker draws immediately.
pub fn run(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
) -> Result<PickerResult> {
    run_catalog(
        providers,
        select_remotes,
        initial_focus,
        PickerOptions {
            mode: PickerMode::Browse,
            wait_after_panic: true,
            publish_title: None,
            publish_expiry: PublicationExpiry::default(),
        },
    )
}

/// Like [`run`], but does not prompt for Enter after a recovered panic.
///
/// The Windows hotkey picker runs in its own console and already waits when it
/// reports errors (`show_pick_error_and_wait`); waiting here as well would make
/// the first keypress appear to be ignored by the panic prompt.
pub fn run_without_panic_wait(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
) -> Result<PickerResult> {
    run_catalog(
        providers,
        select_remotes,
        initial_focus,
        PickerOptions {
            mode: PickerMode::Browse,
            wait_after_panic: false,
            publish_title: None,
            publish_expiry: PublicationExpiry::default(),
        },
    )
}

/// Open the workspace picker as a local publication preview.
pub(crate) fn run_preview(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
    publish_title: Option<String>,
    publish_expiry: PublicationExpiry,
) -> Result<PickerResult> {
    run_catalog(
        providers,
        select_remotes,
        initial_focus,
        PickerOptions {
            mode: PickerMode::Preview,
            wait_after_panic: true,
            publish_title,
            publish_expiry,
        },
    )
}

fn run_catalog(
    providers: &[AgentProvider],
    select_remotes: bool,
    initial_focus: WorkspaceFocus,
    options: PickerOptions,
) -> Result<PickerResult> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let sources = workspace_source_catalog(providers, &cwd)?;
    if sources.is_empty() {
        anyhow::bail!("No terminal or AI sources configured");
    }

    let source_scope: Vec<bool> = sources
        .iter()
        .map(|source| select_remotes || !source.is_remote())
        .collect();
    let source_states: Vec<SourceLoadState> = sources.iter().map(|_| Default::default()).collect();

    let mut terminal = init_tui()?;
    let result = run_picker_guarded(
        &mut terminal,
        sources,
        source_states,
        source_scope,
        cwd,
        initial_focus,
        options,
    );
    finish_tui(&mut terminal, result)
}

/// Open the picker on an already-built session list for one source.
pub fn run_with_sessions(
    source: WorkspaceSource,
    sessions: Vec<crate::tui::workspace::WorkspaceSession>,
    initial_focus: WorkspaceFocus,
) -> Result<PickerResult> {
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let loaded = sessions.len().max(1);
    let mut terminal = init_tui()?;
    let result = run_picker_guarded(
        &mut terminal,
        vec![source],
        vec![SourceLoadState::ready_from_sessions(sessions, loaded)],
        vec![true],
        cwd,
        initial_focus,
        PickerOptions {
            mode: PickerMode::Browse,
            wait_after_panic: true,
            publish_title: None,
            publish_expiry: PublicationExpiry::default(),
        },
    );
    finish_tui(&mut terminal, result)
}

/// Run the picker and convert any panic into a reported error.
///
/// A panic inside the TUI must not leave the terminal in raw mode with the
/// alternate screen active. The unwind is caught and the terminal is restored
/// first; panics that skip this path are caught by the global panic hook
/// ([`crate::tui::panic`]) and by the `Tui` drop. The panic message is
/// printed before the error propagates.
///
/// `wait_after_panic` controls whether a recovered panic prompts for Enter: the
/// hotkey picker reports errors itself, so it passes `false` to avoid consuming
/// a keypress the outer handler is about to wait on.
fn run_picker_guarded(
    terminal: &mut crate::tui::terminal::Tui,
    sources: Vec<WorkspaceSource>,
    source_states: Vec<SourceLoadState>,
    source_scope: Vec<bool>,
    cwd: std::path::PathBuf,
    initial_focus: WorkspaceFocus,
    options: PickerOptions,
) -> Result<PickerResult> {
    // This guard recovers the panic and reports it itself, so the terminal-
    // restoring hook must not also emit the default "uncaught panic" report.
    let _suppress = crate::tui::panic::SuppressDefaultReport::enter();
    match catch_unwind(AssertUnwindSafe(|| {
        run_picker(
            terminal,
            sources,
            source_states,
            source_scope,
            cwd,
            initial_focus,
            &options,
        )
    })) {
        Ok(result) => result,
        Err(payload) => {
            let message = panic_payload_message(&payload);
            let restored = restore_tui(terminal).is_ok();
            // The caller (cli_main / show_pick_error_and_wait) reports the
            // error once; just pause so the user can see the restored screen
            // before the report scrolls past. `wait_for_enter` reads from the
            // console itself, so a redirected stdin cannot skip the prompt.
            if restored && options.wait_after_panic {
                wait_for_enter("press Enter to continue");
            }
            Err(anyhow!("TUI panicked: {message}"))
        }
    }
}

/// Shared cancel sentinel for picker Esc/q.
pub const PICK_CANCELLED_MESSAGE: &str = "Pick cancelled";

pub fn is_pick_cancelled(error: &anyhow::Error) -> bool {
    error.to_string() == PICK_CANCELLED_MESSAGE
}
