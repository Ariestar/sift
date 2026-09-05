---
title: 启动器和热键
description: 从 CLI、VS Code、tmux、Windows 热键、Linux 启动器和 macOS 启动器打开 Agent memory 与终端 picker。
---

Picker 是交互式路径：浏览 workspace memory、选择 dialogue 或命令块、过滤行，并复制结构化片段。

## 从 CLI 打开 picker

打开 provider picker：

```bash
sivtr copy claude --pick
sivtr copy codex --pick
sivtr copy hermes --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

打开终端命令块 picker：

```bash
sivtr copy --pick
sivtr copy out --pick
sivtr copy cmd --pick
```

## Workspace picker 按键

| 按键 | 动作 |
| --- | --- |
| `0` / `1` / `2` / `3` | 聚焦 Source、Sessions、Dialogues 或 Content |
| `j` / `k` | 在当前面板中移动 |
| `h` / `l` | 移到前一个/后一个面板 |
| `Space` | 切换当前 source、session 或 dialogue |
| `a` | 在支持的面板中全选 |
| `g` | 在 Source 面板中选择 agent sources |
| `t` | 在 Source 面板中选择 terminal source，或在其他位置打开 Vim view |
| `v` | Range-select dialogues，或在 Content 中开始可视文本选择 |
| `:` | 为下一次复制启动临时行过滤 |
| `i` / `o` / `y` / `c` | 复制输入、输出、块或命令 |
| `/` | 搜索所有 session |
| `n` / `N` | 下一个/上一个搜索匹配 |
| `z` | 当前面板全屏切换 |
| `?` | 帮助 |
| `q` / `Esc` | 取消或返回 |

Picker 搜索前缀：

| 前缀 | 范围 |
| --- | --- |
| 无 | Content |
| `#` | Dialogue 标题 |
| `>` | Session 标题 |

## Windows 全局热键

内置全局热键 daemon 仅支持 Windows：

```bash
sivtr hotkey start
sivtr hotkey status
sivtr hotkey stop
```

默认按键：

```text
alt+y
```

启动时可覆盖按键或 provider：

```bash
sivtr hotkey start --chord ctrl+shift+y
sivtr hotkey start --provider all
sivtr hotkey start --provider claude
```

支持的 provider 值是 `all`、`codex`、`claude`、`opencode`、`pi`、`cursor`、`openclaw`、`grok`、`hermes`、`dsh`、`gemini`、`goose`、`qoder`、`qodercn` 和 `qwen`。

按下热键后，daemon 会打开一个终端，在 daemon 工作目录中运行内部 picker 命令。Picker 会先尝试最新的非空当前 session，再回退到 session 列表。

## VS Code

安装 Marketplace 扩展：

```text
ariestar.sivtr-vscode
```

扩展会从当前 workspace 打开 Agent session picker。默认快捷键是 `Alt+Y`。如果缺少 `sivtr` CLI，扩展会在可见终端中提供 Cargo 安装流程。

## tmux

安装 tmux helper：

```bash
sivtr init tmux
tmux source-file ~/.tmux.conf
```

这会把 `prefix + y` 绑定为从当前 pane 路径打开 picker。

## Linux 桌面启动器

Linux 没有内置的通用桌面全局热键 daemon。Wayland 和不同桌面环境差异很大，跨 GNOME、KDE、Sway、tmux 和 SSH 通用地打开交互式终端并不可靠。

改为生成项目专属启动器：

```bash
sivtr init linux-shortcut
```

它会写入：

- `~/.local/bin/sivtr-pick-codex`
- `~/.local/share/applications/sivtr-pick-codex.desktop`

生成出来的脚本会使用生成它的那一个 `sivtr` 二进制，直接打开 provider-neutral 的 AI session picker。

把桌面快捷键绑定到生成的脚本，或直接从终端运行它。

## macOS 启动器

生成 Terminal 启动器和 LaunchAgent wrapper：

```bash
sivtr init macos-shortcut
```

它会写入：

- `~/.local/bin/sivtr-pick-codex`
- `~/Library/LaunchAgents/dev.sivtr.pick-codex.plist`

生成出来的脚本会使用生成它的那一个 `sivtr` 二进制，直接打开 provider-neutral 的 AI session picker。

可以直接运行脚本，或加载 LaunchAgent：

```bash
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.sivtr.pick-codex.plist
```
