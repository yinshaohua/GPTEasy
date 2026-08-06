---
phase: 1
slug: trusted-local-state-contract
status: revised
nyquist_compliant: true
wave_0_complete: false
created: 2026-08-05
revision: 3
---

# Phase 1 — Validation Strategy

## Canonical Runner Contract

| Field | Exact Contract |
|-------|----------------|
| Scope | RunnerSelfTest, ProvenanceSelfTest, ContractSelfTest, PackagingSelfTest, Freeze, PhaseComplete |
| Target | Local, WindowsX64, WindowsArm64, MacIntel, MacAppleSilicon, Wsl2 |
| Mode | Strict, AllowBlocked |
| Strict-only combinations | Freeze+Local、PhaseComplete+Local、RunnerSelfTest+Local、ProvenanceSelfTest+Local |
| AllowBlocked | 仅具体目标的 ContractSelfTest/PackagingSelfTest；blocked 可 exit 0，但 strict_gate_eligible=false |
| Exit codes | 0 requested mode complete；2 assertion failure；3 Strict prerequisite blocked；4 provenance invalid；5 security/canary/lifecycle failure；64 usage/combination error |

所有计划中的 runner 调用必须显式提供 -Scope、-Target、-Mode；01-03 parser/consumer tests 与 01-07 source audit 都会扫描全部 PLAN.md。

## Test Infrastructure

| Property | Value |
|----------|-------|
| Rust | Cargo 1.97.1 built-in test harness |
| Frontend | tsc --noEmit + Vite build |
| Windows contracts | PowerShell production predicates + positive/negative fixture tests |
| macOS contracts | 原生 Intel/Apple Silicon reusable Wave 0：zsh -n + zsh verifier tests |
| External evidence | gh preflight + run/job/artifact download + GitHub attestation verify |
| Quick state command | cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart |
| Full local | cargo test --manifest-path src-tauri/Cargo.toml --all-targets；npm run build |
| Final | powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PhaseComplete -Target Local -Mode Strict |

## Per-Plan Automated Verification Map

