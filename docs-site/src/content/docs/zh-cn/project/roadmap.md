---
title: Roadmap
description: sivtr 和更广义 Agent Memory Workspace 的方向性产品路线图。
---

这份 roadmap 是工作计划,不是发布承诺。它用结果导向描述 `sivtr` 的方向:保持一个小而有用的终端工具,同时逐步成为面向人和 Agent 的统一 Agent Memory Workspace。

## Roadmap map

```text
Reliable CLI
  -> Multi-agent workspace
    -> High-quality retrieval
      -> Skills, MCP, and agent interfaces
        -> High-signal TUI
          -> Remote collaboration + privacy lifecycle
            -> Provenance, trust, and memory hygiene
              -> Semantic / multimodal memory
                -> A2A messenger + sivtr-me
```

| Track | 状态 | 目标结果 |
| --- | --- | --- |
| CLI foundation | 进行中 | 一个日常可用的 CLI,用于捕获、搜索、选择和导出终端与 agent 工作。 |
| Agent support | 核心已落地 | 面向 AI Agent 对话记录的 provider-neutral 解析和浏览。 |
| Retrieval quality | 核心已落地 | 结构化、精确、可排序的检索,决定证据范式是否真正好用。 |
| Skills and playbooks | 进行中 | 把 `sivtr` 作为统一记忆入口的可复用 Agent 流程。 |
| Agent interfaces | 核心已落地 | CLI、MCP 和本地 Web API,让其他 Agent 把 `sivtr` 当工作记忆基础设施。 |
| TUI workspace | 规划中 | 面向多 session、多 provider、长对话的高密度键盘优先界面。 |
| Source expansion | 核心已落地 | 在不削弱共享模型的前提下,扩展到更多捕获面。 |
| Remote collaboration | 核心已落地 | 通过 Share / Grant / Mount 有权限地只读访问队友 workspace 记忆。 |
| Privacy and lifecycle | 规划中 | 脱敏、保留、过期与选择性披露,避免敏感工作泄露或腐烂。 |
| Provenance and trust | 规划中 | 可溯源、可版本化、可信任评分、可清理的记忆。 |
| Semantic and multimodal | 更后期 | 在结构化证据之上叠加可选向量 / 多模态检索,而不是取代它。 |
| A2A messenger | 更后期 | 仅作为 client,把选中的 WorkRef / WorkPart 内容推送给其他 Agent。 |
| `sivtr-me` | 更后期 | 基于真实工作记录生成、可追溯证据支撑的个人 AI 时代 profile。 |

## CLI foundation

近期优先级是让命令行表面完整、可预测、可脚本化。在成为更大的个人数据层之前,`sivtr` 必须先是可靠的日常工具。

- [x] 从 pipe mode 捕获命令输出。
- [x] 用 `sivtr run` 捕获子进程输出。
- [x] 导入 shell session log。
- [x] 按 selector 复制最近命令输入、输出和命令块。
- [x] 在统一 archive 中搜索捕获输出。
- [x] 为核心行为提供 TOML 配置。
- [x] 删除过时的 Codex mirror/export 路径,让命令命名围绕统一 archive。
- [ ] 让 selector 和 filter 更容易在 shell 脚本中组合。
- [x] 强化本地 archive 的 import、export 和 search 行为。
- [x] 保持配置显式、可移植、适合安全共享。

## Agent support

Agent session 是一等 memory source。产品目标是让 Agent transcript 像普通 `sivtr` source 一样工作,而不是特殊功能。

- [x] Registry 驱动的 `AgentProvider` 表面（Codex、Claude、Cursor、OpenCode、OpenClaw、Hermes、Grok、Pi…）。
- [x] 通过共享 helpers（JSONL / SQLite）解析 provider session 记录。
- [x] 复制最新 user、assistant、tool、turn 或完整 session block。
- [x] 通过 picker 浏览本地和镜像 session 目录。
- [x] 在共享 session-provider 接口后支持扩展的 provider catalog。
- [x] 让 provider-specific parsing 与共享 selection、search、export 逻辑保持隔离。
- [x] 让 session discovery 覆盖本地、容器和共享 transcript source。
- [x] 在 CLI 命令、hotkey 和 TUI workspace 中一致暴露 provider selection。
- [x] 避免把数据模型绑定到单一 vendor 的 transcript 格式。

## Retrieval quality

检索质量决定证据范式是否真正好用。`sivtr` 应先把结构化搜索做硬,再叠加语义层。

- [x] 扩展搜索能力:明确 scope、source filter、ranking 和上下文丰富的机器可读结果。
- [x] 默认 progressive disclosure:先返回紧凑 ref,选中后再展开全文。
- [x] 改进 recency、status、provider、session、part-kind 排序,让高信号证据先出现。
- [x] 让搜索结果对脚本和 Agent 稳定:可确定排序、丰富 JSON、保留 WorkRef。
- [x] 增加评估 fixture 和 golden query,让检索改动可测量,而不只是凭感觉。
- [x] 把 semantic / vector search 当作这层基础之上的可选方法,而不是替代结构化过滤。

## Skills and playbooks

