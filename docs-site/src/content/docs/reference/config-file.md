---
title: Config File
description: TOML configuration reference.
---

## Location

`sivtr` uses the platform config directory:

| Platform | Current path |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

## Full example

```toml
[editor]
command = "nvim"

[history]
auto_save = true
max_entries = 0

[sync]
max_age_secs = 15

[hotkey]
chord = "alt+y"

[theme]
mode = "auto"

[mcp]
idle_exit_secs = 60
```

## editor

```toml
[editor]
command = "nvim"
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `command` | string | `""` | Editor command. Empty means auto-detect. |

Examples:

```toml
command = "hx"
command = "nvim"
command = "vim"
command = "code --wait"
```

## history

```toml
[history]
auto_save = true
max_entries = 0
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `auto_save` | boolean | `true` | Save captured output to history |
| `max_entries` | integer | `0` | Maximum entries to retain. `0` means unlimited. |

## sync

```toml
[sync]
max_age_secs = 15
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `max_age_secs` | integer | `15` | How stale the archive may be (seconds since the last sync) before a query triggers an incremental re-sync. `0` re-lists on every query. |

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | Chord used by `sivtr hotkey start` |

## theme

```toml
[theme]
mode = "auto"
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `mode` | string | `"auto"` | TUI color scheme: `auto`, `dark`, or `light` |

`auto` follows the system appearance (macOS/Linux XDG/Windows registry) and picks the truecolor vs ANSI palette from terminal capability. `dark` and `light` force a palette. The key rejects unknown values and typos (e.g. `mode = "ligth"` is a hard error).

## mcp

```toml
[mcp]
idle_exit_secs = 60
```

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `idle_exit_secs` | integer | `60` | Seconds without tool calls before the stdio MCP server exits; `0` keeps it alive until the host closes stdin. The `sivtr mcp serve --idle-exit` flag overrides this. |
