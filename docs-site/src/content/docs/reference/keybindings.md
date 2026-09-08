---
title: Keybindings
description: Workspace picker, content, search, line filter, and Vim-view keybindings.
---

This page documents the workspace picker TUI (the default `sivtr` interface). The single-buffer browser that `pipe`/`run`/`import` used was removed — those commands now write to the unified archive and open the external editor.

## Workspace picker

`pipe`/`run`/`import` no longer open a TUI; the picker is reached by running bare `sivtr`, via `copy --pick`, the session picker hotkey, or session import.

| Key | Action |
| --- | --- |
| `0` | Focus Source pane |
| `1` | Focus Sessions pane |
| `2` | Focus Dialogues pane |
| `3` | Focus Content pane |
| `j` / `k` | Move down / up (Content: block cursor, crossing into an adjacent dialogue) |
| `h` / `l` | Focus previous / next pane |
| `Space` | Toggle source/session/dialogue, or mark a content block |
| `a` | 切换当前面板的全部条目 |
| `g` | Select agent sources (Source) or scroll Content to top |
| `G` | Scroll Content to bottom |
| `t` | Select terminal source (Source), or open the Vim-style full view |
| `R` | Refresh the next level under the active rows |
| `v` | Range-select rows (lists) or block-range mark a span (Content) |
| `Tab` | Switch Content between Input and Output halves |
| `r` | Toggle read/raw content mode in Content |
| `Enter` | Focus next / copy (lists); fold/unfold the cursor block (Content) |
| `i` / `o` / `y` / `c` | Copy input / output / input+output block / bare command |
| `z` | Toggle the focused pane fullscreen |
| `?` | Toggle help overlay |
| `/` | Open search |
| `q` / `Esc` | Cancel / go back (Esc also goes up a pane level) |
| `Ctrl-C` | Hard cancel |

Content scrolling:

| Key | Action |
| --- | --- |
| `Ctrl-D` / `PgDn` | Scroll Content down 10 lines |
| `Ctrl-U` / `PgUp` | Scroll Content up 10 lines |

## Structure-block folding

Every workpart is a foldable block; tool call + result are grouped by call id. Consecutive structure units fold into a run tagged like `<:kind xN:>`.

- **`Enter`** on the cursor block — fold/unfold.
- **Single click** on a block tag — toggle it (Reading mode only; raw mode always full).
- **Double-click** (< 400ms) — fold a block.
- **`r`** — switch between read mode (markdown + fold tags) and raw mode (full payloads).

Two levels: a run tag expands to reveal its members (still folded), and a member expands to show its body. Defaults: structure blocks collapsed, body blocks expanded.

## Mouse

| Gesture | Action |
| --- | --- |
| Wheel | Scroll (3 lines; Content smooth-scrolls) |
| Click | Focus + select |
| Click dot gutter | Toggle a block mark |
| Drag | Linear select |
| Ctrl-drag | Block select |
| Double-click | Fold a block |
| Click a link | Open it |

## Workspace search

| Key | Action |
| --- | --- |
| `/` | Open search input |
| `Enter` | Accept search input |
| `Esc` | Clear or close search |
| `Backspace` | Edit query while input is open |
| `Ctrl-U` | Clear the query |
| `n` | Next match |
| `N` | Previous match |

Search prefixes:

| Prefix | Scope |
| --- | --- |
| none | Content |
| `#` | Dialogue titles |
| `>` | Session titles |

## Line filter input

Opened from the picker with `:`. Requires at least one dialogue.

| Key | Action |
| --- | --- |
| digits, `,`, `:` | Build a 1-based line spec |
| `Backspace` | Edit pending filter |
| `Esc` | Clear/cancel |

The filter applies to the next copy shortcut (`i`/`o`/`y`/`c`) automatically. Examples: `2:8`, `1,3,8:12`.

## Vim-style full view

Opened from the picker with `t` on Sessions, Dialogues, or Content. Launches the real `vim` with bindings configured for block navigation and copying.

| Key | Action |
| --- | --- |
| `myy` | Copy block |
| `myi` | Copy input |
| `myo` | Copy output |
| `mvv` | Select block |
| `mvi` | Select input |
| `mvo` | Select output |
| `p`, `q`, `Esc` | Return to picker |
