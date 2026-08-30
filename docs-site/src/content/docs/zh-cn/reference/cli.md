---
title: CLI 参考
description: 命令语法、子命令、选项、provider、selector 和示例。
---

本页记录公开 CLI 表面。事实来源是 `src/cli/`；已安装版本请以 `sivtr --help` 和 `sivtr <command> --help` 为准。

## 顶层

```bash
sivtr [COMMAND]
sivtr --all              # 裸 TTY 打开时也选中 remote mount
```

不提供命令时：

- **TTY** → 多源 workspace 浏览器（Source / Sessions / Dialogues / Content）。
- **管道 stdin** → 等价 `sivtr pipe`：写入历史后用外部编辑器打开。

## run

```bash
sivtr run <COMMAND> [ARGS...]
```

运行命令，捕获合并后的 stdout/stderr，报告退出状态，在启用时保存 history，并打开捕获输出。

```bash
sivtr run cargo test
sivtr run git status --short
```

## pipe

```bash
sivtr pipe
```

读取 stdin 并打开。直接管道到 `sivtr` 等价：

```bash
cargo build 2>&1 | sivtr
```

## import

```bash
sivtr import
```

打开当前结构化 shell session log。需要 shell 集成。

## init

```bash
sivtr init <TARGET>
```

支持的 target：

| Target | 用途 |
| --- | --- |
| `powershell` | 安装 Windows PowerShell hook |
| `pwsh` | PowerShell 集成别名 |
| `bash` | 安装 Bash hook |
| `zsh` | 安装 Zsh hook |
| `nushell` / `nu` | 安装 Nushell hook |
| `tmux` | 安装 tmux picker 绑定 |
| `linux-shortcut` | 生成 Linux 桌面/终端 picker launcher |
| `macos-shortcut` | 生成 macOS Terminal/LaunchAgent picker launcher |

## copy

```bash
sivtr copy [MODE] [SELECTOR] [OPTIONS]
```

命令块 mode：

| Mode | 含义 |
| --- | --- |
| 无 mode | 复制输入加输出 |
| `in` | 复制输入 |
| `out` | 复制输出 |
| `cmd` | 复制裸命令 |

别名：

| 别名 | 展开为 |
| --- | --- |
| `sivtr c` | `sivtr copy` |
| `sivtr ci` | `sivtr copy in` |
| `sivtr co` | `sivtr copy out` |
| `sivtr cc` | `sivtr copy cmd` |

通用选项：

| 选项 | 含义 |
| --- | --- |
| `--ansi` | 有可用 ANSI 内容时复制 ANSI-decorated text |
| `--pick` | 打开交互式 picker |
| `--print` | 复制后打印文本 |
| `--regex <PATTERN>` | 只保留匹配正则的行 |
| `--lines <SPEC>` | 只保留 1-based 行选择 |

可复制输入的 mode 还支持：

| 选项 | 含义 |
| --- | --- |
| `--prompt <TEXT>` | 重写复制出来的输入 prompt |

示例：

```bash
sivtr copy
sivtr copy 3 --print
sivtr copy --prompt ":"
sivtr copy in 2..4
sivtr copy out --pick --regex panic
sivtr copy cmd --pick
```

## copy agent provider sessions

```bash
sivtr copy <PROVIDER> [MODE] [SELECTOR] [OPTIONS]
```

Provider 来自 `AgentProvider` registry（CLI 不手写列表）：

| Provider | 命令 |
| --- | --- |
| Codex | `sivtr copy codex` |
| Claude Code | `sivtr copy claude` |
| Cursor | `sivtr copy cursor` |
| OpenCode | `sivtr copy opencode` |
| OpenClaw | `sivtr copy openclaw` |
| Hermes | `sivtr copy hermes` |
| Grok | `sivtr copy grok` |
| Pi | `sivtr copy pi` |

Mode：

| Mode | 含义 |
| --- | --- |
| 无 mode | 最近完整 user + assistant turn |
| `in` | 最近用户消息 |
| `out` | 最近助手回复 |
| `tool` | 最近工具输出 |
| `all` | 完整解析会话 |

Agent copy 选项包含所有通用 copy 选项，外加：

| 选项 | 含义 |
| --- | --- |
| `--session <N|ID>` | 选择第 N 新的可选 session，或匹配 id/id 前缀 |

示例：

```bash
sivtr copy claude
sivtr copy claude out --print
sivtr copy hermes out --print
sivtr copy claude --session 2
sivtr copy codex 2..4
sivtr copy codex out --pick
sivtr copy opencode all --lines 1:20
sivtr copy pi tool --regex error
```

