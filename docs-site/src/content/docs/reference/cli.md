---
title: CLI Reference
description: Command syntax, subcommands, options, providers, selectors, and examples.
---

This page documents the public CLI surface. The source of truth is `src/cli/`; run `sivtr --help` and `sivtr <command> --help` for installed-version help.

## Top-level

```bash
sivtr [COMMAND]
sivtr --all              # with bare TTY: also select remote mounts on open
```

With no command:

- **TTY** → multi-source workspace browser (Source / Sessions / Dialogues / Content).
- **Piped stdin** → same as `sivtr pipe`: write to history and open the external editor.

## run

```bash
sivtr run <COMMAND> [ARGS...]
```

Runs a command, captures combined stdout/stderr, reports the exit status, saves history when enabled, and opens the captured output in the external editor.

```bash
sivtr run cargo test
sivtr run git status --short
```

## pipe

```bash
sivtr pipe
```

Reads stdin and opens it. Piping directly to `sivtr` is equivalent:

```bash
cargo build 2>&1 | sivtr
```

## import

```bash
sivtr import
```

Opens the current structured shell session log. Requires shell integration.

## init

```bash
sivtr init <TARGET>
```

Supported targets:

| Target | Purpose |
| --- | --- |
| `powershell` | Install Windows PowerShell hook |
| `pwsh` | Alias for PowerShell integration |
| `bash` | Install Bash hook |
| `zsh` | Install Zsh hook |
| `nushell` / `nu` | Install Nushell hook |
| `tmux` | Install tmux picker binding |
| `linux-shortcut` | Generate Linux desktop/terminal picker launcher |
| `macos-shortcut` | Generate macOS Terminal/LaunchAgent picker launcher |

## copy

```bash
sivtr copy [MODE] [SELECTOR] [OPTIONS]
```

Command-block modes:

| Mode | Meaning |
| --- | --- |
| no mode | Copy input plus output |
| `in` | Copy input |
| `out` | Copy output |
| `cmd` | Copy bare command |

Aliases:

| Alias | Expands to |
| --- | --- |
| `sivtr c` | `sivtr copy` |
| `sivtr ci` | `sivtr copy in` |
| `sivtr co` | `sivtr copy out` |
| `sivtr cc` | `sivtr copy cmd` |

Common options:

| Option | Meaning |
| --- | --- |
| `--ansi` | Copy ANSI-decorated text when available |
| `--pick` | Open the interactive picker |
| `--print` | Print copied text after copying |
| `--regex <PATTERN>` | Keep lines matching regex |
| `--lines <SPEC>` | Keep selected 1-based lines |

Input-capable modes also support:

| Option | Meaning |
| --- | --- |
| `--prompt <TEXT>` | Rewrite the copied input prompt |

Examples:

```bash
sivtr copy
sivtr copy 3 --print
sivtr copy --prompt ":"
sivtr copy in 2..4
sivtr copy out --pick --regex panic
sivtr copy cmd --pick
```

## copy agent provider sessions

```bash
sivtr copy <PROVIDER> [MODE] [SELECTOR] [OPTIONS]
```

Providers come from the `AgentProvider` registry (not a hand-written CLI list):

| Provider | Command |
| --- | --- |
| Codex | `sivtr copy codex` |
| Claude Code | `sivtr copy claude` |
| Cursor | `sivtr copy cursor` |
| OpenCode | `sivtr copy opencode` |
| OpenClaw | `sivtr copy openclaw` |
| Hermes | `sivtr copy hermes` |
| Grok | `sivtr copy grok` |
| Pi | `sivtr copy pi` |

Modes:

| Mode | Meaning |
| --- | --- |
| no mode | Last completed user + assistant turn |
| `in` | Last user message |
| `out` | Last assistant reply |
| `tool` | Last tool output |
| `all` | Whole parsed session |

Agent copy options include all common copy options plus:

