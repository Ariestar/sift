---
title: Capture Terminal Output
description: Use pipe mode, run mode, and shell session integration.
---

Capture is the first step in turning terminal output into reusable text. Use the lightest capture path that matches what you need.

## Choose a capture path

| Use case | Best command | Keeps command metadata? |
| --- | --- | --- |
| Inspect one existing command pipeline | `command 2>&1 \| sivtr` | No |
| Let `sivtr` run one command | `sivtr run command` | Partially, for the captured run |
| Browse the current workspace's recorded work | `sivtr` | Yes, after shell integration |
| Copy one recent command block | `sivtr copy out` | Yes, after shell integration |
| Search captured terminal and AI work | `sivtr search "query"` | Yes, from the unified archive |

## Pipe mode

Pipe mode archives stdin in the unified archive and opens the result in the external editor.

```bash
ls -la | sivtr
cargo build 2>&1 | sivtr
rg "TODO" . | sivtr
```

Use pipe mode when:

- the command already exists in your shell history;
- you want normal shell behavior for pipelines and redirection;
- you do not need `sivtr` to know the original command.

For commands that write important output to stderr, redirect stderr to stdout:

```bash
cargo test 2>&1 | sivtr
```

## Run mode

Run mode executes the command through `sivtr`:

```bash
sivtr run cargo test
sivtr run git status --short
```

Use run mode when:

- you want `sivtr` to execute and capture a single command;
- you want the exit status reported and the output saved before the editor opens;
- you prefer not to manage shell redirection manually.

Run mode captures stdout and stderr together and saves the result to the unified archive. If the command produces no output, `sivtr` exits after reporting that nothing was captured.

## Shell session browsing

Shell integration records structured command entries over time. After installing it, open the workspace browser:

```bash
sivtr
```

This is useful when you have been working normally and later want to browse the accumulated terminal work.

Install shell integration with:

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

Restart the shell after installation.

## Search captured output

`pipe` and `run` records are terminal sessions in the same `archive.db` as shell and agent sessions. Search or show them through the normal archive commands:

```bash
sivtr search "panic"
sivtr show terminal/<session>/<record>
```
