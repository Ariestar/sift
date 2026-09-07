use anyhow::{anyhow, bail, Context, Result};
#[cfg(not(windows))]
use crossterm::terminal::disable_raw_mode;
#[cfg(windows)]
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{buffer::CellDiffOption, prelude::*};
use sivtr_core::config::SivtrConfig;
#[cfg(windows)]
use std::cell::Cell;
use std::io::IsTerminal;
use std::{
    cell::RefCell,
    fmt::Display,
    io::{self, Stdout},
    mem,
    ops::{Deref, DerefMut},
    rc::Rc,
};
use unicode_width::UnicodeWidthStr;

use super::panic;
use crate::tui::theme;

type InnerTui = Terminal<CrosstermBackend<Stdout>>;

/// An active terminal session that restores terminal state when it is dropped.
///
/// Explicit restoration is used when errors can be reported. `Drop` is the final safety net for
/// early returns, draw/event failures, and panics.
pub struct Tui {
    terminal: InnerTui,
    drawing_active: bool,
    /// Shared with the panic hook so a crash restores the terminal exactly as `Drop` would.
    /// The TUI and its panic restore live on the same thread, so the state never leaves it.
    state: Rc<RefCell<TerminalState>>,
    /// Whether a synchronized update is open, shared with the guard and the
    /// panic restore. A `Cell` (not a field behind the `RefCell`) so cleanup
    /// paths can read and clear it without a borrow that could panic during
    /// unwinding.
    #[cfg(windows)]
    update_active: Rc<Cell<bool>>,
    /// Clears this session's panic-restore slot when the `Tui` ends, so a later
    /// panic in an outer session still pops the outer closure.
    _panic_restore: panic::RestoreRegistration,
    #[cfg(windows)]
    previous_frame_had_wide_cells: bool,
    #[cfg(windows)]
    synchronized_updates_supported: bool,
}

impl Deref for Tui {
    type Target = InnerTui;

    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        if state.has_pending_cleanup() {
            let _ = restore_terminal_state(&mut state);
        }
        self.drawing_active = false;
        drop(state);
        // Back on the normal screen: diagnostics may mirror to stderr again.
        crate::output::set_tui_owns_screen(false);
    }
}