Skill 让 Agent 可以把 `sivtr` 当成共享记忆入口。它们把通用 memory 命令变成可复用流程，例如"修复最近的终端报错""从上次任务继续""按时间线总结最近工作"。

- [x] 增加初始 `skills/sivtr-memory/` 包，包含命令配方、证据纪律、工作流和示例。
- [x] 在文档中说明 skill 是产品模型的一部分，而不只是可选 prompt 片段。
- [ ] 定义社区 skill 和团队 playbook 的稳定打包约定。
- [ ] 建立 skill registry，让用户发现终端失败调试、timeline 生成、PR handoff、recap、onboarding 等 workflow。
- [ ] 增加示例，展示 Agent 如何使用 ref 和验证证据。
- [ ] 保持 skill procedure 基于现有 CLI 命令，避免社区玩法暗示还不存在的 `sivtr` 功能。

## Agent interfaces

`sivtr` 应成为其他 Agent 可直接调用的工作记忆基础设施。接口暴露的是同一套证据模型,而不是平行 API。

- [x] 覆盖 capture、search、show、filter、nav、zoom、copy 和 remote memory 的 CLI 表面。
- [x] 只读 MCP server 与 host install 流程。
- [x] 围绕 WorkRef / WorkSet 语义和 progressive disclosure 稳定 MCP tool 契约。
- [x] 提供本地 developer API,支持程序化 query、usage、stats 和 export,而不必总 shell out。
- [ ] 等 CLI / MCP 契约稳定后再发布薄 SDK 或 client library。
- [x] 所有接口都以证据为先:返回 ref、provenance 和可选中的 part,而不是不透明 blob。
- [x] 优先本地、opt-in 服务,而不是默认常开的云端 endpoint。

## TUI workspace

TUI 应保持快速和键盘优先,但需要从单个输出浏览扩展到多 source workspace 导航。

- [x] 在 Vim 风格终端 UI 中浏览捕获输出。
- [x] 搜索捕获输出。
- [x] 选择字符、行和块范围。
- [x] 交互式选择 session 和 dialogue block。
- [ ] 优化大量 session、provider 和长对话场景下的 workspace picker。
- [ ] 改进搜索 scope、结果导航和视觉反馈。
- [ ] 统一终端输出、命令块和 Agent dialogue block 的选择行为。
- [ ] 改进 markdown、tool call 和结构化 agent content 的渲染。
- [ ] 保持界面高密度、可预测、editor-friendly。

## Source expansion

更多平台应扩大捕获面,而不是切碎模型。新 source 必须映射进共享的 WorkRecord / WorkPart / WorkRef 抽象。

- [x] 在共享 provider 接口后接入更多 coding agent 和 IDE transcript。
- [ ] 在现有 hook 模型不够用时,支持更多 shell 与终端捕获路径。
- [x] 仅在存在可持久本地 export 时接入网页 AI 对话 importer（Claude.ai、ChatGPT JSON/ZIP）。
- [x] 优先 offline-first import 与本地索引,而不是去 scrape 脆弱的远程 UI。
- [x] 保持 provider adapter 很薄;search、privacy、ranking、export 继续共享。
- [x] 拒绝无法追溯到原始 session 或 artifact 的 source。

## Remote collaboration

远程协作把 local memory 模型扩展到有权限的队友记录。目标不是变成托管 transcript 服务，而是让明确授权的协作者挂载相关 workspace 记忆，使 Agent 能跨机器协作。

核心模型已落地：**Device Daemon + Identity + Share + Grant + Mount**，传输走加密 iroh。ref 使用 `origin:body`（如 `desk:terminal/...`）。

- [x] 设备级 daemon，可自动启动（`sivtr serve`）。
- [x] 显式 workspace 分享（`sivtr share` / `share add` / `invite` / `grants` / `revoke`）。
- [x] workspace 本地 mount（`sivtr remote add|list|remove|rename|test`）。
- [x] peer 身份 list/forget（`sivtr peer`）。
- [x] WorkRef 支持 remote origin（`origin:body`），可用于 search / show / filter / nav / zoom / copy。
- [x] 数据离开本机前默认脱敏。
- [x] 本机 workspace origin 标签（`sivtr ws list`）。
- [ ] Identity CLI（`identity show|rotate|export`）。
- [ ] share audit log 与按 share 的 redact 开关 CLI。
- [ ] 登录时自动启动 daemon（`serve enable|disable`）。
- [ ] peer rename / verify / disconnect 辅助命令。
- [ ] 用 UDS 或 named-pipe 替换 localhost TCP 控制面。
- [ ] 旧服务端协议版本协商。
- [x] 更细的 selective disclosure（`publish` 的 v1 支持连续整轮，`publish preview` 的 v2 支持单个本地 session 内任意原子和非连续片段；更广泛 evidence bundle 仍未完成）。

## Privacy and lifecycle

有权限分享还不够。记忆必须能安全保存、安全分享、也安全遗忘。

