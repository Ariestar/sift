//! Shared TUI palette — single source for every color.
//!
//! The active palette lives in a thread-local so existing accessor call
//! sites (`accent()`, `focus_row()`, …) stay unchanged while the palette is
//! chosen at TUI startup and swapped at runtime when the system appearance
//! changes.
//!
//! Light/dark follows the desktop session's appearance (macOS, Linux XDG
//! portal, Windows registry) via `dark-light`; RGB vs ANSI rendering is
//! decided by the terminal's truecolor advertisement. Agent label colors
//! live in a single table, [`provider_colors`]: one row per agent holding
//! all four palette variants.

use dark_light::Mode;
use ratatui::prelude::{Color, Modifier, Style};
use sivtr_core::agents::AgentProvider;
use sivtr_core::config::ThemeMode;
use std::cell::Cell;
use std::time::Duration;

/// How often the event loop re-checks the system appearance in auto mode.
pub(crate) const AUTO_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Which of the four color schemes is active. Chrome colors and agent label
/// colors are selected from this together, so a single `ACTIVE` cell drives
/// both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    Dark,
    Light,
    Ansi,
    AnsiLight,
}

/// One palette: chrome, text, selection, and markdown colors for a scheme.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) mode: PaletteMode,
    pub(crate) accent: Color,
    pub(crate) muted: Color,
    pub(crate) dim: Color,
    pub(crate) local_origin: Color,
    pub(crate) remote_origin: Color,
    pub(crate) focus_bg: Color,
    pub(crate) selected_bg: Color,
    pub(crate) selected_fg: Color,
    pub(crate) range_fg: Color,
    pub(crate) title_active: Color,
    pub(crate) muted_text: Color,
    pub(crate) key_hint: Color,
    pub(crate) footer: Color,
    pub(crate) failure: Color,
    pub(crate) code: Color,
    pub(crate) link: Color,
    pub(crate) quote: Color,
    pub(crate) success: Color,
}

/// The four agent-label colors for one provider, one per palette. Chosen by
/// hand so light and ANSI terminals get readable, non-RGB variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderColors {
    pub(crate) dark: Color,
    pub(crate) light: Color,
    pub(crate) ansi: Color,
    pub(crate) ansi_light: Color,
}

