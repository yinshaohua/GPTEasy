---
phase: 01-trusted-local-state-contract
plan: 19
subsystem: local-state
tags: [rust, tauri, sqlite, file-lock, installed-smoke, local-only]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-18 的完整 secret-bearing StateSnapshot、生产 commands 与跨进程恢复摘要"
provides:
  - "production phase1-state-smoke seed/verify/cleanup 固定入口与完整状态摘要"
  - "依赖、capability、command、DTO、前端 import 与 Web API 的 local-only 自动门禁"
  - "在任何 SQLite RW/WAL/初始化写入前取得的跨进程 OS exclusive File lock"
  - "崩溃释放、StateBusy non-mutation 与陈旧 owner metadata 覆盖的两进程证据"
affects: [01-20 migration-history, 01-21 historical-fixtures, 01-23 backup-rollback, 01-24 restore, 01-26 windows-package, 01-27 macos-package]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "installed smoke 只接受 mode 与 opaque run ID，状态根固定由 app_local_data_dir 派生"
    - "cleanup 在 marker 匹配后按固定文件允许清单逐项删除，拒绝 symlink、目录和未知工件"
    - "StateStore 生命周期持有 std::fs::File OS lock；owner sidecar 不参与 ownership 判定"

key-files:
  created:
    - src-tauri/src/state_smoke.rs
    - src-tauri/src/state/coordination.rs
    - src-tauri/tests/installed_state_smoke.rs
    - src-tauri/tests/local_only_boundary.rs
    - src-tauri/tests/state_concurrency.rs
  modified:
    - src-tauri/src/lib.rs
    - src-tauri/src/state/mod.rs

key-decisions:
  - "state smoke 根固定为 app_local_data_dir/contract-smoke/state/<opaque-run-id>；CLI 不接受任何路径。"
  - "verify 只重开并逐项比较固定完整 snapshot 与不可逆 digest；只有显式 cleanup 可以删除匹配 marker 的 run-scoped 状态。"
  - "StateCoordinator guard 在 database_path 探测、只读 preflight、RW open、WAL 配置及初始化事务之前取得，并由 StateStore 持有到 Connection 销毁。"
  - "owner metadata 仅含 schema、PID、进程启动 token 与 run ID SHA-256 摘要；锁所有权只由 File::try_lock 的 OS 结果决定。"

requirements-completed: []

coverage:
  - id: D1
    description: "production binary 以 phase1-state-smoke 名称执行 seed、两个独立 verify 与显式 cleanup；verify 输出稳定且不删除状态。"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "src-tauri/tests/installed_state_smoke.rs#production_cli_seed_verify_and_explicit_cleanup_preserve_truthful_state"
        status: pass
    human_judgment: false
  - id: D2
    description: "Cargo/npm 依赖、Tauri capability、注册 commands、公开 DTO、前端 imports/Web APIs 与 COVERAGE 使用精确 local-only 允许清单。"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "src-tauri/tests/local_only_boundary.rs"
        status: pass
    human_judgment: false
  - id: D3
    description: "第二进程在 production setup 中得到 StateBusy，且 DB、WAL、owner 与恢复工件快照不变。"
    requirement: STATE-03
    verification:
      - kind: integration
        ref: "src-tauri/tests/state_concurrency.rs#os_lock_serializes_writers_releases_after_crash_and_ignores_stale_metadata"
        status: pass
    human_judgment: false
  - id: D4
    description: "kill holder 后 Windows 由 OS 释放 LockFileEx ownership；新进程成功重开并覆盖陈旧诊断 metadata。"
    requirement: STATE-04
    verification:
      - kind: integration
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --test state_concurrency"
        status: pass
    human_judgment: false

# Metrics
duration: 1h 06m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 19: 可信 installed smoke、本地边界与跨进程协调 Summary

**以不提前清理的完整状态 CLI、精确 local-only 允许清单和 Windows LockFileEx 生命周期 guard 交付可恢复的本地状态协调边界**

## Performance

- **Duration:** 1 小时 06 分钟
- **Started:** 2026-08-07T05:33:20Z
- **Completed:** 2026-08-07T06:39:03Z
- **Tasks:** 2/2 TDD
- **Files modified:** 7

## Accomplishments