/// Initialize the terminal for TUI rendering.
pub fn init() -> Result<Tui> {
    // The panic hook that restores the terminal is normally installed by `cli_main`; library
    // callers (e.g. `commands::browse::run_with_sessions`) never pass through it, so install
    // here too. `install` is idempotent, and `register_panic_restore` below arms the closure.
    panic::install();
    ensure_tui_stdout()?;

    // Pick the palette before the first frame draws: honor the config
    // override, otherwise detect light/dark and truecolor from the
    // terminal environment. A missing config file already yields defaults;
    // a real read/parse failure must surface instead of silently looking
    // like the theme setting was ignored.
    let config = SivtrConfig::load().context("Failed to load the sivtr config file")?;
    theme::apply(config.theme.mode);

    let mut setup = TerminalSetup::default();
    match ConsoleInputHandle::ensure_console() {
        Ok(input) => setup.state.console_input = input,
        Err(error) => return setup.fail(error.context("Failed to acquire console input")),
    }

    match ConsoleCodePages::capture() {
        Ok(code_pages) => setup.state.code_pages = code_pages,
        Err(error) => return setup.fail(error),
    }
    if let Some(code_pages) = setup.state.code_pages.as_ref() {
        if let Err(error) = code_pages.enable_utf8() {
            return setup.fail(error);
        }
    }

    match TerminalModes::capture() {
        Ok(modes) => setup.state.modes = Some(modes),
        Err(error) => return setup.fail(error),
    }
    #[cfg(windows)]
    let synchronized_updates_supported = setup
        .state
        .modes
        .as_ref()
        .is_some_and(TerminalModes::enable_virtual_terminal_processing);
    // Shared with the synchronized-update guard and the panic restore so they
    // can read and clear the open-update flag without borrowing the RefCell.
    #[cfg(windows)]
    let update_active = Rc::clone(&setup.state.update_active);
    if let Err(error) = enable_raw_mode().context("Failed to enable terminal raw mode") {
        return setup.fail(error);
    }

    let mut stdout = io::stdout();
    // Arm before issuing each command: a failed write may still have partially reached the
    // terminal, so rollback must conservatively send the inverse command.
    setup.state.alternate_screen = true;
    if let Err(error) = execute!(stdout, EnterAlternateScreen)
        .context("Failed to enter the terminal alternate screen")
    {
        return setup.fail(error);
    }

    setup.state.mouse_capture = true;
    if let Err(error) =
        execute!(stdout, EnableMouseCapture).context("Failed to enable terminal mouse capture")
    {
        return setup.fail(error);
    }

    // Bracketed paste keeps pasted text intact (newlines, tab, UTF-8) and
    // arrives as a single Event::Paste instead of a stream of key events.
    setup.state.bracketed_paste = true;
    if let Err(error) =
        execute!(stdout, EnableBracketedPaste).context("Failed to enable terminal bracketed paste")
    {
        return setup.fail(error);
    }

    setup.state.cursor_restore_pending = true;
    if let Err(error) = execute!(stdout, Hide).context("Failed to hide the terminal cursor") {
        return setup.fail(error);
    }
    let backend = CrosstermBackend::new(stdout);
    let terminal = match Terminal::new(backend).context("Failed to initialize terminal buffers") {
        Ok(terminal) => terminal,
        Err(error) => return setup.fail(error),
    };

    let state = Rc::new(RefCell::new(setup.commit()));
    let panic_restore = panic::register_restore(Box::new({
        let state = Rc::clone(&state);
        move || {
            // Restore in place: successful cleanup steps are disarmed, so an
            // unwinding `Drop` afterwards retries only the steps that failed
            // here. The state may be mutably borrowed by the frame that is
            // unwinding, so borrow non-panicking: the `Drop` path is the
            // safety net.
            let Ok(mut state) = state.try_borrow_mut() else {
                return;
            };
            // A library caller may catch the panic itself and keep using this
            // `Tui`; mark it so a later `draw` cannot paint onto the restored
            // normal screen, and a later panic knows cleanup already ran.
            state.panic_restored = true;
            let _ = restore_terminal_state(&mut state);
        }
    }));
    let tui = Tui {
        terminal,
        drawing_active: true,
        state,
        #[cfg(windows)]
        update_active,
        _panic_restore: panic_restore,
        #[cfg(windows)]
        previous_frame_had_wide_cells: false,
        #[cfg(windows)]
        synchronized_updates_supported,
    };
    // Set only once the TUI is fully constructed: every failure path above
    // now exits through `setup.fail` or `?` without leaving the flag latched.
    // Warnings stay in the diagnostics ring while the UI is up (the `!`
    // overlay shows them); stderr mirroring resumes on teardown.
    crate::output::set_tui_owns_screen(true);
    Ok(tui)
}
/// Draw one TUI frame.
///
/// Windows terminals can leave stale trailing cells when an incremental diff replaces CJK or
/// other wide glyphs. A frame containing wide cells, the frame after one, and a resized frame are
/// fully refreshed. ASCII-only steady-state frames keep Ratatui's incremental rendering policy.
pub fn draw<F>(terminal: &mut Tui, render: F) -> Result<()>
where
    F: FnOnce(&mut Frame),
{
    if !terminal.drawing_active {
        bail!("Cannot draw while the terminal session is suspended");
    }
    if terminal.state.borrow().panic_restored {
        bail!("Cannot draw after the terminal was restored by a panic");
    }

    #[cfg(windows)]
    {
        let resized = synchronize_terminal_size(&mut terminal.terminal, || {
            Ok(CrosstermBackend::new(io::stdout()))
        })
        .context("Failed to synchronize the Windows terminal size")?;
        let force_full_redraw = resized || terminal.previous_frame_had_wide_cells;

        let draw_result = if terminal.synchronized_updates_supported {
            let update = SynchronizedUpdateGuard::begin(&terminal.update_active)?;
            let result = draw_terminal_frame(&mut terminal.terminal, force_full_redraw, render)
                .context("Failed to draw a Windows terminal frame");
            combine_results(
                result,
                update.finish(),
                "additionally failed to end the synchronized terminal update",
            )
        } else {
            draw_terminal_frame(&mut terminal.terminal, force_full_redraw, render)
                .context("Failed to draw a Windows terminal frame")
        };

        match draw_result {
            Ok(has_wide_cells) => {
                terminal.previous_frame_had_wide_cells = has_wide_cells;
                Ok(())
            }
            Err(error) => {
                // If a partial frame reached the terminal, make the next successful draw a full
                // refresh rather than trusting Ratatui's previous-buffer assumptions.
                terminal.previous_frame_had_wide_cells = true;
                Err(error)
            }
        }
    }

    #[cfg(not(windows))]
    {
        draw_terminal_frame(&mut terminal.terminal, false, render)
            .context("Failed to draw terminal frame")?;
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn synchronize_terminal_size<B, F>(
    terminal: &mut Terminal<B>,
    create_backend: F,
) -> std::result::Result<bool, B::Error>
where
    B: Backend,
    F: FnOnce() -> std::result::Result<B, B::Error>,
{
    let backend_size = terminal.backend().size()?;
    let viewport_size = terminal.get_frame().area().as_size();
    if backend_size == viewport_size {
        return Ok(false);
    }

    // Recreate Ratatui's buffers without calling Terminal::resize, whose physical clear can leave
    // stale wide-character continuation cells in Windows consoles.
    *terminal = Terminal::new(create_backend()?)?;
    Ok(true)
}

fn draw_terminal_frame<B, F>(
    terminal: &mut Terminal<B>,
    force_full_redraw: bool,
    render: F,
) -> std::result::Result<bool, B::Error>
where
    B: Backend,
    F: FnOnce(&mut Frame),
{
    let mut has_wide_cells = false;
    terminal.draw(|frame| {
        render(frame);
        has_wide_cells = frame
            .buffer_mut()
            .content
            .iter()
            .any(|cell| UnicodeWidthStr::width(cell.symbol()) > 1);

        if force_full_redraw || has_wide_cells {
            apply_full_redraw_policy(frame.buffer_mut());
        }
    })?;
    Ok(has_wide_cells)
}

fn apply_full_redraw_policy(buffer: &mut Buffer) {
    for cell in &mut buffer.content {
        if cell.diff_option == CellDiffOption::None {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
    }
}

/// Restore the terminal to its original state.
///
/// Each resource is tracked separately. Successful cleanup steps are disarmed once later cleanup
/// cannot invalidate them. On Windows, the exact input-mode restore remains armed while mouse
/// cleanup is pending because crossterm can write its cached raw mode during a retry.
pub fn restore(terminal: &mut Tui) -> Result<()> {
    terminal.drawing_active = false;
    let mut state = terminal.state.borrow_mut();
    restore_terminal_state(&mut state)
}

/// Restore a terminal and preserve both the operation error and a cleanup error, if both occur.
pub fn finish<T>(terminal: &mut Tui, operation: Result<T>) -> Result<T> {
    combine_results(
        operation,
        restore(terminal),
        "additionally failed to restore terminal state",
    )
}

/// Extract a panic payload's message for user-facing reporting.
pub(crate) fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Print a prompt and block until the user presses Enter.
///
/// On Windows the TUI binds a redirected stdin to `CONIN$` and restores the
/// original handle during cleanup; rebind the console here so the prompt still
/// reads from the console the user is looking at instead of a redirect.
pub(crate) fn wait_for_enter(prompt: &str) {
    eprintln!("{prompt}");
    #[cfg(windows)]
    let _console = if io::stdin().is_terminal() {
        None
    } else {
        ConsoleInputHandle::ensure_console().ok().flatten()
    };
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}

/// Temporarily exit the TUI while an external program runs, then re-enter it.
///
/// The outer result reports suspend/resume failures. The inner result is the external operation's
/// result, allowing callers such as the main browser to display editor errors and continue.
pub fn suspend<T, F>(terminal: &mut Tui, operation: F) -> Result<Result<T>>
where
    F: FnOnce() -> Result<T>,
{
    restore(terminal)?;
    let operation_result = operation();

    match init() {
        Ok(resumed) => {
            *terminal = resumed;
            Ok(operation_result)
        }
        Err(resume_error) => match operation_result {
            Ok(_) => Err(resume_error.context("Failed to resume the terminal interface")),
            Err(operation_error) => Err(anyhow!(
                "{operation_error:#}; additionally failed to resume the terminal interface: {resume_error:#}"
            )),
        },
    }
}

fn restore_terminal_state(state: &mut TerminalState) -> Result<()> {
    let mut failures = CleanupFailures::default();
    let mut stdout = io::stdout();

    if state.mouse_capture {
        failures.record_flag(
            "disable mouse capture",
            &mut state.mouse_capture,
            execute!(stdout, DisableMouseCapture),
        );
    }
    if state.bracketed_paste {
        failures.record_flag(
            "disable bracketed paste",
            &mut state.bracketed_paste,
            execute!(stdout, DisableBracketedPaste),
        );
    }
    if state.alternate_screen {
        failures.record_flag(
            "leave alternate screen",
            &mut state.alternate_screen,
            execute!(stdout, LeaveAlternateScreen),
        );
    }
    if state.cursor_restore_pending {
        failures.record_flag(
            "show cursor",
            &mut state.cursor_restore_pending,
            execute!(stdout, Show),
        );
    }
    restore_owned_state(&mut failures, state);

    failures.finish("Failed to restore terminal state")
}

fn restore_owned_state(failures: &mut CleanupFailures, state: &mut TerminalState) {
    let display_commands_restored = !state.mouse_capture
        && !state.alternate_screen
        && !state.cursor_restore_pending
        && !state.bracketed_paste;
    let mut input_mode_restored = true;

    if let Some(modes) = state.modes.as_mut() {
        let input_result = modes.restore_input(state.mouse_capture);
        failures.record("restore console input mode", input_result);
        input_mode_restored = modes.is_input_restored();

        // Mouse capture, alternate-screen, and cursor cleanup are emitted as ANSI sequences on
        // Windows. Keep virtual-terminal output enabled until every such sequence succeeds, so a
        // later Drop retry can still take effect instead of merely writing inert escape bytes.
        if display_commands_restored {
            #[cfg(windows)]
            let update_ended = if state.update_active.get() {
                let result = execute!(io::stdout(), EndSynchronizedUpdate);
                let succeeded = result.is_ok();
                failures.record("end synchronized terminal update", result);
                if succeeded {
                    state.update_active.set(false);
                }
                succeeded
            } else {
                true
            };
            #[cfg(not(windows))]
            let update_ended = true;

            if update_ended {
                let output_result = modes.restore_output();
                failures.record("restore console output mode", output_result);
            }
        }

        if modes.is_restored() {
            state.modes = None;
        }
    }

    if display_commands_restored {
        if let Some(code_pages) = state.code_pages.as_mut() {
            let result = code_pages.restore();
            failures.record("restore Windows console code page", result);
            if code_pages.is_restored() {
                state.code_pages = None;
            }
        }
    }

    // TerminalModes holds the current CONIN$ handle. Do not restore STDIN or close that handle
    // until its exact input mode has been restored; otherwise a cleanup retry would target a
    // closed handle and could leave the console in raw mode permanently.
    if input_mode_restored {
        if let Some(console_input) = state.console_input.as_mut() {
            let result = console_input.restore();
            failures.record("restore standard console input", result);
            if console_input.is_restored() {
                state.console_input = None;
            }
        }
    }
}

#[derive(Default)]
struct TerminalState {
    mouse_capture: bool,
    alternate_screen: bool,
    cursor_restore_pending: bool,
    bracketed_paste: bool,
    modes: Option<TerminalModes>,
    code_pages: Option<ConsoleCodePages>,
    console_input: Option<ConsoleInputHandle>,
    /// Set when the panic hook already restored the terminal for a `Tui` that
    /// survived the unwind (library callers may catch the panic themselves).
    panic_restored: bool,
    /// Whether a synchronized update is open on the output. Shared with the
    /// guard behind an `Rc<Cell<bool>>` so cleanup paths can read and clear it
    /// without a `RefCell` borrow that could panic during unwinding.
    #[cfg(windows)]
    update_active: Rc<Cell<bool>>,
}

impl TerminalState {
    fn has_pending_cleanup(&self) -> bool {
        #[cfg(windows)]
        let update_pending = self.update_active.get();
        #[cfg(not(windows))]
        let update_pending = false;

        self.mouse_capture
            || self.alternate_screen
            || self.cursor_restore_pending
            || self.bracketed_paste
            || self.modes.is_some()
            || self.code_pages.is_some()
            || self.console_input.is_some()
            || update_pending
    }
}

#[derive(Default)]
struct TerminalSetup {
    state: TerminalState,
}

impl TerminalSetup {
    fn fail<T>(&mut self, error: anyhow::Error) -> Result<T> {
        match restore_terminal_state(&mut self.state) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(anyhow!(
                "{error:#}; additionally failed to roll back terminal state: {cleanup_error:#}"
            )),
        }
    }

    fn commit(mut self) -> TerminalState {
        mem::take(&mut self.state)
    }
}

fn combine_results<T>(
    primary: Result<T>,
    secondary: Result<()>,
    secondary_context: &str,
) -> Result<T> {
    match (primary, secondary) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(secondary_error)) => Err(anyhow!(
            "{error:#}; {secondary_context}: {secondary_error:#}"
        )),
    }
}

