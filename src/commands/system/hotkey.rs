use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sivtr_core::config::SivtrConfig;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;

use crate::cli::{
    HotkeyAction, HotkeyCommand, HotkeyPickAgentArgs, HotkeyProviderSelection, HotkeyServeArgs,
    HotkeyStartArgs,
};
use crate::commands::browse;
use crate::tui::terminal::{panic_payload_message, wait_for_enter};
use crate::tui::workspace::WorkspaceFocus;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HotkeyState {
    pid: u32,
    chord: String,
    cwd: String,
    #[serde(default)]
    provider: HotkeyProviderSelection,
    #[serde(default)]
    exe: Option<String>,
}

pub fn execute(command: HotkeyCommand) -> Result<()> {
    match command.action.unwrap_or(HotkeyAction::Status) {
        HotkeyAction::Start(args) => start(args),
        HotkeyAction::Stop => stop(),
        HotkeyAction::Status => status(),
    }
}

pub fn serve(args: &HotkeyServeArgs) -> Result<()> {
    #[cfg(windows)]
    {
        serve_windows(args)
    }

    #[cfg(not(windows))]
    {
        let _ = args;
        anyhow::bail!("sivtr hotkey is currently supported on Windows only");
    }
}

pub fn pick_agent(args: &HotkeyPickAgentArgs) -> Result<()> {
    #[cfg(windows)]
    let _ = bind_stdio_to_console();

    if let Err(error) = std::env::set_current_dir(&args.cwd) {
        let error =
            anyhow::Error::new(error).context(format!("Failed to enter {}", args.cwd.display()));
        show_pick_error_and_wait(&error);
        return Ok(());
    }

    let result = std::panic::catch_unwind(|| {
        let providers = args.provider.providers();
        // Hotkey owns the browse product surface; copy only exports the pick.
        let _ = args.current_session;
        // The no-wait variant: this handler reports errors (and waits) itself,
        // so a panic recovered inside the picker must not consume an Enter
        // before show_pick_error_and_wait prompts.
        crate::commands::finish_picker(
            browse::run_without_panic_wait(&providers, args.all, WorkspaceFocus::Sessions)
                .context("热键选择器失败")?,
            false,
            None,
            None,
            false,
        )
    });

    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            if !browse::is_pick_cancelled(&error) {
                show_pick_error_and_wait(&error);
            }
        }
        Err(panic) => {
            let message = panic_payload_message(&panic);
            eprintln!("sivtr: AI session picker panicked");
            eprintln!("{message}");
            wait_for_enter("Press Enter to close this window.");
        }
    }

    Ok(())
}

fn start(args: HotkeyStartArgs) -> Result<()> {
    ensure_windows()?;

    if let Some(state) = read_state()? {
        if process_is_alive(state.pid) {
            eprintln!("sivtr: hotkey daemon already running (pid {})", state.pid);
            eprintln!("  chord: {}", state.chord);
            eprintln!("  cwd:   {}", state.cwd);
            eprintln!("  provider: {}", state.provider.label());
            if let Some(exe) = state.exe {
                eprintln!("  exe:   {exe}");
            }
            return Ok(());
        }
        clear_state_file()?;
    }

    let config = SivtrConfig::load()?;
    let chord = args.chord.unwrap_or(config.hotkey.chord);
    let cwd = std::env::current_dir().context("Failed to resolve current directory")?;
    let exe = std::env::current_exe().context("Failed to resolve current executable")?;

    spawn_daemon(&exe, &cwd, &chord, args.provider)?;
    wait_for_state_file()?;

    if let Some(state) = read_state()? {
        eprintln!("sivtr: hotkey daemon started");
        eprintln!("  chord: {}", state.chord);
        eprintln!("  cwd:   {}", state.cwd);
        eprintln!("  provider: {}", state.provider.label());
        if let Some(exe) = state.exe {
            eprintln!("  exe:   {exe}");
        }
    } else {
        eprintln!("sivtr: hotkey daemon started");
    }

    Ok(())
}

fn stop() -> Result<()> {
    ensure_windows()?;

    let Some(state) = read_state()? else {
        eprintln!("sivtr: hotkey daemon is not running");
        return Ok(());
    };

    if !process_is_alive(state.pid) {
        clear_state_file()?;
        eprintln!("sivtr: cleared stale hotkey state");
        return Ok(());
    }

    terminate_process(state.pid)?;
    clear_state_file()?;
    eprintln!("sivtr: hotkey daemon stopped");
    Ok(())
}

