use std::fmt::Display;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

const COLOR_AUTO: u8 = 0;
const COLOR_ALWAYS: u8 = 1;
const COLOR_NEVER: u8 = 2;

static COLOR_CHOICE: AtomicU8 = AtomicU8::new(COLOR_AUTO);
static TUI_OWNS_SCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// While a TUI owns the screen, stderr mirroring of diagnostics is suppressed:
/// the messages stay in the diagnostics ring buffer for the `!` overlay
/// instead of corrupting the alternate-screen UI.
pub fn set_tui_owns_screen(active: bool) {
    TUI_OWNS_SCREEN.store(active, Ordering::Release);
}

/// Install the one process-wide diagnostics destination for CLI runs: every
/// warning mirrors to stderr unless the TUI owns the screen (the TUI shows
/// the same messages through the `!` overlay). Call once at binary startup,
/// before any command can warn.
pub fn install_diagnostics_listener() {
    sivtr_core::diagnostics::subscribe(Box::new(|message| {
        if !TUI_OWNS_SCREEN.load(Ordering::Acquire) {
            labeled("warning", Style::YellowBold, message);
        }
    }));
}

pub fn set_color_choice(choice: ColorChoice) {
    COLOR_CHOICE.store(
        match choice {
            ColorChoice::Auto => COLOR_AUTO,
            ColorChoice::Always => COLOR_ALWAYS,
            ColorChoice::Never => COLOR_NEVER,
        },
        Ordering::Relaxed,
    );
}

pub fn success(message: impl Display) {
    labeled("success", Style::GreenBold, message);
}

pub fn info(message: impl Display) {
    labeled("info", Style::CyanBold, message);
}

pub fn warning(message: impl Display) {
    labeled("warning", Style::YellowBold, message);
}

pub fn error(message: impl Display) {
    labeled("error", Style::RedBold, message);
}

pub fn hint(message: impl Display) {
    labeled("hint", Style::Dim, message);
}

pub fn detail(label: impl Display, value: impl Display) {
    eprintln!("  {}: {value}", paint(label, Style::Dim));
}

pub fn plain(message: impl Display) {
    eprintln!("{message}");
}

pub fn blank() {
    eprintln!();
}

fn labeled(label: &'static str, style: Style, message: impl Display) {
    eprintln!("{}: {message}", paint(label, style));
}

fn paint(value: impl Display, style: Style) -> String {
    let value = value.to_string();
    if colors_enabled() {
        format!("{}{}\x1b[0m", style.code(), value)
    } else {
        value
    }
}

fn colors_enabled() -> bool {
    match COLOR_CHOICE.load(Ordering::Relaxed) {
        COLOR_ALWAYS => true,
        COLOR_NEVER => false,
        _ => std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal(),
    }
}

#[derive(Clone, Copy)]
enum Style {
    GreenBold,
    YellowBold,
    RedBold,
    CyanBold,
    Dim,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Self::GreenBold => "\x1b[1;32m",
            Self::YellowBold => "\x1b[1;33m",
            Self::RedBold => "\x1b[1;31m",
            Self::CyanBold => "\x1b[1;36m",
            Self::Dim => "\x1b[2m",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{set_color_choice, ColorChoice};

    #[test]
    fn color_choice_can_be_set() {
        set_color_choice(ColorChoice::Never);
        set_color_choice(ColorChoice::Auto);
    }
}