#[derive(Default)]
struct CleanupFailures(Vec<String>);

impl CleanupFailures {
    fn record<E>(&mut self, operation: &str, result: std::result::Result<(), E>)
    where
        E: Display,
    {
        if let Err(error) = result {
            self.0.push(format!("{operation}: {error}"));
        }
    }

    fn record_flag<E>(
        &mut self,
        operation: &str,
        pending: &mut bool,
        result: std::result::Result<(), E>,
    ) where
        E: Display,
    {
        match result {
            Ok(()) => *pending = false,
            Err(error) => self.0.push(format!("{operation}: {error}")),
        }
    }

    fn finish(self, context: &str) -> Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            bail!("{context}: {}", self.0.join("; "))
        }
    }
}

fn ensure_tui_stdout() -> Result<()> {
    ensure_tui_stdout_value(std::io::stdout().is_terminal())
}

fn ensure_tui_stdout_value(is_terminal: bool) -> Result<()> {
    if is_terminal {
        return Ok(());
    }

    bail!("sivtr: TUI mode requires an interactive terminal\n  hint: run inside a terminal")
}

/// Read the next event that can affect the rendered interface.
///
/// Windows Terminal reports passive pointer movement while mouse capture is active. Those events
/// do not change application state, so returning them would only cause unnecessary redraws.
pub fn read_interaction() -> Result<Event> {
    loop {
        let event = crossterm::event::read()?;
        if !is_passive_mouse_motion(&event) {
            return Ok(event);
        }
    }
}