| Option | Meaning |
| --- | --- |
| `--session <N|ID>` | Select the Nth newest selectable session, or match an id/id prefix |

Examples:

```bash
sivtr copy claude
sivtr copy claude out --print
sivtr copy cursor out --print
sivtr copy hermes out --print
sivtr copy grok out --print
sivtr copy claude --session 2
sivtr copy codex 2..4
sivtr copy codex out --pick
sivtr copy opencode all --lines 1:20
sivtr copy pi tool --regex error
```

## diff

```bash
sivtr diff <LEFT> <RIGHT> [OPTIONS]
```

Compares two recent command blocks from the current shell session. Each selector must resolve to exactly one block.

Content options:

| Option | Meaning |
| --- | --- |
| `--output` | Compare output text. This is the default. |
| `--block` | Compare input plus output |
| `--input` | Compare input with prompt |
| `--cmd` | Compare bare command text |

View option:

| Option | Meaning |
| --- | --- |
| `--side-by-side` | Show a two-column text view |

Examples:

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
sivtr diff 2 1 --side-by-side
```

## search

```bash
sivtr search <TARGET> [QUERY] [OPTIONS]
```

Searches captured terminal records and supported AI workspace sessions. The target chooses where to search; filters choose which records match. A plain-text positional `QUERY` (no regex) ranks the source by BM25 relevance and becomes the default sort; `--match` optionally bounds the set with a regex first.

Targets:

| Target | Meaning |
| --- | --- |
| `terminal[/<session>[/<record>[/p<part>]]]` | Terminal command records |
| `agent[/<session>[/<turn>[/p<part>]]]` | All registered AI/agent records |
| `codex` / `claude` / `cursor` / `opencode` / `openclaw` / `grok` / `hermes` / `pi` / `qoder` `[/<session>[/<turn>[/p<part>]]]` | One provider's records |
| `<origin>:<target>` | Named remote or other local workspace origin, for example `desk:terminal` or `docs:codex/4` |

Use `*` for wildcard path segments, for example `terminal/*/3` or `pi/*/*`. Origins come from `sivtr remote add <alias> ...` or local workspace names listed by `sivtr ws list`.

Options:

| Option | Meaning |
| --- | --- |
| `QUERY` | Plain-text search query; BM25 ranks the source by these terms (no regex). Default sort becomes `relevance`. |
| `--match <REGEX>`, `-m <REGEX>` | Case-insensitive regex that bounds the set before relevance ranking |
| `--exclude <REGEX>`, `-v <REGEX>` | Case-insensitive exclusion filter applied after matches are found |
| `--in <FIELD>`, `-i <FIELD>` | `content`, `title`, `session`, `input`, `output`, `command`, or `all`; default is `content` |
| `--kind <KIND>` | Part kind filter: `prompt`, `command`, `user`, `assistant`, `tool`, `tool_call`, `tool_result`, `skill`, `thinking`, `output`, or `error` |
| `--status <STATUS>` | `success`, `failure`, or `unknown` |
| `--exit-code <CODE>` | Exact terminal process exit code |
| `--min-duration <DURATION>` | Minimum command duration, e.g. `500ms`, `2s`, `1m` |
| `--max-duration <DURATION>` | Maximum command duration |
| `--sort <SORT>` | `newest` (default), `relevance` (default with a `QUERY` or `--match`), `oldest`, `duration`, `duration-asc`, `exit-code`, or `exit-code-asc` |
| `--cwd <PATH>` | Workspace directory used to resolve records |
| `--since <TIME>` | Only include records at or after this time |
| `--until <TIME>` | Only include records at or before this time |
| `--last <DURATION>` | Recent time window, e.g. `30m`, `2h`, `7d` |
| `--latest <N>` | Return the latest N matching records before final sort. Defaults to `5` when neither `--latest` nor `--limit` is set (relevance sort ranks the whole set and skips the recency window). |
| `-l, --limit <N>` | Maximum result groups to print (hard ceiling after latest/sort) |
| `--exclude-current`, `--other` | Exclude the current agent session from agent searches |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs`; prints refs, one per line |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset`; terminal stdout defaults to `full`, piped stdout defaults to `workset` |

When stdout is piped and no explicit format is selected, WorkSet commands emit WorkSet JSON for the next command. Use `--refs` or `-f timeline` only at the final display step.

Time filters accept RFC3339 timestamps, Unix seconds/milliseconds, relative durations like `30m`, `2h`, `7d`, and aliases such as `today`, `yesterday`, `tomorrow`, `this morning`, `this afternoon`, `this evening`, `tonight`, and `now`.

Examples:

```bash
sivtr search terminal --status failure --latest 1 --json
sivtr s terminal "docker pull failed" --latest 20 --refs
sivtr s terminal -m "panic|failed" -v "example|sample" --since today --refs
sivtr s terminal -m "panic|failed" | sivtr filter @ -v "demo" -i title -f timeline
sivtr search agent --match "TODO|failed|next step" --since yesterday --format md
sivtr search pi --since today --sort oldest --format timeline
sivtr search pi/019e5941 --match "cargo test" --format compact
sivtr search terminal/session_13104/3 --format workset
```

## eval

```bash
sivtr eval [OPTIONS]
```

Benchmarks retrieval quality against golden queries: freezes the current workspace records into a snapshot, then ranks the corpus per query and reports recall@k / precision@k / MRR / NDCG@k. See [`docs/retrieval-eval.md`](https://github.com/Ariestar/sivtr/blob/main/docs/retrieval-eval.md) for the methodology and measured results.

| Option | Meaning |
| --- | --- |
| `--k <K>` | Evaluation depth (default `5`) |
| `--sort <SORT>` | Sort strategy to benchmark (default `newest`) |
| `--snapshot <PATH>` | Frozen eval snapshot file (queries + corpus JSON) |
| `--create-snapshot <PATH>` | Dump current workspace records into a new snapshot (queries start empty) |
| `--export <DIR>` | Write `qrels.txt` and `results.txt` (trec_eval format) into this directory |
| `--json` | Emit the report as JSON |

Example workflow:

```bash
sivtr eval --create-snapshot snap.json   # then edit snap.json: add labeled queries { name, query, relevant: [...] }
sivtr eval --snapshot snap.json --sort relevance --json
```

## filter

```bash
sivtr filter [SOURCE] [OPTIONS]
```

Filters a source or piped WorkSet with the same shared WorkSet filter surface used by `search`. If `SOURCE` is omitted it defaults to `@`, meaning WorkSet JSON from stdin.

Options:

| Option | Meaning |
| --- | --- |
| `--parts` | Select matching part anchors instead of preserving the input anchor granularity |
| `--match <REGEX>`, `-m <REGEX>` | Case-insensitive content filter |
| `--exclude <REGEX>`, `-v <REGEX>` | Case-insensitive exclusion filter |
| `--in <FIELD>`, `-i <FIELD>` | `content`, `title`, `session`, `input`, `output`, `command`, or `all` |
| `--kind <KIND>` | Part kind filter: `prompt`, `command`, `user`, `assistant`, `tool`, `tool_call`, `tool_result`, `skill`, `thinking`, `output`, or `error` |
| `--status <STATUS>` | `success`, `failure`, or `unknown` |
| `--exit-code <CODE>` | Exact terminal process exit code |
| `--min-duration <DURATION>` | Minimum command duration |
| `--max-duration <DURATION>` | Maximum command duration |
| `--sort <SORT>` | `newest`, `oldest`, `duration`, `duration-asc`, `exit-code`, or `exit-code-asc` |
| `--cwd <PATH>` | Workspace directory used to resolve records |
| `--since <TIME>` / `--until <TIME>` / `--last <DURATION>` | Time filters |
| `--latest <N>` | Return the latest N matching anchors before final sort |
| `-l, --limit <N>` | Maximum result anchors to print |
| `--exclude-current`, `--other` | Exclude the current agent session from agent searches |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |
| `--save <NAME>` | Save the result WorkSet as `@name` |

Examples:

```bash
sivtr search terminal --json | sivtr filter @ -m error --refs
sivtr filter terminal --status failure --refs
sivtr filter @last --parts --kind tool_result --refs
```

## var

```bash
sivtr var <COMMAND>
```

Manages named WorkSet variables.

| Command | Meaning |
| --- | --- |
| `set <name> [source]` | Save a source or piped WorkSet as `@name` |
| `list` | List saved variables with item counts and creation time |
| `rm <name>` | Remove one saved variable |
| `merge <name> <source>...` | Merge sources into a saved variable, deduplicating by anchor |
| `drop <name> <source>...` | Remove source anchors from a saved variable |
| `cleanup` | Remove all saved variables |

Examples:

```bash
sivtr var set ctx @last
sivtr filter terminal -m panic --json | sivtr var set failures
sivtr var list
sivtr var merge ctx @failures @last[1]
sivtr var drop ctx @noise
```

## nav

```bash
sivtr nav <SOURCE> <MOTION> [OPTIONS]
```

Moves WorkSet anchors deterministically through record/part/session structure. `nav` does not default-expand children; child movement must specify a 1-based index with `>N`.

Motion tokens compose left-to-right:

| Token | Meaning |
| --- | --- |
| `<` | Parent. Part/line to record; record to containing session records. |
| `>N` | Nth child, 1-based. Record children are its parts. |
| `+N` | Next sibling by N at the current level. |
| `-N` | Previous sibling by N at the current level. |
| `[A..B]` | Sibling window at the current level, relative to the current anchor. |
| `~` | Containing session records. |

Options:

| Option | Meaning |
| --- | --- |
| `--cwd <PATH>` | Workspace directory used to resolve records |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |

Examples:

```bash
sivtr nav @hit '<' --refs
sivtr nav @hit '>1' --refs
sivtr nav @hit '<+1>1' --refs
sivtr nav @hit '<[-2..+2]' --refs
sivtr nav @hit '~' --refs
```

Use `zoom` for simple neighboring record context. Use `nav` when the exact movement path matters.

## show

```bash
sivtr show <SOURCE> [OPTIONS]
```

Prints a workspace ref or WorkSet source such as `@last`, `@name`, or `@`.

Ref syntax:

```text
source/session[/record-or-turn[/p<part>]]
```

Options:

| Option | Meaning |
| --- | --- |
| `--cwd <PATH>` | Workspace directory used to resolve sessions |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `--full` | Alias for `--format full` |
| `--format <FORMAT>`, `-f <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |

Examples:

```bash
sivtr show claude/<session-id>
sivtr show claude/<session-id>/3
sivtr show claude/<session-id>/3/p7 --json
sivtr show terminal/current/2
sivtr show desk:terminal/session_42/3/p1 --full
sivtr show @last --full
sivtr show @ctx -f timeline
```

## zoom

```bash
sivtr zoom [SOURCE=@last] [OPTIONS]
```

Expands each target WorkRecord with neighboring records from the same session. Defaults to `@last` when no source is given.

Options:

| Option | Meaning |
| --- | --- |
| `-C, --context <N>` | Records before + after each anchor (default `1`) |
| `--before <N>` | Records before each anchor |
| `--after <N>` | Records after each anchor |
| `--cwd <PATH>` | Workspace directory used to resolve sessions |
| `--json` | Alias for `--format workset` |
| `--refs` | Alias for `--format refs` |
| `-f, --format <FORMAT>` | `full`, `timeline`, `compact`, `md`, `refs`, or `workset` |
| `--save <NAME>` | Save the expanded set as a named WorkSet var |

```bash
sivtr zoom                        # expand @last
sivtr zoom claude/<session>/3 -C 2
sivtr zoom @failures --before 2 --after 0 --refs
```

## work

```bash
sivtr work <COMMAND>
```