fn status() -> Result<()> {
    ensure_windows()?;

    let Some(state) = read_state()? else {
        eprintln!("sivtr: hotkey daemon is not running");
        return Ok(());
    };

    if !process_is_alive(state.pid) {
        clear_state_file()?;
        eprintln!("sivtr: hotkey daemon is not running");
        return Ok(());
    }

    eprintln!("sivtr: hotkey daemon is running");
    eprintln!("  pid:   {}", state.pid);
    eprintln!("  chord: {}", state.chord);
    eprintln!("  cwd:   {}", state.cwd);
    eprintln!("  provider: {}", state.provider.label());
    if let Some(exe) = state.exe {
        eprintln!("  exe:   {exe}");
    }
    Ok(())
}

fn state_path() -> Result<PathBuf> {
    let config_path = SivtrConfig::config_path()?;
    let dir = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Config path has no parent directory"))?;
    Ok(dir.join("hotkey-state.json"))
}

fn show_pick_error_and_wait(error: &anyhow::Error) {
    eprintln!("sivtr: AI session picker failed");
    eprintln!("{error:#}");
    wait_for_enter("Press Enter to close this window.");
}

#[cfg(windows)]
fn bind_stdio_to_console() -> Result<()> {
    use winapi::um::processenv::SetStdHandle;
    use winapi::um::winbase::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    use winapi::um::winnt::{GENERIC_READ, GENERIC_WRITE};

    let con_in = open_console_device("CONIN$", GENERIC_READ | GENERIC_WRITE)?;
    let con_out = open_console_device("CONOUT$", GENERIC_READ | GENERIC_WRITE)?;
    let con_err = open_console_device("CONOUT$", GENERIC_READ | GENERIC_WRITE)?;

    unsafe {
        if SetStdHandle(STD_INPUT_HANDLE, con_in) == 0 {
            anyhow::bail!("Failed to set STDIN to CONIN$");
        }
        if SetStdHandle(STD_OUTPUT_HANDLE, con_out) == 0 {
            anyhow::bail!("Failed to set STDOUT to CONOUT$");
        }
        if SetStdHandle(STD_ERROR_HANDLE, con_err) == 0 {
            anyhow::bail!("Failed to set STDERR to CONOUT$");
        }
    }

    Ok(())
}

#[cfg(windows)]
fn open_console_device(name: &str, access: u32) -> Result<winapi::um::winnt::HANDLE> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let wide: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        anyhow::bail!("Failed to open {name}");
    }
    Ok(handle)
}

fn read_state() -> Result<Option<HotkeyState>> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let state = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(Some(state))
}

#[cfg(windows)]
fn write_state(state: &HotkeyState) -> Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(state).context("Failed to encode hotkey state")?;
    std::fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))
}

