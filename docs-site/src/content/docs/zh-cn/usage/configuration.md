---
title: 配置
description: 创建、查看、编辑并理解 sivtr 配置。
---

`sivtr` 使用平台配置目录中的 TOML 配置文件。配置控制编辑器交接、history 保留、archive 同步新鲜度、TUI 主题、MCP idle 退出和 Windows 热键按键。

## 命令

```bash
sivtr config show
sivtr config init
sivtr config edit
```

| 命令 | 行为 |
| --- | --- |
| `sivtr config show` | 打印配置路径和有效文件内容或默认值 |
| `sivtr config init` | 如果配置不存在，则创建默认配置 |
| `sivtr config edit` | 必要时创建配置，并用配置的编辑器打开 |

## 默认配置

```toml
[editor]
command = ""

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

[publish]
endpoint = "https://share.hnnulwh.cn"
```

`[publish].endpoint` 是加密公开链接服务地址。当前正式入口使用 `https://share.hnnulwh.cn`，由 Nginx 反向代理到同一台服务器上仅监听 loopback 的 Sivtr publication 服务；staging 或其他托管方式可以改为兼容同一 `/api/v1` 契约的 endpoint。该配置不会增加第二套认证路径。

字段级说明见[配置文件](/zh-cn/reference/config-file/)。

## History 保留

```toml
[history]
auto_save = true
max_entries = 0
```

`max_entries = 0` 表示无限制。如果不希望 pipe 和 run capture 自动写入 history，设置 `auto_save = false`。

## Archive 同步新鲜度

查询从统一的本地 archive（`archive.db`）读取。当 archive 比 `[sync].max_age_secs` 更旧时，查询会先触发一次增量同步：

```toml
[sync]
# archive 距上次同步多少秒后，查询会触发一次增量重同步。0 = 每次查询都重新列目录。
max_age_secs = 15
```

`0` 表示每次查询都重新列目录；调大则以新鲜度换延迟。用 `sivtr sync` 可强制执行一次同步。见[数据位置](/zh-cn/reference/data-locations/)。

## 热键按键

```toml
[hotkey]
chord = "alt+y"
```

除非使用 `--chord` 覆盖，否则 `sivtr hotkey start` 会使用这个按键。

Provider 选择是运行时 CLI 选项，不是配置项：

```bash
sivtr hotkey start --provider all
sivtr hotkey start --provider claude
```

## TUI 主题

```toml
[theme]
mode = "auto"
```

`auto` 跟随系统外观，并根据终端能力选择 truecolor 或 ANSI 调色板。用 `dark` 或 `light` 强制配色：

```toml
[theme]
mode = "dark"
```

## MCP 空闲退出

```toml
[mcp]
idle_exit_secs = 60
```

无工具调用多少秒后 stdio MCP server 退出（`0` = 保持到宿主关闭 stdin）。`sivtr mcp serve --idle-exit` flag 可在单次调用时覆盖此值。
