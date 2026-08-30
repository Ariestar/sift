---
title: Local-first 与隐私
description: sivtr 如何让 Agent memory、终端输出和 transcript 保持在本地用户控制之下。
---

`sivtr` 围绕本地 Agent memory 设计。终端输出、shell session log、history 和 Agent transcript 可能包含密钥、私有代码、凭据、内部 URL 和未完成推理。默认姿态是让这些数据留在原本产生它们的机器上。

## 默认本地

`sivtr` 读写本地文件和数据库：

- shell 集成产生的 shell session log；
- 捕获终端输出的本地 SQLite history；
- 统一 session archive（`archive.db`）；
- provider 自己的 Agent transcript 文件或数据库；
- 平台配置目录下的本地配置。

默认不提供托管 transcript 服务。

## 本地 archive

终端捕获和 Agent session 会同步进一个本地 SQLite archive（`archive.db`），位于 sivtr 的 data 目录下。这个过程中没有任何数据离开本机：原生 session 文件仍是 source of truth，只有 sync 引擎会读取它们。

## 显式远程分享

跨设备记忆访问同样是 opt-in。只有你创建 share（`sivtr share` / `share add`）、签发 invite（`share invite`），并且 peer 兑换之后，数据才会离开本机：

```bash
sivtr share                   # 交互式；只创建 share
sivtr share invite alice-desk # 单次 invite（stdout = bare key）
sivtr remote add desk <invite> # peer 在其 workspace 里给 remote 起名
```

远程访问是只读的。数据离开本机前默认脱敏（`--no-redact` 可关闭）。邀请会过期（默认 `10m`）。daemon 之间走加密 iroh。默认仍然本地优先：未登记的 origin 会报错。

完整指南见 [远程访问](/zh-cn/usage/remote-access/)。

## 浏览器只读发布

`sivtr publish` 是另一条明确的外发边界：它不是实时设备到设备 mount，而是从本地 WorkSet 生成一次不可变快照。全 record WorkSet 走 v1，只允许单个本地 Agent session 的连续对话轮次并投影 User/Assistant；不带 source 的 `publish preview` 会打开现有 TUI，在同一 session 内选择不连续的 User、Assistant、Tool、Skill、Thinking 原子。原始 WorkSet、WorkRef、`cwd`、session path 和 provider 事件仍留在本机，只有经过投影、脱敏后的快照会进入加密 envelope。

公开服务接收 AES-256-GCM 密文，以及 publication ID、`X-Sivtr-Management-Token` 和 `X-Sivtr-Published-At` 等发布请求元数据。解密密钥位于链接 fragment，不会随 HTTP 请求发送给托管服务；因此查看者无需账号，发布者设备离线也不影响查看。链接持有者均可阅读，默认 7 天失效（可选 2h/1d/3d/30d），可以在本机用 `sivtr publish revoke` 提前撤销。管理凭据只保存在独立的 `publication-state.db`；数据库丢失时没有账号恢复路径。

发布前必须检查 `preview` 的最终文本和风险报告。高置信度 token、私钥、Bearer 和 secret assignment 会自动替换为 `[REDACTED]`；路径、邮箱和内网地址不会擅自改写，命令行发布需要显式使用 `--allow-warnings`，TUI 发布则会要求明确确认。

## 剪贴板是输出边界

Copy 命令会把选中文本放入系统剪贴板：

```bash
sivtr copy out
sivtr copy claude out
```

请把剪贴板内容视为会被桌面环境和剪贴板管理器共享。敏感场景下可用 `--print` 先检查文本。

## History 保留可配置

启用时，捕获的终端输出会保存到 history：

```toml
[history]
auto_save = true
max_entries = 0
```

如果不希望 capture 自动写入，设置 `auto_save = false`。把 `max_entries` 设为正数可以限制保留数量。

## 良好操作习惯

- 把 archive 当作源 transcript 对待：它包含同样的敏感文本，注意其存放位置和访问权限。
- 把内容粘贴到公开聊天、issue、托管 Agent 或外部 AI 工具前，先检查复制文本。
- 用 line 和 regex filter 只复制必要证据。
- 工具链使用 `--format json` / `--refs` search 输出时，也要记住 JSON content 可能包含敏感文本。
- 协作结束后优先用短寿命 invite，并用 `sivtr share revoke` 撤销 grant。

