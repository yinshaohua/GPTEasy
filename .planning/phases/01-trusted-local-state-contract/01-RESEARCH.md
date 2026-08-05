# Phase 1: 可信本地状态与实现契约 - Research

**Researched:** 2026-08-05 `[VERIFIED: orchestrator date]`  
**Domain:** Tauri 2 / Rust / SQLite 本地状态、永久迁移、备份恢复与外部实现契约冻结  
**Confidence:** MEDIUM — SQLite 与 Windows/WSL2 路径已有强证据；Codex 0.146.1 运行回归、真实 macOS 和正式签名工件仍是阶段阻断项。`[VERIFIED: official docs + codebase spikes]`

<user_constraints>
## User Constraints

### Locked Decisions

- 阶段目标是：供应商、环境选择和设置可靠保存在当前用户本机，并在升级或降级边界下保持可恢复。`[VERIFIED: .planning/ROADMAP.md]`
- 本阶段必须覆盖 `STATE-01` 至 `STATE-05`，不能扩展成供应商验证、Codex 配置写入、WSL2 切换、完整托盘 UI 或更新系统。`[VERIFIED: .planning/REQUIREMENTS.md + .planning/ROADMAP.md]`
- 技术架构锁定为 Tauri 2、Rust、TypeScript/React；系统与领域能力由 Rust 实现，React 只能通过受控 Tauri command 访问。`[VERIFIED: docs/adr/0002-tauri-rust-react.md + .planning/PROJECT.md]`
- 应用内部状态锁定为 Rust 后端独占访问的当前用户 SQLite；供应商、明文 API Key、验证状态、各受管环境的当前供应商和应用设置均由该数据库保存。`[VERIFIED: docs/adr/0006-sqlite-application-state.md]`
- 数据库升级锁定为永久保留的顺序迁移、升级前完整备份、事务内执行、失败回滚、禁止清空数据继续运行；只保留最近三份数据库升级备份。`[VERIFIED: docs/adr/0006-sqlite-application-state.md + STATE-03 + STATE-04]`
- 旧版应用遇到更高 schema 必须拒绝写入；降级只能通过恢复升级前备份完成。`[VERIFIED: STATE-05 + docs/adr/0006-sqlite-application-state.md]`
- 首版完全本地运行，不建设产品账户、云端状态或默认上传；诊断日志使用普通文件而不是 SQLite。`[VERIFIED: docs/adr/0007-local-only-product.md + docs/adr/0006-sqlite-application-state.md]`
- 明文供应商凭据是锁定产品决策；数据库、目标 Codex 配置和其备份可以包含凭据，但日志、错误、诊断导出和非必要临时产物必须脱敏。`[VERIFIED: docs/adr/0001-plaintext-provider-credentials.md + .planning/PROJECT.md]`
- 根目录 `CONTEXT.md`、`docs/adr/` 和 `docs/ui/UI-SPEC.md` 是锁定基线；本研究不重新设计领域语言、产品范围或 UI。`[VERIFIED: .planning/PROJECT.md + .planning/STATE.md]`
- 在锁定 SQLite schema 和环境状态模型前，必须冻结目标 Codex、正式宿主应用、代表性 WSL2 和签名打包契约；未验证事实必须显式阻断，不能由业务代码猜测。`[VERIFIED: user phase description + .planning/ROADMAP.md]`

### the agent's Discretion

- 阶段目录不存在 `*-CONTEXT.md`，因此没有额外的阶段级自由度清单；本研究仅在模块边界、DDL 细节、迁移执行器、备份格式、恢复状态机、fixture 布局和 tracer 切分上做实现建议。`[VERIFIED: init.phase-op + phase directory scan]`
- 采用标准粒度与 tracer-first：先贯通“启动 → 打开/迁移 DB → 读取投影 → 修改设置 → 关闭 → 重开确认”，再补齐迁移矩阵、故障恢复和外部契约门禁。`[VERIFIED: user planning mode]`

### Deferred Ideas (OUT OF SCOPE)

- 供应商网络验证、Responses API、配置写入 Saga、进程重启、WSL2 实际切换、Linux 脚本、托盘生命周期、诊断导出和 updater 业务流程属于后续阶段。`[VERIFIED: .planning/ROADMAP.md]`
- 本阶段只建立这些后续能力所依赖的稳定状态模型与版本化 contract fixtures，不提前实现后续用例。`[VERIFIED: user planning mode + roadmap phase boundaries]`
</user_constraints>

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| STATE-01 | 用户关闭并重新打开 GPTEasy 后，供应商目录、验证状态、各受管环境的当前供应商和应用设置保持不变。 | 使用 Rust `StateStore`、类型化 repository、重开集成测试和最小 Tauri bootstrap 投影证明持久化。`[VERIFIED: .planning/REQUIREMENTS.md + proposed architecture]` |
| STATE-02 | 供应商、供应商凭据、当前状态和诊断日志只保存在当前用户本机，应用不要求产品账户，也不默认上传这些数据。 | 使用 `app_local_data_dir()`、不引入网络/账户模块、API Key 明文入 SQLite但不进入日志；诊断目录仅定义本地路径。`[VERIFIED: ADR-0001/0006/0007 + Tauri PathResolver docs]` |
| STATE-03 | 用户从任一正式历史版本升级时，应用能够通过永久保留的顺序迁移直接升级现有 SQLite 数据库。 | 采用 append-only migration registry、`schema_migrations` + `PRAGMA user_version` 双账本和 committed historical fixtures 矩阵。`[VERIFIED: SQLite PRAGMA/transaction docs + ADR-0006]` |
| STATE-04 | 数据库迁移前自动创建完整备份并默认保留最近三份；迁移失败时事务回滚，应用不能通过清空用户数据继续运行。 | 使用 SQLite Online Backup API 生成可打开快照，验证后裁剪到三份；全部 pending migrations 在同一个 `BEGIN IMMEDIATE` 事务中执行。`[CITED: https://sqlite.org/backup.html]` |
| STATE-05 | 旧版 GPTEasy 遇到更高版本数据库时拒绝写入，降级只能通过恢复升级前备份完成。 | 先只读 preflight 再决定是否 RW；更高 schema 进入 recovery-only 模式，只允许列出并恢复兼容备份。`[CITED: https://sqlite.org/pragma.html#pragma_user_version]` |
</phase_requirements>

## Project Constraints (from AGENTS.md)

