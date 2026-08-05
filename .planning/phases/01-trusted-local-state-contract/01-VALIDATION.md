---
phase: 1
slug: trusted-local-state-contract
# status lifecycle: draft (seeded by plan-phase) → validated (set by validate-phase §6)
# audit-milestone distinguishes NOT-VALIDATED (draft) from PARTIAL (validated + nyquist_compliant: false)
status: draft
nyquist_compliant: false
wave_0_complete: false
created: 2026-08-05
---

# Phase 1 — Validation Strategy

> Phase 1 执行期间的反馈采样与最终契约冻结标准。未取得真实平台或目标 Codex 证据的项目必须保持为阻断项，不得用模拟结果代替。

---

## Test Infrastructure

| Property | Value |
|----------|-------|
| **Framework** | Rust built-in test harness via Cargo；前端使用 `tsc + vite build`；桌面边界使用 Tauri debug/release smoke |
| **Config file** | `src-tauri/Cargo.toml`、`vite.config.ts`、`tsconfig.json` — Wave 0 创建 |
| **Quick run command** | `cargo test --manifest-path src-tauri/Cargo.toml state_` |
| **Full suite command** | `cargo test --manifest-path src-tauri/Cargo.toml --all-targets && npm run build` |
| **Estimated runtime** | 快速反馈目标 < 60 秒；完整本地套件目标 < 5 分钟；平台签名与安装 smoke 单独运行 |

---

## Sampling Rate

- **After every task commit:** Run `cargo test --manifest-path src-tauri/Cargo.toml state_`
- **After every plan wave:** Run `cargo test --manifest-path src-tauri/Cargo.toml --all-targets && npm run build`
- **Before `$gsd-verify-work`:** 本地完整套件以及 Phase 1 契约/平台 smoke 必须全部通过
- **Max feedback latency:** 单任务自动反馈不超过 60 秒；超过时拆分快速测试目标

---

## Per-Task Verification Map

| Task ID | Plan | Wave | Requirement | Threat Ref | Secure Behavior | Test Type | Automated Command | File Exists | Status |
|---------|------|------|-------------|------------|-----------------|-----------|-------------------|-------------|--------|
| 1-W0-01 | TBD | 0 | STATE-01 | T-1-STATE-INTEGRITY | 重启后只从本地 SQLite 恢复完整权威状态 | integration | `cargo test --manifest-path src-tauri/Cargo.toml --test state_persistence` | ❌ W0 | ⬜ pending |
| 1-W0-02 | TBD | 0 | STATE-02 | T-1-SECRET-EXPOSURE | 无产品账户或默认上传；公开投影和证据不包含凭据 | integration + static gate | `cargo test --manifest-path src-tauri/Cargo.toml --test local_only_boundary` | ❌ W0 | ⬜ pending |
| 1-W0-03 | TBD | 0 | STATE-03 | T-1-MIGRATION-DRIFT | 所有历史 fixture 只经永久顺序迁移到当前 schema | fixture matrix | `cargo test --manifest-path src-tauri/Cargo.toml --test migration_matrix` | ❌ W0 | ⬜ pending |
| 1-W0-04 | TBD | 0 | STATE-04 | T-1-BACKUP-CORRUPTION | 迁移前备份可打开，失败回滚全部待迁移版本且不重置数据 | fault integration | `cargo test --manifest-path src-tauri/Cargo.toml --test migration_failure --test backup_restore` | ❌ W0 | ⬜ pending |
| 1-W0-05 | TBD | 0 | STATE-05 | T-1-DOWNGRADE-WRITE | 高版本数据库只读拒绝，恢复兼容备份时保留高版本隔离副本 | recovery integration | `cargo test --manifest-path src-tauri/Cargo.toml --test higher_schema_refusal` | ❌ W0 | ⬜ pending |
| 1-W0-06 | TBD | 0 | Phase risk gate | T-1-CONTRACT-GUESS | 目标 Codex、宿主、WSL2 和签名工件契约有版本化、脱敏证据 | contract/smoke | `powershell -File scripts/contracts/run-phase1-contracts.ps1` | ❌ W0 | ⬜ pending |

