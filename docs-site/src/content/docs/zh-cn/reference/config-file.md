---
title: 配置文件
description: TOML 配置参考。
---

## 位置

`sivtr` 使用平台配置目录：

| 平台 | 当前路径 |
| --- | --- |
| Windows | `%APPDATA%\sivtr\config.toml` |
| macOS | `~/Library/Application Support/sivtr/config.toml` |
| Linux | `~/.config/sivtr/config.toml` |

## 完整示例

```toml
[editor]
command = "nvim"

[history]
auto_save = true
max_entries = 0

[sync]
max_age_secs = 15

[hotkey]
chord = "alt+y"

[theme]
mode = "auto"

[mcp]
idle_exit_secs = 60
```

## editor

```toml
[editor]
command = "nvim"
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `command` | string | `""` | 编辑器命令。空值表示自动检测。 |

示例：

```toml
command = "hx"
command = "nvim"
command = "vim"
command = "code --wait"
```

## history

```toml
[history]
auto_save = true
max_entries = 0
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `auto_save` | boolean | `true` | 保存捕获输出到 history |
| `max_entries` | integer | `0` | 最大保留条目数。`0` 表示无限制。 |

## sync

```toml
[sync]
max_age_secs = 15
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `max_age_secs` | integer | `15` | archive 距上次同步多少秒后，查询会触发一次增量重同步。`0` 表示每次查询都重新列目录。 |

## hotkey

```toml
[hotkey]
chord = "alt+y"
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `chord` | string | `"alt+y"` | `sivtr hotkey start` 使用的按键 |

## theme

```toml
[theme]
mode = "auto"
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `mode` | string | `"auto"` | TUI 配色方案：`auto`、`dark` 或 `light` |

`auto` 跟随系统外观（macOS/Linux XDG/Windows registry），并根据终端能力选择 truecolor 或 ANSI 调色板。`dark` 和 `light` 强制调色板。未知值和拼写错误（如 `mode = "ligth"`）是硬错误。

## mcp

```toml
[mcp]
idle_exit_secs = 60
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `idle_exit_secs` | integer | `60` | 无工具调用多少秒后 stdio MCP server 退出；`0` 表示保持到宿主关闭 stdin。`sivtr mcp serve --idle-exit` flag 覆盖此值。 |