/// One row per agent: the four curated label colors, selected by the active
/// palette. Exhaustive by construction — adding a provider to `AgentProvider`
/// makes the compiler demand its row here.
pub(crate) const fn provider_colors(provider: AgentProvider) -> ProviderColors {
    match provider {
        AgentProvider::Codex => ProviderColors {
            dark: Color::Rgb(129, 140, 248), // indigo-400
            light: Color::Rgb(79, 70, 229),  // indigo-600
            ansi: Color::Blue,
            ansi_light: Color::Blue,
        },
        AgentProvider::Claude => ProviderColors {
            dark: Color::Rgb(251, 146, 60), // orange-400
            light: Color::Rgb(234, 88, 12), // orange-600
            ansi: Color::Yellow,
            ansi_light: Color::Red,
        },
        AgentProvider::Cursor => ProviderColors {
            dark: Color::Rgb(167, 139, 250), // violet-400
            light: Color::Rgb(124, 58, 237), // violet-600
            ansi: Color::Magenta,
            ansi_light: Color::Magenta,
        },
        AgentProvider::Dsh => ProviderColors {
            dark: Color::Rgb(77, 107, 254), // DeepSeek blue
            light: Color::Rgb(37, 62, 217), // deeper DeepSeek blue
            ansi: Color::Blue,
            ansi_light: Color::Blue,
        },
        AgentProvider::OpenCode => ProviderColors {
            dark: Color::Rgb(45, 212, 191),  // teal-400
            light: Color::Rgb(13, 148, 136), // teal-600
            ansi: Color::Cyan,
            ansi_light: Color::Blue,
        },
        AgentProvider::OpenClaw => ProviderColors {
            dark: Color::Rgb(248, 113, 113), // red-400
            light: Color::Rgb(220, 38, 38),  // red-600
            ansi: Color::Red,
            ansi_light: Color::Red,
        },
        AgentProvider::Hermes => ProviderColors {
            dark: Color::Rgb(250, 204, 21), // yellow-400
            light: Color::Rgb(161, 98, 7),  // yellow-700
            ansi: Color::LightYellow,
            ansi_light: Color::DarkGray,
        },
        AgentProvider::Grok => ProviderColors {
            dark: Color::Rgb(244, 114, 182), // pink-400
            light: Color::Rgb(219, 39, 119), // pink-600
            ansi: Color::LightMagenta,
            ansi_light: Color::DarkGray,
        },
        AgentProvider::Pi => ProviderColors {
            dark: Color::Rgb(74, 222, 128), // green-400
            light: Color::Rgb(22, 163, 74), // green-600
            ansi: Color::Green,
            ansi_light: Color::Blue,
        },
        AgentProvider::Qoder => ProviderColors {
            dark: Color::Rgb(34, 211, 238), // cyan-400
            light: Color::Rgb(8, 145, 178), // cyan-600
            ansi: Color::LightCyan,
            ansi_light: Color::Blue,
        },
        AgentProvider::QoderCn => ProviderColors {
            dark: Color::Rgb(125, 211, 252), // sky-300
            light: Color::Rgb(3, 105, 161),  // sky-600
            ansi: Color::LightCyan,
            ansi_light: Color::Blue,
        },
        AgentProvider::Gemini => ProviderColors {
            dark: Color::Rgb(96, 165, 250), // blue-400
            light: Color::Rgb(37, 99, 235), // blue-600
            ansi: Color::LightBlue,
            ansi_light: Color::Blue,
        },
        AgentProvider::Goose => ProviderColors {
            dark: Color::Rgb(251, 191, 36), // amber-400
            light: Color::Rgb(217, 119, 6), // amber-600
            ansi: Color::LightYellow,
            ansi_light: Color::Red,
        },
        AgentProvider::Qwen => ProviderColors {
            dark: Color::Rgb(232, 121, 249), // fuchsia-400
            light: Color::Rgb(192, 38, 211), // fuchsia-600
            ansi: Color::LightMagenta,
            ansi_light: Color::Magenta,
        },
        AgentProvider::Zcode => ProviderColors {
            dark: Color::Rgb(163, 230, 53), // lime-400
            light: Color::Rgb(63, 98, 18),  // lime-800
            ansi: Color::LightGreen,
            ansi_light: Color::DarkGray,
        },
        _ => ProviderColors {
            dark: Color::Rgb(16 + provider as u8, 130, 220),
            light: Color::Rgb(16 + provider as u8, 80, 150),
            ansi: Color::White,
            ansi_light: Color::DarkGray,
        },
    }
}

impl Theme {
    /// Dark-background palette (the default).
    pub(crate) const fn dark() -> Self {
        Self {
            mode: PaletteMode::Dark,
            accent: Color::Rgb(56, 189, 248),         // sky-400
            muted: Color::Rgb(100, 116, 139),         // slate-500
            dim: Color::Rgb(71, 85, 105),             // slate-600
            local_origin: Color::Rgb(52, 211, 153),   // emerald-400
            remote_origin: Color::Rgb(244, 114, 182), // pink-400
            focus_bg: Color::Rgb(15, 23, 42),         // slate-900 (veil tint)
            selected_bg: Color::Rgb(51, 65, 85),      // slate-700
            selected_fg: Color::Rgb(226, 232, 240),   // slate-200
            range_fg: Color::Rgb(251, 191, 36),       // amber-400
            title_active: Color::Rgb(224, 242, 254),  // sky-100
            muted_text: Color::Rgb(203, 213, 225),    // slate-300
            key_hint: Color::Rgb(125, 211, 252),      // sky-300
            footer: Color::Rgb(148, 163, 184),        // slate-400
            failure: Color::Rgb(248, 113, 113),       // red-400
            code: Color::Rgb(148, 163, 184),          // slate-400
            link: Color::Rgb(125, 211, 252),          // sky-300
            quote: Color::Rgb(52, 211, 153),          // emerald-400
            success: Color::Rgb(74, 222, 128),        // green-400
        }
    }

