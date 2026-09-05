---
title: Quickstart
description: Open the unified workspace TUI, browse terminal commands and all agent sessions, and let agents reuse the same local memory.
---

`sivtr` has two everyday paths:

- **For humans:** open the `sivtr` TUI panel to browse terminal output and agent sessions together, search across sources, and copy the exact part you need.
- **For agents:** install the bundled `sivtr-memory` skill, then use CLI commands such as `sivtr search`, `sivtr copy --print`, and `sivtr show` to retrieve local workspace memory.

## 1. Install the CLI and skill

Follow [Installation](/start/installation/) first. The important point is that agent workflows need both parts:

- the `sivtr` CLI must be on `PATH`;
- the bundled `sivtr-memory` skill must be installed with `npx skills add Ariestar/sivtr --skill sivtr-memory -g`.

After that, the agent can be told to "use sivtr first" and will have a concrete procedure for searching local memory.

## 2. Put terminal work into the workspace

If this is your first time using `sivtr`, install shell integration once:

```bash
sivtr init powershell
# or: sivtr init bash
# or: sivtr init zsh
# or: sivtr init nushell
```

Restart the shell, then work normally:

```bash
bun run build
cargo test
git status --short
```

Recent commands and output will now appear in the workspace. Agent sessions are read from each registered provider's local session data.

## 3. Open the unified workspace TUI

Run:

```bash
sivtr
```

This is the most important entry point. It is not terminal-only, and it is not tied to one agent provider. It puts local workspace memory into one interface.

The workspace TUI has four panes. Content splits into Input / Output halves.

| Pane | Purpose |
| --- | --- |
| Source | Choose a source, such as terminal or a registered agent provider. |
| Sessions | Choose a terminal record or agent session. |
| Dialogues | Choose a dialogue turn or command block. |
| Content | Inspect input and output (independent halves). |

Common keys:

| Key | Action |
| --- | --- |
| `0` / `1` / `2` / `3` | Focus Source, Sessions, Dialogues, or Content |
| `j` / `k` | Move inside the focused pane |
| `h` / `l` | Move to the previous / next pane |
| `Space` | Toggle the current source, session, or dialogue |
| `Tab` | Switch Content Input ↔ Output half |
| `r` | Fold structure markers / expand full payloads |
| `R` | Refresh next level under active rows |
| `/` | Search the workspace |
| `i` / `o` / `y` / `c` | Copy input / output / block / bare command |
| `z` | Toggle focused pane fullscreen |
| `?` | Help |
| `q` / `Esc` | Back or quit |

Full table: [Browse and Select](/usage/browse-and-select/).

Workspace search supports prefixes:

| Query | Scope |
| --- | --- |
| `error` | Content |
| `#build` | Dialogue or command-block title |
| `>release` | Session title |

## 4. Open only one kind of content

The unified workspace is the default entry point. When you only want one source, use a more specific picker.

Terminal command blocks only:

```bash
sivtr copy --pick
```

One agent provider only:

```bash
sivtr copy claude --pick
sivtr copy codex --pick
sivtr copy hermes --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

These are filtered workspace views, not the main entry point.

## 5. Copy directly when you know what you need

When you already know what to fetch, you do not need to open the TUI:

```bash
sivtr copy          # latest command + output
sivtr copy out      # latest output only
sivtr copy cmd      # latest command only
sivtr copy out 2    # previous command output
sivtr copy 2..4     # recent range
```

Read agent sessions:

```bash
sivtr copy claude out --print
sivtr copy codex out --print
sivtr copy hermes out --print
sivtr copy opencode out --print
sivtr copy pi out --print
```

For agent use, prefer `--print` so the command returns text instead of opening an interactive UI or only writing to the clipboard.

## 6. Let the agent search workspace memory first

When something fails, tell the agent:

```text
Fix the terminal error. Use sivtr first.
```

The agent should search the same workspace memory:

```bash
sivtr search terminal --match "error|failed|panic|Traceback|Exception|exit code|FAILED" --format json --limit 20
sivtr search terminal --status failure --latest 1 --format json
sivtr copy out 1 --print
sivtr copy cmd 1..10 --print
```

Then it can read code, patch the issue, and rerun the smallest relevant verification command.

This is the core value: what you can see in the unified workspace, the agent can also read through commands.

## 7. Return to exact evidence with refs

Search results include refs. A ref can point to a terminal command block, an agent session, a dialogue, or a specific content block.

```bash
sivtr search terminal --match "build error" --format json --limit 20
sivtr show terminal/current/2
sivtr show claude/<session>/3
sivtr show claude/<session>/3/2
```

Refs let humans and agents return to the same evidence instead of relying on vague summaries.

## 8. Use the single-output browser when needed

Besides the workspace TUI, `sivtr` also has a single-buffer browser for temporary long output:

```bash
cargo test 2>&1 | sivtr
sivtr run bun run build
```

This browser is for one captured output. The bare `sivtr` workspace TUI is the main entry point for terminal and agent memory together.

## Next steps

- Read the [Mental Model](/start/core-concepts/) to understand sources, sessions, dialogues, blocks, and refs.
- See [Playbooks](/playbooks/) for real workflows.
- Read [Browse and Select](/usage/browse-and-select/) for full workspace TUI controls.
- Use [Remote Access](/usage/remote-access/) when you need another device's memory.
- Keep [CLI Reference](/reference/cli/) open for exact options.
