---
title: sivtr
description: A shared memory workspace for humans and agents.
---

`sivtr` is a local-first shared memory workspace for humans and agents. It turns the work around a project—terminal commands, command output, AI-agent conversations, tool results, and copied context—into searchable, selectable, referenceable memory that both you and your agents can reuse.

Use it beside the terminals and agents you already have. It gives their local work one shared memory workspace.

For the full workflow, install both pieces: the `sivtr` CLI/TUI and the bundled `sivtr-memory` skill. The CLI captures and retrieves memory; the skill teaches agents to use that memory before asking you to paste context.

## What sivtr is not

`sivtr` is not:

- a terminal emulator;
- a tmux replacement;
- a hosted transcript service;
- another agent runtime.

## What sivtr helps with

- **Keep the output, not just the command** from pipes, subprocesses, shell integration, and local agent transcripts.
- **Browse long logs comfortably** in a keyboard-first Vim-style TUI.
- **Copy the last useful thing** as input, output, bare command, or full block.
- **Search local agent conversations** from registered providers (Codex, Claude Code, Cursor, Hermes, OpenCode, OpenClaw, Grok, Pi, …) when you need an old decision or explanation.
- **Let agents start from evidence** so "fix the terminal error" can begin with the latest captured failure instead of a paste request.
- **Jump back to the source** behind a summary, search hit, or handoff note.
- **Save search results as variables** like `@last` and `@failures`, then reuse them in follow-up commands.
- **Share a workspace read-only** with a teammate and search their sessions as `desk:terminal/...`.
- **Launch memory pickers** from the terminal, tmux, VS Code, Windows hotkeys, or generated desktop shortcuts.

## First useful commands

```bash
# Browse command output as reusable workspace memory.
cargo test 2>&1 | sivtr

# Let sivtr run the command and capture combined output.
sivtr run cargo test

# Copy the latest recorded command output.
sivtr copy out

# Copy the latest assistant answer from an agent provider.
sivtr copy claude out
sivtr copy codex out
sivtr copy cursor out
sivtr copy grok out

# Search current workspace memory.
sivtr search agent --match "panic" --format timeline
```

## Common workflows

| Goal | Start here |
| --- | --- |
| Install the CLI + skill | [Installation](/start/installation/) |
| Learn the daily path | [Quickstart](/start/quickstart/) |
| Understand the model | [Mental Model](/start/core-concepts/) |
| Capture output | [Capture Terminal Output](/usage/capture-output/) |
| Copy recent commands | [Copy Command Blocks](/usage/copy-command-blocks/) |
| Reuse agent memory | [Work with AI Sessions](/usage/ai-sessions/) |
| Teach agents the memory workflow | [Skills and Reusable Procedures](/usage/skills/) |
| See practical community workflows | [Playbooks](/playbooks/) |
| Search and dereference memory | [Search and Show Results](/usage/search-and-show/) |
| Share and add remote memory | [Remote Access](/usage/remote-access/) |
| Open pickers quickly | [Launch Pickers and Hotkeys](/usage/launchers-and-hotkeys/) |
| Check exact syntax | [CLI Reference](/reference/cli/) |

## Mental model

`sivtr` has two layers:

| Layer | What it describes |
| --- | --- |
| What happened | Terminal output, command blocks, agent conversations, tool results, and local history. |
| How you reuse it | TUI browsing, search, copy, show, diff, skills, playbooks, named remotes, and memory variables like `@last`. |

Terminal sources produce command blocks. Agent providers produce conversation blocks. Selectors like `1` and `2..4` pick recent items for copy commands. Search results can be saved as variables such as `@failures`, then shown, expanded, or piped into the next command.

## Local by default

`sivtr` reads local shell logs, local history, and local agent transcripts. Cross-device access is opt-in through [Remote Access](/usage/remote-access/). See [Data Locations](/reference/data-locations/) for where records live.
