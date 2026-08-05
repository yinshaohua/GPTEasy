# Phase 1：可信本地状态与实现契约 - Pattern Map

**Mapped:** 2026-08-05  
**范围来源:** `CONTEXT.md`、`docs/adr/0001-0008`、`.planning/REQUIREMENTS.md`、`01-RESEARCH.md`、`01-VALIDATION.md`  
**最终拓扑:** 28 个计划、18 个 wave、101 个 concrete files_modified 路径  
**关键约束:** 所有路径逐项枚举；关键 artifact 不使用 glob；每份 PLAN 的 Artifacts 集合必须与 frontmatter files_modified 完全相等。

## 锁定边界

- Tauri 2 + Rust + TypeScript/React；Rust 后端是唯一 SQLite 权威。
- Phase 1 只实现 STATE-01..STATE-05 与后续阶段依赖的外部 contract evidence，不实现 Phase 2–8 生产能力。
- 明文 API Key 按 ADR-0001 保存于 SQLite，但不得进入公开 DTO、日志、证据或错误。
- existing DB 的第一连接只读；higher schema 或完整合同失败时不得打开 RW。
- 全部 pending migrations 位于一个 BEGIN IMMEDIATE transaction；VACUUM、ATTACH、DETACH、PRAGMA journal_mode 与外部文件/进程 side effect 禁止进入 migration。
- 数据库升级备份使用 SQLite Online Backup API、verified identity、三份 retention；恢复前保留 verified quarantine。
- 外部 evidence 必须经过 gh preflight、重新下载、attestation、commit/digest 与 runner/account lifecycle 验证。
- macOS evidence 必须先执行原生 zsh Wave 0；Windows/macOS disposable 状态由 per-job OS 用户生命周期和 attested cleanup 证明，应用 marker 不能单独通过。

## 最强类比族

| 类比族 | 可复用 | 必须替换/补全 |
|--------|--------|---------------|
| Spike 001 Codex contract | app-server、隐藏子进程、允许清单摘要 | 目标 0.146.1、host-vs-CLI canary parity、禁止 raw command/config |
| Spike 005 install matrix | Windows current-user NSIS、安装/卸载 smoke | Authenticode、ARM64、positive controls、账户生命周期与 attestation |
| Spike 009 WSL lifecycle | passive list、UTF-16、运行集合前后比较 | duplicate name fail-closed 与 provenance schema |
| Spike 012 desktop E2E | Tauri composition、typed command、rusqlite transaction、scenario tests | 产品 schema、明文 Key、只读 preflight、永久 migration/backup/recovery |
| Spike 017 macOS contract | evidence_level、~/Applications、codesign/Gatekeeper、zsh 入口 | 原生双架构 Wave 0、正式宿主 parity、公证与 uid/home cleanup |

## Concrete Path Classification