Traverses workspace sessions, records, and parts without printing full content — marker-level output you can pipe into other commands.

| Command | Meaning |
| --- | --- |
| `sessions [SOURCE]` | List terminal and agent sessions in the current workspace |
| `records <SOURCE>` | Turn sessions or saved variables into event-level refs |
| `parts <SOURCE>` | Extract only useful inputs/outputs from matching events |

Common flags: `--provider <NAME>` filter, `--cwd <PATH>`, `--json`, `--refs`, `--save <NAME>`.

```bash
sivtr work sessions
sivtr work records codex/ --refs
sivtr work parts @last --kind output --save out_parts
```

## serve

```bash
sivtr serve <COMMAND>
```

Manages the local remote-memory daemon. Share and remote commands auto-start it when needed.

| Command | Meaning |
| --- | --- |
| `start` | Start the daemon in the background |
| `stop` | Stop the running daemon cleanly |
| `restart` | Restart the daemon |
| `status` | Show daemon identity and runtime state |
| `logs` | Print the daemon log path |
| `foreground` | Run the daemon in the foreground |

```bash
sivtr serve start
sivtr serve status
sivtr serve logs
sivtr serve stop
```

## publish

`publish` projects a local WorkSet into an immutable, client-encrypted browser snapshot. It is not `share`: `share` is a live workspace mount that needs Sivtr/daemon; `publish` uploads the encrypted envelope with the publication ID and `X-Sivtr-Management-Token`/`X-Sivtr-Published-At` request metadata, while the viewer key stays in the URL fragment and viewers need no Sivtr install.