*Status: ⬜ pending · ✅ green · ❌ red · ⚠️ flaky*

---

## Required Scenarios

- `state_restart_round_trip`：写入两个供应商、验证记录、native/WSL 环境及不同当前供应商、设置；销毁并重开 `StateStore` 后深比较。
- `migration_all_historical_fixtures`：从 manifest 枚举每个 committed fixture，复制到临时目录后顺序升级。
- `migration_failure_rolls_back_all_pending_versions`：从 v1 模拟 v2/v3，v3 故障后数据库仍完整处于 v1。
- `backup_is_openable_and_retains_three`：每份备份均能 read-only open 且 `quick_check` 通过，第四次后删除最老备份。
- `higher_schema_refusal_is_non_mutating`：记录数据库/WAL hash 与 mtime；拒绝后不得创建迁移、备份或写事务。
- `downgrade_restore_preserves_newer_db_quarantine`：恢复兼容备份前保留 newer DB 隔离副本。
- `evidence_canary_scan`：扫描假 Key、`experimental_bearer_token`、`Authorization` 与完整命令行字段。

---

## Wave 0 Requirements

- [ ] `src-tauri/Cargo.toml`、`src-tauri/src/lib.rs` — Rust/Tauri 可测试 composition root
- [ ] `src-tauri/tests/state_persistence.rs` — STATE-01
- [ ] `src-tauri/tests/local_only_boundary.rs` — STATE-02
- [ ] `src-tauri/tests/migration_matrix.rs` — STATE-03
- [ ] `src-tauri/tests/migration_failure.rs` — STATE-04 rollback
- [ ] `src-tauri/tests/backup_restore.rs` — STATE-04 backup/retention/restore
- [ ] `src-tauri/tests/higher_schema_refusal.rs` — STATE-05
- [ ] `tests/fixtures/databases/v001/state.sqlite3` 与 `manifest.json` — 第一份永久历史数据库样本
- [ ] `scripts/contracts/run-phase1-contracts.ps1` 与目标平台 probe — Phase 1 契约门禁
- [ ] npm 依赖 legitimacy 人工门禁 — 安装研究中标为 `SUS` 的包前完成

---

## Manual-Only Verifications

| Behavior | Requirement | Why Manual | Test Instructions |
|----------|-------------|------------|-------------------|
| 目标 Codex 版本配置与运行时契约冻结 | Phase risk gate | 依赖实际稳定版二进制、宿主与 app-server 行为 | 在隔离用户目录运行 contract harness，保存版本、路径、schema、运行时和脱敏 canary 证据 |
| 停止状态 WSL2 默认用户发现 | Phase risk gate | 不得启动发行版，且需要真实代表性 WSL2 环境 | 对已停止发行版运行只读枚举/probe，确认状态未改变；重复 `DistributionName` 必须 fail-closed |
| Windows x64/ARM64 当前用户安装与签名 smoke | Phase risk gate | 需要原生 runner、正式架构工具链和 Authenticode 资源 | 对签名 NSIS 执行安装、启动、状态 round-trip、卸载与签名验证 |
| macOS Intel/Apple Silicon 当前用户安装与签名公证 smoke | Phase risk gate | 需要真实 macOS 14+、双架构 runner 与 Apple 凭据 | 安装到 `~/Applications/GPTEasy.app`，验证 codesign/notary/Gatekeeper、启动与状态 round-trip |

---

## Validation Sign-Off

- [ ] All tasks have `<automated>` verify or Wave 0 dependencies
- [ ] Sampling continuity: no 3 consecutive tasks without automated verify
- [ ] Wave 0 covers all MISSING references
- [ ] No watch-mode flags
- [ ] Feedback latency < 60s for task-level tests
- [ ] 所有 Phase 1 契约阻断项已有真实证据，或阶段保持未完成
- [ ] `nyquist_compliant: true` set in frontmatter

**Approval:** pending