| Path | Plan(s) | Role | Closest Analog | Classification |
|------|---------|------|----------------|----------------|
| `.github/workflows/phase1-macos-evidence.yml` | 01-14, 01-27 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `.github/workflows/phase1-macos-wave0.yml` | 01-12 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `.github/workflows/phase1-windows-evidence.yml` | 01-13, 01-26 | Windows package/lifecycle | Spike 005 | 角色匹配/生命周期补全 |
| `index.html` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `package-lock.json` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `package.json` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `scripts/contracts/assert-macos-job-lifecycle.zsh` | 01-14 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/assert-windows-job-lifecycle.ps1` | 01-13 | Windows package/lifecycle | Spike 005 | 角色匹配/生命周期补全 |
| `scripts/contracts/audit-phase1-plan-source.ps1` | 01-07 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/preflight-gh-evidence.ps1` | 01-05 | DB validation/preflight | 无只读完整合同类比 | No Analog / Research |
| `scripts/contracts/probe-codex-macos.zsh` | 01-12 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/probe-codex.ps1` | 01-11 | Windows contract probe | Spike 001 | 角色匹配/安全收紧 |
| `scripts/contracts/probe-macos-host.zsh` | 01-12 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/probe-windows-host.ps1` | 01-11 | Windows contract probe | Spike 001 | 角色匹配/安全收紧 |
| `scripts/contracts/probe-wsl2.ps1` | 01-11 | WSL host probe | Spike 009 | 精确/扩展 |
| `scripts/contracts/run-macos.zsh` | 01-14 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/run-phase1-contracts.ps1` | 01-03 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/test-evidence-provenance.ps1` | 01-04 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/test-gh-preflight.ps1` | 01-05 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/test-macos-contract-probes.zsh` | 01-12 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/test-macos-package-verifier.zsh` | 01-14 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/test-macos-wave0-contract.ps1` | 01-12 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `scripts/contracts/test-npm-package-allowlist.ps1` | 01-01 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/test-phase1-plan-source-audit.ps1` | 01-07 | test/config artifact | 无直接类比 | No Analog |
| `scripts/contracts/test-run-phase1-cli.ps1` | 01-03 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/test-windows-contract-probes.ps1` | 01-11 | Windows contract probe | Spike 001 | 角色匹配/安全收紧 |
| `scripts/contracts/test-windows-package-verifier.ps1` | 01-13 | Windows package/lifecycle | Spike 005 | 角色匹配/生命周期补全 |
| `scripts/contracts/test-wsl2-probe.ps1` | 01-11 | WSL host probe | Spike 009 | 精确/扩展 |
| `scripts/contracts/validate-contract-evidence.ps1` | 01-04 | test/config artifact | 无直接类比 | No Analog |
| `scripts/contracts/verify-evidence-provenance.ps1` | 01-04 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/verify-migration-history.ps1` | 01-22 | test/config artifact | 无直接类比 | No Analog |
| `scripts/contracts/verify-migration-policy.ps1` | 01-22 | test/config artifact | 无直接类比 | No Analog |
| `scripts/contracts/verify-npm-package-allowlist.ps1` | 01-01 | contract/security gate | 无完整类比 | No Analog / Research |
| `scripts/contracts/verify-windows-package.ps1` | 01-13 | Windows package/lifecycle | Spike 005 | 角色匹配/生命周期补全 |
| `src-tauri/Cargo.lock` | 01-09 | config/bootstrap | Spike 005/012 | 精确或角色匹配 |
| `src-tauri/Cargo.toml` | 01-09 | config/bootstrap | Spike 005/012 | 精确或角色匹配 |
| `src-tauri/build.rs` | 01-09 | config/bootstrap | Spike 005/012 | 精确或角色匹配 |
| `src-tauri/capabilities/default.json` | 01-09 | config/bootstrap | Spike 005/012 | 精确或角色匹配 |
| `src-tauri/src/bin/generate_v001_fixture.rs` | 01-21 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `src-tauri/src/commands.rs` | 01-18, 01-19, 01-25 | domain/repository/command | Spike 012 | 部分；遵循 ADR-0001/0006 |
| `src-tauri/src/domain/mod.rs` | 01-19 | domain/repository/command | Spike 012 | 部分；遵循 ADR-0001/0006 |
| `src-tauri/src/lib.rs` | 01-09, 01-10, 01-18, 01-20, 01-25 | composition/state service | Spike 012 | 角色匹配/安全收紧 |
| `src-tauri/src/main.rs` | 01-09 | config/bootstrap | Spike 005/012 | 精确或角色匹配 |
| `src-tauri/src/path_smoke.rs` | 01-10 | platform path smoke | Spike 017 + Tauri PathResolver | 部分 |
| `src-tauri/src/state/backup.rs` | 01-24 | backup/rollback | Spike 009 仅普通文件备份 | 部分/Online Backup 绿色实现 |
| `src-tauri/src/state/coordination.rs` | 01-20 | inter-process coordination | 无完整类比 | No Analog / Research |
| `src-tauri/src/state/migrations/0001_initial.sql` | 01-18 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `src-tauri/src/state/migrations/mod.rs` | 01-21, 01-24 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `src-tauri/src/state/mod.rs` | 01-18, 01-19, 01-20, 01-21, 01-23, 01-24 | composition/state service | Spike 012 | 角色匹配/安全收紧 |
| `src-tauri/src/state/preflight.rs` | 01-23, 01-25 | DB validation/preflight | 无只读完整合同类比 | No Analog / Research |
| `src-tauri/src/state/recovery.rs` | 01-25 | recovery/higher schema | 无完整类比 | No Analog / Research |
| `src-tauri/src/state/repositories.rs` | 01-19 | domain/repository/command | Spike 012 | 部分；遵循 ADR-0001/0006 |
| `src-tauri/src/state/validation.rs` | 01-23 | DB validation/preflight | 无只读完整合同类比 | No Analog / Research |
| `src-tauri/tauri.conf.json` | 01-09 | config/bootstrap | Spike 005/012 | 精确或角色匹配 |
| `src-tauri/tests/backup_restore.rs` | 01-24 | backup/rollback | Spike 009 仅普通文件备份 | 部分/Online Backup 绿色实现 |
| `src-tauri/tests/db_contract_validation.rs` | 01-23 | DB validation/preflight | 无只读完整合同类比 | No Analog / Research |
| `src-tauri/tests/fixture_generation.rs` | 01-21 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `src-tauri/tests/higher_schema_refusal.rs` | 01-25 | recovery/higher schema | 无完整类比 | No Analog / Research |
| `src-tauri/tests/installed_recovery_smoke.rs` | 01-25 | recovery/higher schema | 无完整类比 | No Analog / Research |
| `src-tauri/tests/installed_state_smoke.rs` | 01-20 | state integration test | Spike 012 场景矩阵 | 部分/产品 schema 重写 |
| `src-tauri/tests/local_only_boundary.rs` | 01-20 | state integration test | Spike 012 场景矩阵 | 部分/产品 schema 重写 |
| `src-tauri/tests/migration_failure.rs` | 01-24 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `src-tauri/tests/migration_matrix.rs` | 01-22 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `src-tauri/tests/path_smoke.rs` | 01-10 | platform path smoke | Spike 017 + Tauri PathResolver | 部分 |
| `src-tauri/tests/ready_preflight.rs` | 01-23 | DB validation/preflight | 无只读完整合同类比 | No Analog / Research |
| `src-tauri/tests/recovery_validation.rs` | 01-25 | DB validation/preflight | 无只读完整合同类比 | No Analog / Research |
| `src-tauri/tests/state_command_restart.rs` | 01-18 | state integration test | Spike 012 场景矩阵 | 部分/产品 schema 重写 |
| `src-tauri/tests/state_concurrency.rs` | 01-20, 01-24, 01-25 | inter-process coordination | 无完整类比 | No Analog / Research |
| `src-tauri/tests/state_persistence.rs` | 01-19 | state integration test | Spike 012 场景矩阵 | 部分/产品 schema 重写 |
| `src/App.tsx` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `src/global.css` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `src/main.tsx` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `tests/fixtures/contracts/codex/macos-apple-silicon/manifest.json` | 01-16 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/codex/macos-intel/manifest.json` | 01-16 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/codex/windows-arm64/manifest.json` | 01-15 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/codex/windows-x64/manifest.json` | 01-15 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/gh-preflight-cases.json` | 01-05 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/host/macos-apple-silicon/manifest.json` | 01-16 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/host/macos-intel/manifest.json` | 01-16 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/host/windows-arm64/manifest.json` | 01-15 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/host/windows-x64/manifest.json` | 01-15 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/npm-package-allowlist.json` | 01-01 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/packaging/macos-apple-silicon/manifest.json` | 01-16, 01-27 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `tests/fixtures/contracts/packaging/macos-intel/manifest.json` | 01-16, 01-27 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `tests/fixtures/contracts/packaging/macos-positive-control.json` | 01-14 | macOS contract/package | Spike 017 | 角色匹配/原生补全 |
| `tests/fixtures/contracts/packaging/windows-arm64/manifest.json` | 01-15, 01-26 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/packaging/windows-positive-control.json` | 01-13 | Windows package/lifecycle | Spike 005 | 角色匹配/生命周期补全 |
| `tests/fixtures/contracts/packaging/windows-x64/manifest.json` | 01-15, 01-26 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/phase1-plan-audit-lock.json` | 01-07 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/provenance-negative-cases.json` | 01-04 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/runner-cli-matrix.json` | 01-03 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/schema/contract-manifest.schema.json` | 01-04 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/schema/provenance.schema.json` | 01-04 | contract/security gate | 无完整类比 | No Analog / Research |
| `tests/fixtures/contracts/wsl2/windows-arm64/manifest.json` | 01-15 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/contracts/wsl2/windows-x64/manifest.json` | 01-15 | external evidence fixture | Spike 001/005/009/017 | 部分/attested 扩展 |
| `tests/fixtures/databases/history-lock.json` | 01-22 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `tests/fixtures/databases/manifest.json` | 01-21 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `tests/fixtures/databases/v001/state.sqlite3` | 01-21 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `tests/fixtures/migrations/forbidden-migration-cases.json` | 01-22 | migration/history fixture | 无永久历史类比 | No Analog / Research |
| `tsconfig.json` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |
| `vite.config.ts` | 01-08 | config/component | 官方 Tauri React TS template | 角色匹配 |