## diff

```bash
sivtr diff <LEFT> <RIGHT> [OPTIONS]
```

比较当前 shell session 中两个最近命令块。每个 selector 必须解析成单个块。

内容选项：

| 选项 | 含义 |
| --- | --- |
| `--output` | 比较输出文本，默认值 |
| `--block` | 比较输入加输出 |
| `--input` | 比较带 prompt 的输入 |
| `--cmd` | 比较裸命令文本 |

视图选项：

| 选项 | 含义 |
| --- | --- |
| `--side-by-side` | 显示两列文本视图 |

示例：

```bash
sivtr diff 1 2
sivtr diff 3 1 --block
sivtr diff 2 1 --side-by-side
```

## search

```bash
sivtr search <TARGET> [QUERY] [OPTIONS]
```

搜索捕获到的终端记录和受支持的 AI workspace sessions。Target 决定在哪里搜；filter 决定哪些记录匹配。位置参数 `QUERY`（plain-text，非正则）会用 BM25 相关性给整个 source 排序并成为默认排序；`--match` 可选地用正则先圈定集合。

Targets：

| Target | 含义 |
| --- | --- |
| `terminal[/<session>[/<record>[/p<part>]]]` | 终端命令记录 |
| `agent[/<session>[/<turn>[/p<part>]]]` | 所有已注册 AI / Agent 记录 |
| `codex` / `claude` / `cursor` / `opencode` / `openclaw` / `grok` / `hermes` / `pi` / `qoder` `[/<session>[/<turn>[/p<part>]]]` | 单个 provider 的记录 |
| `<origin>:<target>` | 命名 remote 或其他本机 workspace origin，例如 `desk:terminal` 或 `docs:codex/4` |

可以用 `*` 作为 path segment 通配符，例如 `terminal/*/3` 或 `pi/*/*`。origin 来自 `sivtr remote add <alias> ...`，或 `sivtr ws list` 列出的本机 workspace 名。

选项：

| 选项 | 含义 |
| --- | --- |
| `QUERY` | plain-text 搜索查询；BM25 按这些词给 source 排序（非正则）。默认排序变为 `relevance` |
| `--match <REGEX>`、`-m <REGEX>` | 大小写不敏感正则，在相关性排序前圈定集合 |
| `--exclude <REGEX>`、`-v <REGEX>` | 大小写不敏感排除过滤，在找到匹配后应用 |
| `--in <FIELD>`、`-i <FIELD>` | `content`、`title`、`session`、`input`、`output`、`command` 或 `all`；默认是 `content` |
| `--kind <KIND>` | part kind filter：`prompt`、`command`、`user`、`assistant`、`tool`、`tool_call`、`tool_result`、`skill`、`thinking`、`output` 或 `error` |
| `--status <STATUS>` | `success`、`failure` 或 `unknown` |
| `--exit-code <CODE>` | 精确终端进程退出码 |
| `--min-duration <DURATION>` | 最小命令持续时间，例如 `500ms`、`2s`、`1m` |
| `--max-duration <DURATION>` | 最大命令持续时间 |
| `--sort <SORT>` | `newest`（默认）、`relevance`（有 `QUERY` 或 `--match` 时默认）、`oldest`、`duration`、`duration-asc`、`exit-code` 或 `exit-code-asc` |
| `--cwd <PATH>` | 用于解析记录的 workspace 目录 |
| `--since <TIME>` | 只包含此时间之后或等于此时间的记录 |
| `--until <TIME>` | 只包含此时间之前或等于此时间的记录 |
| `--last <DURATION>` | 最近时间窗口，例如 `30m`、`2h`、`7d` |
| `--latest <N>` | 在最终排序前取最新 N 条匹配记录。未设 `--latest`/`--limit` 时默认 `5`（relevance 排序会排整个集合，跳过 recency window）。 |
| `-l, --limit <N>` | 最大打印结果组数（latest/sort 后的硬上限） |
| `--exclude-current`、`--other` | Agent 搜索时排除当前 agent session |
| `--json` | `--format workset` 的别名 |
| `--refs` | `--format refs` 的别名；逐行打印 refs |
| `--format <FORMAT>`、`-f <FORMAT>` | `full`、`timeline`、`compact`、`md`、`refs` 或 `workset`；terminal stdout 默认 `full`，piped stdout 默认 `workset` |

