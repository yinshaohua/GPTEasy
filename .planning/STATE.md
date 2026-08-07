---
gsd_state_version: 1.0
milestone: v1.0
milestone_name: milestone
current_phase: 01
current_phase_name: trusted-local-state-contract
status: executing
stopped_at: Completed 01-19-PLAN.md; next 01-20-PLAN.md
last_updated: "2026-08-07T06:42:12.842Z"
last_activity: 2026-08-07
last_activity_desc: 完成 01-19 installed state smoke、本地边界与跨进程协调
progress:
  total_phases: 1
  completed_phases: 0
  total_plans: 28
  completed_plans: 19
---

# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-08-05)

**Core value:** 非技术用户能够验证供应商，并在保留既有 Codex 配置且可恢复的前提下，可靠地切换各受管环境使用的 API 服务。
**Current focus:** Phase 01 — trusted-local-state-contract

## Current Position

Phase: 01 (trusted-local-state-contract) — EXECUTING
Plan: 20 of 28
Status: Ready to execute
Last activity: 2026-08-07 — 完成 01-19 installed state smoke、本地边界与跨进程协调

Progress: [███████░░░] 68%

## Performance Metrics

**Velocity:**

- Total plans completed: 19
- Average duration: 36m
- Total execution time: 11h 26m

**By Phase:**

| Phase | Plans | Total | Avg/Plan |
|-------|-------|-------|----------|
| Phase 01 | 19 | 11h 26m | 36m |

**Recent Trend:**

- Last 5 plans: 1h 25m, 1h 14m, 33m, 25m, 1h 06m
- Trend: Variable due to TDD cold builds and cross-process coordination coverage

*Updated after each plan completion*
**Per-Plan Metrics:**

| Plan | Duration | Tasks | Files |
|------|----------|-------|-------|
| Phase 01 P01 | 45m | 2 tasks | 3 files |
| Phase 01 P03 | 35m | 2 tasks | 3 files |
| Phase 01 P04 | 22m | 2 tasks | 7 files |
| Phase 01 P05 | 14m | 2 tasks | 3 files |
| Phase 01 P07 | 36m | 2 tasks | 4 files |
| Phase 01 P02 | 22m | 1 tasks | 0 files |
| Phase 01 P08 | 约 15 分钟 | 2 tasks | 8 files |
| Phase 01 P11 | 约 38 分钟 | 2 tasks | 5 files |
| Phase 01 P12 | 约 20 分钟 | 1 tasks | 5 files |
| Phase 01 P06 | 约 10 分钟（不含人工等待） | 1 tasks | 1 files |
| Phase 01 P09 | 约 26 分钟 | 2 tasks | 8 files |
| Phase 01 P10 | 40min | 2 tasks | 5 files |
| Phase 01 P13 | 31min | 2 tasks | 7 files |
| Phase 01 P14 | 49min | 2 tasks | 8 files |
| Phase 01 P15 | 1h 25m | 2 tasks | 5 files |
| Phase 01 P16 | 1h 14m | 3 tasks | 1 files |
| Phase 01 P17 | 33m | 1 tasks | 5 files |
| Phase 01 P18 | 25m | 1 tasks | 6 files |
| Phase 01 P19 | 1h 06m | 2 tasks | 7 files |

## Accumulated Context

### Decisions

Decisions are logged in PROJECT.md Key Decisions table.
Recent decisions affecting current work:

