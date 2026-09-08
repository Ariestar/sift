# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sivtr** is a terminal output workspace that captures, browses, searches, and reuses terminal command output and AI coding assistant sessions. Agent providers are registry-driven (Codex, Claude Code, Cursor, OpenCode, OpenClaw, Grok, Hermes, Pi, …) and four shells (Bash, Zsh, PowerShell, Nushell). Cross-device remote memory uses a local daemon with Share/Grant/Mount over encrypted iroh transport.

Architecture: CLI binary (`src/`) wrapping a core library (`crates/sivtr-core/`). Native sources sync into one local archive; provider adapters only discover and parse, while CLI, TUI, MCP, Web API, and exports share the archive-backed record model. Clap-based subcommands cover capture, search, usage, stats, import/export, quality, sync, session labels, remote access, and diagnostics.

## Development Commands

```bash
cargo build                                         # debug
cargo test --workspace                              # all tests
cargo fmt --all -- --check                          # format check
cargo clippy --workspace --all-targets -- -D warnings # clippy (strict)
```

Pre-commit gate:
```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

## Workspace Structure

```
crates/sivtr-core/src/     ← Core library (no CLI deps)
  agents/                  ← AgentProvider registry + per-provider parsers
    mod.rs / model.rs / jsonl.rs / sqlite.rs
    claude.rs / codex.rs / cursor.rs / grok.rs / hermes.rs / openclaw.rs / opencode.rs / pi.rs
    generic.rs              ← shared adapters for the expanded provider catalog
  archive/                 ← unified SQLite archive, sync, stats, embeddings
  usage/                   ← token extraction, exact microdollar pricing
  record/
    model.rs               ← WorkRecord, WorkPart, WorkTime (canonical model)
    refs.rs                ← WorkRef parsing (local body + origin:body remote form)
    index.rs               ← Record indexing and lookup
  query/                   ← Workspace record/source loading
  search/                  ← Filter/Searcher pipeline, BM25 ranking (types.rs, filter.rs, bm25.rs, eval.rs)
  workspace.rs             ← Workspace resolution (git root → sessions), data_dir()
  workset.rs               ← WorkSet / WorkSelection* canonical selection model
  config/                  ← SivtrConfig, serde TOML
  session.rs               ← Session log reading
  time.rs                  ← Timestamp normalization
src/                       ← CLI binary
  main.rs                  ← Command routing
  cli/
    mod.rs                 ← Top-level Clap definitions (copy agents via registry external subcommand)
    remote.rs              ← serve/share/peer/remote/group/workspace Clap types
  commands/
    capture/               ← copy, pipe, run, init, flush, diff, clear, browse
    memory/                ← search, filter, var, nav, zoom, show, work, workset
    remote/                ← serve, share, mounts, peer, group, workspace
    system/                ← config, doctor, export, hotkey, import, quality, session, stats, sync, usage, version
  remote/                  ← Device daemon, identity, state, protocol, ipc
  tui/                     ← Terminal UI framework
```

## Key Data Types

- `WorkRecord` — single command execution or AI turn
- `WorkPart` — leaf content chunk; `WorkPartKind` is Prompt/Command/User/Assistant/ToolCall/ToolResult/Skill/Thinking/Output/Error
- `WorkRef` — typed address: `WorkScope` + `WorkPath` + `WorkAt` as `[scope:]path[/at]` (e.g. `terminal/session_42/3/p1`, `desk:codex/abc123/5/p2`)
- `WorkTime::from_components(started_at, ended_at, duration_ms)` — time construction
- `AgentProvider` — registry in `agents/mod.rs` (`AgentProvider::all()` / `from_command_name` / `command_names_csv`); do not hardcode provider lists in CLI/help
- Remote model: **Device Daemon + Identity + Share + Grant + Mount**

## Coding Rules

- **anyhow::Result** everywhere, always `.context("description")?`
- **No unwrap()** in production — tests use `expect("reason")`
- **No async** in most CLI paths — remote daemon uses async internally; command handlers stay blocking
- **Workspace separation** — `sivtr-core` must not depend on CLI types
- **clippy strict** — `-D warnings` on CI
- **Rust 2021 edition, MSRV 1.95** — see `Cargo.toml` `rust-version` (toolchain channel is `stable`)
- **Agent lists** — any CLI help / error that names providers must use `AgentProvider::command_names()` / `command_names_csv()`, not a hand-written list
- **Workspace filter** — session cwd filtering uses `filter_sessions_by_workspace` (unbound keep + path/remote match); do not reimplement per provider
## Working Directory

Always confirm before starting work:
```bash
pwd && git branch
```

## Shell Hook System

`sivtr init {shell}` injects precmd hooks using marker blocks (`# >>> sivtr shell integration >>>`). Session logs go to `$XDG_STATE_HOME/sivtr/session_<pid>.log`. Internal `sivtr flush` called by hooks on each prompt.

## Search Pipeline

```bash
sivtr search terminal --status failure --json | sivtr search terminal --exclude "example" -f timeline
sivtr search terminal "docker pull failed"   # positional QUERY = pure BM25 ranking (no regex)
sivtr search agent "bm25" -m ".*passage.*"   # --match regex bounds the set, QUERY ranks it
```

`search` is BM25-primary: a positional `QUERY` (or `--match`) makes relevance the
default sort and BM25 ranks the whole source; `--match` alone keeps the old
regex-filter behavior (its text doubles as the rank query). No query = recency
browse (latest=5).

Target selectors: `terminal/<session>/<record>`, `agent/<session>/<turn>`, `<provider>/<session>/<turn>`. Part refs: `<provider>/<session>/<turn>/p<part>` (1-based, e.g. `pi/019e4f40/3/p2`). Use `*` for wildcards. Named scopes: `desk:terminal/...`, `docs:codex/4`.

## Remote Memory

Device-scoped daemon auto-starts when share/remote commands need it.

```bash
sivtr share                   # interactive: pick workspace, create share only
sivtr share invite <name>       # issue single-use invite (stdout = bare key)
sivtr remote add desk <invite>  # name a peer share in this workspace (git-remote style)
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr serve status            # daemon identity + share/peer counts
sivtr ws list                 # local workspace origin labels
```

Groups (mesh: every member publishes their memory to the group):
```bash
sivtr group create team         # create group + share current workspace
sivtr group invite team         # reusable join link (expires, optional --max-uses)
sivtr group join <link>         # join + contribute current workspace
sivtr group list / members team # roster with last-seen
sivtr group remove team alice   # owner kicks (members self-heal on next sync)
sivtr group leave team          # leave; owner leaving disbands the group
sivtr s team:terminal "q"       # fan out to all members, merged
sivtr s team/alice:terminal "q" # one member (group/member scope form)
```
Group membership is a roster overlay on share/grant/mount: join = one multi-use invite with the owner, mirror roster locally, grant every member read on your group share. Owner is the roster source of truth; members pull-sync on a 5-min TTL (`GroupSync`), kicked devices drop the group on the next sync. `team:` refs are qualified per member (`team/alice:...`) so `show`/`zoom`/`nav` round-trip.

State lives under `data_dir()` (`SIVTR_DATA_DIR` override, else platform config dir `/sivtr`): `identity.key`, `remote-state.db`, `daemon.json`, `daemon.lock`, `daemon.log`.

## Diagnostics

```bash
sivtr doctor        # Check binary, config, session logs, hooks, providers, clipboard
sivtr init show     # Show which shell hooks are installed
sivtr init uninstall # Remove all shell hooks
```

Run `sivtr doctor` after any installation or when troubleshooting.
