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

[sync]
max_age_secs = 15

[hotkey]
chord = "alt+y"

[theme]
mode = "auto"

[mcp]
idle_exit_secs = 60

[embedding]
endpoint = "https://api.openai.com/v1/embeddings"
model = "text-embedding-3-small"
api_key_env = "OPENAI_API_KEY"
batch_size = 64
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

## sync

```toml
[sync]
max_age_secs = 15
```

| Key | 类型 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `max_age_secs` | integer | `15` | archive 距上次同步多少秒后，查询会触发一次增量重同步。`0` 表示每次查询都重新列目录。 |

## embedding

```toml
[embedding]
endpoint = "https://api.openai.com/v1/embeddings"
model = "text-embedding-3-small"
api_key_env = "OPENAI_API_KEY"
batch_size = 64
```

semantic 和 hybrid 搜索必须显式配置 OpenAI-compatible embedding endpoint。
endpoint 必须使用 HTTPS；本地开发允许 loopback HTTP。`api_key_env` 为空时不发送认证 header。

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
