---
title: Installation
description: Install the sivtr CLI, bundled memory skill, and shell integration.
---

`sivtr` is published as a Cargo package and developed at [github.com/Ariestar/sivtr](https://github.com/Ariestar/sivtr).

For the full human + agent workflow, install two pieces:

- the **CLI/TUI**, which captures and retrieves local workspace memory;
- the bundled **`sivtr-memory` skill**, which teaches your agent how to use that memory.

Human-only browsing can work with just the CLI. Agent workflows should install both.

## AI Assistant Quick Install

If you are an AI coding assistant (Claude Code, Codex, etc.) installing sivtr on behalf of a user:

### Pre-Installation Check

```bash
sivtr --version    # Already installed?
sivtr doctor       # Full environment check
```

If `sivtr --version` succeeds, **do not reinstall**. Run `sivtr doctor` to verify the environment and skip to the skill installation below.

### Install Steps

```bash
# 1. Install binary (choose one)
cargo install sivtr          # from source (requires Rust)
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh  # pre-built

# 2. Verify
sivtr --version

# 3. Shell hook (pick one)
sivtr init bash       # Linux default
sivtr init zsh        # macOS default
sivtr init powershell # Windows
sivtr init nushell

# 4. Config
sivtr config init

# 5. Skill (for agent workflows)
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

### Post-Install Verification

```bash
sivtr doctor
```

Expected output: all checks passing. If `shell hooks` shows "not installed", run the appropriate `sivtr init` command. If `session log directory` shows "missing", the user needs to restart their terminal after `sivtr init`.

### Check Current Status

```bash
sivtr init show       # Which shell hooks are installed
sivtr doctor          # Full diagnostics
```

### Uninstall

```bash
sivtr init uninstall  # Remove all shell hooks
cargo uninstall sivtr # Remove binary
```

## Requirements

- Rust and Cargo
- A supported terminal
- Clipboard support for your platform

Optional:

- `nvim`, `vim`, or `vi` for the Vim picker view used by some copy workflows
- PowerShell, Bash, Zsh, or Nushell shell profile access for session logging

## Install with cargo-binstall (recommended)

Download a prebuilt binary via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) (no compile):

```bash
cargo binstall sivtr
```

On Linux x86_64, binstall resolves to the **static musl** release asset, so the binary does not require a recent system GLIBC. This matches `install.sh`.

Verify:

```bash
sivtr --version
sivtr --help
```

## Install with Cargo (from source)

Build the latest published release from crates.io:

```bash
cargo install sivtr
```

## Quick Install script (Linux/macOS)

Download a pre-built binary without compiling:

```bash
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

Pin a specific version:

```bash
SIVTR_VERSION=v0.4.0 curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

This downloads a static musl binary to `~/.local/bin/sivtr` (or `$SIVTR_INSTALL_DIR`). Requires no Rust toolchain.

## Install from source

Clone the repository:

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
```

From the repository root:

```bash
cargo install --path .
```

## Install the bundled skill

Install the `sivtr-memory` skill globally with the Skills CLI:

```bash
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

After installation, ask your agent to use sivtr first when local context may already exist, for example:

```text
Fix the latest terminal error. Use sivtr first.
```

## Update

Update the binary in place:

```bash
sivtr update
```

This downloads the latest GitHub release for your platform, verifies its SHA256 checksum, and replaces the current binary. `sivtr doctor` also reports when a newer release is available.

If you installed from source via Cargo, update the published package instead:

```bash
cargo install sivtr --force
```

Or reinstall from a local checkout after pulling changes:

```bash
git pull
cargo install --path . --force
```

## Shell integration

Shell integration records recent command blocks so `sivtr copy` and command-block navigation have structured data to work with.

Install the hook for your shell:

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

Check which hooks are installed:

```bash
sivtr init show
```

Uninstall all hooks:

```bash
sivtr init uninstall
```

Restart the terminal after installation or removal.

The hook writes a per-process session log:

- Windows PowerShell and PowerShell 7 use `%APPDATA%\sivtr\session_<pid>.log`.
- Bash and Zsh use `$XDG_STATE_HOME/sivtr/session_<pid>.log` or `~/.local/state/sivtr/session_<pid>.log`.
- Nushell uses its config directory with a `sivtr` session file.

## Configuration file

Create the default config file:

```bash
sivtr config init
```

Show the path and current content:

```bash
sivtr config show
```

Edit it with your configured editor:

```bash
sivtr config edit
```

See [Config File](/reference/config-file/) for all supported settings.