    /// Light-background palette (darker variants of the same hues).
    pub(crate) const fn light() -> Self {
        Self {
            mode: PaletteMode::Light,
            accent: Color::Rgb(2, 132, 199),         // sky-600
            muted: Color::Rgb(100, 116, 139),        // slate-500
            dim: Color::Rgb(148, 163, 184),          // slate-400
            local_origin: Color::Rgb(5, 150, 105),   // emerald-600
            remote_origin: Color::Rgb(219, 39, 119), // pink-600
            focus_bg: Color::Rgb(241, 245, 249),     // slate-100 (faint gray tint)
            selected_bg: Color::Rgb(226, 232, 240),  // slate-200
            selected_fg: Color::Rgb(51, 65, 85),     // slate-700
            range_fg: Color::Rgb(217, 119, 6),       // amber-600
            title_active: Color::Rgb(15, 23, 42),    // slate-900
            muted_text: Color::Rgb(100, 116, 139),   // slate-500
            key_hint: Color::Rgb(3, 105, 161),       // sky-700
            footer: Color::Rgb(71, 85, 105),         // slate-600
            failure: Color::Rgb(220, 38, 38),        // red-600
            code: Color::Rgb(71, 85, 105),           // slate-600
            link: Color::Rgb(3, 105, 161),           // sky-700
            quote: Color::Rgb(5, 150, 105),          // emerald-600
            success: Color::Rgb(22, 163, 74),        // green-600
        }
    }

    /// Terminal-defined palette for terminals without truecolor: ANSI 16
    /// colors render correctly everywhere, while RGB sequences may be
    /// ignored or mis-mapped by older terminals.
    pub(crate) const fn ansi() -> Self {
        Self {
            mode: PaletteMode::Ansi,
            accent: Color::Cyan,
            muted: Color::DarkGray,
            dim: Color::DarkGray,
            local_origin: Color::Green,
            remote_origin: Color::Magenta,
            focus_bg: Color::DarkGray,
            selected_bg: Color::DarkGray,
            selected_fg: Color::White,
            range_fg: Color::Yellow,
            title_active: Color::White,
            muted_text: Color::Gray,
            key_hint: Color::Cyan,
            footer: Color::Gray,
            failure: Color::Red,
            code: Color::Gray,
            link: Color::Blue,
            quote: Color::Green,
            success: Color::Green,
        }
    }

    /// Terminal-defined palette for light-background terminals without
    /// truecolor: darker ANSI colors that stay readable against a light
    /// default background (the light `Light*` variants used by [`Theme::ansi`]
    /// would wash out).
    pub(crate) const fn ansi_light() -> Self {
        Self {
            mode: PaletteMode::AnsiLight,
            accent: Color::Blue,
            muted: Color::DarkGray,
            dim: Color::DarkGray,
            local_origin: Color::Blue,
            remote_origin: Color::Magenta,
            focus_bg: Color::Gray,
            selected_bg: Color::DarkGray,
            selected_fg: Color::White,
            range_fg: Color::Blue,
            title_active: Color::Black,
            muted_text: Color::DarkGray,
            key_hint: Color::Blue,
            footer: Color::DarkGray,
            failure: Color::Red,
            code: Color::DarkGray,
            link: Color::Blue,
            quote: Color::DarkGray,
            success: Color::Blue,
        }
    }
}

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(Theme::dark()) };
    static PREFERENCE: Cell<ThemeMode> = const { Cell::new(ThemeMode::Auto) };
}

/// Pick the palette for this process. The preference decides light vs dark —
/// auto follows the system appearance via desktop APIs — and the terminal's
/// truecolor support decides RGB vs ANSI, so a forced light/dark mode still
/// falls back to the ANSI palette when `COLORTERM` is absent, instead of
/// emitting RGB sequences a non-truecolor terminal cannot render.
pub(crate) fn apply(preference: ThemeMode) {
    PREFERENCE.set(preference);
    let light = match preference {
        ThemeMode::Auto => light_from_system(),
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
    };
    let theme = if supports_truecolor() {
        if light {
            Theme::light()
        } else {
            Theme::dark()
        }
    } else if light {
        // No truecolor does not mean the terminal is dark: pick the darker
        // ANSI palette on a light background so labels stay readable.
        Theme::ansi_light()
    } else {
        Theme::ansi()
    };
    ACTIVE.set(theme);
}

/// Truecolor when the terminal advertises it: `COLORTERM=truecolor|24bit`,
/// or a `-direct` terminfo name (`xterm-direct`, …) for terminals that do
/// not export `COLORTERM` (tmux and screen often strip it).
fn supports_truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor") | Ok("24bit")
    ) || std::env::var("TERM")
        .as_deref()
        .is_ok_and(|term| term.ends_with("-direct"))
}

