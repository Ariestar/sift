---
title: Skill 与可复用流程
description: 让 Agent 把 sivtr 当成共享记忆入口。
---

`sivtr` 对人来说是 CLI 和 TUI；对 Agent 来说，还需要 **skill** 才能真正发挥作用。Skill 是一组可复用流程，告诉 Agent 什么时候应该查本地工作记忆、怎么查、查到后如何验证和行动。

Skill 不是另一个存储。它是 Agent 使用同一层记忆的操作手册：

```text
人的工作 + Agent 的工作 -> sivtr 记忆 -> skill 流程 -> Agent 行动
```

关键变化是：Agent 可以把 `sivtr` 当成进入本地工作记忆的统一入口。日志、报错、历史决策和验证结果都能通过同一个接口读取。

内置的 `sivtr-memory` skill 是推荐安装流程的一部分，不只是可选说明。没有 CLI，skill 没有可查询的入口；没有 skill，很多 Agent 不会主动在询问你之前先查本地上下文。

先按照[安装](/zh-cn/start/installation/)装好 CLI 和 skill，再把本页当成 skill 的操作说明。

可选：注册只读 MCP server，让 agent 宿主直接调用结构化工具，而不是拼 shell：

```bash
sivtr mcp install -y
# 或：
sivtr mcp install -p claude,cursor -l global
```

这会把 `sivtr mcp serve` 写入已注册宿主配置（Claude Code、Cursor、Codex、OpenCode、OpenClaw、Grok、Hermes、Pi、Qoder/Qoder-CN、Gemini、Qwen、Goose…）。MCP 工具：`sivtr_search`、`sivtr_show`、`sivtr_zoom`、`sivtr_filter`、`sivtr_status`、`sivtr_usage`、`sivtr_stats`（`status` 含 `ws` 本机 origin 标签和 remotes）。skill 仍负责教 *何时/如何* 取证；MCP 是结构化执行面。见 [CLI 参考](/zh-cn/reference/cli/#mcp)。

## 为什么需要 skill

没有 skill 时，Agent 容易漏掉本地已有证据。

有了 `sivtr` memory skill，Agent 可以：

1. 搜索最近的终端和 Agent 记忆；
2. 只展开最相关的命令输出或对话；
3. 检查当前代码或配置；
4. 修复问题；
5. 运行验证；
6. 用证据汇报结果。

这会把 `sivtr` 从"给人用的剪贴板助手"推进为"人和 Agent 都会操作的共享记忆工作区"。

## 内置的 sivtr-memory skill

仓库中已经包含一个起步 skill：

```text
skills/sivtr-memory/
```

它教 Agent 用 `sivtr` 处理：

- 最近终端失败；
- 缺失或被截断的命令输出；
- 之前 Agent 的决策；
- 会话被压缩或中断后的继续工作；
- 交接和回顾上下文；
- 构建、测试、lint、部署的验证证据。

核心规则：

> 先搜索证据。只展开最小相关上下文。只有在记忆缺失、含糊、过期或需要权限时才问用户。

这就是 `sivtr` 产品模型在 Agent 侧的表达：先查本地证据，尽量使用精确引用，只有本地记忆无法回答时再要求人类澄清。

## Agent 怎么用 sivtr 记忆

在非交互式 Agent 流程中，优先使用打印结果的命令，而不是打开选择器或修改剪贴板。Search format 按任务选择：`timeline` / `compact` / `md` 往往更适合理解和总结；`json` 更适合另一个程序解析 ref 和字段。

有 MCP 时优先用工具。CLI 等价：

```bash
sivtr s terminal -m "error|failed|panic|Traceback|Exception|exit code|FAILED" -f timeline --latest 20
sivtr s terminal --status failure --latest 1 --json
sivtr s agent -m "decision|TODO|next step" --latest 20 --save hits --refs
sivtr filter @hits -m "<topic>" --save narrowed --refs
sivtr show @narrowed[1] --full
sivtr copy out 1 --print
```

除非用户明确要求，否则避免：

- 裸 `sivtr` 与 `--pick` 会开 TUI（Agent 自主运行时只给人用）；
- 启停热键、改 share/remote、改配置；
- 倾倒巨大会话——未设边界时 search 默认 `--latest 5`；
- 取内容时加 `--print`（默认写剪贴板）。

## Skill 的结构

一个有用的 `sivtr` skill 通常包含四部分：

| 部分 | 作用 |
| --- | --- |
| 触发条件 | 什么时候 Agent 应该用 `sivtr` 读取本地记忆。 |
| 检索命令 | 安全的搜索、复制、展示命令配方。 |
| 证据纪律 | 如何引用来源，区分终端证据和历史 Agent 讨论，并验证当前状态。 |
| 工作流配方 | 调试、继续、交接、回顾、时间线或审查流程。 |

内置的 `skills/sivtr-memory/` 目录就是这种结构：主 `SKILL.md`、参考文件和示例。

## Skill 注册表方向

未来的 skill 注册表可以让这些流程被社区和团队发现、复用和改进。每个用户不必都重新写一套"看看刚才的报错""总结今天工作"的提示词，而是可以复用经过验证的流程；这些流程都指向同一个本地记忆接口。

潜在注册表条目包括：

- 终端失败调试器；
- 最近工作时间线生成器；
- PR 交接文档生成器；
- 基于验证记录的发布说明草稿器；
- 事故回顾助手；
- 远程协作记忆读取器（[远程访问](/zh-cn/usage/remote-access/) + `desk:...` refs）；
- 基于本地会话的项目入门指南。

具体玩法见[玩法实例](/zh-cn/playbooks/)。