- 新增 production `phase1-state-smoke seed|verify|cleanup <run-id>`：固定假全量状态含两个 provider/verification、native 与 WSL2 独立关联及完整 settings；报告只输出安全计数与不可逆 digest。
- seed 与 verify 使用真实 Tauri `app_local_data_dir` 和独立 OS 进程；verify 连续执行结果完全一致且保留 DB、marker 和协调工件，显式 cleanup 后才无法再次 verify。
- cleanup 不接受路径；在固定产品根和 opaque run ID 之外不解析任何目标，先验证普通目录与精确 marker，再拒绝 symlink、子目录和未列入允许清单的文件。
- local-only gate 精确比较 Cargo/npm 依赖、capability、生产 invoke handler、公开序列化 DTO、前端 import 与浏览器网络/存储 API，证明没有产品账户、云同步、默认上传或 Web Storage surface。
- 新增 `StateCoordinator`：Rust 1.97.1 标准库 `File::try_lock` 在 Windows 对应 `LockFileEx`；guard 在任何 SQLite/DB/WAL 写 seam 前取得，并与 `StateStore` 同生命周期。
- 两进程测试证明竞争者得到固定 `StateBusy` 且不修改状态；强制终止 holder 后 OS 释放锁，新进程忽略并原子覆盖陈旧 owner metadata。

## Task Commits

1. **Task 1 RED: 添加安装状态与本地边界失败测试** - `105bece` (`test`)
2. **Task 2 RED: 添加跨进程状态协调失败测试** - `6358d90` (`test`)
3. **Task 1 GREEN: 实现可保留的安装状态冒烟** - `10e1436` (`feat`)
4. **Task 2 GREEN: 建立跨进程状态协调锁** - `78ca14e` (`feat`)
5. **Task 2 verification fix: 明确保留状态锁文件内容** - `cfe961e` (`fix`)

## Files Created/Modified

- `src-tauri/src/state_smoke.rs` - 固定 full-state fixture、seed/verify/cleanup、marker 和 bounded cleanup。
- `src-tauri/src/state/coordination.rs` - OS lock、500ms bounded retry、atomic owner sidecar 与 crash-safe ownership。
- `src-tauri/src/lib.rs` - production CLI 分派，同时保留既有 path smoke 与桌面启动。
- `src-tauri/src/state/mod.rs` - 在 DB preflight/RW open 前取得 coordinator，并让 StateStore 生命周期持有 guard。
- `src-tauri/tests/installed_state_smoke.rs` - 实际 production binary 的三模式、多进程保留与显式清理测试。
- `src-tauri/tests/local_only_boundary.rs` - local-only 依赖、capability、command、DTO、前端与 COVERAGE 允许清单。
- `src-tauri/tests/state_concurrency.rs` - production composition/IPC holder、OS ownership probe、StateBusy non-mutation、kill/reopen 与 stale metadata 测试。

## Decisions Made

- 状态 smoke 不复用正式产品根数据库，而是在同一 `app_local_data_dir` 下使用固定 `contract-smoke/state/<run-id>` run-scoped 子根；它证明真实路径语义，同时不接受测试注入路径。
- marker 只证明 run identity/schema/digest 匹配，不证明进程持锁或 disposable；cleanup 还必须通过普通目录、普通文件和文件名允许清单检查。
- coordinator 使用持久 lock file 的 OS ownership，不删除 lock path 解锁；正常 Drop 显式 unlock，panic/kill/进程崩溃依赖 OS 关闭 handle。
- `state-lock-owner.json` 用同目录临时文件同步后原子替换；Windows flags 固定为 `0`，metadata 永远不参与 `StateBusy` 决策。
- `STATE-01..05` 是多个后续计划共同覆盖的 shared requirements，本计划记录覆盖证据但不提前把迁移、备份和恢复 requirements 标为全部完成。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 将 state smoke 实现放入独立 production module**

- **Found during:** Task 1 GREEN。
- **Issue:** 计划只列出 `lib.rs` 作为生产文件，但完整 snapshot、marker、路径和 cleanup 合同若全部内联会破坏已有最小 composition root，并使安全边界难以独立测试和审计。
- **Fix:** 新增 `src-tauri/src/state_smoke.rs`；`lib.rs` 只保留固定 command 分派、Tauri root 初始化和脱敏输出。
- **Files modified:** `src-tauri/src/state_smoke.rs`, `src-tauri/src/lib.rs`
- **Committed in:** `10e1436`

**2. [Rule 1 - Bug] 明确 lock file 的非截断打开语义**