/// Light when the desktop session reports a light appearance. Cross-platform
/// (macOS / Linux XDG portal / Windows registry) via `dark-light`; anything
/// else — dark, unspecified, or a detection error — stays dark.
fn light_from_system() -> bool {
    matches!(dark_light::detect(), Ok(Mode::Light))
}

/// Poll interval for appearance changes while the theme is in auto mode.
/// `None` when a fixed dark/light palette is active, so the event loop does
/// not wake up to re-check.
pub(crate) fn auto_interval() -> Option<Duration> {
    (PREFERENCE.get() == ThemeMode::Auto).then_some(AUTO_POLL_INTERVAL)
}

/// Re-check the system appearance and swap the palette when it changed.
/// Returns true when the next frame must be redrawn. No-op outside auto mode.
pub(crate) fn refresh_if_changed() -> bool {
    if PREFERENCE.get() != ThemeMode::Auto {
        return false;
    }
    let before = ACTIVE.get().mode;
    apply(ThemeMode::Auto);
    ACTIVE.get().mode != before
}

/// Active panel chrome (focused border / scrollbar).
pub(crate) fn accent() -> Color {
    ACTIVE.get().accent
}

/// Inactive panel chrome.
pub(crate) fn muted() -> Color {
    ACTIVE.get().muted
}

/// Dim text / empty placeholders.
pub(crate) fn dim() -> Color {
    ACTIVE.get().dim
}

/// Cursor / focus highlight on a list row or content block: a tint close to
/// the terminal background (thin-veil look, the terminal's stand-in for a
/// low-alpha overlay) that leaves the text foreground untouched.
pub(crate) fn focus_row() -> Style {
    Style::default().bg(ACTIVE.get().focus_bg)
}

/// Text-selection overlay (drag / visual-mode columns), distinct from the
/// list panes' `●`/`○` selection dots.
pub(crate) fn text_selection_row() -> Style {
    let theme = ACTIVE.get();
    Style::default().bg(theme.selected_bg).fg(theme.selected_fg)
}

/// Range selection (visual span).
pub(crate) fn range_row() -> Style {
    Style::default()
        .fg(ACTIVE.get().range_fg)
        .add_modifier(Modifier::BOLD)
}

/// Range / search-highlight foreground.
pub(crate) fn range_fg() -> Color {
    ACTIVE.get().range_fg
}

/// Agent label color for the active palette. Reads the per-provider row from
/// [`provider_colors`] and picks the variant that matches the active scheme.
pub(crate) fn provider_color(provider: AgentProvider) -> Color {
    let colors = provider_colors(provider);
    match ACTIVE.get().mode {
        PaletteMode::Dark => colors.dark,
        PaletteMode::Light => colors.light,
        PaletteMode::Ansi => colors.ansi,
        PaletteMode::AnsiLight => colors.ansi_light,
    }
}

pub(crate) fn terminal_color() -> Color {
    ACTIVE.get().footer
}

/// Origin glyph: local `·`, remote `↗`.
pub(crate) fn origin_glyph(remote: bool) -> &'static str {
    if remote {
        "↗"
    } else {
        "·"
    }
}

pub(crate) fn origin_style(remote: bool) -> Style {
    let theme = ACTIVE.get();
    Style::default().fg(if remote {
        theme.remote_origin
    } else {
        theme.local_origin
    })
}

pub(crate) fn title_style(active: bool) -> Style {
    let theme = ACTIVE.get();
    if active {
        Style::default()
            .fg(theme.title_active)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.muted_text)
    }
}

pub(crate) fn key_hint_style() -> Style {
    Style::default().fg(ACTIVE.get().key_hint)
}

pub(crate) fn footer_style() -> Style {
    Style::default().fg(ACTIVE.get().footer)
}

/// Help / hint descriptions.
pub(crate) fn help_text() -> Color {
    ACTIVE.get().muted_text
}

/// Error / failed-load markers.
pub(crate) fn failure() -> Color {
    ACTIVE.get().failure
}

/// Inline code and code-block content.
pub(crate) fn code() -> Color {
    ACTIVE.get().code
}

/// Clickable links (underlined).
pub(crate) fn link_style() -> Style {
    Style::default()
        .fg(ACTIVE.get().link)
        .add_modifier(Modifier::UNDERLINED)
}

/// Blockquote content.
pub(crate) fn quote() -> Color {
    ACTIVE.get().quote
}