```bash
sivtr publish <SOURCE> [--title <TITLE>] [--expires 7d] [--yes] [--allow-warnings]
sivtr publish preview [<SOURCE>] [--title <TITLE>] [--expires 7d] [--format human|json]
sivtr publish list [--json]
sivtr publish link [<PUBLICATION_ID>]
sivtr publish revoke [<PUBLICATION_ID>] [--yes]
```

Whole-record WorkSets use v1: they accept consecutive local agent records from one provider and session, and publish only User/Assistant text. `publish preview` without a source opens the existing workspace picker; it returns a WorkSet and prints the final snapshot locally. v2 supports User, Assistant, Tool, Skill, and Thinking atoms plus non-contiguous parts; ToolCall and ToolResult remain inseparable.

Both versions reject terminal records, remotes/groups, mixed sessions/providers, attachments, and cross-session evidence bundles. WorkRefs remain valid selection inputs, but public snapshots omit WorkSets, WorkRefs, `cwd`, session paths, and provider envelopes; whole and part anchors cannot be mixed. Search defaults to newest-first and `--latest 5`; v1 publish sorts by record index before the continuity check. `[publish].endpoint` defaults to `https://share.hnnulwh.cn` and can be changed in `config.toml`. Non-interactive publish requires `--yes`. Path/email/internal-URL warnings require `--allow-warnings` in every environment, including a TTY.

Typical flow:

```bash
sivtr search codex/<session-id> --sort oldest --latest 50 --save share_ready --refs
sivtr publish preview share_ready
sivtr publish share_ready --expires 7d --yes
```

Interactive preview flow:

```powershell
sivtr publish preview
sivtr publish preview share_ready --format human
sivtr publish share_ready --expires 7d --yes
```

`publish <SOURCE>` also accepts an existing saved WorkSet name without `@`, which avoids PowerShell splatting syntax. Bare `terminal`, `agent`, provider names, and sources containing `/`, `:`, or `*` are selectors; other bare identifiers are saved WorkSet names and fail directly when missing. In the workspace picker, `p` opens the lifetime panel, `Tab` focuses an optional WorkSet name, and a non-empty name is saved once before the selected publish/preview action continues. In an interactive terminal, `list` includes clickable active links; `link` and `revoke` open a selection prompt when their ID is omitted.

