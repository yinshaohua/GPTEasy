# Phase 1 多来源覆盖审计

**Authored:** 2026-08-05  
**Revision:** 3（checker topology and semantic mapping revision）
**Planning Result:** PLANNED-COVERAGE — 当前映射无 MISSING；不得解释为执行通过  
**Final Pass Dependency:** 01-07 机器只读审计 + 01-28 PhaseComplete Local Strict

SOURCE-AUDIT.md 的文本状态不参与最终通过判定。执行期必须由 audit-phase1-plan-source.ps1 重新解析 requirement、plan/task/path、runner CLI、依赖/wave、threat disposition 与当前 SHA-256 digests。

## Multi-Source Coverage Audit

| SOURCE | ID | Item | Plan(s) | Status |
|--------|----|------|---------|--------|
| GOAL | — | 本地状态可靠持久化，并在升级/降级边界保持可恢复 | 17–28 | PLANNED |
| REQ | STATE-01 | 完整 provider/verification/native+WSL/current provider/settings 重开保持 | 01–03, 07–10, 13–19, 25–28 | PLANNED |
| REQ | STATE-02 | 当前用户本地、无产品账户/默认上传、凭据公开边界 | 01–15, 17–19, 25–28 | PLANNED |
| REQ | STATE-03 | append-only 顺序迁移、历史 fixture、双账本与禁止项 | 03–07, 11–16, 19–23, 25–28 | PLANNED |
| REQ | STATE-04 | verified backup、三份 retention、全 pending rollback、恢复前保全 | 03–07, 13–16, 19, 22–28 | PLANNED |
| REQ | STATE-05 | higher-schema non-mutating 拒写与 backup/quarantine restore | 03–07, 11–16, 19, 22, 24–28 | PLANNED |
| RESEARCH | R-01 | canonical Scope/Target/Mode/exit-code runner contract | 03, 07, 28 | PLANNED |
| RESEARCH | R-02 | package legitimacy machine + human gate | 01, 02, 08 | PLANNED |
| RESEARCH | R-03 | isolated public npm registry，不受 .npmrc/token 影响 | 01 | PLANNED |
| RESEARCH | R-04 | attested external evidence provenance | 04–07, 13–15, 25–28 | PLANNED |
| RESEARCH | R-05 | fixed-minimum gh auth/repo/actions/attestation preflight | 05, 06, 26, 27 | PLANNED |
| RESEARCH | R-06 | Windows official CLI 与正式宿主共享 canary parity | 11, 13, 25, 26 | PLANNED |
| RESEARCH | R-07 | macOS Intel/Apple Silicon zsh Wave 0 与 CLI/宿主 parity | 12, 14, 25, 27 | PLANNED |
| RESEARCH | R-08 | disposable runner/account identity + attested delete/restore | 13, 14, 25–27 | PLANNED |
| RESEARCH | R-09 | package predicate 正控制且 test-only 不 strict eligible | 13, 14 | PLANNED |
| RESEARCH | R-10 | true production Tauri command tracer | 17 | PLANNED |
| RESEARCH | R-11 | full state installed smoke 不提前 cleanup | 19, 25–27 | PLANNED |
| RESEARCH | R-12 | inter-process open/migrate/backup/restore coordination | 19, 23, 24 | PLANNED |
| RESEARCH | R-13 | migration SQL/data-transform prohibition lint | 21, 23 | PLANNED |
| RESEARCH | R-14 | deterministic create-before-verify fixture/history lock | 20, 21 | PLANNED |
| RESEARCH | R-15 | unified read-only DB contract validator | 22–24 | PLANNED |
| RESEARCH | R-16 | backup identity/reuse/retention/quarantine | 16, 23, 24 | PLANNED |
| RESEARCH | R-17 | machine-readable plan/source/path/CLI/digest audit | 07, 15, 28 | PLANNED |
| CONTEXT | CTX-01 | 明文 API Key 保存且日志/证据/公开 DTO 脱敏 | 01, 04, 11–15, 18, 19, 25–28 | PLANNED |
| CONTEXT | CTX-02 | immutable provider ID 与环境独立 current provider | 18, 19, 25–28 | PLANNED |
| CONTEXT | CTX-03 | 原生 Codex 与 WSL2 独立建模 | 11, 18, 19, 25, 26, 28 | PLANNED |
| CONTEXT | CTX-04 | 当前用户安装与完全本地模式 | 08–28 | PLANNED |
| ADR | ADR-0002 | Tauri 2 + Rust + React，Rust 唯一状态权威 | 08–25 | PLANNED |
| ADR | ADR-0006 | SQLite、永久迁移、backup、higher-schema/recovery | 16–28 | PLANNED |
| ADR | ADR-0001/0005/0007/0008 | 凭据、身份、本地、native 环境 | 11–19, 25–28 | PLANNED |
| PATTERNS | P-01 | 101 个 concrete path 全部 Analog/No Analog 分类；无关键 glob | 01–28 | PLANNED |
| VALIDATION | V-01 | 每 task 自动验证、原生 macOS Wave 0、正控制、两进程与 final audit | 01–28 | PLANNED |