fn clear_state_file() -> Result<()> {
    let path = state_path()?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

fn wait_for_state_file() -> Result<()> {
    let path = state_path()?;
    for _ in 0..40 {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("Timed out waiting for hotkey daemon to start")
}

fn ensure_windows() -> Result<()> {
    #[cfg(windows)]
    {
        Ok(())
    }

    #[cfg(not(windows))]
    {
        anyhow::bail!("sivtr hotkey is currently supported on Windows only");
    }
}

#[cfg(windows)]
fn spawn_daemon(
    exe: &Path,
    cwd: &Path,
    chord: &str,
    provider: HotkeyProviderSelection,
) -> Result<()> {
    use std::os::windows::process::CommandExt;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    Command::new(exe)
        .arg("hotkey-serve")
        .arg("--cwd")
        .arg(cwd.to_string_lossy().to_string())
        .arg("--chord")
        .arg(chord)
        .arg("--provider")
        .arg(provider.as_str())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn()
        .context("Failed to start hotkey daemon")?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_daemon(
    _exe: &Path,
    _cwd: &Path,
    _chord: &str,
    _provider: HotkeyProviderSelection,
) -> Result<()> {
    anyhow::bail!("sivtr hotkey is currently supported on Windows only")
}

#[cfg(windows)]
fn serve_windows(args: &HotkeyServeArgs) -> Result<()> {
    use winapi::um::winuser::{DispatchMessageW, GetMessageW, TranslateMessage, MSG, WM_HOTKEY};

    let hotkey = parse_hotkey(&args.chord)?;
    let hotkey_id = 0x5356_5452;

    register_hotkey(hotkey_id, hotkey.modifiers, hotkey.vk)?;

    let state = HotkeyState {
        pid: std::process::id(),
        chord: args.chord.clone(),
        cwd: args.cwd.clone(),
        provider: args.provider,
        exe: std::env::current_exe()
            .ok()
            .map(|path| path.to_string_lossy().to_string()),
    };
    write_state(&state)?;
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let result = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }

        if msg.message == WM_HOTKEY && msg.wParam == hotkey_id as usize {
            let _ = spawn_picker_terminal(&args.cwd, args.provider);
        }

        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = unregister_hotkey(hotkey_id);
    let _ = clear_state_file();
    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct HotkeySpec {
    modifiers: u32,
    vk: u32,
}

#[cfg(windows)]
fn parse_hotkey(chord: &str) -> Result<HotkeySpec> {
    use winapi::um::winuser::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN};

    let mut modifiers = MOD_NOREPEAT as u32;
    let mut key: Option<u32> = None;

    for part in chord.split('+').map(|part| part.trim().to_lowercase()) {
        match part.as_str() {
            "alt" => modifiers |= MOD_ALT as u32,
            "ctrl" | "control" => modifiers |= MOD_CONTROL as u32,
            "shift" => modifiers |= MOD_SHIFT as u32,
            "win" | "meta" => modifiers |= MOD_WIN as u32,
            value => {
                key = Some(parse_virtual_key(value)?);
            }
        }
    }

    let vk = key.ok_or_else(|| anyhow::anyhow!("Hotkey chord must include a key"))?;
    Ok(HotkeySpec { modifiers, vk })
}

#[cfg(windows)]
fn parse_virtual_key(key: &str) -> Result<u32> {
    let bytes = key.as_bytes();
    if bytes.len() == 1 {
        let ch = bytes[0];
        if ch.is_ascii_alphabetic() {
            return Ok(ch.to_ascii_uppercase() as u32);
        }
        if ch.is_ascii_digit() {
            return Ok(ch as u32);
        }
    }

    let rest = key
        .strip_prefix('f')
        .and_then(|value| value.parse::<u32>().ok());
    if let Some(n) = rest {
        if (1..=24).contains(&n) {
            return Ok(0x70 + (n - 1));
        }
    }

    anyhow::bail!("Unsupported hotkey key `{key}`")
}

#[cfg(windows)]
fn register_hotkey(id: i32, modifiers: u32, vk: u32) -> Result<()> {
    use winapi::um::winuser::RegisterHotKey;

    let ok = unsafe { RegisterHotKey(std::ptr::null_mut(), id, modifiers, vk) };
    if ok == 0 {
        anyhow::bail!("Failed to register global hotkey. It may already be in use.")
    }
    Ok(())
}

#[cfg(windows)]
fn unregister_hotkey(id: i32) -> Result<()> {
    use winapi::um::winuser::UnregisterHotKey;

    unsafe {
        UnregisterHotKey(std::ptr::null_mut(), id);
    }
    Ok(())
}

#[cfg(windows)]
fn spawn_picker_terminal(cwd: &str, provider: HotkeyProviderSelection) -> Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let cwd = normalize_windows_path_text(cwd);
    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let exe = normalize_windows_path_text(&exe.to_string_lossy());

    Command::new(exe)
        .current_dir(&cwd)
        .arg("hotkey-pick-agent")
        .arg("--cwd")
        .arg(&cwd)
        .arg("--current-session")
        .arg("--provider")
        .arg(provider.as_str())
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .context("Failed to open picker terminal")?;

    Ok(())
}

#[cfg(windows)]
fn normalize_windows_path_text(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = value.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

#[cfg(not(windows))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::processthreadsapi::TerminateProcess;
    use winapi::um::winnt::PROCESS_TERMINATE;

    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            anyhow::bail!("Failed to open hotkey process");
        }
        let ok = TerminateProcess(handle, 0);
        CloseHandle(handle);
        if ok == 0 {
            anyhow::bail!("Failed to stop hotkey process");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn terminate_process(_pid: u32) -> Result<()> {
    anyhow::bail!("sivtr hotkey is currently supported on Windows only")
}