fn is_passive_mouse_motion(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            ..
        })
    )
}

#[cfg(windows)]
struct ConsoleInputHandle {
    original: winapi::um::winnt::HANDLE,
    replacement: winapi::um::winnt::HANDLE,
    standard_handle_pending: bool,
    close_pending: bool,
}

#[cfg(windows)]
impl ConsoleInputHandle {
    fn ensure_console() -> Result<Option<Self>> {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
        use winapi::um::{
            consoleapi::GetConsoleMode,
            fileapi::{CreateFileW, OPEN_EXISTING},
            handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
            processenv::{GetStdHandle, SetStdHandle},
            winbase::STD_INPUT_HANDLE,
            winnt::{
                FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ,
                GENERIC_WRITE,
            },
        };

        let original = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        if !original.is_null()
            && original != INVALID_HANDLE_VALUE
            && unsafe { GetConsoleMode(original, &mut mode) } != 0
        {
            return Ok(None);
        }

        let name = OsStr::new("CONIN$\0").encode_wide().collect::<Vec<_>>();
        let replacement = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if replacement == INVALID_HANDLE_VALUE {
            bail!("Failed to open CONIN$: {}", io::Error::last_os_error());
        }
        if unsafe { SetStdHandle(STD_INPUT_HANDLE, replacement) } == 0 {
            let error = io::Error::last_os_error();
            unsafe {
                CloseHandle(replacement);
            }
            bail!("Failed to bind standard input to CONIN$: {error}");
        }

        Ok(Some(Self {
            original,
            replacement,
            standard_handle_pending: true,
            close_pending: true,
        }))
    }

