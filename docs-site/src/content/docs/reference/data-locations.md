---
title: Data Locations
description: Where sivtr stores configuration, history, session logs, and provider data.
---

`sivtr` is local-first. Most data it uses is already on your machine, and generated data is written under platform config or state directories unless you explicitly export it elsewhere.

## Config file

| Platform | Path |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

## Shell session logs

Shell integration writes per-process structured session logs.

| Shell/platform | Typical path |
| --- | --- |
| Windows PowerShell / PowerShell 7 | `%APPDATA%\sivtr\session_<pid>.log` |
| Bash / Zsh | `$XDG_STATE_HOME/sivtr/session_<pid>.log` or `~/.local/state/sivtr/session_<pid>.log` |
| Nushell | Nushell config/state area with a `sivtr` session file |

These logs power:

- `sivtr import`;
- `sivtr copy` command-block workflows;
- `sivtr diff`;
- command-block navigation in the browser.

## History database

Captured terminal output is stored in a local SQLite history database when `[history].auto_save = true`.

Use CLI commands instead of editing the database directly:

```bash
sivtr history list
sivtr history search "panic"
sivtr history show 42
```

Retention is controlled by:

```toml
[history]
max_entries = 0
```

`0` means unlimited.

## Agent provider data

`sivtr` reads provider-owned local data. It does not upload transcripts.

| Provider | Data source |
| --- | --- |
| Codex | `~/.codex/sessions` rollout JSONL files |
| Claude Code | Current transcript/session environment and local Claude transcripts |
| Hermes | Primary: `$HERMES_HOME/state.db` (Windows default `%LOCALAPPDATA%\hermes`, else `~/.hermes`). Residual: `sessions/*.jsonl` under the same home. |
| OpenCode | OpenCode local database |
| Cursor | `~/.cursor/projects/**/agent-transcripts/**/*.jsonl` (override home with `CURSOR_HOME`) |
| OpenClaw | `~/.openclaw/agents/<id>/agent/openclaw-agent.sqlite` (legacy JSONL under `sessions/`) |
| Grok | `~/.grok/sessions/**` (`summary.json` + `chat_history.jsonl`; override home with `GROK_HOME`) |
| Pi | Pi agent session JSONL files |

Provider formats differ; `sivtr` normalizes them into sessions and dialogue units for copy, picker, search, and show workflows.

## Unified archive

Search, show, copy, picker, TUI, and MCP queries read from a unified local archive instead of parsing native session files on every query.

| Platform | Path |
| --- | --- |
| Windows | `%APPDATA%\sivtr\archive.db` |
| macOS | `~/Library/Application Support/sivtr/archive.db` |
| Linux | `~/.config/sivtr/archive.db` |

It is a SQLite database (WAL mode) written by the sync engine: `sivtr sync` runs a pass explicitly, and queries run an automatic freshness pass when the archive is older than `[sync].max_age_secs`. Native agent session files and shell session logs remain the source of truth and are only read by the sync engine. Override the root with `SIVTR_DATA_DIR`.

## Generated launchers

Linux shortcut generation writes:

- `~/.local/bin/sivtr-pick-codex`;
- `~/.local/share/applications/sivtr-pick-codex.desktop`.

macOS shortcut generation writes:

- `~/.local/bin/sivtr-pick-codex`;
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`.

Windows hotkey state is stored under `sivtr`'s platform config/state area and is managed by:

```bash
sivtr hotkey status
sivtr hotkey stop
```

## Remote daemon state

Cross-device remote memory uses a device-scoped daemon. Override the root with `SIVTR_DATA_DIR`; otherwise it is the platform config directory under `sivtr` (same root as `data_dir()`).

| File | Purpose |
| --- | --- |
| `identity.key` | Stable device identity for iroh |
| `remote-state.db` | SQLite peers, shares, grants, invites, mounts, audit |
| `daemon.json` | Running daemon control info (port, token, node id) |
| `daemon.lock` | Single-instance lock |
| `daemon.log` | Daemon log file (`sivtr serve logs`) |

```bash
sivtr serve status
sivtr serve logs
sivtr share list
sivtr remote list
sivtr peer list
sivtr ws list
```

Remote access is opt-in. Nothing is shared until `sivtr share` (or `share add`) runs. Mounts are workspace-local aliases registered with `sivtr remote add`. Feature guide: [Remote Access](/usage/remote-access/).
