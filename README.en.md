<p align="center">
  <img src="editors/vscode/icon.png" alt="sivtr logo" width="96" height="96">
</p>

<h1 align="center">sivtr</h1>

<p align="center">
  A unified agent memory workspace for human and agent
  <br>
  agents and terminals share one context to use
  <br>
  <strong>Your agent memory doesn’t need to be a heavyweight knowledge system.</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/sivtr"><img alt="Crates.io" src="https://img.shields.io/crates/v/sivtr?style=flat-square"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ariestar.sivtr-vscode"><img alt="VS Code Marketplace" src="https://vsmarketplacebadges.dev/version/ariestar.sivtr-vscode.svg?style=flat-square&label=VS%20Code&color=007ACC"></a>
  <a href="https://github.com/Ariestar/sivtr/actions/workflows/rust.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/Ariestar/sivtr/rust.yml?branch=main&style=flat-square"></a>
  <a href="https://deepwiki.com/Ariestar/sivtr"><img alt="Ask DeepWiki" src="https://deepwiki.com/badge.svg?repo=Ariestar/sivtr"></a>
  <a href="Cargo.toml"><img alt="Rust" src="https://img.shields.io/badge/rust-1.95%2B-orange?style=flat-square"></a>
  <a href="https://linux.do/"><img alt="linux.do" src="https://img.shields.io/badge/friend-linux.do-1f883d?style=flat-square"></a>
</p>

<p align="center">
  <a href="README.md">简体中文</a>
  ·
  <strong>English</strong>
  ·
  <a href="https://sivtr.pages.dev/">Docs</a>
  ·
  <a href="https://sivtr.pages.dev/zh-cn/">中文文档</a>
</p>

<p align="center">
  <a href="https://sivtr.pages.dev/playbooks/">
    <img src="docs-site/public/demo/4.gif" alt="sivtr demo: save search results as variables and keep narrowing" width="820">
  </a>
  <br>
  <sub>
    Save matches as memory variables and keep narrowing ·
    <a href="https://sivtr.pages.dev/playbooks/fix-terminal-error/">Fix terminal errors</a> ·
    <a href="https://sivtr.pages.dev/playbooks/recent-work-timeline/">Build timelines</a> ·
    <a href="https://sivtr.pages.dev/playbooks/agent-handoff/">Handoff with evidence</a>
  </sub>
</p>

---

## Why sivtr?

Developers and agents lose time reconstructing context that already exists locally: terminal failures, test output, tool logs, and previous AI sessions. `sivtr` turns that work into searchable memory without asking you to adopt a heavyweight knowledge system.

With `sivtr`, you can:

- ask an agent to fix the latest failure without pasting the log;
- find yesterday's test output, build error, or decision in seconds;
- reopen the exact command output or agent reply behind a summary;
- save useful search results as named variables like `@failures` and reuse them in the next command.

> [!IMPORTANT]
> For agent workflows, install the `sivtr` CLI, register the MCP server with `sivtr mcp install`, and optionally add the bundled `sivtr-memory` skill. MCP is the main way agents read local evidence; the skill teaches when and how to use it.

## Features

