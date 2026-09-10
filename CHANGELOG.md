# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0](https://github.com/Ariestar/sivtr/compare/v0.7.1...v0.8.0) - 2026-09-10

### Added

- *(diagnostics)* unified warning sink with TUI `!` overlay ([#281](https://github.com/Ariestar/sivtr/pull/281))
- *(archive)* [**breaking**] unify terminal and agent memory ([#276](https://github.com/Ariestar/sivtr/pull/276))
- *(web)* add local web UI and JSON API over the archive ([#275](https://github.com/Ariestar/sivtr/pull/275))
- *(archive)* unify sessions into one local store ([#274](https://github.com/Ariestar/sivtr/pull/274))

### Fixed

- *(deps)* update astro monorepo ([#291](https://github.com/Ariestar/sivtr/pull/291))
- *(deps)* update rust crate dirs to v7 ([#268](https://github.com/Ariestar/sivtr/pull/268))
- *(deps)* update dependency @astrojs/mdx to v8 ([#248](https://github.com/Ariestar/sivtr/pull/248))
- *(agents)* filter injected user envelopes at the shared block funnel ([#283](https://github.com/Ariestar/sivtr/pull/283))
- *(deps)* update dependency astro to v7.3.1 ([#262](https://github.com/Ariestar/sivtr/pull/262))
- *(deps)* update dependency @astrojs/starlight to v0.42.0 ([#259](https://github.com/Ariestar/sivtr/pull/259))
- *(deps)* update dependency @astrojs/starlight to v0.41.11 ([#253](https://github.com/Ariestar/sivtr/pull/253))
- *(deps)* update dependency astro to v7.2.10 ([#247](https://github.com/Ariestar/sivtr/pull/247))
- *(archive)* single-flight freshness gate + fail-open readers ([#280](https://github.com/Ariestar/sivtr/pull/280))

### Other

- *(deps)* update dependency @cloudflare/workers-types to v5.20260910.1 ([#299](https://github.com/Ariestar/sivtr/pull/299))
- *(deps)* update rust crate uuid to v1.26.1 ([#298](https://github.com/Ariestar/sivtr/pull/298))
- *(deps)* update dependency @types/node to v26.5.1 ([#297](https://github.com/Ariestar/sivtr/pull/297))
- *(deps)* update dependency @types/node to v24.13.4 ([#296](https://github.com/Ariestar/sivtr/pull/296))
- *(deps)* update release-plz/action action to v0.5.135 ([#295](https://github.com/Ariestar/sivtr/pull/295))
- *(deps)* update release-plz/action action to v0.5.134 ([#293](https://github.com/Ariestar/sivtr/pull/293))
- *(deps)* update dependency wrangler to v4.130.0 ([#292](https://github.com/Ariestar/sivtr/pull/292))
- *(deps)* update dependency vitest to v5 ([#264](https://github.com/Ariestar/sivtr/pull/264))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260908.1 ([#289](https://github.com/Ariestar/sivtr/pull/289))
- *(tui)* unify content colors by part kind ([#282](https://github.com/Ariestar/sivtr/pull/282))
- *(record)* derive kind from WorkRef, delete denormalized kind+source ([#279](https://github.com/Ariestar/sivtr/pull/279))
- [**breaking**] replace WorkPartData with two-body WorkPart model ([#278](https://github.com/Ariestar/sivtr/pull/278))
- [**breaking**] replace WorkPartData with two-body WorkPart model ([#284](https://github.com/Ariestar/sivtr/pull/284))
- *(deps)* update dependency wrangler to v4.129.1 ([#287](https://github.com/Ariestar/sivtr/pull/287))
- *(deps)* update dependency @types/node to v26.5.0 ([#288](https://github.com/Ariestar/sivtr/pull/288))
- *(deps)* update dependency marked to v18.0.12 ([#286](https://github.com/Ariestar/sivtr/pull/286))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260907.1 ([#273](https://github.com/Ariestar/sivtr/pull/273))
- *(deps)* update rust crate ureq to v3.4.1 ([#272](https://github.com/Ariestar/sivtr/pull/272))
- *(deps)* update dependency dompurify to v3.4.15 ([#271](https://github.com/Ariestar/sivtr/pull/271))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260906.1 ([#270](https://github.com/Ariestar/sivtr/pull/270))
- *(deps)* update release-plz/action action to v0.5.133 ([#269](https://github.com/Ariestar/sivtr/pull/269))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260905.1 ([#267](https://github.com/Ariestar/sivtr/pull/267))
- *(deps)* update dependency @playwright/test to v1.63.0 ([#266](https://github.com/Ariestar/sivtr/pull/266))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260904.1 ([#265](https://github.com/Ariestar/sivtr/pull/265))
- *(deps)* update dependency wrangler to v4.129.0 ([#263](https://github.com/Ariestar/sivtr/pull/263))
- *(deps)* update release-plz/action action to v0.5.132 ([#261](https://github.com/Ariestar/sivtr/pull/261))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260903.1 ([#260](https://github.com/Ariestar/sivtr/pull/260))
- *(deps)* update dependency @types/vscode to v1.136.0 ([#258](https://github.com/Ariestar/sivtr/pull/258))
- *(deps)* update rust crate toml to v1.1.5 ([#257](https://github.com/Ariestar/sivtr/pull/257))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260902.1 ([#256](https://github.com/Ariestar/sivtr/pull/256))
- *(deps)* update dependency @types/node to v26.4.1 ([#255](https://github.com/Ariestar/sivtr/pull/255))
- *(deps)* update dependency wrangler to v4.128.0 ([#254](https://github.com/Ariestar/sivtr/pull/254))
- *(deps)* update rust crate rmcp to v3.2.0 ([#251](https://github.com/Ariestar/sivtr/pull/251))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260901.1 ([#250](https://github.com/Ariestar/sivtr/pull/250))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260831.1 ([#246](https://github.com/Ariestar/sivtr/pull/246))

## [0.7.1](https://github.com/Ariestar/sivtr/compare/v0.7.0...v0.7.1) - 2026-08-30

### Fixed

- *(update)* allow larger release archives ([#240](https://github.com/Ariestar/sivtr/pull/240))

### Other

- *(deps)* update dependency @cloudflare/workers-types to v5.20260830.1 ([#242](https://github.com/Ariestar/sivtr/pull/242))

## [0.7.0](https://github.com/Ariestar/sivtr/compare/v0.6.0...v0.7.0) - 2026-08-29

### Added

- *(browse)* 发布选择支持有效期浮板与直链生成 ([#207](https://github.com/Ariestar/sivtr/pull/207))
- *(publish)* [**breaking**] 重提加密浏览器发布与有效期后端支持 ([#205](https://github.com/Ariestar/sivtr/pull/205))
- *(browse)* walk cursor across dialogues ([#213](https://github.com/Ariestar/sivtr/pull/213))
- *(agents)* add ZCode agent provider ([#191](https://github.com/Ariestar/sivtr/pull/191))

### Fixed

- *(publish)* resolve review follow-ups ([#238](https://github.com/Ariestar/sivtr/pull/238))
- *(deps)* update dependency marked to v18 ([#239](https://github.com/Ariestar/sivtr/pull/239))
- *(deps)* update rust crate aes-gcm to 0.11 ([#232](https://github.com/Ariestar/sivtr/pull/232))
- *(deps)* update dependency @astrojs/starlight to v0.41.10 ([#234](https://github.com/Ariestar/sivtr/pull/234))
- *(deps)* update dependency @astrojs/starlight to v0.41.10 ([#227](https://github.com/Ariestar/sivtr/pull/227))
- *(deps)* update dependency astro to v7.2.9 ([#225](https://github.com/Ariestar/sivtr/pull/225))
- *(deps)* update dependency astro to v7.2.8 ([#216](https://github.com/Ariestar/sivtr/pull/216))
- *(deps)* update dependency astro to v7.2.7 ([#202](https://github.com/Ariestar/sivtr/pull/202))
- *(deps)* update dependency @astrojs/starlight to v0.41.9 ([#200](https://github.com/Ariestar/sivtr/pull/200))
- *(deps)* update dependency @astrojs/starlight to v0.41.8 ([#197](https://github.com/Ariestar/sivtr/pull/197))
- *(update)* fetch latest release via redirect ([#195](https://github.com/Ariestar/sivtr/pull/195))
- *(deps)* update astro monorepo ([#196](https://github.com/Ariestar/sivtr/pull/196))
- *(search)* keep metadata-only records in browse queries ([#190](https://github.com/Ariestar/sivtr/pull/190))

### Other

- *(deps)* update dependency vitest to v4 ([#237](https://github.com/Ariestar/sivtr/pull/237))
- *(deps)* update dependency vite to v8 ([#236](https://github.com/Ariestar/sivtr/pull/236))
- *(deps)* update dependency typescript to v7 ([#235](https://github.com/Ariestar/sivtr/pull/235))
- *(deps)* update actions/setup-node action to v7 ([#233](https://github.com/Ariestar/sivtr/pull/233))
- *(deps)* update dependency wrangler to v4.127.1 ([#229](https://github.com/Ariestar/sivtr/pull/229))
- *(deps)* update dependency @cloudflare/workers-types to v5.20260829.1 ([#228](https://github.com/Ariestar/sivtr/pull/228))
- *(workset)* validate selection pipeline ([#224](https://github.com/Ariestar/sivtr/pull/224))
- *(browse)* finalize pane state ([#223](https://github.com/Ariestar/sivtr/pull/223))
- *(browse)* unify selection projections ([#222](https://github.com/Ariestar/sivtr/pull/222))
- *(browse)* integrate workset selection ([#214](https://github.com/Ariestar/sivtr/pull/214))
- *(workset)* preserve anchor granularity ([#212](https://github.com/Ariestar/sivtr/pull/212))
- *(copy)* resolve copied dialogues once ([#211](https://github.com/Ariestar/sivtr/pull/211))
- *(browse)* centralize pane state and navigation ([#210](https://github.com/Ariestar/sivtr/pull/210))
- *(browse)* unify selection semantics ([#209](https://github.com/Ariestar/sivtr/pull/209))
- *(browse)* unify content block coordinates ([#208](https://github.com/Ariestar/sivtr/pull/208))
- *(deps)* update rust crate flate2 to v1.1.10 ([#226](https://github.com/Ariestar/sivtr/pull/226))
- *(deps)* update rust crate uuid to v1.26.0 ([#218](https://github.com/Ariestar/sivtr/pull/218))
- *(deps)* update dependency @types/node to v26.4.0 ([#217](https://github.com/Ariestar/sivtr/pull/217))
- *(deps)* update rust crate iroh to v1.1.0 ([#201](https://github.com/Ariestar/sivtr/pull/201))
- *(deps)* update dependency @types/node to v26.3.0 ([#198](https://github.com/Ariestar/sivtr/pull/198))
- *(deps)* update rust crate uuid to v1.25.0 ([#193](https://github.com/Ariestar/sivtr/pull/193))
- *(search)* unify query routing into a single pipeline ([#189](https://github.com/Ariestar/sivtr/pull/189))
- document single-track release cadence (PATCH prompt, MINOR batched) ([#188](https://github.com/Ariestar/sivtr/pull/188))

## [0.6.0](https://github.com/Ariestar/sivtr/compare/v0.5.1...v0.6.0) - 2026-08-20

### Added

- *(search)* unify terminal and agent sources behind a SessionSource trait ([#180](https://github.com/Ariestar/sivtr/pull/180))
- *(search)* incremental listing cache and light/full session views ([#179](https://github.com/Ariestar/sivtr/pull/179))
- *(search)* weight agent dialogue content over tool output ([#175](https://github.com/Ariestar/sivtr/pull/175))
- *(search)* add all: scope for unified cross-origin retrieval ([#174](https://github.com/Ariestar/sivtr/pull/174))

### Fixed

- *(search)* cap all searches at 5 results by default ([#170](https://github.com/Ariestar/sivtr/pull/170))
- *(release)* trigger release.yml on tag push and gate version bumps ([#169](https://github.com/Ariestar/sivtr/pull/169))
- *(deps)* update astro monorepo ([#168](https://github.com/Ariestar/sivtr/pull/168))
- *(deps)* update astro monorepo ([#171](https://github.com/Ariestar/sivtr/pull/171))

### Other

- *(deps)* update rust crate rmcp to v3.1.4 ([#173](https://github.com/Ariestar/sivtr/pull/173))
- *(deps)* update dependency @types/vscode to v1.134.0 ([#172](https://github.com/Ariestar/sivtr/pull/172))
- *(search)* reuse cached BM25 indexes across processes ([#178](https://github.com/Ariestar/sivtr/pull/178))
- codify per-change branch and commit workflow ([#177](https://github.com/Ariestar/sivtr/pull/177))
- gitignore agent eval snapshots ([#176](https://github.com/Ariestar/sivtr/pull/176))
- sync skills provider list and release token note ([#167](https://github.com/Ariestar/sivtr/pull/167))
- *(zh-cn)* mirror reference and usage alignment ([#166](https://github.com/Ariestar/sivtr/pull/166))
- *(usage)* align usage guides with v0.5 features ([#165](https://github.com/Ariestar/sivtr/pull/165))
- *(reference)* align CLI, config, and keybindings with v0.5 ([#164](https://github.com/Ariestar/sivtr/pull/164))
- *(readme)* align README with v0.5 features ([#163](https://github.com/Ariestar/sivtr/pull/163))
- *(browse)* lay out content from blocks once ([#152](https://github.com/Ariestar/sivtr/pull/152))
- *(browse)* fetch session meta at ceiling once ([#151](https://github.com/Ariestar/sivtr/pull/151))
- *(browse)* cap concurrent session-body parses ([#150](https://github.com/Ariestar/sivtr/pull/150))
- *(browse)* drop session bodies that arrive after scroll-away ([#149](https://github.com/Ariestar/sivtr/pull/149))
- *(deps)* bump h2 to 0.4.16 ([#161](https://github.com/Ariestar/sivtr/pull/161))
- *(release)* keep GITHUB_TOKEN for git author alongside release PAT ([#160](https://github.com/Ariestar/sivtr/pull/160))

## [0.5.1](https://github.com/Ariestar/sivtr/compare/v0.5.0...v0.5.1) - 2026-08-18

### Other

- *(deps)* update actions/checkout action to v7 ([#155](https://github.com/Ariestar/sivtr/pull/155))
- *(release)* push release tag with a PAT so release.yml triggers ([#158](https://github.com/Ariestar/sivtr/pull/158))

## [0.5.0](https://github.com/Ariestar/sivtr/compare/v0.4.1...v0.5.0) - 2026-08-18

### Added

- *(tui)* unify selection highlight across panes
- *(tui)* page selected dialogues in content with J/K
- *(tui)* align edit previews with a real line diff
- *(tui)* GitHub-style code gutters and structured read/search results
- *(core)* carry read start line as generic block metadata
- *(tui)* replace content line numbers with selectable dialogue dots
- *(tui)* render tool calls with per-tool names and \$/ > formats
- *(tui)* fold mixed-kind structure runs with two-level expansion
- *(tui)* fold same-kind structure runs into kind xN blocks
- *(tui)* mark list selection with dots only, always visible
- *(tui)* gray full-width block highlight and double-click fold
- *(tui)* make every workpart a foldable block with list-style cursor
- *(tui)* highlight the clicked structure block in content
- *(tui)* group tool calls with results and show descriptions
- *(tui)* click a structure tag to expand its block in read mode
- *(tui)* color unified-diff lines inside code blocks
- *(tui)* configurable theme with light/dark and truecolor detection
- *(remote)* rename groups (owner-only, propagated via roster sync) ([#98](https://github.com/Ariestar/sivtr/pull/98))
- *(remote)* add group mode for multi-device memory sharing ([#70](https://github.com/Ariestar/sivtr/pull/70))
- *(tui)* enable bracketed paste for text inputs ([#77](https://github.com/Ariestar/sivtr/pull/77))
- *(tui)* restore terminal state before panics are reported ([#73](https://github.com/Ariestar/sivtr/pull/73))
- *(agents)* add Qoder-CN provider, align Qoder MCP config
- *(agents)* add Gemini CLI, Goose, and Qwen Code providers ([#104](https://github.com/Ariestar/sivtr/pull/104))
- *(core)* parse codex custom tool call events

### Fixed

- restore SlidingPane is_empty for clippy
- *(tui)* carry collapse review fixes into segment-based folding
- *(remote)* surface daemon cleanup failures and shell exit
- *(tui)* ansi focus contrast and copy hints
- *(tui)* saturate gutter line numbers and range math
- *(tui)* bound range spans and toggle anchors
- *(tui)* keep block marks across folds and blank halves
- *(browse)* copy and mark the displayed dialogue
- *(browse)* sort dialogues newest first
- *(tui)* only select content on a real drag, not on a click
- *(tui)* show the current position in a pane's bottom-right corner
- *(tui)* keep block marks single-dialogue under multi-select
- *(tui)* footer shows hotkeys only, no mode or ref status
- *(tui)* copy the block under the cursor, run members included
- *(tui)* mark content blocks with space like list rows
- *(tui)* pair interleaved parallel tool calls by call id
- *(tui)* cache content layout so wheel scroll stays responsive
- *(tui)* smooth wheel scroll under the cursor and hide cursor on click
- *(tui)* deepen the focus veil to slate-900
- *(tui)* lighten the focus gray to slate-300
- *(tui)* lighten the focus highlight to a faint gray
- *(tui)* soften the focus highlight to a light gray tint
- *(tui)* tighten structure markers and block folding edges
- *(tui)* click any part of a structure block to collapse it
- *(tui)* wheel scroll and click hit-test offsets
- *(tui)* unify body foreground across lists and content
- *(tui)* accept only canonical hunk headers at column 0
- *(tui)* validate unified-diff markers before diff coloring
- *(tui)* render structure markers and fold summaries in gray
- *(tui)* drop appearance detection failure latch
- *(tui)* address theme review feedback
- *(tui)* finish light theme support
- *(tui)* keep the ANSI fallback for forced light/dark themes
- *(tui)* route the body-failure marker through the theme palette
- *(tui)* light/dark ANSI palettes and reject misspelled theme keys
- *(tui)* route provider colors through the palette; tighten light detection
- *(deps)* update dependency astro to v7.2.2 ([#138](https://github.com/Ariestar/sivtr/pull/138))
- *(tui)* hide the cursor outside typing and selection modes ([#79](https://github.com/Ariestar/sivtr/pull/79))
- *(tui)* catch panics, restore the terminal, and report ([#78](https://github.com/Ariestar/sivtr/pull/78))
- *(deps)* update dependency astro to v7.2.1 ([#132](https://github.com/Ariestar/sivtr/pull/132))
- *(deps)* update dependency astro to v7.2.0 ([#112](https://github.com/Ariestar/sivtr/pull/112))
- *(codex)* make watch exports incremental ([#84](https://github.com/Ariestar/sivtr/pull/84))
- *(deps)* update dependency astro to v7 ([#65](https://github.com/Ariestar/sivtr/pull/65))
- *(core)* extract tool results faithfully from raw output
- *(core)* normalize opencode MCP tool names as server:tool
- *(core)* recover complete grok sessions from the ACP stream
- *(core)* tolerate corrupt grok session files and seed record times
- *(core)* backfill tool names onto results and pair by call id

### Other

- *(deps)* update rust crate rmcp to v3.1.3 ([#156](https://github.com/Ariestar/sivtr/pull/156))
- adopt release-plz and PR-gated CI ([#154](https://github.com/Ariestar/sivtr/pull/154))
- *(deps)* update rust crate similar to v3.2.0 ([#148](https://github.com/Ariestar/sivtr/pull/148))
- Merge pull request #146 from Ariestar/renovate/dark-light-3.x
- Merge remote-tracking branch 'origin/main' into feat/dsh-provider
- drop origin aliases and silent fallbacks
- rustfmt TUI terminal hint
- Merge remote-tracking branch 'origin/main' into feat/provider-t1
- Merge remote-tracking branch 'origin/main' into feat/provider-t1
- cut dead config sections and duplicate provider helpers
- expect with context in session tests
- *(tui)* single width source for content text
- *(init)* share shell path lookup helper
- *(remote)* dedupe daemon info removal
- *(tui)* share one truncate_chars across panes
- *(browse)* dedupe mask reads and content hit-test
- *(browse)* unify range select across list panes
- *(tui)* drop duplicated focus-style comment
- *(tui)* unify current-row and position rendering across panes
- *(tui)* drop unused WorkspaceCopyParts::block field
- *(core)* drop unreachable text primitives and unused thiserror dep
- *(tui)* categorize tool display by command/read/search/edit/web
- *(tui)* drop redundant helpers from the audit
- unify _with pairs and prompt fork
- remove dead _with/extension wrappers
- defer the changelog entry to release time
- apply rustfmt
- *(tui)* merge _expanded wrappers into single io-texts functions
- *(tui)* fold structure blocks to tags in read mode
- *(tui)* fold agent colors into one exhaustive table
- *(tui)* drop the reset_workspace_search_state pass-through wrapper
- *(deps)* update rust crate uuid to v1.24.1 ([#140](https://github.com/Ariestar/sivtr/pull/140))
- *(remote)* harden group roster convergence ([#101](https://github.com/Ariestar/sivtr/pull/101))
- *(remote)* split the group domain out of the daemon monolith ([#100](https://github.com/Ariestar/sivtr/pull/100))
- *(tui)* name background loader threads and surface spawn failures ([#81](https://github.com/Ariestar/sivtr/pull/81))
- *(tui)* drop assert!/expect from production paths ([#80](https://github.com/Ariestar/sivtr/pull/80))
- *(tui)* cache dialogue projection and honor held-key repeat ([#76](https://github.com/Ariestar/sivtr/pull/76))
- *(tui)* dedupe per-frame content layout work ([#75](https://github.com/Ariestar/sivtr/pull/75))
- *(tui)* skip redraw when idle and block on input ([#74](https://github.com/Ariestar/sivtr/pull/74))
- *(deps)* update actions/github-script action to v9 ([#123](https://github.com/Ariestar/sivtr/pull/123))
- *(deps)* update rust crate ureq to v3.4.0 ([#122](https://github.com/Ariestar/sivtr/pull/122))
- *(deps)* update dependency rust to 1.97 ([#121](https://github.com/Ariestar/sivtr/pull/121))
- *(deps)* update rust crate thiserror to v2.0.20 ([#120](https://github.com/Ariestar/sivtr/pull/120))
- *(deps)* update rust crate rusqlite to v0.40.2 ([#119](https://github.com/Ariestar/sivtr/pull/119))
- add AGENT.md
- *(deps)* update rust crate clap to v4.6.6 ([#115](https://github.com/Ariestar/sivtr/pull/115))
- *(deps)* update rust crate rmcp to v3.1.2 ([#116](https://github.com/Ariestar/sivtr/pull/116))
- *(deps)* update dependency @types/vscode to v1.125.0 ([#111](https://github.com/Ariestar/sivtr/pull/111))
- *(deps)* update dependency @types/node to v26 ([#113](https://github.com/Ariestar/sivtr/pull/113))
- *(renovate)* enable vulnerability alerts
- *(renovate)* opt out of dependency dashboard approval
- *(deps)* update rust crate similar to v3.1.2 ([#108](https://github.com/Ariestar/sivtr/pull/108))
- *(deps)* update dependency @types/node to v20.19.43 ([#110](https://github.com/Ariestar/sivtr/pull/110))
- *(deps)* update rust crate toml to v1.1.4 ([#109](https://github.com/Ariestar/sivtr/pull/109))
- *(deps)* update rust crate base64 to v0.23.1 ([#106](https://github.com/Ariestar/sivtr/pull/106))
- *(deps)* update dependency @vscode/vsce to v3.9.2 ([#105](https://github.com/Ariestar/sivtr/pull/105))
- add PayPal as a sponsor option
- *(ai-review)* drop unsupported -r flag on gh api --jq
- *(ai-review)* set GH_TOKEN for gh api steps
- optimize release profile for TUI startup and size ([#71](https://github.com/Ariestar/sivtr/pull/71))
- *(renovate)* auto-merge patch and minor dependency updates ([#88](https://github.com/Ariestar/sivtr/pull/88))
- AI-powered pull request review (provider-agnostic LLM) ([#97](https://github.com/Ariestar/sivtr/pull/97))
- Fix typo in README.md description ([#69](https://github.com/Ariestar/sivtr/pull/69))
- *(core)* drop unused CopyConfig prompt_values
- *(config)* cover theme TOML round-trip and typo rejection

## [0.4.1] - 2026-08-05

Full notes: [changelogs/0.4.1.md](changelogs/0.4.1.md)

## [0.4.0] - 2026-08-04

Full notes: [changelogs/0.4.0.md](changelogs/0.4.0.md)

## [0.3.0] - 2026-07-21

Full notes: [changelogs/0.3.0.md](changelogs/0.3.0.md)

## [0.2.6] - 2026-07-13

Full notes: [changelogs/0.2.6.md](changelogs/0.2.6.md)

## [0.2.5] - 2026-07-12

Full notes: [changelogs/0.2.5.md](changelogs/0.2.5.md)

## [0.2.4] - 2026-06-10

Full notes: [changelogs/0.2.4.md](changelogs/0.2.4.md)

## [0.2.3] - 2026-05-28

Full notes: [changelogs/0.2.3.md](changelogs/0.2.3.md)

## [0.1.3] - 2026-05-20

Full notes: [changelogs/0.1.3.md](changelogs/0.1.3.md)

## [0.1.2] - 2026-05-02

Full notes: [changelogs/0.1.2.md](changelogs/0.1.2.md)

## [0.1.1] - 2026-05-01

Full notes: [changelogs/0.1.1.md](changelogs/0.1.1.md)

## [0.1.0] - 2026-04-28

Full notes: [changelogs/0.1.0.md](changelogs/0.1.0.md)