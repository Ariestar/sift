---
title: 快捷键
description: Workspace picker、Content、搜索、行过滤和 Vim view 快捷键。
---

本页记录 workspace picker TUI（默认的 `sivtr` 界面）。`pipe`/`run`/`import` 使用的单缓冲 browser 已移除——这些命令现在写入统一 archive 并用外部编辑器打开。

## Workspace picker

运行裸 `sivtr`、`copy --pick`、session picker 热键或 session import 进入 picker。

| 按键 | 动作 |
| --- | --- |
| `0` | 聚焦 Source 面板 |
| `1` | 聚焦 Sessions 面板 |
| `2` | 聚焦 Dialogues 面板 |
| `3` | 聚焦 Content 面板 |
| `j` / `k` | 下移 / 上移（Content：块光标） |
| `h` / `l` | 聚焦上一个 / 下一个面板 |
| `Space` | 切换 source/session/dialogue，或标记 content 块 |
| `a` | 全选 sources（Source）或切换所有 dialogues（Dialogues） |
| `g` | 选择 agent sources（Source）或 Content 滚动到顶部 |
| `G` | Content 滚动到底部 |
| `t` | 选择 terminal source（Source），或打开 Vim-style full view |
| `R` | 在活动行下刷新下一级 |
| `v` | Range-select 行（列表）或块区间标记 span（Content） |
| `Tab` | 切换 Content 的 Input / Output 半区 |
| `r` | 切换 Content 的 read/raw mode |
| `Enter` | 聚焦下一个 / 复制（列表）；折叠/展开光标块（Content） |
| `i` / `o` / `y` / `c` | 复制输入 / 输出 / 输入+输出块 / 裸命令 |
| `J` / `K` | 翻页上一个/下一个已选 dialogue（Content，多选） |
| `z` | 当前面板全屏切换 |
| `?` | 切换帮助浮层 |
| `/` | 打开搜索 |
| `q` / `Esc` | 取消 / 返回（Esc 也逐级上退面板） |
| `Ctrl-C` | 硬取消 |

Content 滚动：

| 按键 | 动作 |
| --- | --- |
| `Ctrl-D` / `PgDn` | Content 下滚 10 行 |
| `Ctrl-U` / `PgUp` | Content 上滚 10 行 |

## 结构块折叠

每个 workpart 都是可折叠块；工具调用 + 结果按 call id 分组。连续的结构单元折叠成 `<:kind xN:>` 标签。

- **`Enter`** 光标块——折叠/展开。
- **单击** 块标签——切换（仅 Reading mode；raw mode 始终展开）。
- **双击**（< 400ms）——折叠块。
- **`r`**——切换 read mode（markdown + 折叠标签）和 raw mode（完整载荷）。

两层：run 标签展开显示其成员（成员仍折叠），成员再展开显示正文。默认：结构块折叠、正文块展开。

## 鼠标

| 手势 | 动作 |
| --- | --- |
| 滚轮 | 滚动（3 行；Content 平滑滚动） |
| 单击 | 聚焦 + 选择 |
| 单击点状 gutter | 切换块标记 |
| 拖拽 | 线性选择 |
| Ctrl-拖拽 | 块选择 |
| 双击 | 折叠块 |
| 单击链接 | 打开它 |

## Workspace 搜索

| 按键 | 动作 |
| --- | --- |
| `/` | 打开搜索输入 |
| `Enter` | 接受搜索输入 |
| `Esc` | 清除或关闭搜索 |
| `Backspace` | 输入打开时编辑 query |
| `Ctrl-U` | 清除 query |
| `n` | 下一个匹配 |
| `N` | 上一个匹配 |

搜索前缀：

| 前缀 | 范围 |
| --- | --- |
| 无 | Content |
| `#` | Dialogue 标题 |
| `>` | Session 标题 |

## 行过滤输入

在 picker 中按 `:` 打开。需要至少一个 dialogue。

| 按键 | 动作 |
| --- | --- |
| 数字、`,`、`:` | 构建 1-based 行 spec |
| `Backspace` | 编辑待应用过滤 |
| `Esc` | 清除/取消 |

过滤自动应用到下一次复制快捷键（`i`/`o`/`y`/`c`）。示例：`2:8`、`1,3,8:12`。

## Vim-style full view

在 Sessions、Dialogues 或 Content 中按 `t` 打开。启动真实 `vim`，并配置块导航与复制绑定。

| 按键 | 动作 |
| --- | --- |
| `myy` | 复制块 |
| `myi` | 复制输入 |
| `myo` | 复制输出 |
| `mvv` | 选择块 |
| `mvi` | 选择输入 |
| `mvo` | 选择输出 |
| `p`, `q`, `Esc` | 返回 picker |