    fn restore(&mut self) -> Result<()> {
        use winapi::um::{
            handleapi::CloseHandle, processenv::SetStdHandle, winbase::STD_INPUT_HANDLE,
        };

        let mut failures = CleanupFailures::default();
        if self.standard_handle_pending {
            let result = win32_console_call("restore standard input handle", unsafe {
                SetStdHandle(STD_INPUT_HANDLE, self.original)
            });
            if result.is_ok() {
                self.standard_handle_pending = false;
            }
            failures.record("restore standard input handle", result);
        }
        if !self.standard_handle_pending && self.close_pending {
            let result = win32_console_call("close temporary CONIN$ handle", unsafe {
                CloseHandle(self.replacement)
            });
            if result.is_ok() {
                self.close_pending = false;
            }
            failures.record("close temporary CONIN$ handle", result);
        }
        failures.finish("Failed to restore console input handle")
    }

    fn is_restored(&self) -> bool {
        !self.standard_handle_pending && !self.close_pending
    }
}

#[cfg(not(windows))]
struct ConsoleInputHandle;

#[cfg(not(windows))]
impl ConsoleInputHandle {
    fn ensure_console() -> Result<Option<Self>> {
        Ok(None)
    }

    fn restore(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_restored(&self) -> bool {
        true
    }
}

#[cfg(windows)]
struct InputModeRestoreState {
    pending: bool,
}

#[cfg(windows)]
impl InputModeRestoreState {
    fn pending() -> Self {
        Self { pending: true }
    }

    fn is_pending(&self) -> bool {
        self.pending
    }

