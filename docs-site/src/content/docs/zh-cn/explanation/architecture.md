---
title: 架构
description: sivtr memory workspace 如何拆分为 CLI、TUI、命令处理器、remote daemon 和 core 模块。
---

`sivtr` 是一个 Cargo workspace，主要分为两层：

- `sivtr`：位于 `src/` 的二进制 crate；
- `sivtr-core`：位于 `crates/sivtr-core/` 的库 crate。

二进制层负责用户交互：CLI 解析、命令分发、TUI 状态、workspace picker、平台相关 launcher/hotkey，以及 remote-memory daemon。Core crate 负责可复用的 memory 逻辑：capture、解析、buffer、selection、search primitives、history、export、config、workspace 解析和 Agent provider session 解析。

## Workspace 布局

```text
sivtr/
|- Cargo.toml
|- src/
|  |- cli/
|  |  |- mod.rs
|  |  `- remote.rs
|  |- main.rs
|  |- app.rs
|  |- commands/
|  |  |- capture/
|  |  |- memory/
|  |  |- remote/
|  |  `- system/
|  |- remote/          # daemon runtime
|  `- tui/
`- crates/
   `- sivtr-core/
      `- src/
         |- archive/         # unified SQLite archive (archive.db) + sync engine
         |- agents/          # AgentProvider registry + per-provider parsers
         |- buffer/
         |- capture/
         |- config/
         |- export/
         |- history/
         |- parse/
         |- query/
         |- record/          # WorkRecord / WorkRef (scope + path + at)
         |- search/
         |- selection/
         |- session/
         `- workspace.rs
```

## Binary crate

| 区域 | 责任 |
| --- | --- |
| `cli/` | clap 命令定义和 help text（`mod.rs` + `remote.rs`） |
| `commands/capture/` | run、pipe、copy、init、flush、import、diff、clear、browse |
| `commands/memory/` | search、filter、var、nav、zoom、show、work、WorkSet store |
| `commands/remote/` | serve、share、remote（git-remote 风格命名）、peer、workspace list |
| `commands/publish/` | 本地 WorkSet 的隐私投影、AES-GCM envelope、公开链接状态与撤销 |
| `commands/system/` | config、doctor、history、hotkey、migrate、sync、version |
| `remote/` | 设备 daemon、identity、SQLite state、protocol、本地 IPC |
| `app.rs` | 捕获输出 browser 状态机 |
| `tui/` | 终端设置、事件处理、browser 渲染、workspace 渲染 |
| `command_blocks.rs` | session 浏览和复制用的命令块 span |

这一层可以依赖终端 UI 库、平台 API、进程启动，以及（daemon）异步网络。

## Core crate

| 模块 | 责任 |
| --- | --- |
| `archive` | 统一 SQLite archive（`archive.db`）、schema，以及填充它的 sync 引擎 |
| `agents` | `AgentProvider` registry 以及各 provider 发现/解析（Codex、Claude、Cursor、OpenCode、OpenClaw、Hermes、Grok、Pi…） |
| `record` | `WorkRecord`、`WorkPart`、`WorkRef` = `WorkScope` + `WorkPath` + `WorkAt`（`[scope:]path[/at]`） |
| `query` | 为 CLI 和 daemon 加载 workspace records 与 local-shaped sources |
| `capture` | stdin、subprocess、scrollback/session capture helpers |
| `parse` | ANSI 剥离、Unicode display width、行解析 |
| `buffer` | line、cursor、viewport 模型 |
| `selection` | visual / line / block selection 提取 |
| `search` | 文本匹配和导航状态 |
| `history` | SQLite 存储、schema、搜索 |
| `export` | clipboard、file、editor export helpers |
| `config` | TOML config 模型、默认值和路径解析 |
| `session` | 结构化 shell session entries 和渲染 |
| `workspace` | git-root workspace 解析、registry、`data_dir()` |

这种拆分让计算和数据处理可以独立于终端 UI 测试。

## Capture 流程

Pipe mode：

```text
stdin -> capture::pipe -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Run mode：

```text
subprocess -> combined output -> parse::parse_lines -> Buffer -> App -> TUI/editor
```

Session import：

```text
session log -> render entries -> parse::parse_lines -> Buffer -> command block spans -> TUI/editor
```

Command-block copy：

```text
session log -> SessionEntry list -> command blocks -> selector -> filters -> clipboard
```

Agent-provider copy：

```text
provider transcript/db -> AgentSession -> AgentBlock list -> selector -> filters -> clipboard
```

Workspace picker/search：

```text
terminal context + provider sessions -> WorkspaceSession list -> search/pick/show -> clipboard/stdout/json
```

## 统一 archive

查询（search、show、copy、TUI、MCP）从统一的本地 SQLite archive（`archive.db`）读取，而不是每次运行都解析原生文件。sync 引擎——`sivtr sync`，加上查询时自动的新鲜度同步——把每个 Agent provider 和每个 workspace 的终端日志写入 archive。原生 session 文件仍是 source of truth，只有 sync 引擎会读取它们。

```text
终端 log + provider session -> sync（stat stamp 比对）-> archive.db -> 查询路径
```

## Remote memory 流程

```text
owner:  sivtr share -> daemon Share + InviteTicket
peer:   sivtr remote add alias invite -> 当前 workspace 的 Mount
query:  desk:terminal/... -> resolve origin -> daemon IPC -> iroh -> authorize(share_id)
        -> load_workspace_source(root, body) -> SourceResponse -> WorkSet/show
```

模型：**Device Daemon + Identity + Share + Grant + Mount**。分享是 opt-in、只读；默认脱敏。

## Provider 边界

Agent support 在 command 和 workspace 层是 provider-neutral 的。Provider 模块负责找到本地记录，并把 provider-specific 事件格式转换成共享 memory blocks：

```text
AgentProvider -> AgentSessionProvider -> AgentSession -> AgentBlock
```

共享 workspace 代码随后可以 copy、pick、search、show memory，而不依赖某一家 vendor 的 transcript 形状。

## 设计边界

前端层负责呈现和交互。Rust core 负责持久 memory 工作：解析、capture、selection 提取、search、存储、provider 解析和格式化。remote daemon 在已授权 share root 上复用 core query loading，因此 remote 和 local refs 共享同一 record 模型。这样 UI 变化不会泄漏进 provider parsers，provider 变化也不会重写整个 CLI 表面。
