---
title: Troubleshooting
description: Diagnose missing command blocks, empty agent sessions, clipboard issues, hotkey failures, and stale docs assumptions.
---

This page lists common failure modes and the first checks to run.

## `GLIBC_2.xx' not found` after install

Prebuilt **glibc-linked** Linux binaries are built on GitHub `ubuntu-latest` and may require a newer GLIBC than older distros provide. Current `cargo binstall sivtr` metadata serves the **static musl** asset for both `linux-gnu` and `linux-musl` hosts, so a fresh binstall should not hit this.

If you still have an older glibc binary (for example from a previous install, or crates.io metadata not yet updated):

```bash
# reinstall static musl build explicitly
cargo binstall sivtr --targets x86_64-unknown-linux-musl

# or
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

Confirm:

```bash
ldd "$(command -v sivtr)"   # static musl is typically "not a dynamic executable"
sivtr --version
```

## `sivtr copy out` finds no command blocks

Command-block copy requires shell integration and a restarted shell.

Check:

```bash
sivtr init powershell
# or: sivtr init bash / zsh / nushell
```

Then restart the terminal, run a command, and try:

```bash
sivtr copy out --print
```

If pipe mode works but `copy` does not, the issue is usually session logging, not the browser.

## `sivtr import` opens nothing useful

`import` reads the current structured shell session log. It is most useful after shell integration has recorded several commands in the current shell process.

Try:

1. Restart the shell after `sivtr init <shell>`.
2. Run a visible command such as `echo hello`.
3. Run `sivtr import`.

## Agent provider picker is empty

Provider pickers only show local sessions the provider can discover for the current workspace.

Check:

```bash
sivtr copy codex --pick
sivtr copy claude --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

If one provider is empty but another works, the issue is provider discovery or missing provider data. If all are empty, check that you are running from the project directory that matches the sessions' working directory.

Use `--cwd` with search/show flows when running from another directory:

```bash
sivtr search agent --match "panic" --cwd /path/to/project --format timeline
```

## Clipboard copy fails on Linux

Clipboard support depends on the desktop/session environment. Wayland, X11, SSH, and headless environments can behave differently.

First verify the text itself with `--print`:

```bash
sivtr copy out --print
sivtr copy claude out --print
```

If printed text is correct but the clipboard is empty, the problem is likely platform clipboard integration rather than selection or parsing.

## Windows hotkey does not start

Check status:

```bash
sivtr hotkey status
```

Then try a different chord:

```bash
sivtr hotkey start --chord ctrl+shift+y
```

If registration fails, another app may already own the shortcut.

## Linux global hotkey is missing

This is expected. Linux does not currently ship a built-in desktop-wide `sivtr` daemon because Wayland and desktop environments do not provide one universal shortcut API for ordinary CLI apps.

Use one of these instead:

```bash
sivtr init tmux
sivtr init linux-shortcut
```

Or use the VS Code extension shortcut.

## `sivtr show <ref>` cannot find a ref

Refs are resolved against the current workspace session list. If you run `show` from a different directory than the original search, pass the same `--cwd`:

```bash
sivtr search agent --match "panic" --cwd /path/to/project --format json
sivtr show <ref> --cwd /path/to/project
```

Also check that the provider exists in the ref source, such as `codex`, `claude`, `opencode`, or `pi`.

## Regex filters match nothing

`--regex` keeps matching lines only. If the pattern is invalid or too narrow, the result can be empty.

Debug with `--print` and a simpler pattern:

```bash
sivtr copy out --regex error --print
sivtr copy out --regex "error|failed" --print
```

Remember that `--regex` runs before `--lines` when both are set.

## Documentation and CLI disagree

The CLI is the source of truth for the installed binary:

```bash
sivtr --help
sivtr version --verbose
sivtr copy --help
sivtr copy claude --help
```

If the website describes a newer command than your binary supports, update `sivtr`:

```bash
cargo install sivtr --force
```
