---
title: 数据位置
description: sivtr 存放配置、history、session log 和 provider 数据的位置。
---

`sivtr` 是 local-first。它使用的大多数数据已经在你的机器上；它生成的数据默认写在平台配置或状态目录下，除非你显式导出到其他位置。

## 配置文件

| 平台 | 路径 |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

## Shell session log

Shell 集成会写入按进程区分的结构化 session log。

| Shell/平台 | 常见路径 |
| --- | --- |
| Windows PowerShell / PowerShell 7 | `%APPDATA%\sivtr\session_<pid>.log` |
| Bash / Zsh | `$XDG_STATE_HOME/sivtr/session_<pid>.log` 或 `~/.local/state/sivtr/session_<pid>.log` |
| Nushell | Nushell config/state 区域中的 `sivtr` session 文件 |

这些 log 支撑：

- `sivtr import`；
- `sivtr copy` 命令块工作流；
- `sivtr diff`；
- browser 中的命令块导航。

## History 数据库

当 `[history].auto_save = true` 时，捕获的终端输出会保存到本地 SQLite history 数据库。

请通过 CLI 命令访问，而不是直接编辑数据库：

```bash
sivtr history list
sivtr history search "panic"
sivtr history show 42
```

保留策略由以下配置控制：

```toml
[history]
max_entries = 0
```

`0` 表示不限制数量。

## Agent provider 数据

`sivtr` 读取 provider 自己的本地数据，不上传 transcript。

| Provider | 数据来源 |
| --- | --- |
| Codex | `~/.codex/sessions` rollout JSONL 文件 |
| Claude Code | 当前 transcript/session 环境变量和本地 Claude transcripts |
| Hermes | 主路径：`$HERMES_HOME/state.db`（Windows 默认 `%LOCALAPPDATA%\hermes`，其他平台 `~/.hermes`）。Residual：同目录下 `sessions/*.jsonl`。 |
| OpenCode | OpenCode 本地数据库 |
| Cursor | `~/.cursor/projects/**/agent-transcripts/**/*.jsonl`（可用 `CURSOR_HOME` 覆盖） |
| OpenClaw | `~/.openclaw/agents/<id>/agent/openclaw-agent.sqlite`（legacy JSONL 在 `sessions/`） |
| Grok | `~/.grok/sessions/**`（`summary.json` + `chat_history.jsonl`；可用 `GROK_HOME` 覆盖） |
| Pi | Pi agent session JSONL 文件 |

各 provider 格式不同；`sivtr` 会把它们归一化为 session 和 dialogue unit，用于 copy、picker、search 和 show 工作流。

## 统一 archive

search、show、copy、picker、TUI 和 MCP 查询都从统一的本地 archive 读取，而不是每次解析原生 session 文件。

| 平台 | 路径 |
| --- | --- |
| Windows | `%APPDATA%\sivtr\archive.db` |
| macOS | `~/Library/Application Support/sivtr/archive.db` |
| Linux | `~/.config/sivtr/archive.db` |

它是一个 SQLite 数据库（WAL 模式），由 sync 引擎写入：`sivtr sync` 显式执行一次同步，查询在 archive 比 `[sync].max_age_secs` 更旧时也会自动执行新鲜度同步。原生 Agent session 文件和 shell session log 仍是 source of truth，只有 sync 引擎会读取它们。可用 `SIVTR_DATA_DIR` 覆盖根目录。

## 生成的启动器

Linux shortcut generation 会写入：

- `~/.local/bin/sivtr-pick-codex`；
- `~/.local/share/applications/sivtr-pick-codex.desktop`。

macOS shortcut generation 会写入：

- `~/.local/bin/sivtr-pick-codex`；
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`。

Windows hotkey 状态存放在 `sivtr` 的平台 config/state 区域下，由以下命令管理：

```bash
sivtr hotkey status
sivtr hotkey stop
```

## Remote daemon 状态

跨设备远程记忆使用设备级 daemon。可用 `SIVTR_DATA_DIR` 覆盖根目录；否则是平台 config 目录下的 `sivtr`（与 `data_dir()` 相同）。

| 文件 | 用途 |
| --- | --- |
| `identity.key` | iroh 使用的稳定设备身份 |
| `remote-state.db` | SQLite：peers、shares、grants、invites、mounts、audit |
| `publication-state.db` | SQLite：本机公开快照的 id、期限、来源摘要、查看密钥和撤销凭据；不保存公开快照明文 |
| `daemon.json` | 运行中 daemon 控制信息（port、token、node id） |
| `daemon.lock` | 单实例锁 |
| `daemon.log` | daemon 日志（`sivtr serve logs`） |

```bash
sivtr serve status
sivtr serve logs
sivtr share list
sivtr remote list
sivtr peer list
sivtr ws list
```

远程访问是 opt-in。只有 `sivtr share`（或 `share add`）之后才会分享；邀请用 `share invite` 签发。remote 是用 `sivtr remote add` 登记的 workspace 本地名。功能指南见 [远程访问](/zh-cn/usage/remote-access/)。