## Checker Issue → Exact Fix Mapping

| # | Checker dimension | Exact plan/document fix |
|---|-------------------|-------------------------|
| 1 | dependency_correctness | 01-13 在同一 Task 1 创建 Windows guard/verifier/test 后才运行测试；01-20 先创建 v001/state.sqlite3 与 manifest，01-21 才运行 migration_matrix。 |
| 2 | cross_plan_data_contracts | 01-03 定义唯一 Scope/Target/Mode/exit-code matrix；所有 28 份计划命令均显式使用声明组合，consumer scan 拒绝旧 Require* 参数和 Scope=Packaging。 |
| 3 | nyquist_compliance | 01-12 创建 Intel/Apple Silicon reusable macOS Wave 0，真实执行 zsh -n 与 zsh tests；01-14/25/27 明确 needs wave0；01-12 task verify 读取全部修改脚本/workflow。 |
| 4 | external_contract_coverage | 01-11 实现 Windows official CLI 与正式宿主 bundled Codex 的 disposable-user app-server canary parity；01-13/25/26 将 parity 设为 strict workflow/manifest 条件。 |
| 5 | execution_preconditions | 01-05 实现 gh >= 2.49.0、auth、repo/actions/attestation read preflight；01-06 是首次真实 artifact verification 前的 blocking-human gate。 |
| 6 | execution_safety | 01-13/14 绑定 runner/run/job 与 SID 或 uid/home，要求 per-job OS 用户 attested 删除或 ephemeral baseline restore；marker-only 正式负例；01-26/27 统一 finally。 |
| 7 | verification_quality | 01-13/14 新增同一生产 predicate 的 test-only positive controls，success aggregation 必须通过，但 strict_gate_eligible=false；reject-all 实现不能通过。 |
| 8 | key_links_planned | 01-19 将 state smoke 拆成 seed/verify/cleanup；01-24 新增 prepare/refuse/restore/verify/cleanup recovery modes 与独立 run ID；01-25–27 只在 unified finally 清理。 |
| 9 | research_incorporation | 01-19/23/24 覆盖 two-process open/migrate/backup/restore；01-21 lint VACUUM/ATTACH/DETACH/PRAGMA journal_mode 与 Rust 外部文件/进程 side effects。 |
| 10 | scope_sanity | 01-15 只执行诚实的非签名 Freeze 与 digest lock refresh，01-16 承载三个决策，01-17 开始生产 tracer；01-25 接线，01-26/27 分别取得 Windows/macOS 正式证据。 |
| 11 | source_audit_accuracy | 01-07 计划 audit-phase1-plan-source.ps1 + digest lock + negative tests；01-15 在计划语义稳定后刷新 lock；01-28 PhaseComplete 强制依赖它；本文不声称 final COVERED。 |
| 12 | pattern_compliance | 01-PATTERNS.md 重建为 28 plans / 19 waves / 101 concrete paths，每个 path 的全部 owner 与 PLAN files_modified 精确一致，并有 role、analog/No Analog；wildcard critical artifact=0。 |
| 13 | security_threat_model | 01-01 使用临时空 user/global config、固定 registry、隔离 cwd、清除 token 环境，并测试恶意 .npmrc/private registry/token 不影响或泄漏。 |

## Cross-Plan Interface Audit

- Runner producer: 01-03；consumers: 01-04/11–17/26–28；machine checks: 01-03 + 01-07。
- gh preflight producer: 01-05；blocking approval: 01-06；real consumers: 01-26/27。
- macOS Wave 0 producer: 01-12；consumer workflow: 01-14/25；execution evidence: 01-27。
- State smoke producer: 01-19；recovery smoke producer: 01-24；platform consumers: 01-25–27。
- v001 fixture producer: 01-20；migration matrix/history/policy consumers: 01-21。
- StateCoordinator producer: 01-19；migrate/backup extension: 01-23；restore extension: 01-24。
- Source audit producer: 01-07；lock refresh: 01-15；final consumer: 01-28。

## Deferred / Excluded

Phase 2–8 的生产 Codex 配置写入、供应商网络验证、真实 WSL2 切换、Linux functions、完整 UI/托盘、诊断导出与 updater 业务仍明确排除。Phase 1 的 Codex/宿主/WSL/package 脚本仅为 contract/signed smoke harness，不开放对应产品用例。

## Final Audit Rule

本文件只说明计划覆盖。执行完成只能由以下命令判定：

powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PhaseComplete -Target Local -Mode Strict

该命令必须实时运行 01-07 的 read-only audit；静态 PLANNED、COVERED 或人工文字不能替代。