| Plan | Requirement / Risk | Automated Command(s) |
|------|--------------------|----------------------|
| 01-01 | 建立首次 npm install 前可重复、隔离且不会受本机或仓库 npm 配置污染的包身份门禁。 | powershell -NoProfile -File scripts/contracts/verify-npm-package-allowlist.ps1 -Allowlist tests/fixtures/contracts/npm-package-allowlist.json<br>powershell -NoProfile -File scripts/contracts/test-npm-package-allowlist.ps1 |
| 01-02 | 在首次 npm install 前完成人工官方来源批准。 | powershell -NoProfile -File scripts/contracts/test-npm-package-allowlist.ps1; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/verify-npm-package-allowlist.ps1 -Allowlist tests/fixtures/contracts/npm-package-allowlist.json |
| 01-03 | 定义 Phase 1 唯一 runner CLI、合法调用矩阵与稳定退出码。 | powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope RunnerSelfTest -Target Local -Mode Strict<br>powershell -NoProfile -File scripts/contracts/test-run-phase1-cli.ps1 -ScanPlans .planning/phases/01-trusted-local-state-contract |
| 01-04 | 建立外部 evidence schema、脱敏校验和可独立验证的 provenance 核心。 | powershell -NoProfile -File scripts/contracts/validate-contract-evidence.ps1 -SelfTest<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ProvenanceSelfTest -Target Local -Mode Strict |
| 01-05 | 实现固定最低版本、认证与仓库权限的 gh evidence preflight。 | powershell -NoProfile -File scripts/contracts/preflight-gh-evidence.ps1 -SelfTest<br>powershell -NoProfile -File scripts/contracts/test-gh-preflight.ps1 |
| 01-06 | 在首次真实 artifact verification 前完成阻断式 gh 环境确认。 | powershell -NoProfile -File scripts/contracts/preflight-gh-evidence.ps1 -Repository yinshaohua/GPTEasy -MinimumVersion 2.49.0 |
| 01-07 | 建立只读、机器可执行的 Phase 1 计划/来源/接口审计。 | powershell -NoProfile -File scripts/contracts/audit-phase1-plan-source.ps1 -PhaseDir .planning/phases/01-trusted-local-state-contract -ReadOnly<br>powershell -NoProfile -File scripts/contracts/test-phase1-plan-source-audit.ps1 |
| 01-08 | 创建 exact、可重复构建且无业务范围的 React/TypeScript 前端壳。 | powershell -NoProfile -File scripts/contracts/verify-npm-package-allowlist.ps1 -Allowlist tests/fixtures/contracts/npm-package-allowlist.json; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; npm ci<br>npm run build |
| 01-09 | 创建最小 Tauri 2/Rust 当前用户应用壳。 | cargo metadata --manifest-path src-tauri/Cargo.toml --locked --no-deps<br>cargo test --manifest-path src-tauri/Cargo.toml --lib; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; npm run tauri -- build --debug --no-bundle |
| 01-10 | 用真实 Tauri AppHandle 建立固定 app_local_data_dir path smoke。 | cargo test --manifest-path src-tauri/Cargo.toml --lib path_smoke<br>cargo test --manifest-path src-tauri/Cargo.toml --test path_smoke |
| 01-11 | 实现 Windows Codex/正式宿主共享配置 canary parity 与无副作用 WSL2 contract probes。 | powershell -NoProfile -File scripts/contracts/test-windows-contract-probes.ps1<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ContractSelfTest -Target Wsl2 -Mode Strict |
| 01-12 | 建立 macOS 原生 zsh Wave 0 与 CLI/正式宿主共享配置 contract。 | powershell -NoProfile -File scripts/contracts/test-macos-wave0-contract.ps1 -Scripts scripts/contracts/probe-codex-macos.zsh,scripts/contracts/probe-macos-host.zsh,scripts/contracts/test-macos-contract-probes.zsh -Workflow .github/workflows/phase1-macos-wave0.yml |
| 01-13 | 建立 Windows 双架构 package predicate、一次性 OS 用户生命周期证据与 attested workflow。 | powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet All; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet Workflow<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target Local -Mode Strict |
| 01-14 | 建立 macOS 双架构 package predicate、一次性 OS 用户生命周期证据与 Wave 0 依赖 workflow。 | powershell -NoProfile -File scripts/contracts/test-macos-wave0-contract.ps1 -Scripts scripts/contracts/assert-macos-job-lifecycle.zsh,scripts/contracts/run-macos.zsh,scripts/contracts/test-macos-package-verifier.zsh -Workflow .github/workflows/phase1-macos-wave0.yml<br>powershell -NoProfile -File scripts/contracts/test-macos-wave0-contract.ps1 -Scripts scripts/contracts/probe-codex-macos.zsh,scripts/contracts/probe-macos-host.zsh,scripts/contracts/test-macos-contract-probes.zsh,scripts/contracts/assert-macos-job-lifecycle.zsh,scripts/contracts/run-macos.zsh,scripts/contracts/test-macos-package-verifier.zsh -Workflow .github/workflows/phase1-macos-wave0.yml -EvidenceWorkflow .github/workflows/phase1-macos-evidence.yml; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target Local -Mode Strict |
| 01-15 | 建立诚实的非签名合同冻结门，让 SQLite schema/backup 单向决策不再被尚未具备的发布签名凭据阻塞。 | powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope RunnerSelfTest -Target Local -Mode Strict; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/test-run-phase1-cli.ps1 -ScanPlans .planning/phases/01-trusted-local-state-contract; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict<br>powershell -NoProfile -File scripts/contracts/test-phase1-plan-source-audit.ps1; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/audit-phase1-plan-source.ps1 -PhaseDir .planning/phases/01-trusted-local-state-contract -ReadOnly |
| 01-16 | 在诚实的非签名合同冻结后，明确批准两个 one-way 本地磁盘合同，并记录发布签名延期边界。 | powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict |
| 01-17 | 交付 Phase 1 生产 tracer：Tauri command 写入 SQLite，进程退出后由全新进程 bootstrap 读回。 | cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart -- --nocapture |
| 01-18 | 把 tracer 扩展为完整供应商目录、验证状态、受管环境当前供应商与应用设置的权威持久状态。 | cargo test --manifest-path src-tauri/Cargo.toml --test state_persistence |
| 01-19 | 提供 truthful installed full-state smoke、完全本地边界与跨进程状态协调合同。 | cargo test --manifest-path src-tauri/Cargo.toml --test installed_state_smoke --test local_only_boundary<br>cargo test --manifest-path src-tauri/Cargo.toml --test state_concurrency |
| 01-20 | 创建 append-only migration registry 与确定性、create-once 的 v001 历史数据库 fixture。 | cargo test --manifest-path src-tauri/Cargo.toml --test fixture_generation<br>cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart --test state_persistence --test fixture_generation |
| 01-21 | 建立 manifest-driven 历史迁移矩阵、history lock/tag drift gate 与迁移禁止项 lint。 | cargo test --manifest-path src-tauri/Cargo.toml --test migration_matrix<br>powershell -NoProfile -File scripts/contracts/verify-migration-history.ps1<br>powershell -NoProfile -File scripts/contracts/verify-migration-policy.ps1 -SelfTest; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/verify-migration-policy.ps1 -RepositoryRoot . |
| 01-22 | 实现 Ready、Migrate source、backup 与 recovery candidate 共用的只读数据库合同 validator。 | cargo test --manifest-path src-tauri/Cargo.toml --test db_contract_validation<br>cargo test --manifest-path src-tauri/Cargo.toml --test ready_preflight --test db_contract_validation |
| 01-23 | 实现批准后的 verified SQLite backup、三份 retention、全 pending 单事务 rollback 与并发协调。 | cargo test --manifest-path src-tauri/Cargo.toml --test backup_restore<br>cargo test --manifest-path src-tauri/Cargo.toml --test migration_failure --test backup_restore --test state_concurrency |
| 01-24 | 实现 higher-schema non-mutating refusal、opaque backup restore、verified quarantine 与 installed recovery smoke。 | cargo test --manifest-path src-tauri/Cargo.toml --test higher_schema_refusal<br>cargo test --manifest-path src-tauri/Cargo.toml --test recovery_validation --test state_concurrency<br>cargo test --manifest-path src-tauri/Cargo.toml --test installed_recovery_smoke --test higher_schema_refusal --test recovery_validation |
| 01-25 | 把完整状态与恢复 smoke 接入 Windows/macOS 最终工作流，并在不具备签名凭据时完成所有可执行接线验证。 | powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet Workflow<br>powershell -NoProfile -File scripts/contracts/test-macos-wave0-contract.ps1 -Scripts scripts/contracts/assert-macos-job-lifecycle.zsh,scripts/contracts/run-macos.zsh,scripts/contracts/test-macos-package-verifier.zsh -Workflow .github/workflows/phase1-macos-wave0.yml -EvidenceWorkflow .github/workflows/phase1-macos-evidence.yml; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target Local -Mode Strict |
| 01-26 | 获取 Windows x64/ARM64 真实 Authenticode installed contract、状态与恢复证据。 | $required = @('WINDOWS_AUTHENTICODE_PFX_BASE64','WINDOWS_AUTHENTICODE_PFX_PASSWORD'); $configured = @(gh secret list --repo yinshaohua/GPTEasy --app actions --json name --jq '.[].name'); if ($LASTEXITCODE -ne 0) { exit 3 }; $missing = @($required \| Where-Object { $_ -notin $configured }); if ($missing.Count -gt 0) { Write-Error ('缺少 Actions secret 名称: ' + ($missing -join ', ')); exit 3 }<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ContractSelfTest -Target WindowsX64 -Mode Strict; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target WindowsX64 -Mode Strict<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ContractSelfTest -Target WindowsArm64 -Mode Strict; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target WindowsArm64 -Mode Strict |
| 01-27 | 获取 macOS Intel/Apple Silicon 真实 Developer ID、公证与 installed 状态/恢复证据。 | $required = @('APPLE_CERTIFICATE','APPLE_CERTIFICATE_PASSWORD','APPLE_SIGNING_IDENTITY','APPLE_API_ISSUER','APPLE_API_KEY','APPLE_API_KEY_ID'); $configured = @(gh secret list --repo yinshaohua/GPTEasy --app actions --json name --jq '.[].name'); if ($LASTEXITCODE -ne 0) { exit 3 }; $missing = @($required \| Where-Object { $_ -notin $configured }); if ($missing.Count -gt 0) { Write-Error ('缺少 Actions secret 名称: ' + ($missing -join ', ')); exit 3 }<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ContractSelfTest -Target MacIntel -Mode Strict; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target MacIntel -Mode Strict<br>powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ContractSelfTest -Target MacAppleSilicon -Mode Strict; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target MacAppleSilicon -Mode Strict |
| 01-28 | 执行 fail-closed PhaseComplete 只读统一门禁，并由用户逐条确认 STATE-01～STATE-05。 | powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PhaseComplete -Target Local -Mode Strict |

