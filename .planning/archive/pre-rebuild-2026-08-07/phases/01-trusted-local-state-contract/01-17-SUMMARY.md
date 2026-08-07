---
phase: 01-trusted-local-state-contract
plan: 17
subsystem: local-state
tags: [rust, tauri, sqlite, ipc, subprocess, schema-v1]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-16 明确批准的 freeze-approved、approve-schema-version-1 与 approve-db-backup-contract"
provides:
  - "update_app_settings Tauri command 经 SQLite 持久化并由全新 OS 进程 bootstrap_state 读回的生产 tracer"
  - "固定 APPLICATION_ID、user_version=1、database UUID、schema fingerprint 与双账本的六张 STRICT 表 schema"
  - "不含 API Key、绝对路径或内部数据库错误的窄 BootstrapState 投影"
affects: [01-18 complete-state, 01-19 local-only-coordination, 01-20 migration-history, 01-22 validation]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Tauri mock IPC 测试由 integration-test executable 的两个独立子进程按注册 command 名调用"
    - "existing DB 先只读验证 APPLICATION_ID、user_version、metadata、ledger 与 quick_check，再打开 RW connection"
    - "StateStore 内部以 Mutex<Connection> 持有 SQLite connection，并只通过 typed command 公开脱敏投影"

key-files:
  created:
    - src-tauri/src/state/mod.rs
    - src-tauri/src/state/migrations/0001_initial.sql
    - src-tauri/src/commands.rs
    - src-tauri/tests/state_command_restart.rs
  modified:
    - src-tauri/src/lib.rs

key-decisions:
  - "APPLICATION_ID 固定为 0x47505445（ASCII GPTE），schema fingerprint 以版本化域、APPLICATION_ID、user_version 与 0001 checksum 计算。"
  - "固定 run ID 只关联两个测试子进程及其脱敏报告，不进入 app_settings 或其它永久 schema。"
  - "production configure_builder 在 Tauri setup 中从 app_local_data_dir 解析唯一状态根并注册 StateStore 与两个 typed command。"

patterns-established:
  - "Production tracer：RED 先证明 command composition 缺失，再由同一测试在 GREEN 贯通 command→SQLite→process exit→new process→bootstrap。"
  - "Public state boundary：公开成功 DTO 使用字段允许清单，公开失败只返回固定 state_unavailable code。"

requirements-completed: []

coverage:
  - id: D1
    description: "write/read 两个独立 OS 子进程分别按 update_app_settings/bootstrap_state command 名通过 Tauri mock IPC，第二个进程读回 theme=dark。"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart -- --nocapture"
        status: pass
    human_judgment: false
  - id: D2
    description: "0001 固定六张 STRICT 表、APPLICATION_ID、user_version、database UUID、schema fingerprint 与 schema_migrations checksum ledger；新进程只读验证后才能重开。"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "src-tauri/tests/state_command_restart.rs#registered_commands_round_trip_settings_across_os_processes"
        status: pass
      - kind: static-contract
        ref: "src-tauri/src/state/migrations/0001_initial.sql"
        status: pass
    human_judgment: false
  - id: D3
    description: "BootstrapState 只公开 schema_version/settings，测试输出与公开投影均拒绝 API Key、Authorization、Bearer、测试绝对路径和用户 profile。"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "src-tauri/tests/state_command_restart.rs#registered_commands_round_trip_settings_across_os_processes"
        status: pass
      - kind: static-contract
        ref: "rg StateStore/rusqlite/Connection::open/direct repository calls src-tauri/tests/state_command_restart.rs"
        status: pass
    human_judgment: false

# Metrics
duration: 33m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 17: Tauri Command 跨进程状态 Tracer Summary

**以固定身份的 SQLite v1、typed Tauri commands 和两个真实 OS 子进程贯通设置写入与 bootstrap 重开**

## Performance

- **Duration:** 33 分钟
- **Started:** 2026-08-07T02:28:00Z
- **Completed:** 2026-08-07T03:01:00Z
- **Tasks:** 1/1 TDD tracer
- **Files modified:** 5

## Accomplishments

- 创建最终批准的 `0001_initial.sql`：`state_metadata`、`schema_migrations`、`providers`、`provider_verifications`、`managed_environments`、`app_settings` 六张表全部使用 SQLite STRICT，并带 FK/CHECK/唯一性约束。
- 固定 `APPLICATION_ID=0x47505445` 与 `user_version=1`；新库生成 database UUID，保存版本化 schema fingerprint 和 0001 SHA-256 ledger，重开时先通过只读连接核对完整身份与 `quick_check`。
- `StateStore` 使用固定 `app_local_data_dir/state.sqlite3`、WAL、FULL synchronous、foreign keys、trusted_schema off、busy timeout、互斥 connection 和 prepared SQL 持久化 `theme`。
- 注册 `update_app_settings` 与 `bootstrap_state` 两个 typed Tauri commands；公开 DTO 只含 schema version 与无秘密 settings，内部错误统一映射为固定 `state_unavailable` code。
- integration test 不导入 `StateStore` 或 `rusqlite`，也不直接 open/read/write 数据库；两个子进程都构造 production builder 并按 command 名通过 mock IPC 完成 `theme=dark` round-trip。

