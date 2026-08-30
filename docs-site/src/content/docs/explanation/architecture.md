---
title: Architecture
description: How the sivtr memory workspace is split between CLI, TUI, command handlers, remote daemon, and core modules.
---

`sivtr` is a Cargo workspace with two main layers:

- `sivtr`, the binary crate in `src/`;
- `sivtr-core`, the library crate in `crates/sivtr-core/`.

The binary owns user interaction: CLI parsing, command dispatch, TUI state, workspace pickers, platform-specific launcher/hotkey behavior, and the remote-memory daemon. The core crate owns reusable memory logic: capture, parsing, buffers, selection, search primitives, history, export, config, workspace resolution, and agent-provider session parsing.

## Workspace layout

```text
sivtr/
|- Cargo.toml
|- src/
|  |- cli/
|  |  |- mod.rs
|  |  `- remote.rs
|  |- main.rs
|  |- app.rs
|  |- commands/
|  |  |- capture/
|  |  |- memory/
|  |  |- remote/
|  |  `- system/
|  |- remote/          # daemon runtime
|  `- tui/
`- crates/
   `- sivtr-core/
      `- src/
         |- archive/         # unified SQLite archive (archive.db) + sync engine
         |- agents/          # AgentProvider registry + per-provider parsers
         |- buffer/
         |- capture/
         |- config/
         |- export/
         |- history/
         |- parse/
         |- query/
         |- record/          # WorkRecord / WorkRef (scope + path + at)
         |- search/
         |- selection/
         |- session/
         `- workspace.rs
```

## Binary crate

| Area | Responsibility |
| --- | --- |
| `cli/` | clap command definitions and help text (`mod.rs` + `remote.rs`) |
| `commands/capture/` | run, pipe, copy, init, flush, import, diff, clear, browse |
| `commands/memory/` | search, filter, var, nav, zoom, show, work, WorkSet store |
| `commands/remote/` | serve, share, remote (git-remote style names), peer, workspace list |
| `commands/system/` | config, doctor, history, hotkey, migrate, sync, version |
| `remote/` | device daemon, identity, SQLite state, protocol, local IPC |
| `app.rs` | captured-output browser state machine |
| `tui/` | terminal setup, event handling, browser rendering, workspace rendering |
| `command_blocks.rs` | parsed command-block spans for session browsing and copying |

This layer can depend on terminal UI libraries, platform APIs, process spawning, and (for the daemon) async networking.

## Core crate

| Module | Responsibility |
| --- | --- |
| `archive` | Unified SQLite archive (`archive.db`), schema, and the sync engine that fills it |
| `agents` | `AgentProvider` registry plus per-provider discovery/parsing (Codex, Claude, Cursor, OpenCode, OpenClaw, Hermes, Grok, Pi, …) |
| `record` | `WorkRecord`, `WorkPart`, `WorkRef` as `WorkScope` + `WorkPath` + `WorkAt` (`[scope:]path[/at]`) |
| `query` | load workspace records and local-shaped sources for CLI and daemon |
| `capture` | stdin, subprocess, and scrollback/session capture helpers |
| `parse` | ANSI stripping, Unicode display width, and line parsing |
| `buffer` | line, cursor, and viewport models |
| `selection` | visual, line, and block selection extraction |
| `search` | text matching and navigation state |
| `history` | SQLite storage, schema, and search |
| `export` | clipboard, file, and editor export helpers |
| `config` | TOML config model, defaults, and path resolution |
| `session` | structured shell session entries and rendering |
| `workspace` | git-root workspace resolution, registry, `data_dir()` |

This split keeps computation and data handling testable independently from the terminal UI.

## Capture flow

Pipe mode:

```text
stdin -> capture::pipe -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Run mode:

```text
subprocess -> combined output -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Session import:

```text
session log -> render entries -> parse::parse_lines -> Buffer -> command block spans -> TUI/editor
```

Command-block copy:

```text
session log -> SessionEntry list -> command blocks -> selector -> filters -> clipboard
```

Agent-provider copy:

```text
provider transcript/db -> AgentSession -> AgentBlock list -> selector -> filters -> clipboard
```

Workspace picker/search:

```text
terminal context + provider sessions -> WorkspaceSession list -> search/pick/show -> clipboard/stdout/json
```

## Unified archive

Queries (search, show, copy, TUI, MCP) read from one local SQLite archive (`archive.db`) instead of parsing native files on every run. The sync engine — `sivtr sync`, plus an automatic freshness pass on query — fills the archive from every agent provider and every workspace's terminal logs. Native session files remain the source of truth and are only read by the sync engine.

```text
terminal logs + provider sessions -> sync (stat-stamp compare) -> archive.db -> query paths
```

## Remote memory flow

```text
owner:  sivtr share -> daemon Share + InviteTicket
peer:   sivtr remote add alias invite -> Mount in current workspace
query:  desk:terminal/... -> resolve origin -> daemon IPC -> iroh -> authorize(share_id)
        -> load_workspace_source(root, body) -> SourceResponse -> WorkSet/show
```

Model: **Device Daemon + Identity + Share + Grant + Mount**. Sharing is opt-in and read-only; redaction is on by default.

## Provider boundary

Agent support is provider-neutral at the command and workspace layers. Provider modules are responsible for finding local records and converting provider-specific event formats into shared memory blocks:

```text
AgentProvider -> AgentSessionProvider -> AgentSession -> AgentBlock
```

The shared workspace code can then copy, pick, search, and show memory without depending on one vendor transcript shape.

## Design boundary

The frontend layer is presentation and interaction. The Rust core performs the durable memory work: parsing, capture, selection extraction, search, storage, provider parsing, and formatting. The remote daemon reuses core query loading over an authorized share root so remote and local refs share the same record model. This keeps UI changes from leaking into provider parsers and keeps provider changes from rewriting the whole CLI surface.