## share

```bash
sivtr share [OPTIONS]
sivtr share <COMMAND>
```

Explicitly shares a local workspace for remote peers. Bare `sivtr share` is interactive: pick a workspace (Enter = current) and ensure the share exists (no invite). Create an invite with `sivtr share invite <name>`.

Default interactive options:

| Option | Meaning |
| --- | --- |
| `--path <PATH>` | Workspace path; skips the picker after confirm |
| `--name <NAME>` | Stable share name; defaults to the workspace directory name |
| `--no-redact` | Disable secret redaction for this share |

Subcommands:

| Command | Meaning |
| --- | --- |
| `add [PATH] [--name NAME] [--no-redact]` | Expose a workspace through the daemon |
| `list` | List local shares |
| `remove <SHARE>` | Remove a share and all grants and invitations attached to it |
| `enable <SHARE>` / `disable <SHARE>` | Toggle a share without deleting it |
| `invite <SHARE> [--expires DURATION]` | Create a single-use invite; prints the bare key on stdout |
| `grants <SHARE>` | List active peer grants for a share |
| `revoke <SHARE> <PEER>` | Revoke a peer's access to a share |

```bash
sivtr share
sivtr share add --name alice-desk
sivtr share invite alice-desk --expires 10m
sivtr share list
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
```

## remote

```bash
sivtr remote <COMMAND>
```

Names a peer share in the current git workspace (like `git remote`). The name is the left side of `name:path` refs.

| Command | Meaning |
| --- | --- |
| `list` | List remotes in the current workspace |
| `add <NAME> <INVITE>` | Redeem an invite and add the remote |
| `remove <NAME>` | Remove a local remote name (grant remains until the owner revokes it) |
| `rename <NAME> <NEW>` | Rename a remote in this workspace |
| `test <NAME>` | Reachability + authorization probe |

```bash
sivtr remote add desk <invite-key>
sivtr remote test desk
sivtr remote list
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr show desk:agent/<session>/3 --full
sivtr remote rename desk bob-desk
sivtr remote remove desk
```

## peer

```bash
sivtr peer <COMMAND>
```

| Command | Meaning |
| --- | --- |
| `list` | List known peer identities |
| `forget <PEER>` | Forget a peer and remove all local remotes and grants involving it |

```bash
sivtr peer list
sivtr peer forget <peer>
```

## group

```bash
sivtr group <COMMAND>
```

Groups are a named set of devices that share memory with each other: every member contributes workspaces, and roster changes sync automatically.

| Command | Meaning |
| --- | --- |
| `create <NAME> [--workspace PATH] [--share-name NAME]` | Create the group and contribute the current workspace in one transaction |
| `invite <GROUP> [--expires DURATION] [--max-uses N]` | Owner-only; mint a multi-use join link (stdout = bare key) |
| `join <INVITE> [--workspace PATH] [--share-name NAME] [--no-redact]` | Redeem an invite and contribute workspaces; re-run to adjust contributions |
| `list` | List groups you belong to |
| `members <GROUP>` | List members and their contributions |
| `remove <GROUP> <PEER>` | Owner-only; remove a member |
| `rename <GROUP> <NAME>` | Owner-only; rename the group |
| `leave <GROUP>` | Leave the group (owner leaving disbands it) |
| `sync <GROUP>` | Force a roster pull from the owner |

```bash
sivtr group create team
sivtr group invite team --expires 1d --max-uses 10
sivtr group join <invite-key>
sivtr group list
sivtr group members team
sivtr group sync team
```

Membership changes broadcast to every member automatically (and re-pull on a 5-minute TTL). Group access to a member's contributions is read-only and redacts secrets like shares do.

## origin

```bash
sivtr origin <COMMAND>
```

