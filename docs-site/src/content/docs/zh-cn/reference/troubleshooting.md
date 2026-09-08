---
title: 故障排查
description: 诊断命令块缺失、Agent session 为空、剪贴板问题、热键失败和文档版本不一致。
---

本页列出常见失败模式和优先检查项。

## 安装后报 `GLIBC_2.xx' not found`

预编译的 **glibc 动态链接** Linux 二进制在 GitHub `ubuntu-latest` 上构建，可能要求比旧发行版更高的 GLIBC。当前 `cargo binstall sivtr` 的 metadata 会为 `linux-gnu` 与 `linux-musl` 主机都下发 **静态 musl** 资源，新装一般不会再撞这个问题。

若仍是旧的 glibc 二进制（例如上次安装残留，或 crates.io 上的 metadata 尚未更新）：

```bash
# 显式重装静态 musl 构建
cargo binstall sivtr --targets x86_64-unknown-linux-musl

# 或
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

确认：

```bash
ldd "$(command -v sivtr)"   # 静态 musl 通常显示 "not a dynamic executable"
sivtr --version
```

## `sivtr copy out` 找不到命令块

命令块复制需要 shell 集成，并且需要重启 shell。

检查：

```bash
sivtr init powershell
# 或：sivtr init bash / zsh / nushell
```

然后重启终端，运行一个命令，再尝试：

```bash
sivtr copy out --print
```

如果 pipe mode 正常但 `copy` 不工作，问题通常在 session logging，而不是 browser。

## `sivtr import` 没打开有用内容

`import` 读取当前结构化 shell session log。它在 shell 集成已经在当前 shell 进程记录过多个命令后最有用。

尝试：

1. `sivtr init <shell>` 后重启 shell。
2. 运行一个可见命令，例如 `echo hello`。
3. 运行 `sivtr import`。

## Agent provider picker 为空

Provider picker 只显示当前工作区可发现的本地 session。

检查：

```bash
sivtr copy codex --pick
sivtr copy claude --pick
sivtr copy opencode --pick
sivtr copy pi --pick
```

如果某个 provider 为空但另一个能用，问题通常是该 provider 的发现逻辑或本地数据缺失。如果全部为空，确认你是否在与 session 工作目录匹配的项目目录中运行。

从其他目录运行 search/show 时，使用 `--cwd`：

```bash
sivtr search agent --match "panic" --cwd /path/to/project --format timeline
```

## Linux 剪贴板复制失败

剪贴板支持依赖桌面/session 环境。Wayland、X11、SSH 和 headless 环境行为可能不同。

先用 `--print` 验证文本本身：

```bash
sivtr copy out --print
sivtr copy claude out --print
```

如果打印文本正确但剪贴板为空，问题很可能在平台剪贴板集成，而不是选择或解析。

## Windows 热键无法启动

检查状态：

```bash
sivtr hotkey status
```

再尝试不同按键：

```bash
sivtr hotkey start --chord ctrl+shift+y
```

如果注册失败，可能是其他应用已经占用了这个快捷键。

## Linux 没有全局热键

这是预期行为。Linux 当前没有内置桌面级 `sivtr` daemon，因为 Wayland 和各桌面环境没有给普通 CLI 应用提供统一快捷键 API。

请改用：

```bash
sivtr init tmux
sivtr init linux-shortcut
```

或使用 VS Code 扩展快捷键。

## `sivtr show <ref>` 找不到 ref

Ref 是基于当前 workspace session list 解析的。如果你运行 `show` 的目录不同于原始搜索目录，传入相同的 `--cwd`：

```bash
sivtr search agent --match "panic" --cwd /path/to/project --format json
sivtr show <ref> --cwd /path/to/project
```

同时检查 ref source 是否存在，例如 `codex`、`claude`、`opencode`、`pi` 或 `terminal`。

## Regex filter 匹配不到内容

`--regex` 只保留匹配行。如果 pattern 无效或过窄，结果可能为空。

用 `--print` 和更简单的 pattern 调试：

```bash
sivtr copy out --regex error --print
sivtr copy out --regex "error|failed" --print
```

同时设置 `--regex` 和 `--lines` 时，`--regex` 先运行。

## 文档和 CLI 不一致

已安装二进制的 CLI 是事实来源：

```bash
sivtr --help
sivtr version --verbose
sivtr copy --help
sivtr copy claude --help
```

如果网站描述的是更新命令，而你的二进制不支持，请更新 `sivtr`：

```bash
cargo install sivtr --force
```