## Explicit macOS Wave 0 Dependency

1. 01-12 创建 .github/workflows/phase1-macos-wave0.yml。
2. Intel 与 Apple Silicon 原生 jobs 都执行 zsh -n，覆盖 probe-codex-macos.zsh、probe-macos-host.zsh、test-macos-contract-probes.zsh、assert-macos-job-lifecycle.zsh、run-macos.zsh、test-macos-package-verifier.zsh。
3. 同一 jobs 运行 zsh contract/package tests，而不是由 Windows synthetic 聚合替代。
4. .github/workflows/phase1-macos-evidence.yml 以 job-level reusable workflow 调用 Wave 0，所有 native evidence jobs needs wave0。
5. 01-27 的 strict manifests 必须包含 wave0_passed=true；缺失或失败时 exit 3/5，不能 strict pass。

## Required Scenarios

- runner_cli_parser_and_all_plan_consumers：合法 matrix、旧参数、缺 Target/Mode、AllowBlocked 升级负例。
- npm_public_registry_isolated_from_npmrc_and_tokens：固定公开 registry、临时空 config、恶意 .npmrc/token canary。
- evidence_provenance_rejects_self_reported_stale_or_mismatched_artifacts。
- gh_preflight_blocks_old_version_unauthenticated_or_missing_repo_actions_attestation_read。
- windows_cli_and_bundled_host_share_disposable_user_config_canary：config root、origin/provider digest、credential carrier、shared user layer。
- wsl_passive_probe_does_not_enter_or_start_distribution_and_duplicate_name_is_unresolvable。
- macos_wave0_runs_zsh_syntax_and_verifier_tests_on_both_architectures。
- package_positive_control_uses_production_predicate_but_is_not_strict_eligible。
- runner_account_lifecycle_records_job_instance_and_attested_delete_or_baseline_restore。
- state_command_restart：两个 OS 进程通过 Tauri mock IPC，禁止直接调用 StateStore。
- full_state_restart_round_trip：provider/Key/verification/native+WSL/current provider/settings 深比较。
- installed_state_smoke_seed_verify_without_cleanup_then_explicit_cleanup。
- two_process_open_migrate_backup_restore_coordination_and_crash_lock_release。
- fixture_generation_is_deterministic_create_once_and_precedes_migration_matrix。
- migration_all_historical_fixtures_manifest_driven。
- migration_policy_rejects_vacuum_attach_detach_journal_mode_and_external_side_effects。
- db_contract_rejects_truncated_forged_ledger_fk_schema_identity_without_mutation。
- backup_is_verified_retains_three_reuses_same_source_and_rejects_replacement。
- migration_failure_rolls_back_all_pending_versions。
- higher_schema_refusal_is_non_mutating。
- recovery_rejects_arbitrary_replaced_incompatible_backup_and_preserves_verified_quarantine。
- installed_recovery_smoke_uses_distinct_run_id_and_cleanup_only_in_unified_finally。
- four_target_signed_installed_full_state_recovery_and_lifecycle_attestation。
- source_audit_reparses_requirements_plans_paths_runner_waves_threats_and_current_digests。

