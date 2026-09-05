---
title: 浏览和选择
description: 导航 workspace 浏览器、折叠结构块、选择并复制。
---

**workspace 浏览器**（TTY 下裸 `sivtr`，或 `sivtr copy --pick` / 热键）是交互界面：多源 Source → Sessions → Dialogues → Content。`sivtr pipe` 和 `sivtr run` 不再打开 TUI——它们写入统一 archive 并用外部编辑器打开。

## 打开 workspace 浏览器

```bash
sivtr                     # TTY：多源 workspace 浏览器
sivtr --all               # 打开时也选中 remote mount
sivtr copy --pick         # 同一浏览器，面向复制
sivtr copy claude --pick
```

布局：Source · Sessions · Dialogues · Content。Content 拆成独立滚动的 **Input** / **Output** 半窗。

### Workspace 导航

| 按键 | 动作 |
| --- | --- |
| `0` / `1` / `2` / `3` | 聚焦 Source、Sessions、Dialogues 或 Content |
| `h` / `l` | 上一 / 下一面板 |
| `j` / `k` | 下移 / 上移 |
| `Space` | 切换选择（source / session / dialogue）· 标记 content 块 |
| `a` | 全选 source（Source）· 切换全部 dialogue（Dialogues） |
| `g` / `t` | 选 agent 源 / terminal 源（Source） |
| `R` | 刷新活动行下一级 |
| `v` | Range-select 行 · 块区间标记 span（Content） |
| `Tab` | Content 半窗 Input ↔ Output |
| `r` | 切换 read/raw content（结构标记 + 折叠标签 vs 完整载荷） |
| `Ctrl-d` / `Ctrl-u` · `PgDn` / `PgUp` | 滚动 Content |
| `g` / `G` | Content 顶 / 底 |
| `J` / `K` | 翻页上一/下一个已选 dialogue（Content，多选） |
| `i` / `o` / `y` / `c` | 复制输入 / 输出 / 块 / 命令 |
| `Enter` | 确认 / 打开下一级 / 复制；折叠/展开光标块（Content） |
| `/` | 搜索 |
| `z` | 当前面板全屏 |
| `t` | Vim 风格 full view（Sessions/Dialogues/Content） |
| `?` | 帮助 |
| `q` / `Esc` | 退出 / 返回 |

鼠标：单击聚焦 + 选择；拖拽线性选择，`Ctrl`-拖为块选；点击点状 gutter 切换块标记；单击/双击折叠结构块。Content 半窗高度偏向当前焦点半窗。

### 结构块

每个 workpart 都是可折叠块，工具调用 + 结果按 call id 分组。连续结构单元（tool / skill / thinking）折叠成 `<:kind xN:>` run：

- `Enter` 折叠/展开光标块。
- 单击 run 标签展开成员；单击成员展开正文（两层）。
- `r` 切换 read mode（markdown + 折叠标签）和 raw mode（完整载荷）。

run 成员和工具调用按 call id 分组，交错的并行工具调用也能正确配对。

完整按键见[快捷键](/zh-cn/reference/keybindings/)。

## 复制到剪贴板

配置 `pipe`/`run`/`import` 使用的编辑器（不是 TUI）：

```toml
[editor]
command = "nvim"
```