/// Completed-task and assistant-success markers.
pub(crate) fn success() -> Color {
    ACTIVE.get().success
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_switches_the_active_palette() {
        apply(ThemeMode::Dark);
        let dark_accent = accent();
        apply(ThemeMode::Light);
        assert_ne!(accent(), dark_accent, "light palette must differ from dark");
        apply(ThemeMode::Dark);
        assert_eq!(accent(), dark_accent);
    }

    #[test]
    fn provider_colors_cover_every_agent_with_distinct_schemes() {
        let mut seen_dark = std::collections::HashSet::new();
        for spec in AgentProvider::all() {
            let colors = provider_colors(spec.provider);
            assert_ne!(
                colors.dark, colors.light,
                "{} must have a light variant distinct from its dark one",
                spec.name
            );
            assert!(
                seen_dark.insert(colors.dark),
                "{} collides with another provider's dark color",
                spec.name
            );
        }
        apply(ThemeMode::Dark);
        let dark = provider_color(AgentProvider::Hermes);
        apply(ThemeMode::Light);
        assert_ne!(provider_color(AgentProvider::Hermes), dark);
    }

    #[test]
    fn ansi_provider_colors_emit_no_rgb() {
        for spec in AgentProvider::all() {
            let colors = provider_colors(spec.provider);
            for color in [colors.ansi, colors.ansi_light] {
                assert!(
                    !matches!(color, Color::Rgb(..)),
                    "{} leaks RGB through the ANSI fallback: {color:?}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn ansi_light_uses_darker_foregrounds_for_light_backgrounds() {
        let dark = Theme::ansi();
        let light = Theme::ansi_light();
        assert_ne!(light.title_active, dark.title_active);
        // These ANSI colors can wash out against a light default background.
        let assert_dark_foreground = |name: &str, color: Color| {
            assert!(
                !matches!(
                    color,
                    Color::White
                        | Color::Yellow
                        | Color::Green
                        | Color::Cyan
                        | Color::Gray
                        | Color::LightRed
                        | Color::LightGreen
                        | Color::LightYellow
                        | Color::LightBlue
                        | Color::LightMagenta
                        | Color::LightCyan
                ),
                "low-contrast foreground for {name} on a light ANSI background: {color:?}"
            );
        };
        for (name, color) in [
            ("title", light.title_active),
            ("footer", light.footer),
            ("muted text", light.muted_text),
            ("range", light.range_fg),
            ("local origin", light.local_origin),
            ("remote origin", light.remote_origin),
            ("code", light.code),
            ("link", light.link),
            ("quote", light.quote),
            ("success", light.success),
        ] {
            assert_dark_foreground(name, color);
        }
        for spec in AgentProvider::all() {
            assert_dark_foreground(spec.name, provider_colors(spec.provider).ansi_light);
        }
    }

    #[test]
    fn light_theme_colors_keep_contrast_on_white() {
        // Light-palette label colors render on a white terminal background;
        // each must clear WCAG AA for large text / UI components (3:1).
        // Expose the exact floor here so a new provider cannot silently ship
        // an unreadable light color.
        let contrast = |color: Color| {
            let Color::Rgb(r, g, b) = color else {
                return f64::INFINITY;
            };
            let linear = |channel: u8| {
                let value = channel as f64 / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
            (1.0 + 0.05) / (luminance + 0.05)
        };
        for spec in AgentProvider::all() {
            let light = provider_colors(spec.provider).light;
            assert!(
                contrast(light) >= 3.0,
                "{} light color {light:?} has contrast {:.1}:1 on white; darken it",
                spec.name,
                contrast(light)
            );
        }
    }

    #[test]
    fn fixed_preference_disables_auto_polling() {
        assert!(auto_interval().is_some(), "auto mode polls by default");
        apply(ThemeMode::Dark);
        assert_eq!(auto_interval(), None, "fixed dark does not poll");
        apply(ThemeMode::Light);
        assert_eq!(auto_interval(), None, "fixed light does not poll");
        apply(ThemeMode::Auto);
        assert!(auto_interval().is_some());
    }

    #[test]
    fn refresh_outside_auto_mode_never_swaps() {
        apply(ThemeMode::Dark);
        assert!(!refresh_if_changed());
        apply(ThemeMode::Light);
        assert!(!refresh_if_changed());
    }
}
