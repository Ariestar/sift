---
title: Skills and Reusable Procedures
description: Teach agents to use sivtr as the shared memory entry point.
---

`sivtr` is useful to humans as a CLI and TUI. It becomes much more useful to agents through **skills**: reusable procedure packs that tell an agent when and how to query the local memory workspace.

A skill is not another transcript store. It is the agent-side operating manual for the same memory layer:

```text
human work + agent work -> sivtr memory -> skill procedure -> agent action
```

The key shift is that agents can use `sivtr` as the unified entry point into local workspace memory: logs, errors, prior decisions, and validation results are all available through the same interface.

The bundled `sivtr-memory` skill is part of the recommended setup, not just optional prose. Without the CLI, the skill has nothing to query; without the skill, many agents will not know when to use the CLI before asking you for context.

Install the CLI and skill from [Installation](/start/installation/), then use this page as the operating guide for what the skill does.

Optionally register the read-only MCP server so agent hosts can call structured tools instead of shelling out:

```bash
sivtr mcp install -y
# or:
sivtr mcp install -p claude,cursor -l global
```

This writes `sivtr mcp serve` into host config for registered hosts (Claude Code, Cursor, Codex, OpenCode, OpenClaw, Grok, Hermes, Pi, Qoder/Qoder-CN, Gemini, Qwen, Goose, …). MCP tools: `sivtr_search`, `sivtr_show`, `sivtr_zoom`, `sivtr_filter`, `sivtr_status`, `sivtr_usage`, and `sivtr_stats` (status includes local workspace origin labels from `ws` and remotes). The skill still teaches *when* and *how* to retrieve evidence; MCP is the structured execution surface. See [CLI Reference](/reference/cli/#mcp).

## Why skills matter

Without a skill, an agent may miss useful local evidence.

With a `sivtr` memory skill, the agent can:

1. search recent terminal and agent memory;
2. expand the smallest relevant command output or dialogue;
3. inspect the current code or config;
4. fix the issue;
5. run verification;
6. report the evidence.

That turns `sivtr` from a human clipboard helper into a shared memory workspace that both humans and agents know how to operate.

## The bundled sivtr-memory skill

This repository includes a starter skill at:

```text
skills/sivtr-memory/
```

It teaches an agent to use `sivtr` for:

- recent terminal failures;
- missing or truncated command output;
- prior agent decisions;
- continuation after a compacted or interrupted session;
- handoff and recap context;
- validation evidence from build, test, lint, or deploy commands.

The skill's core rule is:

> Search for evidence first. Expand only the smallest relevant context. Ask when memory is missing, ambiguous, stale, or permission is required.

This is the agent-facing expression of `sivtr`'s product model: local evidence first, exact refs when possible, and human clarification only when local memory cannot answer the question.

## How an agent should use sivtr memory

For non-interactive agent workflows, prefer commands that print results instead of opening pickers or mutating the clipboard. Choose the search format for the job: `timeline`/`compact`/`md` are often easier to reason over, while `json` is best when a program will parse refs and fields.

Prefer MCP tools when registered. CLI equivalents:

```bash
sivtr s terminal -m "error|failed|panic|Traceback|Exception|exit code|FAILED" -f timeline --latest 20
sivtr s terminal --status failure --latest 1 --json
sivtr s agent -m "decision|TODO|next step" --latest 20 --save hits --refs
sivtr filter @hits -m "<topic>" --save narrowed --refs
sivtr show @narrowed[1] --full
sivtr copy out 1 --print
```

Avoid interactive or state-changing commands unless the human explicitly asked for them:

- bare `sivtr` and `--pick` open the TUI (human-only in agent runs);
- hotkey start/stop, share/remote mutations, config mutation;
- huge transcript dumps — search defaults to `--latest 5` when unbounded;
- use `--print` when copying content for the agent itself.

## Skill shape

A useful `sivtr` skill usually contains four parts:

| Part | Purpose |
| --- | --- |
| Trigger conditions | When the agent should use `sivtr` for local memory retrieval. |
| Retrieval commands | Safe search, copy, and show command recipes. |
| Evidence discipline | How to quote refs, distinguish terminal evidence from prior agent discussion, and verify current state. |
| Workflow recipes | Debugging, continuation, handoff, recap, timeline, or review procedures. |

The bundled `skills/sivtr-memory/` directory follows this shape with a main `SKILL.md`, reference files, and examples.

## Skill registry direction

A future skill registry can make these procedures discoverable and reusable across teams and communities. Instead of every user inventing their own prompts for "look at the last error" or "summarize what happened today," a registry can provide tested procedures that all target the same local memory interface.

Potential registry entries include:

- terminal failure debugger;
- recent work timeline generator;
- PR handoff writer;
- release-note drafter from validated work;
- incident recap assistant;
- remote collaboration memory reader ([Remote Access](/usage/remote-access/) + `desk:...` refs);
- project onboarding guide from local sessions.

See [Playbooks](/playbooks/) for concrete workflows built on top of skills.