- **MCP-first agent memory**: install once with `sivtr mcp install`, then agents call search/show/zoom/filter/status/usage/stats tools instead of asking you to paste logs.
- **Terminal capture that keeps the output**: capture commands from Bash, Zsh, PowerShell, and Nushell, including stdout, stderr, exit code, cwd, and timing; `pipe` and `run` write directly to the same archive.
- **One local archive**: terminal and agent sessions sync into one `archive.db`; providers only discover and parse, while search, filters, exports, the TUI, MCP, and the Web API share the same record model.
- **40+ agent providers**: Codex, Claude Code, Cursor, Dsh, Gemini, Goose, Hermes, OpenCode, OpenClaw, Grok, Pi, Qoder, Qoder-CN, Qwen, plus JSONL, SQLite, directory, and container formats through one registry and query surface.
- **Exact evidence, not summaries**: every hit resolves to a stable ref you can show, zoom, filter, or hand to the next agent.
- **Named memory variables**: save result sets as `@failures`, reuse `@last`, pipe with `@`, and slice with `@failures[1,3..5]`.
- **Usage and costs**: extract transcript token usage and calculate costs with exact integer microdollars and an embedded pricing snapshot; unknown prices stay explicitly unpriced.
- **Optional semantic / hybrid search**: configure an OpenAI-compatible embedding endpoint for vector ranking or RRF fusion with BM25; structured search remains complete without it.
- **Stats, quality, and portable import/export**: `stats`, secret findings, starred sessions, Claude.ai/ChatGPT JSON/ZIP import, and JSON/JSONL/Markdown/HTML export.
- **Cross-device access**: share a workspace read-only and browse another device with a `desk:...` ref; multiple devices can form a `group` that syncs membership and lets members read each other's memory.
- **Configurable theme**: `[theme] mode = auto|dark|light`, follows the system appearance and detects truecolor.
- **One-command setup**: `sivtr setup` for hooks + MCP host install; `sivtr doctor --fix` to repair.
- **CLI still there when you want it**: search, show, filter, nav, and a TUI browser for humans — useful, not the main product story.

## Quick start

Install the prebuilt CLI (no Rust toolchain needed):

```bash
cargo binstall sivtr
```

On Linux, `cargo binstall` installs the static musl build by default (no GLIBC version requirement). Same asset as `install.sh`.

Other ways:

```bash
cargo install sivtr                  # build from source (needs Rust)
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh   # Linux/macOS/WSL one-liner
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Ariestar/sivtr/main/install.ps1 | iex
```

Upgrade:

```bash
sivtr update    # download the latest release, verify SHA256, replace in place
```

First-time setup (hooks + MCP hosts):

```bash
sivtr setup             # hooks + MCP hosts + sivtr-memory skill (if missing)
# or step by step:
sivtr init powershell   # or bash, zsh, nushell
sivtr mcp install       # detect installed hosts; or -p claude,cursor,codex,opencode,openclaw,grok,hermes,pi,qoder,qodercn,gemini,qwen,goose
npx skills add Ariestar/sivtr --skill sivtr-memory -g -y
sivtr doctor
```

Sync and inspect the archive:

```bash
sivtr sync
sivtr usage daily
sivtr stats
```

> [!NOTE]
> On Windows, if `sivtr init powershell` reports that the profile did not load, raise the current-user execution policy once with `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. sivtr never edits the registry — the hook lives only in your PowerShell profile.

## Agent memory (MCP)

This is the main path. After `sivtr mcp install`, agents get structured tools over local terminal + AI session memory:

| Tool | Use |
| --- | --- |
| `sivtr_search` | Find recent failures, decisions, or commands |
| `sivtr_show` | Open the exact record/part behind a hit |
| `sivtr_zoom` | Expand surrounding context |
| `sivtr_filter` | Narrow a result set |
| `sivtr_status` | Workspace / remote / origin health |
| `sivtr_usage` | Token usage and model costs |
| `sivtr_stats` | Archive activity, quality, and usage statistics |

Optional skill (teaches the agent when to call those tools):

```bash
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

Then ask:

```text
Fix the latest terminal error. Use sivtr first.
```

The agent should search local evidence, open the matching output, patch, and verify — without you pasting logs.

CLI search is still available when you want it yourself:

```bash
sivtr s terminal --status failure --latest 5 --refs
sivtr s agent -m "TODO|decision|failed" --since today -f timeline
```

## Examples