当 stdout 被管道接走且没有显式选择格式时，WorkSet 命令会输出 WorkSet JSON 给下一条命令。`--refs` 或 `-f timeline` 适合放在最后展示步骤。

时间过滤支持 RFC3339 时间戳、Unix 秒/毫秒、`30m`、`2h`、`7d` 这样的相对时间，以及 `today`、`yesterday`、`tomorrow`、`this morning`、`this afternoon`、`this evening`、`tonight`、`now` 等别名。

示例：

```bash
sivtr search terminal --status failure --latest 1 --json
sivtr s terminal "docker pull failed" --latest 20 --refs
sivtr s terminal -m "panic|failed" -v "example|sample" --since today --refs
sivtr s terminal -m "panic|failed" | sivtr filter @ -v "demo" -i title -f timeline
sivtr search agent --match "TODO|failed|next step" --since yesterday --format md
sivtr search pi --since today --sort oldest --format timeline
sivtr search pi/019e5941 --match "cargo test" --format compact
sivtr search terminal/session_13104/3 --format workset
```

## eval

```bash
sivtr eval [OPTIONS]
```

用 golden queries 基准测试检索质量：把当前 workspace records 冻结成 snapshot，再按查询对语料排序，报告 recall@k / precision@k / MRR / NDCG@k。方法与实测结果见 [`docs/retrieval-eval.md`](https://github.com/Ariestar/sivtr/blob/main/docs/retrieval-eval.md)。

| 选项 | 含义 |
| --- | --- |
| `--k <K>` | 评估深度（默认 `5`） |
| `--sort <SORT>` | 要基准测试的排序策略（默认 `newest`） |
| `--snapshot <PATH>` | 冻结的 eval snapshot 文件（queries + corpus JSON） |
| `--create-snapshot <PATH>` | 把当前 workspace records 导出为新 snapshot（queries 为空） |
| `--export <DIR>` | 往该目录写 `qrels.txt` 和 `results.txt`（trec_eval 格式） |
| `--json` | 以 JSON 输出报告 |

示例工作流：

```bash
sivtr eval --create-snapshot snap.json   # 然后编辑 snap.json：加标注查询 { name, query, relevant: [...] }
sivtr eval --snapshot snap.json --sort relevance --json
```

## filter

```bash
sivtr filter [SOURCE] [OPTIONS]
```

用统一 WorkSet filter 表面对 source 或管道传入的 WorkSet 进行过滤。如果省略 `SOURCE`，默认是 `@`，也就是从 stdin 读取 WorkSet JSON。

选项：

| 选项 | 含义 |
| --- | --- |
| `--parts` | 选择匹配的 part anchors，而不是保留输入 anchor 粒度 |
| `--match <REGEX>`、`-m <REGEX>` | 大小写不敏感内容过滤 |
| `--exclude <REGEX>`、`-v <REGEX>` | 大小写不敏感排除过滤 |
| `--in <FIELD>`、`-i <FIELD>` | `content`、`title`、`session`、`input`、`output`、`command` 或 `all` |
| `--kind <KIND>` | part kind filter：`prompt`、`command`、`user`、`assistant`、`tool`、`tool_call`、`tool_result`、`skill`、`thinking`、`output` 或 `error` |
| `--status <STATUS>` | `success`、`failure` 或 `unknown` |
| `--exit-code <CODE>` | 精确 terminal process exit code |
| `--min-duration <DURATION>` | 最小 command duration |
| `--max-duration <DURATION>` | 最大 command duration |
| `--sort <SORT>` | `newest`、`oldest`、`duration`、`duration-asc`、`exit-code` 或 `exit-code-asc` |
| `--cwd <PATH>` | 用于解析 records 的 workspace 目录 |
| `--since <TIME>` / `--until <TIME>` / `--last <DURATION>` | 时间过滤 |
| `--latest <N>` | final sort 前返回最新 N 个匹配 anchors |
| `-l, --limit <N>` | 最多打印的 result anchors 数 |
| `--exclude-current`、`--other` | Agent 搜索中排除当前 session |
| `--json` | `--format workset` 别名 |
| `--refs` | `--format refs` 别名 |
| `--format <FORMAT>`、`-f <FORMAT>` | `full`、`timeline`、`compact`、`md`、`refs` 或 `workset` |
| `--save <NAME>` | 把结果 WorkSet 保存为 `@name` |

示例：

```bash
sivtr search terminal --json | sivtr filter @ -m error --refs
sivtr filter terminal --status failure --refs
sivtr filter @last --parts --kind tool_result --refs
```

## var

```bash
sivtr var <COMMAND>
```

管理命名 WorkSet 变量。

| Command | 含义 |
| --- | --- |
| `set <name> [source]` | 把 source 或管道 WorkSet 保存为 `@name` |
| `list` | 列出已保存变量、item 数和创建时间 |
| `rm <name>` | 删除一个已保存变量 |
| `merge <name> <source>...` | 把 sources 合并进已保存变量，并按 anchor 去重 |
| `drop <name> <source>...` | 从已保存变量中移除 source anchors |
| `cleanup` | 删除所有已保存变量 |

示例：

```bash
sivtr var set ctx @last
sivtr filter terminal -m panic --json | sivtr var set failures
sivtr var list
sivtr var merge ctx @failures @last[1]
sivtr var drop ctx @noise
```

## nav

```bash
sivtr nav <SOURCE> <MOTION> [OPTIONS]
```

在 record / part / session 结构中确定性移动 WorkSet anchors。`nav` 不会默认展开 child；移动到 child 必须用 `>N` 明确指定 1-based index。

Motion token 从左到右组合：

| Token | 含义 |
| --- | --- |
| `<` | 父级。part 到 record；record 到所属 session records。 |
| `>N` | 第 N 个 child，1-based。record 的 children 是 parts。 |
| `+N` | 当前层级向后移动 N 个 sibling。 |
| `-N` | 当前层级向前移动 N 个 sibling。 |
| `[A..B]` | 当前层级相对 sibling window。 |
| `~` | 所属 session records。 |

选项：

| 选项 | 含义 |
| --- | --- |
| `--cwd <PATH>` | 用于解析 records 的 workspace 目录 |
| `--json` | `--format workset` 别名 |
| `--refs` | `--format refs` 别名 |
| `--format <FORMAT>`、`-f <FORMAT>` | `full`、`timeline`、`compact`、`md`、`refs` 或 `workset` |

示例：

```bash
sivtr nav @hit '<' --refs
sivtr nav @hit '>1' --refs
sivtr nav @hit '<+1>1' --refs
sivtr nav @hit '<[-2..+2]' --refs
sivtr nav @hit '~' --refs
```

只想围绕命中补 record 上下文时用 `zoom`；需要精确移动路径时用 `nav`。

## show

```bash
sivtr show <SOURCE> [OPTIONS]
```

打印 workspace ref 或 WorkSet source，例如 `@last`、`@name` 或 `@`。

Ref 语法：

```text
source/session[/record-or-turn[/p<part>]]
```

选项：

| 选项 | 含义 |
| --- | --- |
| `--cwd <PATH>` | 用于解析 session 的工作区目录 |
| `--json` | `--format workset` 别名 |
| `--refs` | `--format refs` 别名 |
| `--full` | `--format full` 别名 |
| `--format <FORMAT>`、`-f <FORMAT>` | `full`、`timeline`、`compact`、`md`、`refs` 或 `workset` |

示例：

```bash
sivtr show claude/<session-id>
sivtr show claude/<session-id>/3
sivtr show claude/<session-id>/3/p7 --json
sivtr show terminal/current/2
sivtr show desk:terminal/session_42/3/p1 --full
sivtr show @last --full
sivtr show @ctx -f timeline
```

## publish

`publish` 把本地 WorkSet 投影成不可变、端侧加密的浏览器只读快照。它与 `share` 不同：`share` 是需要 Sivtr/daemon 的实时 workspace mount；`publish` 上传的只有密文，查看者无需登录或安装 Sivtr，分享者设备离线也能查看。

```bash
sivtr publish <SOURCE> [--title <TITLE>] [--expires 7d] [--yes] [--allow-warnings]
sivtr publish preview [<SOURCE>] [--title <TITLE>] [--expires 7d] [--format human|json]
sivtr publish list [--json]
sivtr publish link [<PUBLICATION_ID>]
sivtr publish revoke [<PUBLICATION_ID>] [--yes]
```

v1 只接受同一 provider、同一 session 中连续的本地 Agent record，并只发布 User/Assistant 文本。`publish preview` 不带 source 时会打开现有 workspace picker，返回 WorkSet 后在本地打印最终快照。v2 支持 User、Assistant、Tool、Skill、Thinking 原子以及不连续片段；ToolCall 与 ToolResult 不可拆开。Terminal、remote/group、附件和跨 session 证据包仍会被拒绝。公开快照不包含 WorkSet、WorkRef、`cwd`、session path 或本地 provider 原始事件。

`preview` 完全离线生成最终快照和风险报告；已识别的 token、私钥、Bearer 和 secret assignment 自动替换为 `[REDACTED]`，绝对路径、邮箱和内网地址只警告。发布前会显示轮次数、大小、脱敏项和期限；非交互环境发布必须使用 `--yes`，存在未自动处理的风险时还必须使用 `--allow-warnings`。`[publish].endpoint` 默认是 `https://share.hnnulwh.cn`，可在 `config.toml` 里改成自建或其他兼容服务。成功创建时 stdout 只输出完整链接，说明和警告写 stderr，方便复制。

密钥只放在 URL fragment（`#k=...`），托管服务只保存 AES-256-GCM 密文、管理 token 哈希、期限和 envelope 版本。链接默认 7 天，可选 `2h/1d/3d/7d/30d`，不提供永久链接；修改内容必须创建新链接。链接持有者均可查看，管理 token 只保存在本机的独立 `publication-state.db` 中。

典型流程：

```bash
sivtr search codex/<session-id> --save share_ready --refs
sivtr publish preview share_ready
sivtr publish share_ready --expires 7d
```

交互式预览流程：

```powershell
sivtr publish preview
sivtr publish preview share_ready --format human
sivtr publish share_ready --expires 7d --yes
```

`publish <SOURCE>` 也接受不带 `@` 的已保存 WorkSet 名称，因此 PowerShell 中不需要为这个发布流程写引号。裸 `terminal`、`agent`、provider 名称以及包含 `/`、`:` 或 `*` 的 source 会按 selector 解析；其他裸标识符只按已保存 WorkSet 名称解析，缺失时直接报错，不会 fallback 到另一种 source。workspace picker 中按 `p` 打开有效期浮板，`Tab` 跳到可选的 WorkSet 名称输入；输入非空名称后会先保存一次，再继续发布或预览。交互式终端中的 `list` 会显示可点击的 active 链接；`link` 和 `revoke` 省略 ID 时会打开选择器。

## zoom

```bash
sivtr zoom [SOURCE=@last] [OPTIONS]
```

把每个目标 WorkRecord 用同一 session 的相邻 records 展开。未给 source 时默认 `@last`。

| 选项 | 含义 |
| --- | --- |
| `-C, --context <N>` | 每个 anchor 前后各取 N 条（默认 `1`） |
| `--before <N>` | 每个 anchor 前取 N 条 |
| `--after <N>` | 每个 anchor 后取 N 条 |
| `--cwd <PATH>` | 用于解析 sessions 的 workspace 目录 |
| `--json` | `--format workset` 别名 |
| `--refs` | `--format refs` 别名 |
| `-f, --format <FORMAT>` | `full`、`timeline`、`compact`、`md`、`refs`、`workset` |
| `--save <NAME>` | 把展开结果保存为命名 WorkSet 变量 |

```bash
sivtr zoom                        # 展开 @last
sivtr zoom claude/<session>/3 -C 2
sivtr zoom @failures --before 2 --after 0 --refs
```

## work

```bash
sivtr work <COMMAND>
```

遍历 workspace 的 sessions、records、parts，只输出 marker 级信息（不打印全文），可管道给其他命令。

| 命令 | 含义 |
| --- | --- |
| `sessions [SOURCE]` | 列出当前 workspace 的 terminal 与 agent sessions |
| `records <SOURCE>` | 把 sessions 或已保存变量转成事件级 refs |
| `parts <SOURCE>` | 从匹配事件里抽出真正有用的输入/输出片段 |

常用 flag：`--provider <NAME>` 过滤、`--cwd <PATH>`、`--json`、`--refs`、`--save <NAME>`。

```bash
sivtr work sessions
sivtr work records codex/ --refs
sivtr work parts @last --kind output --save out_parts
```

## serve

```bash
sivtr serve <COMMAND>
```

管理本机 remote-memory daemon。share / remote 命令需要时会自动启动。

| 命令 | 含义 |
| --- | --- |
| `start` | 后台启动 daemon |
| `stop` | 干净停止正在运行的 daemon |
| `restart` | 重启 daemon |
| `status` | 显示 daemon 身份和运行状态 |
| `logs` | 打印 daemon 日志路径 |
| `foreground` | 前台运行 daemon |

```bash
sivtr serve start
sivtr serve status
sivtr serve logs
sivtr serve stop
```

## share

```bash
sivtr share [OPTIONS]
sivtr share <COMMAND>
```

显式分享本机 workspace 给远端。裸 `sivtr share` 是交互入口：选择 workspace（Enter = 当前）并确保 share 存在（不出 invite）。出 invite 请用 `sivtr share invite <name>`。

默认交互选项：

| 选项 | 含义 |
| --- | --- |
| `--path <PATH>` | workspace 路径；确认后跳过选择器 |
| `--name <NAME>` | 稳定 share 名；默认取 workspace 目录名 |
| `--no-redact` | 关闭此 share 的密钥脱敏 |

子命令：

| 命令 | 含义 |
| --- | --- |
| `add [PATH] [--name NAME] [--no-redact]` | 通过 daemon 暴露一个 workspace |
| `list` | 列出本机 shares |
| `remove <SHARE>` | 删除 share 及其 grants 和 invitations |
| `enable <SHARE>` / `disable <SHARE>` | 启用/禁用 share，不删除 |
| `invite <SHARE> [--expires DURATION]` | 签发单次 invite；bare key 打印到 stdout |
| `grants <SHARE>` | 列出 share 的活跃 peer grants |
| `revoke <SHARE> <PEER>` | 撤销某 peer 对该 share 的访问 |

```bash
sivtr share
sivtr share add --name alice-desk
sivtr share invite alice-desk --expires 10m
sivtr share list
sivtr share grants alice-desk
sivtr share revoke alice-desk <peer>
```

## remote

```bash
sivtr remote <COMMAND>
```

在当前 git workspace 里给远端 share 起名（类似 `git remote`）。该名是 `name:path` refs 的左侧。

| 命令 | 含义 |
| --- | --- |
| `list` | 列出当前 workspace 的 remotes |
| `add <NAME> <INVITE>` | 兑换 invite 并添加 remote |
| `remove <NAME>` | 删除本地 remote 名（grant 仍在，需所有者 revoke） |
| `rename <NAME> <NEW>` | 重命名本 workspace 的 remote |
| `test <NAME>` | 可达性 + 授权探测 |

```bash
sivtr remote add desk <invite-key>
sivtr remote test desk
sivtr remote list
sivtr s desk:terminal --status failure --latest 5 --refs
sivtr show desk:agent/<session>/3 --full
sivtr remote rename desk bob-desk
sivtr remote remove desk
```

## peer

```bash
sivtr peer <COMMAND>
```

| 命令 | 含义 |
| --- | --- |
| `list` | 列出已知 peer 身份 |
| `forget <PEER>` | 忘记 peer，并删除所有涉及它的本地 remotes 和 grants |

```bash
sivtr peer list
sivtr peer forget <peer>
```

## group

```bash
sivtr group <COMMAND>
```

群组是互相共享记忆的一组设备：每个成员贡献 workspace，成员变更自动同步。

| 命令 | 含义 |
| --- | --- |
| `create <NAME> [--workspace PATH] [--share-name NAME]` | 建组并在同一事务里贡献当前 workspace |
| `invite <GROUP> [--expires DURATION] [--max-uses N]` | 仅 owner；签发多设备 join 链接（stdout = bare key） |
| `join <INVITE> [--workspace PATH] [--share-name NAME] [--no-redact]` | 兑换邀请并贡献 workspace；重跑可调整贡献 |
| `list` | 列出你所属的组 |
| `members <GROUP>` | 列出成员及其贡献 |
| `remove <GROUP> <PEER>` | 仅 owner；移除成员 |
| `rename <GROUP> <NAME>` | 仅 owner；给组改名 |
| `leave <GROUP>` | 退出组（owner 退出会解散整组） |
| `sync <GROUP>` | 强制从 owner 拉一次成员清单 |

```bash
sivtr group create team
sivtr group invite team --expires 1d --max-uses 10
sivtr group join <invite-key>
sivtr group list
sivtr group members team
sivtr group sync team
```

成员变更会自动广播给每个成员（并每 5 分钟 TTL 重新拉取）。成员对他人贡献的访问是只读的，默认脱敏密钥，与 share 一致。

## origin

```bash
sivtr origin <COMMAND>
```

所有可寻址来源的统一改名入口——一个名字既代表本地 workspace 别名，也代表远端 mount，通过 origin registry 解析。

| 命令 | 含义 |
| --- | --- |
| `rename <NAME> <NEW_NAME>` | 改本地 workspace 别名或远端 mount 的名字；两种都走同一条命令 |

```bash
sivtr origin rename docs knowledge
sivtr origin rename desk alice-desk
```

## workspace

```bash
sivtr workspace [list]
sivtr ws list
```

列出已知本机 workspaces 及其 origin 标签，用于 `name:body` refs（例如 `docs:codex/4`）。别名：`sivtr ws`。

```bash
sivtr ws list
```

各 remote 子命令的精确语法见上。模型、设置路径和安全默认见 [远程访问](/zh-cn/usage/remote-access/)。协作场景见 [远程协作记忆](/zh-cn/playbooks/remote-collaboration-memory/)。

## mcp

```bash
sivtr mcp serve
sivtr mcp install [OPTIONS]
sivtr mcp uninstall [OPTIONS]
sivtr mcp print-config <claude|cursor|codex>
```

面向 agent 宿主的只读 MCP server，以及一键写入宿主配置。

### serve

在 stdio 上运行 MCP server：

```bash
sivtr mcp serve
sivtr mcp serve --idle-exit 60
```

`--idle-exit <SECS>` 让 server 在连续多少秒没有 tool call 后退出；host 会在下次使用工具时重新拉起，空闲 server 因此不会常驻（否则每个 agent session 会各保留一个直到退出）。`0` = 直到 host 关闭 stdin 才退出。也可以用全局配置键 `[mcp] idle_exit_secs` 设置同样的值，默认 `60`（默认开启；设为 `0` 关闭）；CLI flag 优先于配置。

工具：

| 工具 | 用途 |
| --- | --- |
| `sivtr_search` | 搜索 terminal/agent 记忆；支持 `desk:...` origin。与 CLI search 相同边界（默认 `latest=5`）。 |
| `sivtr_show` | 展开 ref 或 WorkSet handle |
| `sivtr_zoom` | 邻近 record 上下文 |
| `sivtr_filter` | 缩小 `@last` / `@name` / source |
| `sivtr_status` | 版本、hooks、providers、daemon、`ws` 本机 origin、remotes、vars |

### install / uninstall

把 sivtr MCP 写入或移出 agent 宿主配置（类似 `codegraph install`）：

```bash
sivtr mcp install -y                      # 检测已安装宿主，global
sivtr mcp install -p claude,cursor -l global
sivtr mcp install -p claude -l local      # 项目 .mcp.json
sivtr mcp uninstall -p all -y
```

| 选项 | 含义 |
| --- | --- |
| `-p, --provider` | Provider 宿主：`claude`、`cursor`、`codex`、`opencode`、`openclaw`、`grok`、`hermes`、`pi`、`qoder`、`qodercn`、`gemini`、`qwen`、`goose` 或 `all`。省略则自动检测已安装宿主。 |
| `-l, --location` | `global`（默认）或 `local` |
| `-y, --yes` | 非交互 |

安装位置（registry 驱动；路径为主机默认）：

| 目标 | Global 路径 |
| --- | --- |
| Claude Code | `~/.claude.json` → `mcpServers.sivtr` |
| Cursor | `~/.cursor/mcp.json` → `mcpServers.sivtr` |
| Codex | `~/.codex/config.toml` → `[mcp_servers.sivtr]` |
| OpenCode | OpenCode MCP config → `mcp.sivtr` |
| OpenClaw | OpenClaw config → `mcp.servers.sivtr` |
| Grok | Grok config TOML → MCP entry |
| Hermes | Hermes YAML → `mcp_servers.sivtr` |
| Pi | Pi config → `mcpServers.sivtr` |
| Qoder / Qoder-CN | Qoder settings.json → MCP entry |
| Gemini | Gemini settings.json → `mcpServers.sivtr` |
| Qwen | Qwen settings.json → MCP entry |
| Goose | Goose config.yaml extensions → MCP entry |

注册命令始终为：

```text
sivtr mcp serve
```

### print-config

只打印配置片段，不写文件：

```bash
sivtr mcp print-config claude
sivtr mcp print-config cursor
sivtr mcp print-config codex
sivtr mcp print-config grok
sivtr mcp print-config goose
```

MCP 不是完整 CLI 镜像。交互、写入和捕获命令仍走 CLI。策略仍在 `sivtr-memory` skill。

## version

```bash
sivtr version [--verbose]
```

打印 Sivtr 版本。使用 `--verbose` 诊断当前运行的是哪个 binary，以及它是否和当前仓库里的本地 debug build 不同。

```bash
sivtr version
sivtr version --verbose
```

Verbose 输出包含：

- package version；
- binary 路径；
- 当前工作目录；
- debug/release profile；
- 可用时的 git commit 和 build time；
- 检测到的 repo root；
- 本地 `target/debug/sivtr` binary 状态；
- 在 repo 内运行不同的全局 binary 时给出 warning。

## doctor

```bash
sivtr doctor [--fix] [--json]
```

诊断安装与环境：binary、config、session logs、shell hooks、provider session 数量、clipboard 与可用更新。

| 选项 | 含义 |
| --- | --- |
| `--fix` | 尝试自动修复检测到的问题 |
| `--json` | 机器可读 JSON 输出 |

```bash
sivtr doctor
sivtr doctor --fix
sivtr doctor --json
```

## update

```bash
sivtr update
```

自更新到最新 GitHub release：下载匹配平台的资产，校验 SHA256 后原地替换 binary。

```bash
sivtr update
```

## setup

```bash
sivtr setup
```

新装一键配置：检测环境、安装 shell hooks、给检测到的 agent 宿主注册 MCP server、缺失时安装 `sivtr-memory` skill，并跑一次 smoke test。等价于分步执行 `sivtr init`、`sivtr mcp install`、skill 安装与 `sivtr doctor`。

```bash
sivtr setup
```

## history

```bash
sivtr history [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `list [-l, --limit <N>]` | 列出最近条目 |
| `search <KEYWORD> [-l, --limit <N>]` | 搜索保存的捕获 history |
| `show <ID>` | 展示指定 history 条目 |

不提供 history 子命令时，默认使用 `list`。

## config

```bash
sivtr config [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `show` | 显示配置路径和内容 |
| `init` | 创建默认配置 |
| `edit` | 在编辑器中打开配置 |

不提供 config 子命令时，默认使用 `show`。

## hotkey

```bash
sivtr hotkey [COMMAND]
```

子命令：

| 命令 | 含义 |
| --- | --- |
| `start [--chord <CHORD>] [--provider <PROVIDER>]` | 启动 Windows 全局热键 daemon |
| `status` | 显示 daemon 状态 |
| `stop` | 停止 daemon |

不提供 hotkey 子命令时，默认使用 `status`。

示例：

```bash
sivtr hotkey start
sivtr hotkey start --chord alt+y
sivtr hotkey start --provider claude
sivtr hotkey status
sivtr hotkey stop
```

## web

```bash
sivtr web [--port <PORT>] [--host <HOST>]
```

在统一 archive 之上提供本地 web UI 和只读 JSON API：session 浏览器（按 provider 过滤、打开 session、复制 refs）和按 BM25 相关性排序的全文搜索。

| 选项 | 含义 |
| --- | --- |
| `--port <PORT>` | 要绑定的 TCP 端口（默认 `8080`） |
| `--host <HOST>` | 绑定地址（默认 `127.0.0.1`，仅 loopback） |

server 会校验浏览器 `Host` header 以防范 DNS rebinding。它是只读的——不做任何写入——数据也不会离开本机。详见 [Web UI](/zh-cn/usage/web-ui/)。

```bash
sivtr web
sivtr web --port 8081
```

## sync

```bash
sivtr sync [--full] [--json]
```

把所有 Agent provider 的 session 和每个 workspace 的终端日志同步进统一的本地 archive（`archive.db`）。查询（search、show、copy、TUI、MCP）从这个 archive 读取；当 archive 比 `[sync].max_age_secs`（默认 `15` 秒）更旧时，查询会先触发一次增量同步。

选项：

| 选项 | 含义 |
| --- | --- |
| `--full` | 重新解析所有 session，忽略缓存的 stat stamp |
| `--json` | 输出机器可读报告，包含每个 source 的计数（added / updated / unchanged / failed） |

默认增量同步：用 stat stamp 把每个源文件与 archive 比对，只重新解析有变化的文件。失败按 source 单独上报，一个 provider 出错不会掩盖其余结果。

示例：

```bash
sivtr sync
sivtr sync --full
sivtr sync --json
```

## clear

```bash
sivtr clear [--all]
```

清理当前 shell session log。`--all` 会清理由 `sivtr` 管理的所有记录 session log 和 state 文件。

## 共享语法

Recency selector、`--session`、provider、`--regex`、`--lines`、`--ansi`、`--print` 和 workspace ref 见 [Selector 和 Filter](/zh-cn/reference/selectors-and-filters/)。