## Task Commits

1. **Task 1 RED: 添加状态命令跨进程失败测试** - `355671b` (`test`)
2. **Task 1 GREEN: 贯通状态命令跨进程持久化** - `6522446` (`feat`)

## Files Created/Modified

- `src-tauri/src/state/migrations/0001_initial.sql` - 批准后的六表 STRICT schema version 1。
- `src-tauri/src/state/mod.rs` - 固定数据库身份、初始 migration、只读重开验证、settings repository 与连接所有权。
- `src-tauri/src/commands.rs` - typed settings input、脱敏 BootstrapState/PublicStoreError 与两个 commands。
- `src-tauri/src/lib.rs` - production builder setup、StateStore 管理与 invoke handler 注册。
- `src-tauri/tests/state_command_restart.rs` - 两个独立子进程的 command-name mock IPC tracer 与公开输出 allowlist。

## Decisions Made

- `APPLICATION_ID` 选择 ASCII `GPTE` 对应的稳定 32-bit 值 `0x47505445`；未来 release 不得改写，schema 演进只追加 migration。
- schema fingerprint 使用 `gpteasy-schema-fingerprint-v1\0` 域分隔，并绑定 application ID、当前 schema version 和 0001 SQL checksum，避免把 migration checksum 与完整 schema identity 混为同一语义。
- 测试固定 run ID 只出现在测试环境变量和脱敏报告中，不为 tracer 临时需求污染批准后的永久 schema。
- production builder 的 setup 是状态初始化唯一 composition seam；release CLI 不接受测试 root 或数据库路径。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 在 mock IPC 前执行一次 Tauri test setup iteration**

- **Found during:** Task 1 GREEN 首次真实 IPC。
- **Issue:** Tauri mock `Builder::build()` 与 production event loop 不同，不自动运行 `.setup()`，command 因 managed `StateStore` 尚未注册而拒绝执行。
- **Fix:** 测试子进程在构造 production builder 后调用一次 mock app `run_iteration` 触发同一 setup，再创建 mock webview 并发送 IPC；production 仍由正常 event loop 触发 setup。
- **Files modified:** `src-tauri/tests/state_command_restart.rs`
- **Verification:** 两个 child command 与父级跨进程 round-trip 共 3 个测试全部通过；Clippy `-D warnings` 通过。
- **Committed in:** `6522446`

---

**Total deviations:** 1 auto-fixed（Rule 3：1）
**Impact on plan:** 只补齐 Tauri mock 的真实生命周期，没有增加 test-only production 参数、直接 StateStore 调用或替代 IPC 路径。

## Issues Encountered

- 01-16 清理约 3.7 GiB Cargo 缓存后，RED 与全 targets Clippy 首次构建耗时较长；缓存重建完成后目标测试约 9 秒返回。
- `sha2` 0.11 digest 不再实现 `LowerHex`，使用固定 lowercase hex encoder 生成 64 位 checksum/fingerprint，没有增加依赖。

## Known Stubs

None。没有 TODO、FIXME、placeholder、跳过测试或直接数据库测试旁路。

## User Setup Required

None - 本 tracer 不需要外部服务、供应商凭据或签名文件。

## Next Phase Readiness

- 01-18 可以在同一六表 schema 和 command/repository seam 上扩展完整 provider、verification、managed environment 与 settings round-trip。
- 01-19 仍需补齐跨进程 coordinator、installed smoke 和完整 local-only 静态边界；因此本 Summary 不提前标记 STATE-01/02 完成。
- PFX、Developer ID 与 notarization 继续延期到 01-26/01-27，与本地 SQLite tracer 无耦合。

## Self-Check: PASSED

- `cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart -- --nocapture`：3 passed；write/read 两个子进程均按 command 名走 Tauri mock IPC。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：返回 0。
- `cargo check --manifest-path src-tauri/Cargo.toml --locked --lib --bin gpteasy`：返回 0。
- test source 静态扫描没有 `StateStore`、`rusqlite`、`Connection::open` 或直接 repository call。
- RED/GREEN commits `355671b`、`6522446` 均存在，且未提交 `.planning/config.json` 或 `.planning/research/.cache/*`。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 17*
*Completed: 2026-08-07*