- [Roadmap]: `CONTEXT.md`、`docs/adr/` 和 `docs/ui/UI-SPEC.md` 是锁定基线，不重新打开其领域、架构、产品或 UI 决策。
- [Roadmap]: 采用用户选择的 Horizontal Layers 顺序，先完成数据与配置安全基础，再进入供应商、平台集成、统一 UI 和发布门禁。
- [Roadmap]: 8 个阶段均为内部实施边界；完整 v1 只在 Phase 8 全部门禁通过后一次发布。
- [Phase 1]: 01-01：npm 身份门禁固定公开 registry、空 user/global config 和无祖先 .npmrc 的隔离 cwd；验证结果 fail-closed。
- [Phase 1]: 01-01：测试必须复用生产 verifier predicate，并覆盖恶意 npm 配置、私有 registry、token canary 与伪造 metadata。
- [Phase 1]: 01-03：runner-cli-matrix.json 作为 Scope/Target/Mode、合法组合、dispatch 和退出码的唯一事实源。
- [Phase 1]: 01-03：RunnerSelfTest/ProvenanceSelfTest/Freeze/PhaseComplete 只允许 Local+Strict，AllowBlocked 只允许具体目标的 ContractSelfTest/PackagingSelfTest。
- [Phase 1]: 01-03：资源阻断时 Strict 返回 3，AllowBlocked 返回 0 但 outcome=blocked 且 strict_gate_eligible=false。
- [Phase 1]: 01-04：manifest validator 只证明结构与脱敏边界，永远不自行授予 strict eligibility。
- [Phase 1]: 01-04：API artifact archive digest 与下载文件/attestation subject digest 分开交叉验证。
- [Phase 1]: 01-04：Transcript 仅用于生产 predicate 自测，test_only=true 且 strict_gate_eligible=false。
- [Phase 1]: 01-05：gh evidence preflight 固定 github.com、yinshaohua/GPTEasy 与最低版本 2.49.0，调用方不能放宽策略。
- [Phase 1]: 01-05：repository、Actions 与 attestation API 均显式只读；固定不存在 digest 的 404 表示 endpoint 可读，401/403 阻断。
- [Phase 1]: 01-05：preflight 始终 artifact_verified=false；fixture 仅用于生产 parser 自测且不能获得 strict eligibility。
- [Phase 1]: 01-07：digest lock 固定 34 个规划/合同输入并排除自身；仅显式 -UpdateLock 可写，门禁只用 -ReadOnly。
- [Phase 1]: 01-07：SOURCE-AUDIT 的 PLANNED/COVERED 状态不授予通过，requirement、拓扑、路径、CLI、key link、threat 与 digest 必须实时重算。
- [Phase 1]: 01-07：PhaseComplete 在 aggregate 前强制执行只读来源审计，审计非零直接 fail-closed。
- [Phase 01]: 01-02：人工官方来源 checkpoint 结论为 approved；批准严格覆盖 allowlist 中的 7 个 package@version、官方 repository 与官方 create-tauri-app React TypeScript 模板来源。 — 机器 exact allowlist verifier 与真实公开 registry 查询均通过，但发布者和官方模板来源属于不可自动替代的人工门禁；用户已明确回复 approved，因此仅解除 01-08 的首次安装前置条件。
- [Phase 1]: 01-08：前端依赖使用 exact pins，React 仅作为无业务状态的单一 root；Vite 8 使用内置 Oxc minifier，避免额外 esbuild 依赖。
- [Phase 01]: Codex 探针固定 0.146.1，app-server 只执行 initialize、initialized、config/read(includeLayers=true)，raw response、配置正文和凭据值不进入证据。
- [Phase 01]: Windows 正式宿主身份固定为 OpenAI.Codex_2p2nqsd0c76g0 与 app/resources/codex.exe，调用方不能放宽 allowlist。
- [Phase 01]: WSL2 探针只允许 version/list/list-running 固定只读调用，重复 DistributionName 固定 command_target_resolvable=false。
- [Phase 01]: macOS Codex probe 固定 0.146.1 与 initialize/initialized/config-read(includeLayers=true)，原始响应和配置正文不进入证据。 — 保持与 Windows manifest schema 一致并满足 T-01-27 脱敏边界。
- [Phase 01]: macOS 正式宿主候选固定 Codex.app/ChatGPT.app 与 Contents/Resources/codex，fixture 永不获得 strict eligibility。 — 防止调用方放宽宿主身份或用 synthetic fixture 冒充原生证据。
- [Phase 01]: macOS Wave 0 固定 exact checkout，并在 macos-15 arm64 与 macos-15-intel x86_64 上先执行全部 zsh 语法与 fixture tests。 — 任何后续 macOS evidence 必须依赖可复用 Wave 0，不能以 Windows 静态结果替代。
- [Phase 01]: 01-06：接受 PowerShell 7.6.4 下真实只读 gh preflight 成功结果并解除 blocking-human checkpoint；不宣称 Windows PowerShell 5.1 已完成同等真实网络兼容性验证。
- [Phase 01]: 01-06：后续真实 evidence 仍须执行固定 preflight，任何非零结果在下载或验签前 fail-closed，历史批准不能覆盖当前失败。
- [Phase 01]: 01-09：Rust/Tauri composition root 保持最小，main 只调用 library run，不注册业务 command 或高权限 plugin。
- [Phase 01]: 01-09：capability 仅授权 main window 的 core:default；Windows NSIS 使用 currentUser，macOS minimumSystemVersion 固定 14.0。
- [Phase 01]: 01-10：path smoke 固定使用 app_local_data_dir/contract-smoke/path 与 1–64 位 ASCII opaque ID。 — 调用者不能传入路径；marker/report 仅含 run_id、OS、arch、schema 与 reopened。
- [Phase 01]: 01-10：跨进程 reopen 必须由真实 Tauri mock AppHandle 和当前 integration-test executable 的独立子进程证明。 — Windows test target 显式链接 tauri-build resource，避免测试宿主与正式应用清单漂移。
- [Phase 01]: 01-13：Windows package strict pass 同时要求 Authenticode、目标 PE 架构、current-user 路径、跨进程 reopen、profile digest 绑定与 finalized cleanup attestation。
- [Phase 01]: 01-13：一次性账户证据绑定 GitHub run/attempt/job、runner tracking、SID/profile；marker 不能单独证明 disposable。
- [Phase 01]: 01-13：PackagingSelfTest Local 必须实际运行 Windows 正负例与 macOS Wave 0 contract，不能只检查脚本存在。
- [Phase 1]: 01-14：macOS strict pass 同时要求 Developer ID codesign、公证 stapling、Gatekeeper、HOME_APPLICATIONS、path smoke、archive/app 关联与 finalized cleanup。
- [Phase 1]: 01-14：一次性账户证据绑定 repository/run/attempt/job、runner tracking/arch、UID/profile；状态与 evidence 不保存账户密码或 Apple 凭据。
- [Phase 1]: 01-14：最终上传 archive 必须解包重验签并与 build app 关联，fixture 永远 test_only 且不能获得 strict eligibility。
- [Phase 1]: 01-14：Intel/Apple Silicon evidence matrix 统一 needs Wave 0；本地 Windows 静态验证不能替代真实 macOS/Apple/GitHub evidence。
- [Phase 01]: 01-15：Freeze 只执行六类非签名合同并保持 release_ready=false；Windows Authenticode 与 macOS Developer ID/notary 只记录为 deferred。 — 本地 schema/backup 决策可以继续，但不得冒充发布身份。
- [Phase 01]: 01-15：PhaseComplete 独立要求八个四目标 evidence-set 和实时 strict provenance；Freeze、fixture 与 test-only override 均不能晋升。 — 最终门继续绑定真实签名、公证、attestation 与当前工件。
- [Phase 01]: 01-15：-Matrix 只用于负例自测，结果固定 test_only=true、strict_gate_eligible=false、release_ready=false。 — 防止测试矩阵成为正式发布旁路。
- [Phase 01]: 01-16：用户明确批准 freeze-approved、approve-schema-version-1 与 approve-db-backup-contract；六表 schema v1 和 verified backup/quarantine 合同成为后续 one-way 实现前置。 — 正式 Windows/macOS 签名与公证继续延期到 01-26/01-27，release_ready=false 且 PhaseComplete 仍 blocked。
- [Phase 01]: 01-17：APPLICATION_ID 固定为 0x47505445（ASCII GPTE），schema fingerprint 绑定版本化域、application ID、user_version 与 0001 checksum。 — 两个独立 OS 子进程均按注册 command 名走 Tauri mock IPC；run ID 只用于测试关联，不进入永久 schema。
- [Phase 01]: 01-18：ProviderId 与 EnvironmentId 只接受 UUID，显示名、地址、模型、Key 和平台身份都不承担主键语义。 — 确保可编辑字段不会破坏供应商或环境引用。
- [Phase 01]: 01-18：组合指纹绑定 base URL、默认模型与 API Key，验证记录不匹配时在写入前拒绝。 — 确保验证证据只能用于同一关键配置组合。
- [Phase 01]: 01-18：公开 state digest 使用版本化 canonical 编码覆盖完整 secret-bearing snapshot。 — 无需公开秘密即可证明跨进程恢复的内部状态逐字段一致。
- [Phase 01]: 01-18：完整 snapshot 在单个 SQLite IMMEDIATE transaction 内整体替换并提交后权威重读。 — 任何约束或完整性失败都回滚，避免部分新状态。
- [Phase 01]: 01-19：state smoke 根固定由 app_local_data_dir 与 opaque run ID 派生，verify 不清理，cleanup 必须通过 marker 与文件允许清单。
- [Phase 01]: 01-19：StateStore 在任何 DB/WAL/backup 写 seam 前取得 OS exclusive File lock，并持有到 SQLite Connection 销毁。
- [Phase 01]: 01-19：owner metadata 仅含 PID、进程启动 token 与 run ID 摘要，ownership 只由 File::try_lock 决定。
- [Phase 01]: 01-19：local-only gate 对依赖、capability、注册 command、公开 DTO 与前端 API surface 使用精确允许清单。

### Pending Todos

None yet.

### Blockers/Concerns

- [Phase 1]: 目标 Codex 版本的配置路径、字段、认证模式、供应商凭据载体和配置优先级必须通过实机与 fixture 冻结。
- [Phase 5]: 停止 WSL2 环境的 WSL2 默认用户被动发现和 WSL2 临时启动所有权仍需真实环境验证。
- [Phase 8]: Windows ARM64、macOS 当前用户安装、正式签名/公证、Tauri updater 密钥与 N-1/N-2 升级链路需要正式工件验证。

## Deferred Items

Items acknowledged and carried forward from previous milestone close:

| Category | Item | Status | Deferred At |
|----------|------|--------|-------------|
| *(none)* | | | |

## Session Continuity

Last session: 2026-08-07T06:42:12.828Z
Stopped at: Completed 01-19-PLAN.md; next 01-20-PLAN.md
Resume file: None
Previous resume context: 2026-08-07T05:20:18.248Z — Session resumed, proceeding to execute 01-19-PLAN.md (`.planning/phases/01-trusted-local-state-contract/.continue-here.md`)