- [x] remote share 路径默认 secret redaction。
- [x] 在 archive 记录不暴露 secret 值的高风险发现。
- [ ] 扩展 token、key、cookie、env dump 等高风险模式的脱敏规则。
- [ ] 支持 private tag / exclude marker,让敏感片段永不进入持久索引。
- [ ] 为本地 archive 与 shared mount 提供 retention / expiry 策略。
- [ ] 支持一致的 forget / purge:删除 record、index 与 remote grant。
- [ ] 让数据生命周期动作可审计:分享了什么、脱敏了什么、保留了什么、删除了什么。
- [ ] 隐私控制保持 local-first 且显式;不做静默云端外传。

## Provenance and trust

只有能验证来源、判断是否仍然有效的证据,才真正有用。

- [x] 在每个 record、part、summary、export 上保留 source provenance。
- [x] 只保留一个当前 derived archive projection;re-sync 原子替换 session,不维护平行 memory version。
- [x] 附加 source stamp 与 sync status 等 freshness 信号。
- [ ] 支持过期与隔离陈旧或被否定的记忆,避免旧数据污染检索。
- [x] 让 search 与 profile 表面展示足够 provenance,使人或 Agent 能回到原始证据。
- [ ] 优先可引用的 WorkRef,而不是漂浮的再生 summary。

## Semantic and multimodal memory

语义与多模态检索能抬高上限,但前提是结构化证据搜索已经足够强。

- [x] 可选的本地 vector / embedding index,作为 literal 与结构化 ranking 之外的一种搜索方法。
- [x] 保持 hybrid retrieval:先结构化过滤,再语义排序。
- [x] 不强制云端 embedding provider;仅支持本地或用户自选后端。
- [ ] 仅为可被稳定 ref 寻址、之后能重新打开的多模态 artifact 建索引。
- [ ] 支持定位历史图像等非文本 artifact,同时不丢掉文本 provenance。
- [x] 永远不让 embedding 成为唯一真相;原始 record 仍是权威源。

## A2A messenger

A2A 是独立的 push-out 能力:`sivtr` 不运行 LLM。它选择结构化证据,再发给另一个 Agent。

- [ ] 仅 client 的 A2A messenger:把选中的 WorkRef / WorkPart / WorkSet 内容包装为 A2A Message / Artifact。
- [ ] 通过 HTTP + JSON-RPC 2.0 调用目标 Agent Card endpoint,不把 `sivtr` 变成 agent runtime。
- [ ] 与 remote collaboration 保持独立:share/mount 是 pull-in,A2A 是 push-out。
- [ ] 等 WorkRef 选择与隐私脱敏足够稳后再开放出站 handoff。
- [ ] 优先最小协议子集或可维护的 Rust client,而不是重型 agent 框架依赖。

## sivtr-me

当 CLI 和 workspace foundation 稳定后,更大的方向是 `sivtr-me`:从累积工作记录生成个人 profile。它不像静态简历,而是持续从真实 terminal session、Agent conversation、project history 和选中 artifact 中更新,并由证据支撑。

- [ ] 定义长期个人工作记录的本地数据模型。
- [ ] 从真实记录总结项目、工具、领域和工作方式。
- [ ] 展示代表性的 conversation、decision、code change、debug trace 和 shipped outcome。
- [ ] 构建可公开或私有的 profile,用于回答"这个人实际做过什么?"
- [ ] 支持 selective disclosure,让敏感记录保持本地,同时共享高信号 summary。
- [ ] 为每个展示 claim 保留到源 session 或 artifact 的 provenance。

## Non-goals

Roadmap 不表示 `sivtr` 会变成:

- 终端模拟器;
- 默认托管 transcript storage 服务；
- 没有明确权限的远程 chat 镜像；
- 某一个 AI assistant 的 vendor-specific wrapper;
- source control、issue tracker 或笔记工具的替代品;
- 静默改写历史的自动长期记忆压缩器;
- 完整 agent runtime 或多 agent 编排器;
- 必须依赖云端 embedding 才能工作的 cloud-first RAG 平台。

`sivtr` 应该在边缘保持小,在核心保持结构化。

## Principles

- **Capture first.** 重要工作应该在发生时记录,而不是事后凭记忆重建。
- **Local by default.** 个人 transcript 和 terminal record 应由用户控制,除非显式分享或导出。
- **Provider-neutral.** Agent support 应通过可替换 provider 和稳定共享抽象实现。
- **Evidence over paraphrase.** 优先可引用的原始 record 与 WorkRef,而不是不透明再生 summary。
- **Structured search first.** 语义与多模态检索是加成,不是对精确 filter 与 ref 的替代。
- **Skills are interfaces.** Skill 是 Agent 学会操作共享记忆层的方式；它应该精确、可验证、以证据为先。
- **Composable interfaces.** CLI、MCP、API、SDK 应暴露同一模型,并在可行时提供脚本化路径。
- **Provenance matters.** Summary、profile 和 export 应能追溯到源 session 和命令输出。
- **Privacy is a lifecycle.** 脱敏、保留、过期与 forget 是产品能力,不是事后补丁。
- **Editor-friendly.** `sivtr` 应交给已有编辑器和工作流,而不是试图拥有整个开发环境。