## Wave 0 / Create-Before-Verify Guarantees

- 01-01/03/04/05/07 在依赖安装和真实 evidence 前创建 package、runner、provenance、gh 与 source-audit tests。
- 01-12 的原生 zsh Wave 0 是 01-14/25/27 的显式 workflow 依赖。
- 01-13/14 先创建 package positive controls 和 verifier，再由 workflow/evidence plans 消费。
- 01-20 在 01-21 migration_matrix 之前创建 v001/state.sqlite3 与 manifest.json；verify 不调用 generator。
- 01-19 创建 state_concurrency，01-23/24 逐步扩展 migrate/backup/restore 两进程场景。
- 01-24 创建 recovery smoke 后，01-25 才修改最终 platform workflows，01-26/27 再取得正式 manifests。

## Manual / Blocking Gates

| Plan | Gate |
|------|------|
| 01-02 | Task 1: 批准官方 npm package 身份 |
| 01-06 | Task 1: 确认 gh 安装、认证与仓库权限 |
| 01-16 | Task 1: 批准非签名合同冻结范围<br>Task 2: 批准永久 schema version 1<br>Task 3: 批准数据库 backup identity 与 quarantine 合同 |
| 01-26 | Task 1: 核验 Windows 正式凭据与原生 runner 前置条件 |
| 01-27 | Task 1: 核验 macOS 正式凭据与原生宿主前置条件 |
| 01-28 | Task 1: 批准 Phase 1 最终统一门禁 |

## Sign-Off Conditions

- [x] 每个 task verify 含 automated，命令只消费该 task 已创建或其依赖已创建的 artifact。
- [x] 所有 runner invocation 符合唯一 CLI matrix。
- [x] macOS zsh 语法与单元测试在原生双架构 Wave 0 实际执行。
- [x] package verifier 同时有生产 predicate 正控制与负例；正控制不能授予 strict pass。
- [x] full-state/recovery smoke 使用不同 run IDs，verify 不清理，cleanup 只在 unified finally。
- [x] two-process open/migrate/backup/restore 与 migration prohibition lint 已映射。
- [x] final gate 依赖 read-only machine source audit，不接受静态 COVERED 文本。
- [x] high/critical threats 均 disposition=mitigate。

wave_0_complete 保持 false，直到执行期文件创建并在目标平台通过；nyquist_compliant=true 表示最终计划集已为每项行为安排先行自动验证。