- 所有回复和本研究叙述使用中文；代码、命令、标识符与路径保持其自然技术语言。`[VERIFIED: user-provided AGENTS instructions]`
- Git 提交说明使用中文；在主干管理，不主动创建分支。`[VERIFIED: user-provided AGENTS instructions + .planning/config.json git.branching_strategy=none]`
- 普通文本使用 Unix 换行。`[VERIFIED: user-provided AGENTS instructions]`
- 需要代理访问网络时使用 `127.0.0.1:7897`；本次官方文档与 registry 查询未需要启用代理。`[VERIFIED: user-provided AGENTS instructions + tool execution]`
- 根 `AGENTS.md` 不超过 200 行，大段规则放入其他文档并从 AGENTS 指向。`[VERIFIED: user-provided AGENTS instructions]`
- Issue/PRD 使用 `yinshaohua/GPTEasy` GitHub Issues，操作通过 `gh`；本机当前没有 `gh`，但本研究未创建或修改 Issue。`[VERIFIED: AGENTS.md + docs/agents/issue-tracker.md + environment probe]`
- 使用五个 triage 标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`。`[VERIFIED: docs/agents/triage-labels.md]`
- 仓库采用 single-context；实现命名必须使用根 `CONTEXT.md` 的领域词汇，并显式报告 ADR 冲突。`[VERIFIED: docs/agents/domain.md]`
- `spike-findings-gpteasy` 是本阶段强制实现证据；研究已读取其全部 reference 与相关原始 Spike。`[VERIFIED: AGENTS.md + skill files + codebase scan]`

## Summary

本阶段应交付一个可安装、可启动、可重开的最小 Tauri/React walking skeleton，但真正的产品内核是 Rust `StateStore`：它在任何写连接之前只读检查 `application_id` 与 `user_version`，为旧 schema 创建并验证完整 SQLite 快照，再在单个 immediate transaction 中顺序执行全部 pending migrations；失败时保留原数据库与备份并进入可解释的启动错误，而不是清库重建。`[VERIFIED: ADR-0006 + SQLite official docs]`

状态 schema 应只持久化稳定领域事实：不可变供应商 ID 与当前已保存配置、独立验证记录、不可变受管环境 ID 与当前供应商关联、类型化单行应用设置，以及迁移账本。运行中进程、最终有效 Codex 层、WSL2 当前运行状态等动态事实不应在本阶段作为数据库权威；后续阶段通过迁移添加其持久化操作状态。`[VERIFIED: CONTEXT.md + ADR-0005/0006/0008 + spike 007/008]`

阶段不能直接把 2026-08-05 已有 Spike 结论视为最终契约。官方 Codex latest API 已把最新稳定版推进到 `0.146.1`，而本机与 Spike 仍是 `0.146.0`；相关核心配置和 `config/read` schema 文件在两个 tag 间未变化，但目标二进制、正式宿主 bundle 与 stdio 启动仍需重新跑 contract fixture。真实 macOS、Windows Authenticode、Windows ARM64 构建和签名、公证也仍未验证，因此必须成为 Phase 1 结束门禁而不是备注。`[VERIFIED: official OpenAI release API + GitHub compare + environment probe + spike 017]`

**Primary recommendation:** 计划按“契约门禁与项目骨架 → 状态 tracer → 备份/迁移/高版本拒写 → 历史 fixture 与打包 smoke”四个波次推进；在 Codex 0.146.1、真实 macOS 和正式签名工件证据齐全前，不宣称 schema/host contract 已冻结。`[VERIFIED: project risk boundary + evidence gaps]`

## Architectural Responsibility Map

| Capability | Primary Tier | Secondary Tier | Rationale |
|------------|-------------|----------------|-----------|
| 当前用户状态根目录解析 | API / Backend | Database / Storage | Rust 通过 Tauri `PathResolver::app_local_data_dir()` 决定唯一产品路径，前端不接收任意路径参数。`[CITED: https://docs.rs/tauri/2.11.5/tauri/path/struct.PathResolver.html]` |
| 供应商目录、验证记录、环境当前供应商、应用设置 | Database / Storage | API / Backend | SQLite 是权威持久层，Rust repository 负责事务与领域不变量。`[VERIFIED: ADR-0006]` |
| schema preflight、迁移、备份、恢复 | API / Backend | Database / Storage | 决策、错误分类和恢复状态机属于应用服务；SQLite 提供事务与 backup primitive。`[CITED: https://sqlite.org/backup.html]` |
| 更高 schema 拒写与 recovery-only 启动 | API / Backend | Browser / Client | Rust 决定只读/拒写；React 只渲染恢复投影并发起受限 backup-id 恢复。`[VERIFIED: STATE-05 + ADR-0002]` |
| 外部 Codex/宿主/WSL/打包契约采集 | API / Backend | External OS / Service Boundary | 探针必须运行在目标主机并输出脱敏、版本化 fixture；其结果是 schema/计划门禁输入。`[VERIFIED: phase risk boundary + spike 001/009/017]` |
| 最小 Phase 1 状态页面 | Browser / Client | API / Backend | React 只调用 `bootstrap_state`、`update_app_settings`、`list_compatible_backups`、`restore_backup`，不持有第二份权威状态。`[VERIFIED: ADR-0002 + UI-SPEC feedback/recovery principles]` |
| NSIS / `.app` 打包与签名 smoke | CDN / Static Artifact | API / Backend | 构建系统产出工件，应用 tracer 验证安装后状态路径、重启和恢复。`[VERIFIED: Tauri distribution docs + spike 005/017]` |

## Contract Freeze Matrix

| Contract | Proven Evidence | Current Verdict | Required Freeze Artifact / Gate |
|----------|-----------------|-----------------|---------------------------------|
| Codex 默认配置根 | 官方 0.146.1 source 仍定义 `CODEX_HOME`，未设置时为 `~/.codex`；Windows Spike 001 观察到 CLI 与正式宿主均未覆盖 `CODEX_HOME`。`[CITED: https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/utils/home-dir/src/lib.rs]` | **Source verified；0.146.1 runtime pending**。`[VERIFIED: source diff + local codex 0.146.0 probe]` | `tests/contracts/codex/0.146.1/manifest.json`：二进制版本、路径、SHA-256、生成 schema SHA-256、隔离 `CODEX_HOME` 结果。`[VERIFIED: proposed gate]` |
| Provider 字段与凭据载体 | 0.146.1 source 包含 `base_url`、`env_key`、`experimental_bearer_token`、`wire_api=responses`、`requires_openai_auth`、`supports_websockets`；0.146.0 Spike 已实发 Bearer 请求。`[CITED: https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/model-provider-info/src/lib.rs]` | **Source verified；target binary regression pending**。`[VERIFIED: spike 001 + source compare]` | 0.146.1 fake-provider fixture：`env_key`、direct bearer、缺失 env、Responses request summary，绝不保存完整 Key。`[VERIFIED: proposed gate]` |
| 配置优先级与 `config/read` | 0.146.1 `ConfigReadParams` 仍含 `cwd`/`includeLayers`，响应含 effective config、origins、layers；优先级 source 明确 user < project < session flags。`[CITED: https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/app-server-protocol/src/protocol/v2/config.rs]` | **Schema verified；0.146.1 stdio smoke pending**。`[VERIFIED: spike 008 + source compare]` | 运行 `codex app-server generate-json-schema --out ...`，再执行 initialize/initialized/config-read 三场景并提交脱敏 golden。`[CITED: https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/app-server/README.md]` |
| Windows 正式宿主身份 | 2026-08-05 Spike 观察到 AppX `OpenAI.Codex 26.730.8199.0`、`ChatGPT.exe` 根进程和 bundled `resources/codex.exe app-server`。`[VERIFIED: .planning/spikes/001.../windows-evidence.json]` | **Observed on one x64 host；not durable identity**。`[VERIFIED: spike 004 limitation]` | 新 fixture 只保存 package family、bundle version、exe SHA-256、PID/PPID 角色与布尔分类理由；禁止完整命令行。`[VERIFIED: spike discrepancy audit]` |
| WSL2 被动发现 | 官方命令支持 `--list`、`--running`、`--terminate` 与显式 `--user`；Spike 009 证明 list 探针不启动发行版。`[CITED: https://learn.microsoft.com/en-us/windows/wsl/basic-commands]` | **List/lifecycle verified；stopped default username not publicly guaranteed**。`[VERIFIED: official docs negative check + spike 009]` | `wsl-host-contract.json` 保存 GUID、显示名、DefaultUid、运行集合和 `command_target_resolvable`；重复名称必须为 false 并 fail-closed。`[VERIFIED: spike 009 evidence]` |
| 真实 WSL2 写入载体 | Spike 013 用一次性 Ubuntu Base 24.04.3 amd64 完成 10/10，证明 stdin 可避免 Key 进入参数并恢复生命周期。`[VERIFIED: spike 013 summary]` | **Representative amd64 verified；ARM64 pending**。`[VERIFIED: spike 013 limitation]` | Phase 1 只冻结 host probe schema；实际 guest 写入仍留给 Phase 5。`[VERIFIED: roadmap boundary]` |
| Windows 当前用户安装 | Spike 005 用 Tauri 2.11.5/CLI 2.11.4 生成 NSIS `currentUser` 工件并安装到 `%LOCALAPPDATA%`。`[VERIFIED: spike 005 build/install summaries]` | **Unsigned x64 verified**。`[VERIFIED: spike 005]` | 正式 Authenticode x64 smoke：签名有效、安装范围、启动、写状态、重启读取、卸载不破坏状态备份策略。`[CITED: https://v2.tauri.app/distribute/sign/windows/]` |
| Windows ARM64 | Rust target 已安装，但本机没有 VS ARM64 C++ tools；Spike 未生成 ARM64 installer。`[VERIFIED: environment probe + spike 005]` | **BLOCKER**。`[VERIFIED: environment probe]` | 原生 ARM64 runner 产出签名 NSIS 并运行同一状态 tracer。`[VERIFIED: project platform matrix]` |
| macOS 当前用户安装/宿主/签名 | 只有 Windows contract harness 与 CI 模板；没有真实 macOS 14+、`~/Applications`、Codex/ChatGPT bundle、codesign、公证或 updater 证据。`[VERIFIED: spike 017 summary]` | **BLOCKER**。`[VERIFIED: spike 017]` | Intel 与 Apple Silicon 各自生成签名/公证 `.app`，安装到 `~/Applications/GPTEasy.app`，运行状态 tracer 并提交 Gatekeeper/codesign/bundle/host fixture。`[CITED: https://v2.tauri.app/distribute/sign/macos/]` |

## Standard Stack

### Core

| Library | Version / Publish Date | Purpose | Why Standard |
|---------|------------------------|---------|--------------|
| Rust toolchain | `1.97.1` / installed 2026 toolchain | 后端、测试与跨平台构建 | 当前环境已安装，满足 Tauri 2.11.5 的最低 Rust 版本要求。`[VERIFIED: environment probe + crates.io tauri metadata]` |
| `tauri` | `2.11.5` / 2026-07-01 | 桌面壳、Rust command、状态与路径解析 | ADR 锁定且 Spike 004/005/012/017 已编译验证。`[VERIFIED: crates.io + project spikes]` |
| `tauri-build` | `2.6.3` / 2026-06-17 | Tauri build script | 与已验证 Spike manifests 一致。`[VERIFIED: crates.io + codebase manifests]` |
| `@tauri-apps/api` | `2.11.1` / 2026-06-17 | React 调用 Rust command | 官方 Tauri React template 使用该包；legitimacy verdict `OK`。`[VERIFIED: npm registry]` |
| `@tauri-apps/cli` | `2.11.4` / 2026-06-28 | dev/build/bundle | 与 Spike 005/012/017 实际构建版本一致；legitimacy verdict `OK`。`[VERIFIED: npm registry + codebase manifests]` |
| `rusqlite` | `0.40.1` / 2026-06-06 | SQLite、事务、只读 flags、Online Backup API | Rust 后端独占 SQLite；启用 `bundled,backup` 可固定 SQLite 实现且无需系统 `sqlite3`。`[VERIFIED: crates.io + docs.rs]` |
| `react` [WARNING: flagged as suspicious — verify before using.] | `19.1.0` / 2025-03-28 | 最小设置/恢复 UI | ADR 锁定 React；版本取官方 create-tauri-app 4.7.3 template，而非 2026-08-05 同日发布的最新 patch。`[VERIFIED: npm registry + official Tauri template]` |
| `react-dom` [WARNING: flagged as suspicious — verify before using.] | `19.1.0` / 2025-03-28 | React DOM runtime | 与官方 Tauri React template 配对。`[VERIFIED: npm registry + official Tauri template]` |
| `vite` [WARNING: flagged as suspicious — verify before using.] | `8.0.16` / 2026-06-01 | 前端 dev/build | 采用官方 Tauri React template 已声明版本，避免直接跳到 8.2.0。`[VERIFIED: npm registry + official Tauri template]` |
| `typescript` [WARNING: flagged as suspicious — verify before using.] | `6.0.3` / 2026-04-16 | 前端类型检查 | 采用官方 Tauri React TypeScript template 版本。`[VERIFIED: npm registry + official Tauri template]` |
| `@vitejs/plugin-react` [WARNING: flagged as suspicious — verify before using.] | `6.0.2` / 2026-05-14 | Vite React transform | 官方 Tauri React template 标准组合。`[VERIFIED: npm registry + official Tauri template]` |

### Supporting

| Library | Version / Publish Date | Purpose | When to Use |
|---------|------------------------|---------|-------------|
| `serde` | `1.0.229` / 2026-07-18 | command DTO 与领域投影序列化 | 所有 Tauri 输入/输出 DTO；不序列化 raw DB row。`[VERIFIED: crates.io]` |
| `serde_json` | `1.0.151` / 2026-07-20 | contract fixture 与测试 manifest | 仅存脱敏 evidence 与 fixture metadata。`[VERIFIED: crates.io]` |
| `thiserror` | `2.0.19` / 2026-07-18 | 类型化后端错误 | 区分 `DatabaseTooNew`、`MigrationFailed`、`BackupInvalid` 等。`[VERIFIED: crates.io]` |
| `uuid` | `1.24.0` / 2026-07-15 | 不可变供应商/环境 ID | 开启 `v4,serde`；显示名、地址、WSL 名称不能作为身份。`[VERIFIED: crates.io + ADR-0005 + spike 009]` |
| `sha2` | `0.11.0` / 2026-03-25 | migration checksum、backup hash、contract artifact hash | 使用版本化域分隔，不实现自定义 hash。`[VERIFIED: crates.io + spike 012 pattern]` |
| `chrono` | `0.4.45` / 2026-06-04 | UTC RFC3339 与 sortable backup 名 | 复用 Spike 的无时区业务逻辑模式，feature 仅 `clock,std`。`[VERIFIED: crates.io + codebase manifests]` |
| `windows-sys` | `0.61.2` / current registry | Windows `ReplaceFileW` 等原子恢复边界 | 仅 `cfg(windows)`，feature 限定 `Win32_Foundation,Win32_Storage_FileSystem`。`[VERIFIED: crates.io + spike 006/012 pattern]` |
| `tempfile` | `3.27.0` / 2026-03-11 | 测试隔离目录与 DB copy | 仅 dev-dependency；避免并行测试目录冲突。`[VERIFIED: crates.io]` |
| `@types/react` [WARNING: flagged as suspicious — verify before using.] | `19.1.8` / 2025-06-11 | React TypeScript types | 与官方 Tauri template 一致。`[VERIFIED: npm registry + official Tauri template]` |
| `@types/react-dom` [WARNING: flagged as suspicious — verify before using.] | `19.1.6` / 2025-06-04 | ReactDOM types | 与官方 Tauri template 一致。`[VERIFIED: npm registry + official Tauri template]` |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Rust `rusqlite` repository | 前端 SQL plugin | 会让 React 绕过 Rust 领域边界并成为第二状态源，违反 ADR-0002/0006；不采用。`[VERIFIED: project ADRs]` |
| SQLite Online Backup API | 直接复制 `state.sqlite3` | WAL 模式下单文件复制可能遗漏已提交 WAL 内容；不采用。`[CITED: https://sqlite.org/backup.html]` |
| 一次涵盖全部 pending migrations 的事务 | 每个 migration 单独提交 | 中途失败会留下半升级 schema，不满足“原数据库完整回滚”；不采用。`[VERIFIED: STATE-04 + SQLite transaction docs]` |
| 类型化 `app_settings` 单行表 | key/value JSON 或 EAV | EAV 绕过 schema 迁移与约束，增加降级/字段重命名歧义；不采用。`[VERIFIED: phase migration goals]` |
| `app_local_data_dir()` | `app_data_dir()` | Windows 的 local-data 路径更符合“仅当前设备本机”意图；不主动选 roaming data。`[CITED: https://docs.rs/tauri/2.11.5/tauri/path/struct.PathResolver.html]` |

**Installation:** npm legitimacy gate 将 React/Vite/TypeScript 相关包标为 `SUS`（原因仅为 `too-new`）；planner 必须先插入一次覆盖全部 flagged npm 包的 `checkpoint:human-verify`，确认它们来自官方 Tauri template / React / Microsoft / Vite 仓库后再安装。`[VERIFIED: package-legitimacy seam]`

```bash
npm install --save-exact \
  @tauri-apps/api@2.11.1 \
  react@19.1.0 \
  react-dom@19.1.0

npm install --save-dev --save-exact \
  @tauri-apps/cli@2.11.4 \
  @types/react@19.1.8 \
  @types/react-dom@19.1.6 \
  @vitejs/plugin-react@6.0.2 \
  typescript@6.0.3 \
  vite@8.0.16
```

`Cargo.toml` 应显式关闭 `rusqlite` 默认 features，并提交 `Cargo.lock` 与 `package-lock.json`。`[VERIFIED: cargo info rusqlite 0.40.1 + deterministic build requirement]`

```toml
[build-dependencies]
tauri-build = { version = "=2.6.3", features = [] }

[dependencies]
chrono = { version = "=0.4.45", default-features = false, features = ["clock", "std"] }
rusqlite = { version = "=0.40.1", default-features = false, features = ["bundled", "backup"] }
serde = { version = "=1.0.229", features = ["derive"] }
serde_json = "=1.0.151"
sha2 = "=0.11.0"
tauri = { version = "=2.11.5", features = [] }
thiserror = "=2.0.19"
uuid = { version = "=1.24.0", features = ["v4", "serde"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Foundation", "Win32_Storage_FileSystem"] }

[dev-dependencies]
tempfile = "=3.27.0"
```

## Package Legitimacy Audit

| Package | Registry | Age / Downloads | Source Repo | Verdict | Disposition |
|---------|----------|-----------------|-------------|---------|-------------|
| `@tauri-apps/api` | npm | 2021 起 / 2.31M 周下载 | `tauri-apps/tauri` | OK | Approved。`[VERIFIED: npm registry]` |
| `@tauri-apps/cli` | npm | 2021 起 / 2.07M 周下载 | `tauri-apps/tauri` | OK | Approved。`[VERIFIED: npm registry]` |
| `react` | npm | 2011 起 / 163M 周下载 | `react/react` | SUS (`too-new`) | 保留；安装前人工核验官方 template 与 exact version。`[VERIFIED: npm registry]` |
| `react-dom` | npm | 2011 起 / 154M 周下载 | `react/react` | SUS (`too-new`) | 保留；安装前人工核验。`[VERIFIED: npm registry]` |
| `vite` | npm | 2017 起 / 163M 周下载 | `vitejs/vite` | SUS (`too-new`) | 保留；安装前人工核验。`[VERIFIED: npm registry]` |
| `typescript` | npm | 2012 起 / 263M 周下载 | `microsoft/TypeScript` | SUS (`too-new`) | 保留；安装前人工核验。`[VERIFIED: npm registry]` |
| `@vitejs/plugin-react` | npm | 2021 起 / 78.8M 周下载 | `vitejs/vite-plugin-react` | SUS (`too-new`) | 保留；安装前人工核验。`[VERIFIED: npm registry]` |
| `@types/react` | npm | DefinitelyTyped / 150M 周下载 | `DefinitelyTyped/DefinitelyTyped` | SUS (`too-new`) | 保留；安装前人工核验。`[VERIFIED: npm registry]` |
| `@types/react-dom` | npm | DefinitelyTyped / 123M 周下载 | `DefinitelyTyped/DefinitelyTyped` | SUS (`too-new`) | 保留；安装前人工核验。`[VERIFIED: npm registry]` |
| `tauri` | crates.io | 2019 起 / 713K 周下载 | `tauri-apps/tauri` | OK | Approved。`[VERIFIED: crates.io]` |
| `tauri-build` | crates.io | 2021 起 / 715K 周下载 | `tauri-apps/tauri` | OK | Approved。`[VERIFIED: crates.io]` |
| `rusqlite` | crates.io | 2014 起 / 2.22M 周下载 | `rusqlite/rusqlite` | OK | Approved。`[VERIFIED: crates.io]` |
| `serde` | crates.io | 2014 起 / 20.0M 周下载 | `serde-rs/serde` | OK | Approved。`[VERIFIED: crates.io]` |
| `serde_json` | crates.io | 2015 起 / 20.2M 周下载 | `serde-rs/json` | OK | Approved。`[VERIFIED: crates.io]` |
| `thiserror` | crates.io | 2019 起 / 24.4M 周下载 | `dtolnay/thiserror` | OK | Approved。`[VERIFIED: crates.io]` |
| `uuid` | crates.io | 2014 起 / 12.6M 周下载 | `uuid-rs/uuid` | OK | Approved。`[VERIFIED: crates.io]` |
| `sha2` | crates.io | 2016 起 / 16.7M 周下载 | `RustCrypto/hashes` | OK | Approved。`[VERIFIED: crates.io]` |
| `chrono` | crates.io | 2014 起 / 11.7M 周下载 | `chronotope/chrono` | OK | Approved。`[VERIFIED: crates.io]` |
| `tempfile` | crates.io | 2015 起 / 12.6M 周下载 | `Stebalien/tempfile` | OK | Approved。`[VERIFIED: crates.io]` |
| `windows-sys` | crates.io | 2021 起 / 30.4M 周下载 | `microsoft/windows-rs` | OK | Approved。`[VERIFIED: crates.io]` |

**Packages removed due to [SLOP] verdict:** none。`[VERIFIED: package-legitimacy seam]`  
**Packages flagged as suspicious [SUS]:** `react`、`react-dom`、`vite`、`typescript`、`@vitejs/plugin-react`、`@types/react`、`@types/react-dom`；planner 必须在首次 npm install 前加入 `checkpoint:human-verify`。`[VERIFIED: package-legitimacy seam]`

## Architecture Patterns

### System Architecture Diagram

```text
OS launch / signed installer smoke
              |
              v
     Tauri Rust bootstrap
              |
              v
     Resolve app_local_data_dir
              |
              v
  Does state.sqlite3 exist? -------------------- no ------------------+
              | yes                                                    |
              v                                                        v
 Read-only preflight                                            Create new DB
 application_id + user_version                                  in one transaction
              |
       +------+-------------------+
       |                          |
 user_version > current     user_version <= current
       |                          |
       v                          v
 Recovery-only mode       Equal? ---- yes ----> Open ready store
 no write connection         |
       |                    no / older
       |                      |
       |                      v
       |              Online Backup API
       |              verify quick_check
       |              retain latest 3
       |                      |
       |                      v
       |              BEGIN IMMEDIATE
       |              apply N..CURRENT sequentially
       |              verify FK + quick_check
       |                      |
       |              success? ---- no ----> rollback + MigrationFailed
       |                 |
       |                yes
       |                 v
       |              commit / ready
       |                 |
       +-----------------+----------------------+
                                                 v
                                      Rust repositories/use cases
                                                 |
                                      typed Tauri command projections
                                                 |
                                                 v
                                          React tracer / recovery UI

External contract boundary:
Codex 0.146.1 + official host + WSL2 + signed packages
        -> versioned, redacted fixtures
        -> phase gate before "contract frozen"
```

该图的关键是：更高 schema 分支绝不能经过 read-write open；迁移失败分支绝不能调用“create fresh database”。`[VERIFIED: STATE-04/05]`

### Recommended Project Structure

```text
/
├── package.json
├── package-lock.json
├── src/
│   ├── App.tsx                       # Phase 1 bootstrap/recovery projection only
│   ├── main.tsx
│   └── state-api.ts                  # narrow invoke wrappers
├── src-tauri/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── tauri.conf.json
│   ├── capabilities/default.json     # no fs/shell/http plugin permissions
│   ├── src/
│   │   ├── lib.rs                    # composition root
│   │   ├── commands/
│   │   │   ├── bootstrap.rs
│   │   │   ├── settings.rs
│   │   │   └── recovery.rs
│   │   ├── domain/
│   │   │   ├── provider.rs
│   │   │   ├── environment.rs
│   │   │   └── settings.rs
│   │   └── state/
│   │       ├── mod.rs
│   │       ├── paths.rs
│   │       ├── error.rs
│   │       ├── preflight.rs
│   │       ├── connection.rs
│   │       ├── backup.rs
│   │       ├── recovery.rs
│   │       ├── migrations/
│   │       │   ├── mod.rs
│   │       │   └── 0001_initial.sql
│   │       └── repositories/
│   │           ├── providers.rs
│   │           ├── environments.rs
│   │           └── settings.rs
│   └── tests/
│       ├── state_persistence.rs
│       ├── migration_matrix.rs
│       ├── migration_failure.rs
│       ├── higher_schema_refusal.rs
│       └── backup_restore.rs
├── tests/
│   └── fixtures/
│       ├── databases/
│       │   ├── v001/state.sqlite3
│       │   └── manifest.json
│       └── contracts/
│           ├── codex/0.146.1/
│           ├── windows-host/
│           ├── wsl2/
│           └── packaging/
└── scripts/
    └── contracts/
        ├── probe-codex.ps1
        ├── probe-windows-host.ps1
        ├── probe-wsl2.ps1
        ├── verify-windows-package.ps1
        └── run-macos.sh
```

该布局让 `state/` 不依赖 Tauri，集成测试可以直接调用应用内核；Tauri command 只做 DTO 转换。`[VERIFIED: ADR-0002 + official Tauri command/state pattern]`

### Recommended Initial Schema

`CURRENT_SCHEMA_VERSION` 从 `1` 开始；每个正式发布后都把对应可打开 DB fixture 永久提交，未来只能追加 `0002_*`、`0003_*`，不能编辑已发布 migration。`[VERIFIED: ADR-0006 + STATE-03]`

```sql
-- 0001_initial.sql
CREATE TABLE schema_migrations (
    version      INTEGER PRIMARY KEY CHECK (version > 0),
    name         TEXT NOT NULL UNIQUE,
    checksum     TEXT NOT NULL CHECK (length(checksum) = 64),
    applied_at   TEXT NOT NULL
) STRICT;

CREATE TABLE providers (
    id                 TEXT PRIMARY KEY,
    provider_kind      TEXT NOT NULL
                       CHECK (provider_kind IN ('built_in_recommended', 'custom')),
    built_in_key       TEXT UNIQUE,
    display_name       TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    base_url           TEXT,
    api_key            TEXT,
    default_model      TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,
    CHECK (
      (provider_kind = 'built_in_recommended' AND built_in_key IS NOT NULL)
      OR
      (provider_kind = 'custom' AND built_in_key IS NULL)
    )
) STRICT;

CREATE TABLE provider_verifications (
    provider_id             TEXT PRIMARY KEY
                            REFERENCES providers(id) ON DELETE CASCADE,
    combination_fingerprint TEXT NOT NULL CHECK (length(combination_fingerprint) = 64),
    verified_at             TEXT NOT NULL,
    contract_version        TEXT NOT NULL
) STRICT;

CREATE TABLE managed_environments (
    id                  TEXT PRIMARY KEY,
    environment_kind    TEXT NOT NULL
                        CHECK (environment_kind IN ('native_codex', 'wsl2')),
    platform_identity   TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    current_provider_id TEXT
                        REFERENCES providers(id) ON DELETE RESTRICT,
    first_seen_at       TEXT NOT NULL,
    last_seen_at        TEXT NOT NULL,
    UNIQUE (environment_kind, platform_identity)
) STRICT;

CREATE TABLE app_settings (
    singleton_id                INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    locale                      TEXT NOT NULL
                                CHECK (locale IN ('system', 'zh-CN', 'en-US')),
    theme                       TEXT NOT NULL
                                CHECK (theme IN ('system', 'light', 'dark')),
    launch_at_login_desired     INTEGER NOT NULL CHECK (launch_at_login_desired IN (0, 1)),
    close_to_tray_notice_seen   INTEGER NOT NULL CHECK (close_to_tray_notice_seen IN (0, 1)),
    onboarding_completed        INTEGER NOT NULL CHECK (onboarding_completed IN (0, 1)),
    last_update_check_at        TEXT,
    updated_at                  TEXT NOT NULL
) STRICT;

INSERT INTO app_settings (
    singleton_id, locale, theme, launch_at_login_desired,
    close_to_tray_notice_seen, onboarding_completed,
    last_update_check_at, updated_at
) VALUES (1, 'system', 'system', 0, 0, 0, NULL, :applied_at);
```

此 schema 故意不保存运行中的 Codex/WSL 状态、最终有效 config layer、临时 probe 错误或诊断正文；这些不是本阶段的稳定业务事实。`[VERIFIED: spike 007/008/009 + roadmap boundaries]`

ADR-0006 明确要求 API Key 进入 SQLite，因此不得照搬 Spike 012 的“数据库不包含 API Key”实验 schema；Spike 012 只证明验证/切换流水线的泄漏边界，不能覆盖锁定产品决策。`[VERIFIED: ADR-0006 + codebase grep in spike 012]`

### Pattern 1: Two-Stage Open

**What:** 先用只读 connection 查询 `application_id`/`user_version`，只有版本兼容后才创建 read-write connection。`[CITED: https://docs.rs/rusqlite/0.40.1/rusqlite/struct.Connection.html]`  
**When to use:** 每次启动、恢复后重开、测试 fixture 打开。`[VERIFIED: STATE-05]`

```rust
// Source basis:
// https://sqlite.org/pragma.html#pragma_user_version
// https://docs.rs/rusqlite/0.40.1/rusqlite/struct.Connection.html
fn inspect_existing(path: &Path) -> Result<DbHeader, StoreError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let application_id =
        conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))?;
    let user_version =
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    Ok(DbHeader { application_id, user_version })
}
```

### Pattern 2: Backup Before One Encompassing Migration Transaction

**What:** Online Backup API 先复制完整 snapshot 并从独立只读 connection 运行 `quick_check`；随后一个 `TransactionBehavior::Immediate` 覆盖全部 pending migrations。`[CITED: https://sqlite.org/backup.html]`  
**When to use:** 任何 existing formal schema `< CURRENT_SCHEMA_VERSION`。`[VERIFIED: STATE-03/04]`

```rust
// Source:
// https://docs.rs/rusqlite/0.40.1/rusqlite/backup/struct.Backup.html
fn create_verified_backup(source: &Connection, target: &Path) -> Result<(), StoreError> {
    let mut destination = Connection::open(target)?;
    let backup = rusqlite::backup::Backup::new(source, &mut destination)?;
    backup.run_to_completion(128, Duration::from_millis(10), None)?;
    drop(backup);
    drop(destination);

    let check = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let quick_check: String =
        check.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        return Err(StoreError::BackupInvalid);
    }
    Ok(())
}
```

### Pattern 3: Recovery-Only Startup State

**What:** 初始化返回 `Ready(StateStore)` 或 `RecoveryRequired(RecoveryState)`，不能通过 nullable connection 让普通 commands “碰碰运气”。`[VERIFIED: STATE-04/05 + typed boundary recommendation]`  
**When to use:** 更高 schema、迁移失败、application ID 不匹配、backup 校验失败。`[VERIFIED: proposed failure taxonomy]`

```rust
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BootstrapState {
    Ready {
        schema_version: u32,
        snapshot: StateSnapshot,
    },
    DatabaseTooNew {
        found: u32,
        supported: u32,
        compatible_backups: Vec<BackupSummary>,
    },
    MigrationFailed {
        from: u32,
        to: u32,
        backup: BackupSummary,
        error_code: String,
    },
}
```

### Pattern 4: Versioned Contract Fixtures

**What:** 每个外部契约 fixture 都包含 `contract_name`、`observed_version`、`evidence_level`、`captured_at`、`artifact_sha256`、`assertions` 和 `redactions`；禁止只提交 README 结论。`[VERIFIED: spike 017 evidence-level pattern + phase risk boundary]`  
**When to use:** Codex 升级、宿主应用升级、WSL 版本变化、签名工件候选变化。`[VERIFIED: spike limitations]`

### Anti-Patterns to Avoid

- **打开 RW 后才检查 schema:** SQLite 可能创建 journal/WAL 或进入写路径，破坏“旧版拒绝写入”的可证明性；必须只读 preflight。`[VERIFIED: STATE-05 + SQLite open semantics]`
- **失败时删除 DB 并重建:** 直接违反 STATE-04，且会掩盖 migration defect。`[VERIFIED: STATE-04]`
- **迁移逐步 commit:** 从 v1 升 v5 在 v4 失败会留下 v3/v4 半升级；全部 pending migrations 必须在一个事务。`[VERIFIED: STATE-04 interpretation + SQLite transactions]`
- **复制单个 DB 文件作为 WAL backup:** 可能缺少已提交 WAL 页；使用 Online Backup API。`[CITED: https://sqlite.org/backup.html]`
- **把 settings 做成 JSON/EAV:** 会把字段兼容性问题推迟到运行时并削弱历史 schema 测试。`[VERIFIED: phase migration goals]`
- **把前端 `validated=true` 当权威:** 验证状态只能由 Rust repository 和后续验证用例写入。`[VERIFIED: spike 012]`
- **保存完整进程命令行到 contract evidence:** Spike 001 原始 evidence 实际包含完整命令行，正式 fixture 必须改成白名单摘要。`[VERIFIED: codebase grep]`
- **把 Windows 构建或 macOS CI 当真实 Mac 证据:** 托盘、LaunchServices、`~/Applications`、Gatekeeper 和签名更新仍需真实宿主。`[VERIFIED: spike 017]`
- **把 0.146.0 写成当前目标:** 截至 2026-08-05 最新稳定 Codex 是 0.146.1。`[VERIFIED: https://api.github.com/repos/openai/codex/releases/latest]`

## Tracer-First Decomposition

1. **Tracer A — 可构建空壳:** 建立官方 Tauri React TS 结构、最小 capability、`app_local_data_dir` 路径解析和 `bootstrap_state` command；`npm run build`、`cargo test`、Windows debug launch 全通。`[VERIFIED: official Tauri template + phase scope]`
2. **Tracer B — 重启持久化:** v1 schema、repository、合成 provider/verification/native environment/settings 集成测试；drop connection 后重开并逐字段断言。`[VERIFIED: STATE-01]`
3. **Tracer C — 升级安全:** committed v1 fixture、test-only v2 migration、Online Backup API、三份 retention、故障注入、higher-schema refusal、兼容 backup restore。`[VERIFIED: STATE-03/04/05]`
4. **Tracer D — 契约冻结:** Codex 0.146.1、Windows host、WSL2 probe、Windows signed x64/ARM64、macOS Intel/ARM64 fixture 全部产出 manifest；未完成项保持 phase blocker。`[VERIFIED: phase risk boundary]`
5. **Tracer E — 安装后状态 canary:** 从已签名安装包启动，写入设置 canary，退出/重开，确认 state path、schema 和值保持；升级/降级用 fixture 运行，不触碰真实供应商配置。`[VERIFIED: STATE-01 + packaging gate]`

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| 完整 SQLite snapshot | 手工复制主 DB/WAL/SHM | `rusqlite::backup::Backup` | SQLite 自己提供一致 snapshot 语义。`[CITED: https://sqlite.org/backup.html]` |
| SQL escaping | 字符串拼接 INSERT/UPDATE | rusqlite prepared statements/params | 避免注入和类型错误。`[CITED: https://docs.rs/rusqlite/0.40.1/rusqlite/]` |
| migration atomicity | 自定义 undo SQL | SQLite transaction rollback | 事务已经定义失败回滚边界。`[CITED: https://sqlite.org/lang_transaction.html]` |
| 随机 ID | 时间戳、名称、地址 hash | `uuid` v4 | 供应商和环境显示字段可变或重复。`[VERIFIED: ADR-0005 + spike 009]` |
| checksum | 自定义 rolling hash | `sha2::Sha256` | migration/fixture 需要稳定 64 hex digest。`[VERIFIED: crates.io + spike 012]` |
| 时间目录 | locale-dependent date string | UTC RFC3339/basic sortable timestamp via `chrono` | 备份 retention 不能依赖本地化或 mtime。`[VERIFIED: spike 013 pattern]` |
| Windows 原子替换 | 删除目标再 rename | `ReplaceFileW` via `windows-sys` | 避免无目标窗口；Spike 已验证 flags 必须为 0。`[VERIFIED: spike safe-config-editing reference]` |
| Codex 配置层合并 | GPTEasy 自己实现 precedence | `codex app-server config/read` fixture | 与目标 Codex 同源并能返回 origins/layers。`[VERIFIED: spike 008 + official source]` |
| 签名/公证 | 自定义签名格式 | Tauri bundler + SignTool / Apple codesign/notary | OS 信任链与 updater 签名不是同一问题。`[VERIFIED: Tauri signing docs + spike 005/017]` |

**Key insight:** 本阶段真正要手写的是“领域顺序与失败状态机”，不是数据库、hash、签名或配置层算法；底层 primitive 必须来自 SQLite、Tauri、OS 与 Codex 官方接口。`[VERIFIED: first-principles synthesis]`

## Common Pitfalls

### Pitfall 1: 备份文件存在但不可打开

**What goes wrong:** 只检查 `copy` 成功或文件大小非零，实际 backup 可能损坏或不属于 GPTEasy。`[VERIFIED: STATE-04 risk]`  
**Why it happens:** 把文件 I/O 成功误当成 SQLite snapshot 成功。`[VERIFIED: SQLite backup semantics]`  
**How to avoid:** backup 完成后独立只读打开，检查 `application_id`、源 `user_version`、`PRAGMA quick_check='ok'`；验证成功后才进入 migration 并裁剪旧备份。`[CITED: https://sqlite.org/pragma.html#pragma_quick_check]`  
**Warning signs:** backup retention 已减少，但新 backup 无法通过只读打开。`[VERIFIED: proposed verification]`

### Pitfall 2: 失败重启不断生成备份并挤掉真正历史备份

**What goes wrong:** 同一损坏 migration 每次启动都创建新快照，三次后 retention 只剩重复备份。`[VERIFIED: retention reasoning]`  
**How to avoid:** backup manifest 记录 source schema、target schema、source file identity/mtime/size 和 snapshot SHA-256；同一未变 source + 同一 target migration 可复用最近已验证 backup。`[VERIFIED: recommended policy]`  
**Warning signs:** 连续失败启动产生多个同 source/target 的相邻 backup。`[VERIFIED: proposed observability]`

### Pitfall 3: Higher schema refusal 仍创建 `-wal`/写事务

**What goes wrong:** 代码先走普通 `Connection::open`/初始化 PRAGMA，再发现版本太高。`[VERIFIED: rusqlite default open behavior]`  
**How to avoid:** existing DB 必须 `SQLITE_OPEN_READ_ONLY` preflight，兼容前不设置 WAL、不建表、不写 migration ledger。`[VERIFIED: STATE-05]`  
**Warning signs:** higher-schema refusal test 前后 DB/WAL hash 或 mtime 变化。`[VERIFIED: required test]`

### Pitfall 4: 把 Spike 012 的无 Key 数据库当产品 schema

**What goes wrong:** 应用重启后无法从 SQLite 恢复供应商凭据，违反 ADR-0006/STATE-01。`[VERIFIED: ADR-0006 + spike 012 schema]`  
**How to avoid:** v1 `providers.api_key` 明文持久化；日志/DTO/fixture 使用允许清单，不靠“不存 Key”逃避脱敏。`[VERIFIED: ADR-0001/0006]`  
**Warning signs:** repository `Provider` DTO 没有 secret field，或只在 Codex config 中保留 Key。`[VERIFIED: architecture check]`

### Pitfall 5: 迁移脚本发生非事务操作

**What goes wrong:** `VACUUM`、journal mode 切换或外部文件操作不能纳入预期事务回滚。`[VERIFIED: SQLite transaction constraints]`  
**How to avoid:** migration registry 只允许 transactional SQL/Rust data transforms；journal mode、optimize、backup retention 在事务外固定位置执行。`[VERIFIED: recommended migration policy]`  
**Warning signs:** migration SQL 包含 `VACUUM`、`ATTACH`、`DETACH`、`PRAGMA journal_mode`。`[VERIFIED: proposed lint]`

### Pitfall 6: 环境表绑定显示名称或 WSL username

**What goes wrong:** 重命名、重复 Ubuntu registration 或默认用户名尚未解析时，current provider 关联错对象。`[VERIFIED: spike 009 duplicate DistributionName evidence]`  
**How to avoid:** `managed_environments.id` 为 GPTEasy immutable UUID，`platform_identity` 保存 opaque native/registration identity；显示名和用户名是属性，不是主键。`[VERIFIED: ADR-0005 pattern + spike 009]`  
**Warning signs:** SQL FK 或 repository API 使用 `display_name` 查找环境。`[VERIFIED: architecture check]`

### Pitfall 7: Contract evidence 本身泄密

**What goes wrong:** 进程完整命令行、app-server raw response、SQLite dump 或配置正文进入 `.planning`/CI artifact。`[VERIFIED: spike evidence boundary]`  
**How to avoid:** evidence 使用字段允许清单；保存版本、hash、origin type、布尔判据和长度，不保存 raw response/command line。`[VERIFIED: spike 008/012 guidance]`  
**Warning signs:** fixture 中出现 `experimental_bearer_token`、`Authorization`、`command_line` 或 config TOML 正文。`[VERIFIED: required canary scan]`

## Code Examples

### Open Ready Connection Only After Compatibility

```rust
// Source basis:
// https://sqlite.org/pragma.html
// https://docs.rs/rusqlite/0.40.1/rusqlite/enum.TransactionBehavior.html
fn configure_ready_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "trusted_schema", false)?;
    conn.pragma_update(None, "synchronous", "FULL")?;

    let mode: String =
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    Ok(())
}
```

`foreign_keys` 是 per-connection setting；`trusted_schema=OFF` 限制 schema 对不安全函数/虚表的使用；WAL 是持久数据库属性，必须只在版本兼容后设置。`[CITED: https://sqlite.org/pragma.html]`

### Apply All Pending Migrations in One Transaction

```rust
// Source basis:
// https://sqlite.org/lang_transaction.html
fn migrate(
    conn: &mut Connection,
    from: u32,
    migrations: &[Migration],
    applied_at: &str,
) -> Result<(), StoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let observed: u32 =
        tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if observed != from {
        return Err(StoreError::ConcurrentSchemaChange { expected: from, observed });
    }

    for migration in migrations.iter().filter(|m| m.version > from) {
        tx.execute_batch(migration.sql)?;
        tx.execute(
            "INSERT INTO schema_migrations(version, name, checksum, applied_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![migration.version, migration.name, migration.checksum, applied_at],
        )?;
        tx.pragma_update(None, "user_version", migration.version)?;
    }

    ensure_no_rows(&tx, "PRAGMA foreign_key_check")?;
    let quick: String = tx.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick != "ok" {
        return Err(StoreError::IntegrityCheckFailed);
    }

    tx.commit()?;
    Ok(())
}
```

### Narrow Tauri Recovery Command

```rust
// Source basis:
// https://v2.tauri.app/develop/calling-rust/
#[tauri::command]
fn restore_database_backup(
    backup_id: String,
    recovery: tauri::State<'_, RecoveryService>,
) -> Result<BootstrapState, PublicStoreError> {
    // backup_id is an opaque ID from list_compatible_backups();
    // the UI never supplies an arbitrary filesystem path.
    recovery.restore_compatible(&backup_id).map_err(Into::into)
}
```

恢复 command 必须只接受后端枚举出的 opaque backup ID，并在后端重新验证 canonical path、application ID、schema version 和 quick check。`[VERIFIED: path traversal mitigation + STATE-05]`

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Spike target `codex-cli 0.146.0` | 稳定目标 `0.146.1` | 2026-08-05 15:55:06Z | 相关配置 schema source 未变，但必须重新跑 runtime fixture。`[VERIFIED: OpenAI releases API + compare API]` |
| 复制单个 SQLite 文件 | Online Backup API snapshot | SQLite 官方长期接口 | WAL 下仍得到 transactionally consistent、可打开备份。`[CITED: https://sqlite.org/backup.html]` |
| 迁移失败自动清库 | recovery-only + 原 DB 回滚 | 项目 ADR 锁定 | 数据损坏不再被假成功掩盖。`[VERIFIED: ADR-0006]` |
| 前端或 plugin 直连数据库 | Rust repository + narrow commands | ADR-0002/0006 | UI 无法绕过领域与 schema gate。`[VERIFIED: project ADRs]` |
| Windows/macOS 共用 roaming-ish app data 假设 | Windows 使用 local app data，macOS 使用 user Application Support | Tauri 2 PathResolver | 更贴合当前用户、本机默认存储边界。`[CITED: https://docs.rs/tauri/2.11.5/tauri/path/struct.PathResolver.html]` |
| 只保存 README 结论 | versioned machine-readable contract fixtures | Spike 017 evidence-level pattern | 依赖升级可自动 diff 并阻断。`[VERIFIED: spike 017]` |

**Deprecated/outdated:**

- `codex-cli 0.146.0` 不应再作为“当前最新”契约标签；仅保留为 historical fixture。`[VERIFIED: OpenAI releases API]`
- Spike 012 的无 API Key provider 表不适合作为产品 schema。`[VERIFIED: ADR-0006 + codebase grep]`
- `app_data_dir()` 不作为 Windows 状态根的首选；使用 `app_local_data_dir()`。`[CITED: https://docs.rs/tauri/2.11.5/tauri/path/struct.PathResolver.html]`

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| — | 本研究没有把未经验证的外部事实写成锁定实现；缺失事实均列为 blocker 或 fail-closed contract。`[VERIFIED: research audit]` | 全文 | — |

## Open Questions

1. **Codex 0.146.1 在目标二进制上的实际 app-server/config/provider 回归是否通过？**
   - What we know: 0.146.1 是截至 2026-08-05 的最新稳定版，关键 source/schema 文件与 0.146.0 相同。`[VERIFIED: OpenAI release + compare API]`
   - What's unclear: 本机只安装 0.146.0，正式宿主 bundled binary 也没有按 0.146.1 fixture 重跑。`[VERIFIED: environment probe + spikes]`
   - Recommendation: Phase 1 第一个契约任务必须安装/取得官方 0.146.1 隔离二进制并生成 schema + runtime fixture；失败则阻断 schema freeze。`[VERIFIED: risk boundary]`

2. **重复 WSL `DistributionName` 如何安全映射到 `wsl.exe -d NAME`？**
   - What we know: 当前主机 registry 有两个 GUID 都显示 Ubuntu，而 `wsl --list` 只展示一个 Ubuntu。`[VERIFIED: spike 009 evidence]`
   - What's unclear: Microsoft 公共 CLI 文档没有提供按 registration GUID 选择发行版的命令。`[CITED: https://learn.microsoft.com/en-us/windows/wsl/basic-commands]`
   - Recommendation: contract 固定为 `command_target_resolvable=false -> needs_attention`；不得猜映射。`[VERIFIED: fail-closed requirement]`

3. **真实 macOS 与正式签名资源何时可用？**
   - What we know: 当前 Windows 主机无法形成 macOS 14+、codesign、公证、Gatekeeper、LaunchServices 或 `~/Applications` 证据。`[VERIFIED: spike 017]`
   - What's unclear: 是否已有 Intel/Apple Silicon runner、Developer ID、notary credentials 和真实 Codex/ChatGPT host。`[VERIFIED: environment gap]`
   - Recommendation: planner 加 `checkpoint:human-action` 获取 runner/凭据；没有证据不得关闭 Phase 1。`[VERIFIED: project risk boundary]`

4. **Windows Authenticode 与 ARM64 build host 是否可用？**
   - What we know: x64 unsigned NSIS 已通过；`signtool.exe` 不在当前 PATH，VS ARM64 C++ tools 不存在。`[VERIFIED: environment probe + spike 005]`
   - What's unclear: 正式证书位置/CI secret 与 ARM64 runner。`[VERIFIED: environment gap]`
   - Recommendation: signed x64 和 signed ARM64 分开验收；不接受 updater `.sig` 代替 Authenticode。`[VERIFIED: Tauri docs + spike 005]`

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|-------------|-----------|---------|----------|
| Node.js | React/Vite/Tauri CLI | ✓ | `24.15.0` | — ` [VERIFIED: environment probe]` |
| npm | 前端依赖与 scripts | ✓ | `11.14.1` | — ` [VERIFIED: environment probe]` |
| Rust / Cargo | 后端与测试 | ✓ | `1.97.1` | — ` [VERIFIED: environment probe]` |
| MSVC x64 build tools | Windows x64 Tauri | ✓ | VS Build Tools `17.12.4` | — ` [VERIFIED: environment probe]` |
| Rust Windows x64 target | Windows x64 | ✓ | `x86_64-pc-windows-msvc` | — ` [VERIFIED: rustup probe]` |
| Rust Windows ARM64 target | Windows ARM64 | ✓ | `aarch64-pc-windows-msvc` | 仍缺 ARM64 C++ tools。`[VERIFIED: rustup + VS component probe]` |
| VS ARM64 C++ tools | Windows ARM64 bundle | ✗ | — | 原生 ARM64 CI/runner；blocking。`[VERIFIED: environment probe]` |
| Tauri CLI global | build | ✗ | — | 使用 pinned local `@tauri-apps/cli@2.11.4`。`[VERIFIED: environment probe + npm registry]` |
| NSIS | Windows current-user bundle | ✓ cached | Tauri cache `makensis.exe` | local Tauri CLI 调用。`[VERIFIED: environment probe]` |
| SignTool / Authenticode certificate | signed Windows smoke | ✗ / unknown | — | 需要正式 Windows signing runner；blocking。`[VERIFIED: environment probe]` |
| Codex target CLI | contract fixture | ✗ exact target | installed `0.146.0`, required `0.146.1` | 下载/安装官方 0.146.1 到隔离 harness；blocking until run。`[VERIFIED: environment + OpenAI release API]` |
| WSL2 | WSL contract probe | ✓ | `2.5.7.0` | — ` [VERIFIED: spike/environment probe]` |
| `sqlite3` CLI | 手工 DB 检查 | ✗ | — | `rusqlite` bundled SQLite 与 Rust tests；not blocking。`[VERIFIED: environment + crate features]` |
| macOS 14+ Intel host | mac host/signing | ✗ | — | 原生 Intel runner/real Mac；blocking。`[VERIFIED: spike 017]` |
| macOS 14+ Apple Silicon host | mac host/signing | ✗ | — | 原生 ARM64 runner/real Mac；blocking。`[VERIFIED: spike 017]` |
| Apple Developer ID / notarization credentials | signed macOS smoke | ✗ / unknown | — | 需要 CI secret 与真实签名候选；blocking。`[VERIFIED: spike 017 + Tauri docs]` |

**Missing dependencies with no fallback:**

- 正式 Windows Authenticode 资源、Windows ARM64 build host、真实 macOS Intel/Apple Silicon 与 Apple signing/notary 资源；这些阻断“契约冻结”和 Phase 1 完成。`[VERIFIED: project risk boundary + environment probe]`

**Missing dependencies with fallback:**

- 全局 Tauri CLI 用 local npm devDependency；`sqlite3` CLI 用 bundled `rusqlite` 测试；本机 Codex 0.146.0 可保留为 historical fixture，但不能替代 0.146.1 target run。`[VERIFIED: environment audit]`

## Validation Architecture

### Test Framework

| Property | Value |
|----------|-------|
| Framework | Rust built-in test harness via Cargo `1.97.1`；前端使用 `tsc + vite build`；桌面边界使用 Tauri `2.11.5` debug/release smoke。`[VERIFIED: environment + official Tauri tests docs]` |
| Config file | `src-tauri/Cargo.toml`、`vite.config.ts`、`tsconfig.json`；当前均不存在，Wave 0 创建。`[VERIFIED: codebase scan]` |
| Quick run command | `cargo test --manifest-path src-tauri/Cargo.toml state_` `[VERIFIED: proposed naming convention]` |
| Full suite command | `cargo test --manifest-path src-tauri/Cargo.toml --all-targets && npm run build`，再由目标平台 job 执行 Tauri bundle/安装 smoke。`[VERIFIED: official Tauri testing guidance + platform boundary]` |

### Phase Requirements → Test Map

| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| STATE-01 | provider、verification、environment current provider、settings 在 close/reopen 后逐字段一致 | Rust integration | `cargo test --manifest-path src-tauri/Cargo.toml --test state_persistence` | ❌ Wave 0 |
| STATE-02 | DB/backups/log roots 位于 `app_local_data_dir`；无账户表、无 HTTP/shell/fs plugin capability；Key 不进入 public snapshot/evidence | integration + static gate | `cargo test --manifest-path src-tauri/Cargo.toml --test local_only_boundary` | ❌ Wave 0 |
| STATE-03 | 每个 committed historical fixture 顺序升级到 current，migration ledger/checksum/user_version 一致 | fixture matrix | `cargo test --manifest-path src-tauri/Cargo.toml --test migration_matrix` | ❌ Wave 0 |
| STATE-04 | 迁移前 backup 可打开、quick_check ok、只留三份；注入失败后原 schema/rows/user_version 不变且无 reset | fault integration | `cargo test --manifest-path src-tauri/Cargo.toml --test migration_failure --test backup_restore` | ❌ Wave 0 |
| STATE-05 | higher schema 只读拒绝；普通 commands 不可用；兼容 pre-upgrade backup 可显式恢复 | recovery integration | `cargo test --manifest-path src-tauri/Cargo.toml --test higher_schema_refusal` | ❌ Wave 0 |
| Phase risk gate | Codex 0.146.1 schema/runtime、host、WSL、signed packages fixture 完整且脱敏 | contract/smoke | `powershell -File scripts/contracts/run-phase1-contracts.ps1` / `zsh scripts/contracts/run-macos.sh` | ❌ Wave 0 |

### Required Test Scenarios

- `state_restart_round_trip`：写入两个供应商、一个验证记录、native + WSL 环境及不同当前供应商、设置；drop `StateStore`，重开后深比较。`[VERIFIED: STATE-01]`
- `migration_all_historical_fixtures`：枚举 manifest 而不是手工列版本；每个 fixture copy 到 temp 后升级。`[VERIFIED: ADR-0006]`
- `migration_failure_rolls_back_all_pending_versions`：从 v1 模拟 v2/v3，v3 故障后必须仍是 v1，不接受停在 v2。`[VERIFIED: STATE-04]`
- `backup_is_openable_and_retains_three`：每次 backup 独立 read-only open + quick_check，第四次后最老一份删除。`[VERIFIED: STATE-04]`
- `higher_schema_refusal_is_non_mutating`：记录 DB/WAL hash 与 mtime，open 返回 `DatabaseTooNew`，不创建 migration/backup。`[VERIFIED: STATE-05]`
- `downgrade_restore_preserves_newer_db_quarantine`：恢复兼容 backup 前把 newer DB 保留为 quarantine，不覆盖唯一新数据副本。`[VERIFIED: non-destructive recovery recommendation]`
- `evidence_canary_scan`：contract/diagnostic fixture 扫描固定假 Key、`experimental_bearer_token`、Authorization 与完整 command line 字段。`[VERIFIED: project secret boundary + spike discrepancy]`

### Sampling Rate

- **Per task commit:** `cargo test --manifest-path src-tauri/Cargo.toml state_`。`[VERIFIED: proposed quick suite]`
- **Per wave merge:** `cargo test --manifest-path src-tauri/Cargo.toml --all-targets && npm run build`。`[VERIFIED: proposed full suite]`
- **Phase gate:** 全部 Rust tests + frontend build + Codex 0.146.1 contract + Windows signed x64/ARM64 install/restart smoke + macOS Intel/Apple Silicon signed/notarized smoke 全绿。`[VERIFIED: project risk boundary]`

### Wave 0 Gaps

- [ ] `src-tauri/Cargo.toml` / `src-tauri/src/lib.rs` — Rust/Tauri testable composition root。`[VERIFIED: codebase currently greenfield]`
- [ ] `src-tauri/tests/state_persistence.rs` — STATE-01。`[VERIFIED: required map]`
- [ ] `src-tauri/tests/local_only_boundary.rs` — STATE-02。`[VERIFIED: required map]`
- [ ] `src-tauri/tests/migration_matrix.rs` — STATE-03。`[VERIFIED: required map]`
- [ ] `src-tauri/tests/migration_failure.rs` — STATE-04 rollback。`[VERIFIED: required map]`
- [ ] `src-tauri/tests/backup_restore.rs` — STATE-04 backup/retention/restore。`[VERIFIED: required map]`
- [ ] `src-tauri/tests/higher_schema_refusal.rs` — STATE-05。`[VERIFIED: required map]`
- [ ] `tests/fixtures/databases/v001/state.sqlite3` + `manifest.json` — 第一份永久历史样本。`[VERIFIED: ADR-0006]`
- [ ] `scripts/contracts/run-phase1-contracts.ps1` 与各 target probe。`[VERIFIED: phase risk boundary]`
- [ ] npm package `checkpoint:human-verify` — legitimacy `SUS` 包安装前门禁。`[VERIFIED: package-legitimacy seam]`

## Security Domain

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|---------------|---------|-----------------|
| V2 Authentication | no | 产品没有 GPTEasy 账户；不新增账户/session 表。`[VERIFIED: ADR-0007]` |
| V3 Session Management | no | Phase 1 无产品 session。`[VERIFIED: phase scope]` |
| V4 Access Control | yes | Tauri narrow command、最小 capability、Rust 后端唯一 DB 入口、当前用户数据根。`[VERIFIED: ADR-0002/0006 + Tauri capabilities model]` |
| V5 Input Validation | yes | Rust typed DTO、domain validation、prepared SQL、DB CHECK/FK、opaque backup ID。`[VERIFIED: proposed architecture + rusqlite]` |
| V6 Cryptography | yes | `sha2` 只用于 checksum/fingerprint；不手写 crypto；凭据明文是已接受产品决策而非遗漏。`[VERIFIED: ADR-0001 + crates.io]` |
| V8 Data Protection | yes | DB/backup 当前用户目录、Unix 0600/0700、Windows inherited user ACL、日志/fixture allowlist 与 canary scan。`[VERIFIED: project constraints + spike 013]` |

### Known Threat Patterns for Tauri/Rust/SQLite

| Pattern | STRIDE | Standard Mitigation |
|---------|--------|---------------------|
| SQL injection through names/settings | Tampering | rusqlite params；不允许 raw SQL command。`[CITED: https://docs.rs/rusqlite/0.40.1/rusqlite/]` |
| 恶意/错误高版本 DB 诱导旧 app 写入 | Tampering | read-only preflight + `application_id` + higher-schema recovery-only。`[VERIFIED: STATE-05]` |
| migration script 被历史修改 | Tampering | append-only files、compiled checksum、historical fixture manifest、CI diff gate。`[VERIFIED: ADR-0006]` |
| backup/path traversal | Tampering / Information Disclosure | UI 只传 opaque ID；后端 canonicalize 并限制在 backup root。`[VERIFIED: proposed recovery API]` |
| 明文 API Key 从日志/fixture 泄漏 | Information Disclosure | 字段允许清单、禁止 raw config/app-server/command line、fixed canary scan。`[VERIFIED: ADR-0001 + spike 008/012]` |
| WebView/XSS 直接操作 DB | Elevation of Privilege | 无 SQL/fs/shell/http plugin capability；只暴露窄 command。`[VERIFIED: ADR-0002 + Tauri architecture]` |
| 两个实例同时迁移 | Denial of Service / Tampering | `BEGIN IMMEDIATE`、事务内再次读取 user_version、busy timeout；加入双进程启动测试。`[VERIFIED: spike 007 + SQLite transaction behavior]` |
| 假签名/未签名工件冒充完成 | Spoofing | Windows Authenticode、Apple codesign/notary/Gatekeeper 与 Tauri updater 签名分别记录。`[VERIFIED: Tauri docs + spike 005/017]` |

## Sources

### Primary (HIGH confidence)

- `CONTEXT.md`、`.planning/PROJECT.md`、`.planning/REQUIREMENTS.md`、`.planning/ROADMAP.md`、`.planning/STATE.md`、`docs/adr/0001-0008`、`docs/ui/UI-SPEC.md` — 锁定领域、范围、状态与 UI 基线。`[VERIFIED: codebase read]`
- `.codex/skills/spike-findings-gpteasy/` 与 Spikes 001/004/005/007/008/009/012/013/017 — Windows、WSL2、Codex、Tauri 与打包实验。`[VERIFIED: codebase read + evidence JSON]`
- https://sqlite.org/backup.html — Online Backup API consistency。`[CITED: https://sqlite.org/backup.html]`
- https://sqlite.org/pragma.html — `user_version`、`application_id`、`foreign_keys`、`quick_check`、`trusted_schema`、`synchronous`。`[CITED: https://sqlite.org/pragma.html]`
- https://sqlite.org/lang_transaction.html — transaction/rollback。`[CITED: https://sqlite.org/lang_transaction.html]`
- https://sqlite.org/wal.html — WAL persistence/concurrency。`[CITED: https://sqlite.org/wal.html]`
- https://docs.rs/rusqlite/0.40.1/rusqlite/backup/struct.Backup.html — Rust backup API。`[CITED: https://docs.rs/rusqlite/0.40.1/rusqlite/backup/struct.Backup.html]`
- https://v2.tauri.app/develop/calling-rust/ 与 https://v2.tauri.app/develop/state-management/ — command/state boundary。`[CITED: https://v2.tauri.app/develop/calling-rust/]`
- https://v2.tauri.app/distribute/windows-installer/ 与 https://v2.tauri.app/distribute/sign/windows/ — NSIS/currentUser/signing。`[CITED: https://v2.tauri.app/distribute/windows-installer/]`
- https://v2.tauri.app/distribute/macos-application-bundle/ 与 https://v2.tauri.app/distribute/sign/macos/ — `.app`、签名、公证。`[CITED: https://v2.tauri.app/distribute/macos-application-bundle/]`
- https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/utils/home-dir/src/lib.rs — Codex home。`[CITED: official OpenAI source]`
- https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/model-provider-info/src/lib.rs — provider/auth fields。`[CITED: official OpenAI source]`
- https://raw.githubusercontent.com/openai/codex/rust-v0.146.1/codex-rs/app-server-protocol/src/protocol/v2/config.rs — config/read/layers/precedence。`[CITED: official OpenAI source]`
- https://api.github.com/repos/openai/codex/releases/latest 与 compare API — 0.146.1 release 和 source diff。`[VERIFIED: official GitHub API]`
- https://learn.microsoft.com/en-us/windows/wsl/basic-commands — WSL public CLI guarantees。`[CITED: official Microsoft docs]`

### Secondary (MEDIUM confidence)

- npm registry、crates.io API 与 `cargo info/search` — exact versions、publish dates、repos、downloads 和 features。`[VERIFIED: registry tools]`
- Official `tauri-apps/create-tauri-app` tag `create-tauri-app-js-v4.7.3` React TS template — frontend exact baseline。`[VERIFIED: official repository source]`

### Tertiary (LOW confidence)

- None；所有未闭合宿主事实均保留为 blocker，没有升级为实现事实。`[VERIFIED: research audit]`

## Metadata

**Confidence breakdown:**

- Standard stack: **MEDIUM** — Rust/Tauri/SQLite 由 registry、官方 docs 与 Spike 交叉验证；npm legitimacy seam 因近期发布将 React/Vite/TypeScript 包标为 SUS，需人工 checkpoint。`[VERIFIED: registry + package gate]`
- Architecture: **HIGH for SQLite state core / MEDIUM overall** — SQLite 事务、backup、preflight 与项目 ADR 明确；真实 macOS、签名工件和目标 Codex runtime 尚未闭合。`[VERIFIED: official docs + blockers]`
- Pitfalls: **HIGH** — 主要来自本项目已运行 Spike、原始 evidence 和 SQLite/Tauri/OpenAI/Microsoft 官方接口。`[VERIFIED: codebase + official docs]`

**Research date:** 2026-08-05 `[VERIFIED: orchestrator date]`  
**Valid until:** 2026-08-12 — Codex/host/Tauri/registry 属快速变化契约，建议 7 天后或任一目标版本变化时重新验证。`[VERIFIED: project contract-change risk]`