## 关键实现分配

### Canonical runner 与 provenance

- `tests/fixtures/contracts/runner-cli-matrix.json` 是 Scope/Target/Mode/exit-code 唯一事实源。
- `scripts/contracts/test-run-phase1-cli.ps1` 扫描全部 28 份 PLAN.md；旧参数或未声明组合必须失败。
- `scripts/contracts/preflight-gh-evidence.ps1` 必须先验证 gh >= 2.49.0、认证、repo/actions/attestation read 权限。
- `scripts/contracts/verify-evidence-provenance.ps1` 每次下载到新临时目录，并重新匹配 run/job/workflow/commit/artifact/attestation digest。

### macOS Wave 0 与平台 lifecycle

- `.github/workflows/phase1-macos-wave0.yml` 在 Intel/Apple Silicon 上运行 zsh -n 与全部 zsh verifier tests。
- `.github/workflows/phase1-macos-evidence.yml` 以 job-level reusable workflow + needs: wave0 消费它。
- Windows/macOS workflows 创建 per-job OS 用户，记录 runner/job/SID 或 uid/home，并在统一 finally 删除账户/profile或验证 baseline restore；cleanup 失败不得生成 strict pass。
- test-only package positive controls走同一生产 predicate，但始终 strict_gate_eligible=false。

