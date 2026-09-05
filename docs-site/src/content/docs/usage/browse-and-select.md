---
title: Browse and Select
description: Navigate the workspace browser, fold structure blocks, select, and copy.
---

The **workspace browser** (bare `sivtr` on a TTY, or `sivtr copy --pick` / hotkey) is the interactive surface: multi-source Source → Sessions → Dialogues → Content. `sivtr pipe` and `sivtr run` no longer open a TUI — they write to the unified archive and open the external editor.

## Open the workspace browser

```bash
sivtr                     # TTY: multi-source workspace browser
sivtr --all               # also select remote mounts on open
sivtr copy --pick         # same browser, for copy
sivtr copy claude --pick
```

Layout: Source · Sessions · Dialogues · Content. Content splits into **Input** and **Output** halves with independent scroll.

### Workspace navigation

| Key | Action |
| --- | --- |
| `0` / `1` / `2` / `3` | Focus Source, Sessions, Dialogues, or Content |
| `h` / `l` | Previous / next pane |
| `j` / `k` | Move down / up |
| `Space` | Toggle selection (source, session, or dialogue) · mark a content block |
| `a` | 切换当前面板的全部条目 |
| `g` / `t` | Select agent sources / terminal source (Source) |
| `R` | Refresh next level under active rows |
| `v` | Range-select rows · block-range mark a span (Content) |
| `Tab` | Switch Content Input ↔ Output half |
| `r` | Toggle read/raw content (structure markers + fold tags vs expanded payloads) |
| `Ctrl-d` / `Ctrl-u` · `PgDn` / `PgUp` | Scroll content |
| `g` / `G` | Content top / bottom |
| `i` / `o` / `y` / `c` | Copy input / output / block / command |
| `Enter` | Confirm / open next / copy; fold/unfold the cursor block (Content) |
| `/` | Search |
| `z` | Toggle focused pane fullscreen |
| `t` | Open Vim-style full view (Sessions/Dialogues/Content) |
| `?` | Help |
| `q` / `Esc` | Quit / back |

Mouse: click focuses + selects; drag selects linearly and `Ctrl`-drag is block select; clicking the dot gutter toggles a block mark; a single/double click folds a structure block. Content half heights bias toward the focused half.

### Structure blocks

Every workpart is a foldable block, and tool call + result group into one block. Consecutive structure units (tool / skill / thinking) fold into a **run** shown as `<:kind xN:>`:

- `Enter` on the cursor block folds/unfolds it.
- Clicking a run tag expands it to its members; clicking a member expands its body (two levels).
- `r` toggles read mode (markdown + fold tags) and raw mode (full payloads).

Run members and tool calls are grouped by call id, so interleaved parallel tool calls pair correctly.

See [Keybindings](/reference/keybindings/) for the full table.

## Copy to clipboard

Configure the editor used by `pipe`/`run`/`import` (not a TUI):

```toml
[editor]
command = "nvim"
```
