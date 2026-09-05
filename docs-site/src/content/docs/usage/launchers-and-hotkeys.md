---
title: Launch Pickers and Hotkeys
description: Open agent-memory and terminal pickers from the CLI, VS Code, tmux, Windows hotkeys, Linux launchers, and macOS launchers.
---

Pickers are the interactive path for browsing workspace memory, selecting dialogue or command blocks, filtering lines, and copying structured parts.

## Open a picker from the CLI

Open the current provider picker:

```bash
sivtr copy claude --pick
sivtr copy codex --pick
sivtr copy hermes --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

Open the terminal command-block picker:

```bash
sivtr copy --pick
sivtr copy out --pick
sivtr copy cmd --pick
```

## Workspace picker keys

| Key | Action |
| --- | --- |
| `0` / `1` / `2` / `3` | Focus Source, Sessions, Dialogues, or Content |
| `j` / `k` | Move in the current pane |
| `h` / `l` | Move to previous or next pane |
| `Space` | Toggle current source, session, or dialogue |
| `a` | Select all in supported panes |
| `g` | Select agent sources in the Source pane |
| `t` | Select terminal source in the Source pane, or open Vim view elsewhere |
| `v` | Range-select dialogues, or start visual text selection in Content |
| `:` | Start a temporary line filter for the next copy |
| `i` / `o` / `y` / `c` | Copy input, output, block, or command |
| `/` | Search all sessions |
| `n` / `N` | Next or previous search match |
| `z` | Toggle focused pane fullscreen |
| `?` | Help |
| `q` / `Esc` | Cancel or go back |

Search prefixes inside the picker:

| Prefix | Scope |
| --- | --- |
| none | Content |
| `#` | Dialogue titles |
| `>` | Session titles |

## Windows global hotkey

The built-in global hotkey daemon is Windows-only:

```bash
sivtr hotkey start
sivtr hotkey status
sivtr hotkey stop
```

Default chord:

```text
alt+y
```

Override the chord or provider when starting:

```bash
sivtr hotkey start --chord ctrl+shift+y
sivtr hotkey start --provider all
sivtr hotkey start --provider claude
```

Supported provider values are `all`, `codex`, `claude`, `hermes`, `opencode`, `pi`, `cursor`, `openclaw`, `grok`, `dsh`, `gemini`, `goose`, `qoder`, `qodercn`, and `qwen`.

When the chord is pressed, the daemon opens a terminal running an internal picker command for the daemon working directory. The picker tries the newest non-empty current session first and falls back to the session list.

## VS Code

Install the Marketplace extension:

```text
ariestar.sivtr-vscode
```

The extension launches the agent-session picker from the current workspace. Its default keybinding is `Alt+Y`. If the `sivtr` CLI is missing, the extension offers to install it with Cargo in a visible terminal.

## tmux

Install the tmux helper:

```bash
sivtr init tmux
tmux source-file ~/.tmux.conf
```

This binds `prefix + y` to open a picker from the current pane path.

## Linux desktop launcher

Linux does not ship a portable built-in global hotkey daemon. Wayland and desktop environments differ, and opening an interactive terminal is not universal across GNOME, KDE, Sway, tmux, and SSH.

Generate a project-specific launcher instead:

```bash
sivtr init linux-shortcut
```

This writes:

- `~/.local/bin/sivtr-pick-codex`
- `~/.local/share/applications/sivtr-pick-codex.desktop`

The generated script opens the provider-neutral AI session picker with the
exact `sivtr` binary that created it.

Bind your desktop shortcut to the generated script, or run it directly from a terminal.

## macOS launcher

Generate a Terminal launcher plus LaunchAgent wrapper:

```bash
sivtr init macos-shortcut
```

This writes:

- `~/.local/bin/sivtr-pick-codex`
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`

The generated script opens the provider-neutral AI session picker with the
exact `sivtr` binary that created it.

Run the script directly, or load the LaunchAgent:

```bash
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.sivtr.pick-codex.plist
```