    fn record_success(&mut self, mouse_capture_pending: bool) {
        // DisableMouseCapture restores the input mode cached by crossterm when capture was
        // enabled. That cached mode is raw, so a later mouse-cleanup retry can invalidate an exact
        // mode restore that already succeeded. Keep the restore armed until mouse cleanup has
        // completed, then replay it once more before releasing CONIN$.
        if !mouse_capture_pending {
            self.pending = false;
        }
    }
}

#[cfg(windows)]
struct TerminalModes {
    input_handle: winapi::um::winnt::HANDLE,
    input_mode: u32,
    output_handle: winapi::um::winnt::HANDLE,
    output_mode: u32,
    input_restore: InputModeRestoreState,
    output_pending: bool,
}

#[cfg(windows)]
impl TerminalModes {
    fn capture() -> Result<Self> {
        use winapi::um::{
            consoleapi::GetConsoleMode,
            handleapi::INVALID_HANDLE_VALUE,
            processenv::GetStdHandle,
            winbase::{STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        };

        let input_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let output_handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if input_handle.is_null()
            || input_handle == INVALID_HANDLE_VALUE
            || output_handle.is_null()
            || output_handle == INVALID_HANDLE_VALUE
        {
            bail!("Failed to resolve Windows console handles");
        }

        let mut input_mode = 0;
        let mut output_mode = 0;
        win32_console_call("read console input mode", unsafe {
            GetConsoleMode(input_handle, &mut input_mode)
        })?;
        win32_console_call("read console output mode", unsafe {
            GetConsoleMode(output_handle, &mut output_mode)
        })?;

        Ok(Self {
            input_handle,
            input_mode,
            output_handle,
            output_mode,
            input_restore: InputModeRestoreState::pending(),
            output_pending: true,
        })
    }

    fn restore_input(&mut self, mouse_capture_pending: bool) -> Result<()> {
        use winapi::um::consoleapi::SetConsoleMode;

        if self.input_restore.is_pending() {
            win32_console_call("restore console input mode", unsafe {
                SetConsoleMode(self.input_handle, self.input_mode)
            })?;
            self.input_restore.record_success(mouse_capture_pending);
        }
        Ok(())
    }

    fn restore_output(&mut self) -> Result<()> {
        use winapi::um::consoleapi::SetConsoleMode;

        if self.output_pending {
            win32_console_call("restore console output mode", unsafe {
                SetConsoleMode(self.output_handle, self.output_mode)
            })?;
            self.output_pending = false;
        }
        Ok(())
    }

    fn enable_virtual_terminal_processing(&self) -> bool {
        use winapi::um::{consoleapi::SetConsoleMode, wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING};

        unsafe {
            SetConsoleMode(
                self.output_handle,
                self.output_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) != 0
        }
    }

    fn is_restored(&self) -> bool {
        !self.input_restore.is_pending() && !self.output_pending
    }

    fn is_input_restored(&self) -> bool {
        !self.input_restore.is_pending()
    }
}

#[cfg(not(windows))]
struct TerminalModes {
    pending: bool,
}

#[cfg(not(windows))]
impl TerminalModes {
    fn capture() -> Result<Self> {
        Ok(Self { pending: true })
    }

    fn restore_input(&mut self, _mouse_capture_pending: bool) -> Result<()> {
        if self.pending {
            disable_raw_mode()?;
            self.pending = false;
        }
        Ok(())
    }

    fn restore_output(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_restored(&self) -> bool {
        !self.pending
    }

    fn is_input_restored(&self) -> bool {
        !self.pending
    }
}

#[cfg(windows)]
struct ConsoleCodePages {
    output: u32,
    pending: bool,
}

#[cfg(windows)]
impl ConsoleCodePages {
    fn capture() -> Result<Option<Self>> {
        use winapi::um::consoleapi::GetConsoleOutputCP;

        let output = unsafe { GetConsoleOutputCP() };
        if output == 0 {
            bail!(
                "Failed to read Windows console output code page: {}",
                io::Error::last_os_error()
            );
        }
        Ok(Some(Self {
            output,
            pending: true,
        }))
    }

    fn enable_utf8(&self) -> Result<()> {
        use winapi::um::wincon::SetConsoleOutputCP;

        const CP_UTF8: u32 = 65_001;
        win32_console_call("set console output to UTF-8", unsafe {
            SetConsoleOutputCP(CP_UTF8)
        })
    }

    fn restore(&mut self) -> Result<()> {
        use winapi::um::wincon::SetConsoleOutputCP;

        if self.pending {
            win32_console_call("restore console output code page", unsafe {
                SetConsoleOutputCP(self.output)
            })?;
            self.pending = false;
        }
        Ok(())
    }

    fn is_restored(&self) -> bool {
        !self.pending
    }
}

#[cfg(not(windows))]
struct ConsoleCodePages;

#[cfg(not(windows))]
impl ConsoleCodePages {
    fn capture() -> Result<Option<Self>> {
        Ok(None)
    }