- **Found during:** 计划级 Clippy `-D warnings`。
- **Issue:** `OpenOptions.create(true)` 未显式声明 truncate 行为，被 `suspicious_open_options` 拒绝；状态锁必须保持同一文件身份且不能因 open 清空。
- **Fix:** 增加 `.truncate(false)`，随后 Clippy、locked check 与 concurrency 回归全部通过。
- **Files modified:** `src-tauri/src/state/coordination.rs`
- **Committed in:** `cfe961e`

---

**Total deviations:** 2 auto-fixed（Rule 3：1，Rule 1：1）
**Impact on plan:** 只补齐安全模块边界与显式文件打开语义；没有增加网络、账户、任意路径、测试专用数据库入口或 metadata ownership 旁路。

## Issues Encountered

- 清理后的首次 Cargo/Tauri RED 构建耗时 11 分 03 秒；执行器等待到明确的 production CLI 失败，没有把冷构建无输出或超时当作 RED。
- 最初 concurrency RED harness 用 stdout 等待无限 holder，产生测试同步超时；改为 test-only ready 文件和 RAII kill/reap 后得到真实的“缺少 `state.lock`” RED。
- Tauri mock 将 production `setup` 延迟到第一次 `run_iteration`；contender 改为执行该 seam 并捕获固定 `StateBusy` payload，避免误把 `build()` 成功当成 coordinator 成功。
- Windows holder 锁定的 `state.lock` 不能被普通读取；non-mutation 快照只记录 lock 文件存在，仍逐字节比较 DB/WAL/owner 和其他恢复工件。

## TDD Gate Compliance

- Task 1 RED `105bece` 明确得到 production CLI 未注册与 local-only name gate 失败，随后 GREEN `10e1436` 通过 4/4 任务测试。
- Task 2 RED `6358d90` 明确得到 production state root 缺少 `state.lock`，随后 GREEN `78ca14e` 通过 OS ownership、StateBusy、kill/reopen 与 stale metadata 测试。
- `cfe961e` 是 GREEN 后静态验证发现的单行安全修复，不改变 RED→GREEN 顺序。

## Authentication Gates

None。

## Known Stubs

None。没有 TODO、FIXME、placeholder、跳过测试、空 UI 数据源或直接 StateStore/repository/SQLite 测试旁路。

## User Setup Required

None - 本计划不需要真实供应商凭据、外部服务、签名资源或人工系统配置。

## Next Phase Readiness

- 01-20 及后续 migration/backup/restore 写路径必须继续复用同一个 `StateStore` 生命周期 guard；不得在取得 coordinator 前创建 RW connection、WAL、backup 或 quarantine。
- installed verify 后的 run-scoped 状态可以由后续 recovery/package smoke 继续消费；cleanup allowlist 如新增正式 backup 子结构，必须显式扩展并新增拒绝未知工件测试。
- Windows/macOS 正式安装、签名、公证和真实工件 evidence 仍由 01-26/01-27 关闭；本计划没有把本地开发 smoke 晋升为发布证据。
- `STATE-01..05` 继续由 01-20 至 01-24 的历史迁移、backup/rollback 和 higher-schema restore 计划共同完成。

## Self-Check: PASSED

- 7 个 key files 均存在；RED/GREEN/fix commits `105bece`、`6358d90`、`10e1436`、`78ca14e`、`cfe961e` 均存在于 main。
- `cargo test --manifest-path src-tauri/Cargo.toml --test installed_state_smoke --test local_only_boundary --test state_concurrency -- --nocapture`：6 passed。
- `path_smoke` 2/2、`state_command_restart` 3/3、`state_persistence` 3/3，无既有状态合同回归。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：返回 0。
- `cargo check --manifest-path src-tauri/Cargo.toml --locked --lib --bin gpteasy`：返回 0。
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`：返回 0；stub/skip 扫描无命中。
- `StateCoordinator::acquire` 位于 `database_path` 探测和全部 `Connection::open` 之前；生产 CLI 名称由实际 binary 测试并由 local-only gate 静态确认。
- 可再生成的 `src-tauri/target`（5.1 GiB）与 `src-tauri/gen` 已清理；未提交 `.planning/config.json`、`.planning/research/.cache/*` 或预先存在的 `.planning/HANDOFF.json` 删除。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 19*
*Completed: 2026-08-07*
