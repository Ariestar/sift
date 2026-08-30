---
title: Configuration
description: Create, inspect, edit, and understand sivtr configuration.
---

`sivtr` uses a TOML config file in the platform config directory. Configuration controls editor handoff, history retention, archive sync freshness, TUI theme, MCP idle exit, and the Windows hotkey chord.

## Commands

```bash
sivtr config show
sivtr config init
sivtr config edit
```

| Command | Behavior |
| --- | --- |
| `sivtr config show` | Print config path and effective file content or defaults |
| `sivtr config init` | Create the default config if it does not exist |
| `sivtr config edit` | Create the config if needed and open it in the configured editor |

## Default config

```toml
[editor]
command = ""

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

For a field-by-field reference, see [Config File](/reference/config-file/).

## History retention

```toml
[history]
auto_save = true
max_entries = 0
```

`max_entries = 0` means unlimited. Set `auto_save = false` when you do not want pipe and run captures written to history automatically.

## Archive sync freshness

Queries read from the unified local archive (`archive.db`). When the archive is older than `[sync].max_age_secs`, a query triggers an incremental re-sync first:

```toml
[sync]
# How stale the archive may be (seconds since last sync) before a query
# triggers an incremental re-sync. 0 = re-list on every query.
max_age_secs = 15
```

`0` re-lists sources on every query; raise it to trade freshness for latency. Run `sivtr sync` to force a pass. See [Data Locations](/reference/data-locations/).

## Hotkey chord

```toml
[hotkey]
chord = "alt+y"
```

This chord is used by `sivtr hotkey start` unless overridden with `--chord`.

Provider selection is a runtime CLI option, not a config key:

```bash
sivtr hotkey start --provider all
sivtr hotkey start --provider claude
```

## TUI theme

```toml
[theme]
mode = "auto"
```

`auto` follows the system appearance and picks the truecolor vs ANSI palette from the terminal. Force a scheme with `dark` or `light`:

```toml
[theme]
mode = "dark"
```

## MCP idle exit

```toml
[mcp]
idle_exit_secs = 60
```

Seconds without tool calls before the stdio MCP server exits (`0` = stay alive until the host closes stdin). The `sivtr mcp serve --idle-exit` flag overrides this per invocation.