One rename path for every addressable source — a name is either a local workspace alias or a remote mount, unified through an origin registry.

| Command | Meaning |
| --- | --- |
| `rename <NAME> <NEW_NAME>` | Rename a local workspace alias or a remote mount; both kinds resolve through the same command |

```bash
sivtr origin rename docs knowledge
sivtr origin rename desk alice-desk
```

## workspace

```bash
sivtr workspace [list]
sivtr ws list
```

Lists known local workspaces and their origin labels for `name:body` refs (for example `docs:codex/4`). Alias: `sivtr ws`.

```bash
sivtr ws list
```

Exact syntax for every remote subcommand is above. For the model, setup path, and safety defaults, see [Remote Access](/usage/remote-access/). For a teammate scenario, see [Remote collaboration memory](/playbooks/remote-collaboration-memory/).

## mcp

```bash
sivtr mcp serve
sivtr mcp install [OPTIONS]
sivtr mcp uninstall [OPTIONS]
sivtr mcp print-config <claude|cursor|codex>
```

Read-only MCP server for agent hosts, plus one-shot host registration.

### serve

Runs the MCP server on stdio:

```bash
sivtr mcp serve
sivtr mcp serve --idle-exit 60
```

`--idle-exit <SECS>` makes the server exit after that many seconds with no tool calls; the host respawns it on the next tool use, so an idle server never lingers (each agent session otherwise keeps one alive until it exits). `0` = stay alive until the host closes stdin. The same value can be set globally with the `[mcp] idle_exit_secs` config key, which defaults to `60` (idle exit on; set `0` to disable); the CLI flag wins over the config.

Tools:

| Tool | Purpose |
| --- | --- |
| `sivtr_search` | Search terminal/agent memory; supports `desk:...` origins. Same bounds as CLI search (`latest=5` by default). |
| `sivtr_show` | Expand a ref or WorkSet handle |
| `sivtr_zoom` | Neighboring record context |
| `sivtr_filter` | Narrow `@last` / `@name` / a source |
| `sivtr_status` | Version, hooks, providers, daemon, `ws` local origins, remotes, vars |

### install / uninstall

Writes or removes the sivtr MCP entry in agent host config (same idea as `codegraph install`):

```bash
sivtr mcp install -y                      # detect installed hosts, global
sivtr mcp install -p claude,cursor -l global
sivtr mcp install -p claude -l local      # project .mcp.json
sivtr mcp uninstall -p all -y
```

| Flag | Meaning |
| --- | --- |
| `-p, --provider` | Provider host(s): `claude`, `cursor`, `codex`, `opencode`, `openclaw`, `grok`, `hermes`, `pi`, `qoder`, `qodercn`, `gemini`, `qwen`, `goose`, or `all`. Omit to detect installed hosts. |
| `-l, --location` | `global` (default) or `local` |
| `-y, --yes` | Non-interactive |

Install locations (registry-driven; paths are host defaults):

| Target | Global path |
| --- | --- |
| Claude Code | `~/.claude.json` → `mcpServers.sivtr` |
| Cursor | `~/.cursor/mcp.json` → `mcpServers.sivtr` |
| Codex | `~/.codex/config.toml` → `[mcp_servers.sivtr]` |
| OpenCode | OpenCode MCP config → `mcp.sivtr` |
| OpenClaw | OpenClaw config → `mcp.servers.sivtr` |
| Grok | Grok config TOML → MCP entry |
| Hermes | Hermes YAML → `mcp_servers.sivtr` |
| Pi | Pi config → `mcpServers.sivtr` |
| Qoder / Qoder-CN | Qoder settings.json → MCP entry |
| Gemini | Gemini settings.json → `mcpServers.sivtr` |
| Qwen | Qwen settings.json → MCP entry |
| Goose | Goose config.yaml extensions → MCP entry |

Registered command is always:

```text
sivtr mcp serve
```

### print-config

Print a snippet without writing files:

