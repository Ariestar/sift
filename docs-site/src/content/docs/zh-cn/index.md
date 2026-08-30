---
title: sivtr
description: 面向人和 Agent 的 shared memory workspace。
---

`sivtr` 是一个本地优先的 shared memory workspace，服务于人和 Agent。它把项目周围已经发生的工作——终端命令、命令输出、AI Agent 对话、工具结果和被复制的上下文——变成可搜索、可选择、可引用、可复用的记忆，让人和 Agent 都能重新使用。

它放在你已有的终端和 Agent 旁边，为这些本地工作提供一个共享记忆工作区。

完整工作流需要同时安装 `sivtr` CLI/TUI 和内置 `sivtr-memory` skill。CLI 负责捕获和取回记忆；skill 负责教 Agent 在让你粘贴上下文之前，先主动查询这些记忆。

## sivtr 不是什么

`sivtr` 不是：

- 终端模拟器；
- tmux 替代品；
- 云端 transcript 服务；
- 另一个 Agent runtime。

## sivtr 解决什么问题

- **保留输出，而不只是命令**：来自管道、子进程、shell 集成和本地 Agent transcript。
- **舒服地浏览长日志**：提供键盘优先的 Vim 风格 TUI。
- **复制最近有用内容**：可复制输入、输出、裸命令或完整块。
- **搜索本地 Agent 对话**：从已注册 provider（Codex、Claude Code、Cursor、Hermes、OpenCode、OpenClaw、Grok、Pi…）找回旧决策和解释。
- **让 Agent 从证据开始**：让"解决终端报错"从最近捕获的失败输出开始，而不是先让你粘贴日志。
- **从摘要跳回原文**：搜索命中、交接说明和时间线都能继续追到原始上下文。
- **把搜索结果保存成变量**：例如 `@last`、`@failures`，后续命令可以继续复用。
- **只读分享 workspace**：让队友用 `desk:terminal/...` 搜索你的 session。
- **快速打开 memory picker**：支持 CLI、tmux、VS Code、Windows 热键和生成的桌面启动器。

## 最先要会的命令

```bash
# 把命令输出作为可复用 workspace memory 浏览。
cargo test 2>&1 | sivtr

# 让 sivtr 执行命令并捕获 stdout/stderr。
sivtr run cargo test

# 复制最近一次记录的命令输出。
sivtr copy out

# 复制某个 Agent provider 的最近回复。
sivtr copy claude out
sivtr copy codex out
sivtr copy cursor out
sivtr copy grok out

# 搜索当前 workspace memory。
sivtr search agent --match "panic" --format timeline
```

## 文档地图

| 目标 | 从这里开始 |
| --- | --- |
| 安装 CLI + skill | [安装](/zh-cn/start/installation/) |
| 走一遍日常路径 | [快速开始](/zh-cn/start/quickstart/) |
| 理解模型 | [心智模型](/zh-cn/start/core-concepts/) |
| 捕获输出 | [捕获终端输出](/zh-cn/usage/capture-output/) |
| 复制最近命令 | [复制命令块](/zh-cn/usage/copy-command-blocks/) |
| 复用 Agent 记忆 | [Agent 会话](/zh-cn/usage/ai-sessions/) |
| 让 Agent 学会 memory workflow | [Skill 与可复用流程](/zh-cn/usage/skills/) |
| 查看社区玩法 | [玩法实例](/zh-cn/playbooks/) |
| 搜索和按 ref 展示记忆 | [搜索和展示结果](/zh-cn/usage/search-and-show/) |
| 发布浏览器只读对话链接 | [发布对话链接](/zh-cn/usage/publish/) |
| 分享并添加远端记忆 | [远程访问](/zh-cn/usage/remote-access/) |
| 快速打开 picker | [启动器和热键](/zh-cn/usage/launchers-and-hotkeys/) |
| 查询精确语法 | [CLI 参考](/zh-cn/reference/cli/) |

## 心智模型

`sivtr` 分成两层：

| 层 | 说明 |
| --- | --- |
| 发生过什么 | 终端输出、命令块、Agent 对话、工具结果和本地 history。 |
| 怎么复用 | TUI 浏览、search、copy、show、diff、skill、playbook、命名 remote，以及 `@last` 这类记忆变量。 |

终端 source 产生命令块，Agent provider 产生对话块。`1`、`2..4` 这样的 selector 用来选择最近条目。搜索结果可以保存成 `@failures` 这样的变量，再展示、扩展或管道传给下一条命令。

## 默认本地优先

`sivtr` 读取本地 shell 日志、本地 history 和本地 Agent transcript。跨设备访问通过 [远程访问](/zh-cn/usage/remote-access/) 显式开启。数据位置见 [数据位置](/zh-cn/reference/data-locations/)。
