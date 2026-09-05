---
title: 快速开始
description: 打开统一 workspace TUI，浏览终端命令和所有 Agent 会话，并让 Agent 复用同一份本地记忆。
---

`sivtr` 有两条日常路径：

- **给人用**：打开 `sivtr` TUI 面板，一起浏览终端输出和 Agent 会话，跨来源搜索，再复制真正需要的片段。
- **给 Agent 用**：安装内置 `sivtr-memory` skill，然后通过 `sivtr search`、`sivtr copy --print` 和 `sivtr show` 等 CLI 指令检索本地 workspace memory。

## 1. 安装 CLI 和 skill

先按[安装](/zh-cn/start/installation/)完成准备。重点是 Agent 工作流需要两个部分都在位：

- `sivtr` CLI 必须在 `PATH` 中可用；
- 内置 `sivtr-memory` skill 必须用 `npx skills add Ariestar/sivtr --skill sivtr-memory -g` 安装。

之后你就可以要求 Agent “先用 sivtr 查”，它会有明确流程去搜索本地记忆。

## 2. 先让终端工作进入 workspace

如果是第一次使用，先安装一次 shell 集成：

```bash
sivtr init powershell
# 或：sivtr init bash
# 或：sivtr init zsh
# 或：sivtr init nushell
```

重启 shell，然后照常工作：

```bash
bun run build
cargo test
git status --short
```

这一步会让最近命令和输出出现在 workspace 里。Agent 会话会从各已注册 provider 的本地 session 数据读取。

## 3. 打开统一 workspace TUI

运行：

```bash
sivtr
```

这是最重要的入口。它不是只看 terminal，也不是只看某一个 Agent，而是把当前机器上的本地工作记忆合在一个界面里。

Workspace TUI 有四个面板。Content 拆成 Input / Output 半窗。

| 面板 | 用途 |
| --- | --- |
| Source | 选择来源，比如 terminal 或某个已注册的 Agent provider |
| Sessions | 选择某个终端记录或 Agent session |
| Dialogues | 选择某轮对话或命令块 |
| Content | 查看输入与输出（独立半窗） |

常用按键：

| 按键 | 动作 |
| --- | --- |
| `0` / `1` / `2` / `3` | 聚焦 Source、Sessions、Dialogues、Content |
| `j` / `k` | 当前面板上下移动 |
| `h` / `l` | 前一个 / 后一个面板 |
| `Space` | 切换当前 source、session 或 dialogue |
| `Tab` | Content 半窗 Input ↔ Output |
| `r` | 折叠结构标记 / 展开完整载荷 |
| `R` | 刷新活动行下一级 |
| `/` | 搜索 workspace |
| `i` / `o` / `y` / `c` | 复制输入 / 输出 / 块 / 裸命令 |
| `z` | 当前面板全屏 |
| `?` | 帮助 |
| `q` / `Esc` | 返回或退出 |

完整表见[浏览和选择](/zh-cn/usage/browse-and-select/)。

Workspace 搜索支持前缀：

| 写法 | 搜索范围 |
| --- | --- |
| `error` | 内容 |
| `#build` | 对话 / 命令块标题 |
| `>release` | session 标题 |

## 4. 只打开某一类内容

统一 workspace 是默认入口。如果你只想看某一类内容，可以用更具体的 picker。

只看最近终端命令块：

```bash
sivtr copy --pick
```

只看某个 Agent provider：

```bash
sivtr copy claude --pick
sivtr copy codex --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

这些是过滤后的 workspace 视图，不是主入口。

## 5. 需要快速复制时，用命令直接取

已经知道要拿什么时，不必打开 TUI：

```bash
sivtr copy          # 最近一次命令 + 输出
sivtr copy out      # 最近一次输出
sivtr copy cmd      # 最近一次命令
sivtr copy out 2    # 上一次命令输出
sivtr copy 2..4     # 一段最近命令块
```

读取 Agent 会话：

```bash
sivtr copy claude out --print
sivtr copy codex out --print
sivtr copy opencode out --print
sivtr copy pi out --print
```

给 Agent 用时通常加 `--print`，避免打开交互界面或只写入剪贴板。

## 6. 让 Agent 先查 workspace memory

当你遇到报错，可以直接说：

```text
解决刚才的终端报错，先用 sivtr 查。
```

Agent 应该搜索同一份 workspace memory：

```bash
sivtr search terminal --match "error|failed|panic|Traceback|Exception|exit code|FAILED" --format json --limit 20
sivtr search terminal --status failure --latest 1 --format json
sivtr copy out 1 --print
sivtr copy cmd 1..10 --print
```

然后它再读代码、修复问题、重新运行验证命令。

这就是 `sivtr` 的核心价值：你在统一 workspace 里能看到的东西，Agent 也能用命令读取。

## 7. 用 ref 回到精确证据

搜索结果会包含 ref。ref 可以指向终端命令块、Agent session、某轮 dialogue，或其中一段内容。

```bash
sivtr search terminal --match "build error" --format json --limit 20
sivtr show terminal/current/2
sivtr show claude/<session>/3
sivtr show claude/<session>/3/2
```

Ref 让人和 Agent 回到同一份证据，而不是凭印象复述。

## 8. 单次长输出也可以打开 browser

除了 workspace TUI，`sivtr` 也有一个单缓冲区 browser，适合临时查看一段长输出：

```bash
cargo test 2>&1 | sivtr
sivtr run bun run build
```

这个界面只浏览当前这次输出；裸 `sivtr` 打开的 workspace TUI 才是统一查看终端和 Agent 记忆的主入口。

## 下一步

- 阅读[心智模型](/zh-cn/start/core-concepts/)理解 source、session、dialogue、block 和 ref。
- 看[玩法实例](/zh-cn/playbooks/)了解真实工作流。
- 查看[浏览和选择](/zh-cn/usage/browse-and-select/)学习完整 workspace TUI 操作。
- 需要另一台设备记忆时，看[远程访问](/zh-cn/usage/remote-access/)。
- 查看 [CLI 参考](/zh-cn/reference/cli/)确认精确选项。