    fn enable_utf8(&self) -> Result<()> {
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_restored(&self) -> bool {
        true
    }
}

#[cfg(windows)]
fn win32_console_call(operation: &str, succeeded: i32) -> Result<()> {
    if succeeded == 0 {
        bail!("{operation}: {}", io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
struct SynchronizedUpdateGuard {
    active: bool,
    update_active: Rc<Cell<bool>>,
}

#[cfg(windows)]
impl SynchronizedUpdateGuard {
    fn begin(update_active: &Rc<Cell<bool>>) -> Result<Self> {
        // Arm before writing Begin: an I/O error can occur after bytes have partially reached the
        // terminal, so Drop must still attempt End.
        let guard = Self {
            active: true,
            update_active: Rc::clone(update_active),
        };
        update_active.set(true);
        execute!(io::stdout(), BeginSynchronizedUpdate)
            .context("Failed to begin synchronized terminal update")?;
        Ok(guard)
    }

    fn finish(mut self) -> Result<()> {
        let result = execute!(io::stdout(), EndSynchronizedUpdate)
            .context("Failed to end synchronized terminal update");
        if result.is_ok() {
            self.update_active.set(false);
            self.active = false;
        }
        result
    }
}

#[cfg(windows)]
impl Drop for SynchronizedUpdateGuard {
    fn drop(&mut self) {
        // The panic restore ends the update itself and clears the flag; end
        // here only while the update is still open so the restore's End is
        // not echoed a second time after VT output has been restored. The
        // flag is a Cell, so this cannot borrow or panic during unwinding.
        if self.active
            && self.update_active.get()
            && execute!(io::stdout(), EndSynchronizedUpdate).is_ok()
        {
            self.update_active.set(false);
        }
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use std::{io, num::NonZeroU16};

    use ratatui::{
        backend::TestBackend, buffer::Buffer, layout::Rect, text::Line, widgets::Paragraph,
        Terminal,
    };

    use super::{
        apply_full_redraw_policy, combine_results, draw_terminal_frame, ensure_tui_stdout_value,
        is_passive_mouse_motion, CleanupFailures, Event, MouseEvent, MouseEventKind,
    };
    use crossterm::event::{KeyModifiers, MouseButton};

    #[test]
    fn wide_frame_overwrites_stale_cells_and_forces_output() {
        let backend = TestBackend::with_lines([
            "鏃у抚涓枃娈嬬暀 ABC    ",
            "頃滉竴 stale 馃榾      ",
            "should disappear   ",
        ]);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let has_wide = draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from("[x] 鏂颁細璇?涓枃 頃滉竴 馃榾"),
                    Line::from("[ ] second row"),
                ]),
                frame.area(),
            );
        })
        .expect("wide redraw");

        assert!(has_wide);
        assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "[");
    }

    #[test]
    fn frame_after_wide_content_clears_stale_tail_cells() {
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).expect("terminal");

        draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(Paragraph::new("涓枃 wide 馃榾"), frame.area());
        })
        .expect("wide frame");
        draw_terminal_frame(&mut terminal, true, |frame| {
            frame.render_widget(Paragraph::new("short"), frame.area());
        })
        .expect("short frame");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "s");
        assert!(buffer.content[5..].iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn full_redraw_preserves_special_cell_diff_options() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        let cells = &mut buffer.content;
        cells[0].set_diff_option(ratatui::buffer::CellDiffOption::Skip);
        cells[1].set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
            NonZeroU16::new(2).unwrap(),
        ));

        apply_full_redraw_policy(&mut buffer);

        let cells = &buffer.content;
        assert_eq!(cells[0].diff_option, ratatui::buffer::CellDiffOption::Skip);
        assert_eq!(
            cells[1].diff_option,
            ratatui::buffer::CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap())
        );
        assert_eq!(
            cells[2].diff_option,
            ratatui::buffer::CellDiffOption::AlwaysUpdate
        );
    }

    #[test]
    fn incremental_ascii_draw_keeps_default_diff_policy() {
        let backend = TestBackend::new(12, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let has_wide = draw_terminal_frame(&mut terminal, false, |frame| {
            frame.render_widget(Paragraph::new("plain frame"), frame.area());
        })
        .expect("incremental redraw");

        assert!(!has_wide);
        terminal
            .backend()
            .assert_buffer_lines(["plain frame ", "            "]);
        assert!(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .all(|cell| cell.diff_option == ratatui::buffer::CellDiffOption::None));
    }

    #[test]
    fn resize_recreates_buffers_without_terminal_clear() {
        let backend = TestBackend::new(8, 2);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.backend_mut().resize(14, 4);

        let recreated =
            super::synchronize_terminal_size(&mut terminal, || Ok(TestBackend::new(14, 4)))
                .expect("resize synchronization");

        assert!(recreated);
        assert_eq!(terminal.get_frame().area().as_size(), (14, 4).into());
    }

    #[test]
    fn successful_cleanup_is_disarmed_while_failed_cleanup_remains_pending() {
        let mut failures = CleanupFailures::default();
        let mut succeeded = true;
        let mut failed = true;

        failures.record_flag("success", &mut succeeded, Ok::<(), io::Error>(()));
        failures.record_flag(
            "failure",
            &mut failed,
            Err(io::Error::other("still pending")),
        );

        assert!(!succeeded);
        assert!(failed);
        assert!(failures
            .finish("cleanup failed")
            .unwrap_err()
            .to_string()
            .contains("failure: still pending"));
    }

    #[cfg(windows)]
    #[test]
    fn exact_input_restore_stays_armed_until_mouse_cleanup_succeeds() {
        let mut input_restore = super::InputModeRestoreState::pending();

        // The first mouse cleanup fails. Restoring the exact input mode protects the current shell
        // state, but it must remain armed because crossterm's retry can write cached raw mode.
        input_restore.record_success(true);
        assert!(input_restore.is_pending());

        // Once mouse cleanup succeeds, replaying the exact mode is final and can be disarmed.
        input_restore.record_success(false);
        assert!(!input_restore.is_pending());
    }

    #[cfg(windows)]
    #[test]
    fn pending_display_commands_keep_vt_mode_and_code_page_available_for_retry() {
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;

        let mut state = super::TerminalState {
            mouse_capture: true,
            modes: Some(super::TerminalModes {
                input_handle: INVALID_HANDLE_VALUE,
                input_mode: 0,
                output_handle: INVALID_HANDLE_VALUE,
                output_mode: 0,
                input_restore: super::InputModeRestoreState { pending: false },
                output_pending: true,
            }),
            code_pages: Some(super::ConsoleCodePages {
                output: 65001,
                pending: true,
            }),
            ..super::TerminalState::default()
        };
        let mut failures = CleanupFailures::default();

        super::restore_owned_state(&mut failures, &mut state);

        assert!(state.modes.as_ref().unwrap().output_pending);
        assert!(state.code_pages.as_ref().unwrap().pending);
        assert!(failures.finish("cleanup").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn failed_input_mode_restore_keeps_conin_handle_open_for_retry() {
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;

        let mut state = super::TerminalState {
            modes: Some(super::TerminalModes {
                input_handle: INVALID_HANDLE_VALUE,
                input_mode: 0,
                output_handle: INVALID_HANDLE_VALUE,
                output_mode: 0,
                input_restore: super::InputModeRestoreState::pending(),
                output_pending: false,
            }),
            console_input: Some(super::ConsoleInputHandle {
                original: INVALID_HANDLE_VALUE,
                replacement: INVALID_HANDLE_VALUE,
                standard_handle_pending: true,
                close_pending: true,
            }),
            ..super::TerminalState::default()
        };
        let mut failures = CleanupFailures::default();

        super::restore_owned_state(&mut failures, &mut state);

        let console_input = state.console_input.as_ref().unwrap();
        assert!(console_input.standard_handle_pending);
        assert!(console_input.close_pending);
        assert!(failures.finish("cleanup").is_err());
    }

    #[test]
    fn primary_and_cleanup_errors_are_both_preserved() {
        let error = combine_results::<()>(
            Err(anyhow::anyhow!("draw failed")),
            Err(anyhow::anyhow!("restore failed")),
            "cleanup also failed",
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("draw failed"));
        assert!(error.contains("cleanup also failed: restore failed"));
    }

    #[test]
    fn rejects_non_interactive_tui_stdout() {
        let error = ensure_tui_stdout_value(false).unwrap_err();

        assert!(error
            .to_string()
            .contains("TUI mode requires an interactive terminal"));
    }

    fn mouse_event(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 12,
            row: 8,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn passive_mouse_movement_does_not_request_a_redraw() {
        assert!(is_passive_mouse_motion(&mouse_event(MouseEventKind::Moved)));
    }

    #[test]
    fn mouse_actions_still_reach_the_interface() {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Drag(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::ScrollUp,
            MouseEventKind::ScrollDown,
        ] {
            assert!(!is_passive_mouse_motion(&mouse_event(kind)));
        }
    }
}
