---
title: 安装
description: 安装 sivtr CLI、内置 memory skill，并设置 shell 集成。
---

`sivtr` 以 Cargo 包发布，源码位于 [github.com/Ariestar/sivtr](https://github.com/Ariestar/sivtr)。

完整的人 + Agent 工作流需要安装两部分：

- **CLI/TUI**：负责捕获和取回本地 workspace memory；
- 内置 **`sivtr-memory` skill**：负责教 Agent 怎么使用这些记忆。

只给人浏览时，单独安装 CLI 也可以；要让 Agent 复用本地上下文，建议两个都装。

## AI 助手快速安装

如果你是 AI 编码助手（Claude Code、Codex 等）替用户安装 sivtr：

### 安装前检查

```bash
sivtr --version    # 是否已安装？
sivtr doctor       # 完整环境检查
```

如果 `sivtr --version` 成功，**不要重新安装**。运行 `sivtr doctor` 检查环境，然后跳到下面的 skill 安装。

### 安装步骤

```bash
# 1. 安装二进制（选一个）
cargo install sivtr          # 从源码编译（需要 Rust）
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh  # 预编译二进制

# 2. 验证
sivtr --version

# 3. Shell hook（选一个）
sivtr init bash       # Linux 默认
sivtr init zsh        # macOS 默认
sivtr init powershell # Windows
sivtr init nushell

# 4. 配置
sivtr config init

# 5. Skill（用于 Agent 工作流）
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

### 安装后验证

```bash
sivtr doctor
```

预期：所有检查通过。若 `shell hooks` 显示 "not installed"，运行对应的 `sivtr init` 命令。若 `session log directory` 显示 "missing"，用户需在 `sivtr init` 后重启终端。

### 查看当前状态

```bash
sivtr init show       # 查看已安装的 shell hook
sivtr doctor          # 完整诊断
```

### 卸载

```bash
sivtr init uninstall  # 移除所有 shell hook
cargo uninstall sivtr # 移除二进制
```

## 要求

- Rust 和 Cargo
- 一个受支持的终端
- 当前平台可用的剪贴板能力

可选：

- `nvim`、`vim` 或 `vi`，用于部分 copy 工作流中的 Vim picker view
- 可修改 PowerShell、Bash、Zsh 或 Nushell profile，用于会话记录

## 用 cargo-binstall 安装（推荐）

通过 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) 下载预编译二进制（无需编译）：

```bash
cargo binstall sivtr
```

Linux x86_64 上 binstall 会解析到 **静态 musl** 发布资源，不依赖较新的系统 GLIBC，与 `install.sh` 同源。

验证：

```bash
sivtr --version
sivtr --help
```

## 用 Cargo 从源码安装

从 crates.io 编译安装最新发布版本：

```bash
cargo install sivtr
```

## 快速安装脚本（Linux/macOS）

下载预编译二进制，无需编译：

```bash
curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

指定版本：

```bash
SIVTR_VERSION=v0.4.0 curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
```

安装静态 musl 二进制到 `~/.local/bin/sivtr`（或 `$SIVTR_INSTALL_DIR`）。无需 Rust 工具链。

## 从源码安装

克隆仓库：

```bash
git clone https://github.com/Ariestar/sivtr.git
cd sivtr
```

在仓库根目录运行：

```bash
cargo install --path .
```

## 安装内置 skill

用 Skills CLI 全局安装 `sivtr-memory` skill：

```bash
npx skills add Ariestar/sivtr --skill sivtr-memory -g
```

安装后，当本地上下文可能已经存在时，可以要求 Agent 先用 sivtr：

```text
解决最近的终端报错，先用 sivtr 查。
```

## 更新

原地更新二进制：

```bash
sivtr update
```

该命令会下载当前平台最新的 GitHub release，校验 SHA256 后替换现有二进制。`sivtr doctor` 也会在新版本可用时提示。

通过 Cargo 从源码安装的用户可以更新已发布包：

```bash
cargo install sivtr --force
```

或在本地 checkout 拉取后重新安装：

```bash
git pull
cargo install --path . --force
```

## Shell 集成

Shell 集成会记录最近的命令块，让 `sivtr copy` 和命令块导航有结构化数据可用。

为你的 shell 安装 hook：

```bash
sivtr init powershell
sivtr init bash
sivtr init zsh
sivtr init nushell
```

查看已安装的 hook：

```bash
sivtr init show
```

卸载所有 hook：

```bash
sivtr init uninstall
```

安装或卸载后重启终端。

Hook 会写入按进程区分的 session log：

- Windows PowerShell 和 PowerShell 7 使用 `%APPDATA%\sivtr\session_<pid>.log`。
- Bash 和 Zsh 使用 `$XDG_STATE_HOME/sivtr/session_<pid>.log` 或 `~/.local/state/sivtr/session_<pid>.log`。
- Nushell 使用自己的 config/state 区域中的 `sivtr` session 文件。

## 配置文件

创建默认配置：

```bash
sivtr config init
```

查看路径和当前内容：

```bash
sivtr config show
```

用配置的编辑器打开：

```bash
sivtr config edit
```

完整配置项见[配置文件](/zh-cn/reference/config-file/)。
