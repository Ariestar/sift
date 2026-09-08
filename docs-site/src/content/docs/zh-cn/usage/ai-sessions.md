---
title: Agent 会话
description: 把已注册 Agent provider（Codex、Claude、Cursor、Hermes、OpenCode、OpenClaw、Grok、Pi、Dsh、Gemini、Goose、Qoder、Qwen…）变成可复用的 Agent 记忆。
---

`sivtr` 把 Agent transcript 当成本地 workspace memory source。你可以复制最新的有用 turn，在 picker 中浏览旧 session，跨 provider 搜索，并通过精确 ref 展示内容，而不必手动打开原始 transcript 文件。过去的 Agent 工作会成为人和后续 Agent 都能复用的记忆。

Skill 是让后续 Agent 学会使用这份记忆的方式。`sivtr` memory skill 可以要求 Agent 先搜索本地终端和 Agent 历史，只展开最小相关结果，并在信任历史讨论前验证当前代码。见 [Skill 与可复用流程](/zh-cn/usage/skills/)。

## 支持的 provider

Provider 来自 `AgentProvider` registry。copy 使用同一组名字：

| Provider | 复制命令 | 默认发现方式 |
| --- | --- | --- |
| Codex | `sivtr copy codex ...` | `~/.codex/sessions` 下的 Codex rollout JSONL |
| Claude Code | `sivtr copy claude ...` | Claude transcript/session 环境变量和本地 transcripts |
| Cursor | `sivtr copy cursor ...` | `~/.cursor/projects` 下 agent transcript JSONL |
| OpenCode | `sivtr copy opencode ...` | OpenCode 本地数据库 |
| OpenClaw | `sivtr copy openclaw ...` | OpenClaw agent SQLite（+ legacy JSONL） |
| Hermes | `sivtr copy hermes ...` | Hermes `state.db`（`sessions/` 下 JSONL 为 residual） |
| Grok | `sivtr copy grok ...` | Grok sessions（`~/.grok`，可用 `GROK_HOME`） |
| Dsh | `sivtr copy dsh ...` | Dsh agent sessions |
| Gemini | `sivtr copy gemini ...` | Gemini CLI sessions |
| Goose | `sivtr copy goose ...` | Goose agent sessions |
| Pi | `sivtr copy pi ...` | Pi agent 目录下的 session JSONL |
| Qoder / Qoder-CN | `sivtr copy qoder ...` | Qoder 与 Qoder-CN sessions |
| Qwen | `sivtr copy qwen ...` | Qwen Code sessions |

在 search 命令中，用 `agent` target 表示所有已注册 provider：

```bash
sivtr search agent --match "panic" --format timeline
```

## 复制最新有用文本

```bash
sivtr copy codex out
sivtr copy claude out
sivtr copy cursor out
sivtr copy hermes out
sivtr copy openclaw out
sivtr copy opencode out
sivtr copy grok out
sivtr copy pi out
```

`out` 会复制所选 provider 的最近助手回复。

## 复制输入、输出、工具输出或完整会话

每个 provider 都支持相同 mode：

```bash
sivtr copy claude
sivtr copy claude in
sivtr copy claude out
sivtr copy claude tool
sivtr copy claude all
```

| Mode | 复制内容 |
| --- | --- |
| 省略 | 最近完整 user + assistant turn |
| `in` | 最近用户消息 |
| `out` | 最近助手回复 |
| `tool` | 最近工具输出 |
| `all` | 完整解析会话 |

把 `claude` 换成任意已注册 provider 名（`codex`、`cursor`、`hermes`、`opencode`、`openclaw`、`grok`、`pi`、`dsh`、`gemini`、`goose`、`qoder`、`qodercn`、`qwen`…）即可。

## 选择更早内容

Selector 按"从新到旧"计数，和命令块 selector 一致：

```bash
sivtr copy claude 2
sivtr copy claude 2..4
sivtr copy codex out 3
```

用 `--session` 按 picker 编号、session id 或 id 前缀选择 session：

```bash
sivtr copy codex --session 2
sivtr copy codex --session 019df7fb
sivtr copy claude out --session 3 --print
```

## 过滤复制文本

```bash
sivtr copy claude tool --regex error
sivtr copy codex all --lines 1:40
sivtr copy hermes out --print
sivtr copy pi out --print
```

Filter 在选中文本组装后运行。见 [Selector 和 Filter](/zh-cn/reference/selectors-and-filters/)。

## 交互式选择

打开 provider 专属 picker：

```bash
sivtr copy codex --pick
sivtr copy claude --pick
sivtr copy hermes --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

Workspace picker 中常用按键：

| 按键 | 动作 |
| --- | --- |
| `0` / `1` / `2` / `3` | 聚焦 source、sessions、dialogues 或 content |
| `h` / `l` | 在面板间移动 |
| `j` / `k` | 在当前面板中移动 |
| `Space` | 切换当前 source、session 或 dialogue |
| `/` | 搜索所有已加载 session |
| `n` / `N` | 下一个/上一个搜索匹配 |
| `i` | 复制当前输入/问题 |
| `o` | 复制当前输出/回答 |
| `y` | 复制当前输入加输出 |
| `c` | 在可用时复制裸命令 |
| `:` | 为下一次复制启动临时行过滤 |
| Content 中 `v` | 开始可视文本选择 |
| `t` | 用 Vim 风格 full view 打开当前内容 |
| `?` | 打开帮助 |

Windows hotkey 和 VS Code extension 这类上下文启动器可以直接打开当前工作区 picker。

## 搜索 Agent 记忆和终端上下文

用 target-first search 搜索当前 workspace 的 Agent sessions 和终端上下文：

```bash
sivtr search agent --match "panic" --format timeline
sivtr search agent --match "workspace picker" --in title --format compact
sivtr search codex --match "sivtr" --in session --format md
sivtr search terminal --match "build error" --format json --limit 20
```

使用 JSON ref 配合 `sivtr show`。这些精确 ref 可以交还给你自己、另一个终端工作流，或后续 Agent prompt：

```bash
sivtr show claude/<session>/<dialogue>
sivtr show pi/<session>/<dialogue>/<line>
sivtr show terminal/current/<block>
```

## 统一 archive

所有已注册 provider 的 session 会和终端捕获一起同步进一个本地 SQLite archive（`archive.db`）。查询会自动执行新鲜度同步，新 session 无需手动操作就会出现；`sivtr sync` 可按需强制执行一次同步。

跨设备共享是 opt-in 的，由远程访问（`sivtr share` / `sivtr group`）处理，而不是导出 session 树。见[远程访问](/zh-cn/usage/remote-access/)。