```bash
sivtr mcp print-config claude
sivtr mcp print-config cursor
sivtr mcp print-config codex
sivtr mcp print-config grok
sivtr mcp print-config goose
```

MCP is not a full CLI mirror. Interactive, write, and capture commands stay on the CLI. Strategy still lives in the `sivtr-memory` skill.

## version

```bash
sivtr version [--verbose]
```

Prints the Sivtr version. Use `--verbose` to diagnose which binary is running and whether it differs from the local debug build in the current repository.

```bash
sivtr version
sivtr version --verbose
```

Verbose output includes:

- package version;
- binary path;
- current working directory;
- debug/release profile;
- git commit and build time when available;
- detected repo root;
- local `target/debug/sivtr` binary status;
- a warning when a different global binary is being used inside the repo.

## doctor

```bash
sivtr doctor [--fix] [--json]
```

Diagnoses the installation and environment: binary, config, session logs, shell hooks, provider session counts, clipboard, and available updates.

| Option | Meaning |
| --- | --- |
| `--fix` | Attempt to repair detected problems automatically |
| `--json` | Machine-readable JSON output |

```bash
sivtr doctor
sivtr doctor --fix
sivtr doctor --json
```

## update

```bash
sivtr update
```

Self-updates to the latest GitHub release: downloads the matching platform asset, verifies its SHA256, and replaces the binary in place.

```bash
sivtr update
```

## setup

```bash
sivtr setup
```

One-command setup for a fresh install: detects the environment, installs shell hooks, registers the MCP server with detected agent hosts, installs the `sivtr-memory` skill when missing, and runs a smoke test. Equivalent to running `sivtr init`, `sivtr mcp install`, the skill install, and `sivtr doctor` step by step.

```bash
sivtr setup
```

## history

```bash
sivtr history [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `list [-l, --limit <N>]` | List recent entries |
| `search <KEYWORD> [-l, --limit <N>]` | Search saved capture history |
| `show <ID>` | Show a specific history entry |

If no history subcommand is provided, `list` is used.

## config

```bash
sivtr config [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `show` | Show config path and content |
| `init` | Create default config |
| `edit` | Open config in editor |

If no config subcommand is provided, `show` is used.

## hotkey

```bash
sivtr hotkey [COMMAND]
```

Subcommands:

| Command | Meaning |
| --- | --- |
| `start [--chord <CHORD>] [--provider <PROVIDER>]` | Start Windows global hotkey daemon |
| `status` | Show daemon status |
| `stop` | Stop daemon |

If no hotkey subcommand is provided, `status` is used.

Examples:

```bash
sivtr hotkey start
sivtr hotkey start --chord alt+y
sivtr hotkey start --provider claude
sivtr hotkey status
sivtr hotkey stop
```

## sync

```bash
sivtr sync [--full] [--json]
```

Syncs every agent provider's sessions and every workspace's terminal logs into the unified local archive (`archive.db`). Queries (search, show, copy, TUI, MCP) read from this archive; when it is older than `[sync].max_age_secs` (default `15` seconds), a query first triggers an incremental re-sync.

Options:

| Option | Meaning |
| --- | --- |
| `--full` | Re-parse every session, ignoring cached stamps |
| `--json` | Print a machine-readable report with per-source counts (added / updated / unchanged / failed) |

Sync is incremental by default: each source file is compared against the archive by its stat stamp, and only changed files are re-parsed. Failures are per-source and reported, so one broken provider never hides the rest.

Examples:

```bash
sivtr sync
sivtr sync --full
sivtr sync --json
```

## clear

```bash
sivtr clear [--all]
```

Clears current shell session logs. `--all` clears all recorded session logs and state files managed by `sivtr`.

## Shared syntax

See [Selectors and Filters](/reference/selectors-and-filters/) for recency selectors, `--session`, providers, `--regex`, `--lines`, `--ansi`, `--print`, and workspace refs.