More end-to-end walkthroughs live in the [Playbooks](https://sivtr.pages.dev/playbooks/).

| Workflow | What you do | Demo |
| --- | --- | --- |
| Fix the latest terminal error | Ask your agent (MCP): <br><code>Fix the latest terminal error. Use sivtr first.</code> | <img src="docs-site/public/demo/1.gif" alt="Fix the latest terminal error with sivtr" width="320"> |
| Continue after interruption | Ask your agent: <br><code>Continue. Use sivtr memory first.</code> | <img src="docs-site/public/demo/5.gif" alt="Continue after interruption with sivtr memory" width="320"> |
| Prepare a handoff for the next agent | Ask your agent: <br><code>Give the next agent a handoff with evidence.</code> | <img src="docs-site/public/demo/6.gif" alt="Prepare an evidence-backed handoff" width="320"> |
| Turn recent work into a timeline | <code>sivtr s agent --since today --sort oldest -f timeline</code><br><code>sivtr s terminal --since today --sort oldest -f timeline</code> | <img src="docs-site/public/demo/3.gif" alt="Build a recent work timeline" width="320"> |
| Save results as variables and chain them | <code>sivtr s terminal -m "panic" --save failures</code><br><code>sivtr filter @failures --status failure --refs</code> | <img src="docs-site/public/demo/4.gif" alt="Chain saved memory variables" width="320"> |

## Core concepts

| Concept | Meaning |
| --- | --- |
| WorkRecord | One useful work event: a terminal command, agent turn, tool call, or captured output block. |
| WorkPart | The command, output, assistant reply, tool output, or error inside a record. Use this when you only need the useful part, not the whole event. |
| WorkRef | A stable address for one exact piece of memory, for example `pi/<session>/3/p1`. Good for citations and reproducible handoffs. |
| WorkSet | The data behind memory variables such as `@last` and `@failures`: an ordered list of refs you can filter, save, slice, pipe, navigate, expand, and show. |

Memory variables:

| Handle | Use |
| --- | --- |
| `@last` | The latest search or projection result. |
| `@name` | A named variable created with `--save name` or `sivtr var set name`, for example `@failures`. |
| `@name[1,3..5]` | Pick only a few items from a saved variable. |
| `@` | Use the result coming from the previous command in a pipeline. |

## Command cheat sheet

The full command, subcommand, and flag reference lives in the [CLI Reference](https://sivtr.pages.dev/reference/cli/). Core commands at a glance:

**Setup & maintenance**

```bash
sivtr setup                              # one-command setup: env detect + hooks + MCP + skill + smoke
sivtr doctor --fix                       # diagnose and auto-repair binary/config/hooks/providers
sivtr mcp install -p claude,cursor,codex # register MCP for specific hosts (or detect installed)
sivtr update                             # self-update to the latest release
sivtr config show                        # view config (init writes defaults / edit opens $EDITOR)
```

**Everyday use**

```bash
sivtr                                    # TUI workspace browser
sivtr run cargo test                     # run a command and capture its output (run <COMMAND> [ARGS...])
sivtr s terminal --status failure --latest 5 --refs   # 5 most recent failed terminal events
sivtr s agent -m "panic|TODO" --since today -f timeline # today's agent decisions as a timeline
sivtr show @last                         # open the content behind the last search
sivtr show desk:terminal/session_42/3    # open an exact remote ref
sivtr copy                               # copy the most recent command block
sivtr copy out 2..4                      # output of blocks 2..4
sivtr copy in --pick --regex panic       # interactively pick the input block matching "panic"
sivtr copy cmd --pick                    # interactively pick a command itself
sivtr copy 3 --print                     # print block 3 to stdout
sivtr usage daily --breakdown             # token and cost by day/provider/model
sivtr usage session codex/<session-id>    # usage for one session
sivtr export sessions --format markdown   # export archived sessions
sivtr import sessions --provider chatgpt conversations.json
```

**Remote & collaboration**

```bash
sivtr share                              # interactively create a read-only share of a workspace
sivtr share invite <share> --expires 10m # mint a single-use invite (stdout = bare key)
sivtr remote add desk <invite-key>       # mount a teammate's share as local remote `desk`
sivtr group create <name>                  # create a group and contribute the current workspace
sivtr group invite <name> --expires 1d --max-uses 10   # mint a multi-device join link
sivtr group join <invite-key>            # join and contribute your own workspace
sivtr group members team                 # members and their contributions
sivtr s <peer>:terminal --status failure --latest 5 --refs  # read a teammate like local
```

## Remote access

Two devices running sivtr can read each other's workspace sessions like reading local — for collaborative work where you want to see a teammate's terminal output or AI session without leaving your machine.

Refs use a single form: `origin:body`.

```text
codex/4                 # local current workspace
docs:codex/4            # another local workspace by name
desk:terminal/...       # remote name from `remote add`
alice/sivtr:hermes/...  # device/workspace coordinate
```

On the device that owns the workspace:

```bash
sivtr share                   # pick workspace (Enter = current); create share only
sivtr share invite <name>     # single-use invite (stdout = bare key)
sivtr ws list                 # see local workspace origin labels
```

On the other device:

```bash
sivtr remote add desk <invite>   # bare key from `sivtr share invite` stdout
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr show desk:terminal/session_42/3
sivtr zoom desk:terminal/session_42/3 -C 2
sivtr nav desk:terminal/session_42/3 +1 --refs
sivtr copy desk:terminal/session_42/3 --print
```

Sharing is opt-in and read-only. Secrets are redacted by default before data leaves the machine. Remote access uses encrypted iroh transport; the daemon auto-starts when needed. Unregistered origins error — register remotes with `sivtr remote add`, or list local workspaces with `sivtr ws`.

## Groups

When more than two devices want long-lived shared memory, instead of each `share` + `remote add`, form a group: every device contributes its own workspace and members sync automatically and read each other's memory.

```bash
# Owner on device A:
sivtr group create <name>        # create the group and contribute the current workspace
sivtr group invite <name>        # mint a multi-device join link (stdout = bare key)

# Member on device B:
sivtr group join <invite>      # join and contribute your own workspace
sivtr group list               # all groups
sivtr group members <name>     # members and their contributions
sivtr group sync <name>        # force a roster pull

# Day to day: read a teammate's memory like local
sivtr s <peer>:terminal --status failure --latest 5 --refs
```

The owner manages the group: `rename` to rename, `remove <group> <peer>` to kick; when the owner `leave`s, the group disbands. Membership changes sync to every member automatically.

## Supported sources

| Source | Support |
| --- | --- |
| Terminal | Bash, Zsh, PowerShell, Nushell shell hooks; pipe and run capture. |
| Codex | Local rollout/session JSONL files. |
| Claude Code | Local transcript/session files. |
| Cursor | Local Cursor agent transcript JSONL. |
| OpenCode | Local session database. |
| OpenClaw | Local OpenClaw agent SQLite (+ legacy JSONL). |
| Hermes | Local Hermes `state.db` (JSONL under `sessions/` as residual). |
| Grok | Local Grok agent sessions under `~/.grok` (`GROK_HOME`). |
| Dsh | Local Dsh agent sessions. |
| Gemini | Local Gemini CLI sessions. |
| Goose | Local Goose agent sessions. |
| Pi | Local Pi agent session logs. |
| Qoder / Qoder-CN | Local Qoder and Qoder-CN agent sessions. |
| Qwen | Local Qwen Code sessions. |
| More providers | Amp, Aider, Antigravity, Copilot, Forge, gptme, iFlow, Kimi, Kiro, OpenHands, Poolside, RooCode, Trae, Vibe, VSCode Copilot, Windsurf, Zed, Zencoder, and more. |
| Web imports | Claude.ai and ChatGPT JSON/ZIP exports via `sivtr import sessions`, stored in the archive. |

## Documentation

- Documentation: [https://sivtr.pages.dev/](https://sivtr.pages.dev/)
- 中文文档: [https://sivtr.pages.dev/zh-cn/](https://sivtr.pages.dev/zh-cn/)
- Playbooks: [https://sivtr.pages.dev/playbooks/](https://sivtr.pages.dev/playbooks/)
- CLI reference: [docs-site/src/content/docs/reference/cli.md](docs-site/src/content/docs/reference/cli.md)
- Memory skill: [skills/sivtr-memory](skills/sivtr-memory)

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, PR expectations, and coding guidelines.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Docs site:

```bash
cd docs-site
bun install --frozen-lockfile
bun run build
```

Repository layout:

```text
crates/sivtr-core/  core model, provider parsers, archive, usage, search, config
src/                CLI commands, TUI, shell hooks, hotkey integration
docs-site/          Astro/Starlight documentation site
editors/vscode/     VS Code bridge for the AI session picker
skills/             bundled agent skills
```
