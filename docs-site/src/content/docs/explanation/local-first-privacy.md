---
title: Local-first and Privacy
description: How sivtr keeps agent memory, terminal output, and transcripts under local user control.
---

`sivtr` is designed around local agent memory. Terminal output, shell session logs, history, and agent transcripts can contain secrets, private code, credentials, internal URLs, and unfinished reasoning. The default posture is to keep that data on the machine that already produced it.

## Local by default

`sivtr` reads and writes local files and databases:

- shell session logs from shell integration;
- local SQLite history for captured terminal output;
- the unified session archive (`archive.db`);
- provider-owned agent transcript files or databases;
- local config under the platform config directory.

It does not provide a hosted transcript service by default.

## The local archive

Terminal captures and agent sessions sync into one local SQLite archive (`archive.db`) under sivtr's data directory. Nothing leaves the machine in the process: native session files remain the source of truth and are only read by the sync engine.

## Explicit remote share

Cross-device memory access is also opt-in. Nothing leaves the machine until you create a share (`sivtr share` / `share add`), issue an invite (`share invite`), and a peer redeems it:

```bash
sivtr share                   # interactive; create share only
sivtr share invite alice-desk # single-use invite (stdout = bare key)
sivtr remote add desk <invite> # peer names the remote in their workspace
```

Remote access is read-only. Secret redaction is on by default before records leave the device (`--no-redact` to disable for a share). Invites expire (default `10m`). Transport between daemons is encrypted iroh. Local-first remains the default: unregistered origins error.

Full guide: [Remote Access](/usage/remote-access/).

`sivtr publish` is a separate outbound boundary: an immutable snapshot from a local WorkSet, not a live mount. Whole-record WorkSets use v1 and project consecutive User/Assistant turns from one local agent session; `publish preview` without a source opens the existing TUI to select non-contiguous User, Assistant, Tool, Skill, and Thinking atoms within one session. Raw WorkSets, WorkRefs, `cwd`, session paths, and provider envelopes stay local; only the projected, redacted snapshot enters the encrypted envelope. The hosted service stores AES-256-GCM ciphertext; the viewing key stays in the URL fragment. `[publish].endpoint` defaults to `https://share.hnnulwh.cn` and can be changed in `config.toml`; there is no automatic failover between self-host and Cloudflare.

Guide: [Publish conversation links](/usage/publish/).

## Clipboard is an output boundary

Copy commands place selected text on the system clipboard:

```bash
sivtr copy out
sivtr copy claude out
```

Treat clipboard contents as shared with your desktop environment and clipboard managers. Use `--print` to inspect text before copying sensitive content in risky contexts.

## History retention is configurable

Captured terminal output is saved to history when enabled:

```toml
[history]
auto_save = true
max_entries = 0
```

Set `auto_save = false` if captures should not be written automatically. Set `max_entries` to a positive number to bound retained history.

## Good operational habits

- Treat the archive like the source transcripts: it holds the same sensitive text, so protect its location and access.
- Review copied text before pasting it into public chats, issues, hosted agents, or external AI tools.
- Use line and regex filters to copy only the necessary evidence.
- Prefer `--format json` / `--refs` search output for tooling, but remember JSON content can still include sensitive text.
- Prefer short-lived invites and revoke grants (`sivtr share revoke`) when collaboration ends.