### SQLite 状态、迁移与恢复

- 01-18 是首个生产 tracer：两 OS 进程都经 Tauri mock IPC 调用注册 command。
- `StateCoordinator` 使用 Rust 标准库 File exclusive lock；是否占用由 OS lock 决定，不由 metadata/marker 自报。
- historical fixture 先由 01-21 创建，再由 01-22 migration_matrix 消费；verify 不运行 generator。
- `verify-migration-policy.ps1` 同时限制 SQL token 与 Rust transform capability。
- full-state smoke 使用 seed/verify/cleanup；recovery smoke 使用 prepare/refuse/restore/verify/cleanup，且两个 run ID 必须不同。
- cleanup 只在最终 workflow 统一 finally 执行：recovery cleanup → state cleanup → app cleanup → account/profile cleanup。

## 明确不可复制

| 模式 | 原因 |
|------|------|
| RW open 后再判断 schema | 违反 STATE-05 non-mutating refusal |
| 普通文件复制作为 SQLite backup | WAL 下不能保证一致 snapshot |
| 每 migration 单独 commit | 失败会停在中间版本 |
| migration 中 VACUUM/ATTACH/DETACH/journal_mode/fs/process | 不能由单一 transaction 回滚 |
| 完整 command line/config/read/raw response | 泄漏凭据与用户配置 |
| run-ID marker 证明 disposable | 长期 runner 可自行伪造 |
| 只有 package verifier 负例 | reject-all 实现仍可通过 |
| Windows synthetic 代替 zsh -n | 不能发现 zsh 语法/控制流错误 |
| settings-only smoke 声称 STATE-01 | 与完整状态要求不一致 |

## Planner 采用顺序

1. package legitimacy、canonical runner、provenance/gh/audit 基础。
2. frontend/Tauri/path、Windows/macOS/WSL probes 与平台 package lifecycle workflows。
3. 四目标独立 attested freeze evidence 与 one-way checkpoints。
4. true production tracer → complete state → local-only/headless/coordinator。
5. fixture/history/policy → validator → backup/rollback → higher/recovery。
6. 四目标最终 installed full-state/recovery evidence → read-only PhaseComplete。

## Metadata

- Concrete paths: 101
- Plans: 28
- Waves: 18
- Pattern extraction date: 2026-08-05
- Wildcard critical artifacts: 0
