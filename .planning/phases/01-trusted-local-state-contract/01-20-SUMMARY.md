---
phase: 01-trusted-local-state-contract
plan: 20
subsystem: database
tags: [rust, sqlite, migrations, fixture, deterministic, create-once]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-19 的 StateStore 生命周期锁、完整 secret-bearing snapshot 与跨进程重开路径"
provides:
  - "连续版本、唯一名称、SQL checksum 与 schema fingerprint 自校验的 append-only migration registry"
  - "固定 SQLite 配置、UUID、UTC 时间与合成凭据的 create-once v001 fixture generator"
  - "含文件、schema、data、logical digest 与假数据声明的永久 v001 数据库清单"
  - "StateStore 与 generator 共用 MIGRATIONS 的初始化、重开和漂移拒绝路径"
affects: [01-21 migration-matrix, 01-22 database-validator, 01-23 backup-rollback, STATE-03]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "migration metadata 只存在于 MIGRATIONS；CURRENT_SCHEMA_VERSION 从最后一项派生"
    - "历史 fixture 只允许 create-once，测试仅在 TempDir 重生成并比较"
    - "manifest 不保存凭据值，只保存不可逆摘要和 synthetic-only 声明"

key-files:
  created:
    - src-tauri/src/state/migrations/mod.rs
    - src-tauri/src/bin/generate_v001_fixture.rs
    - src-tauri/tests/fixture_generation.rs
    - tests/fixtures/databases/v001/state.sqlite3
    - tests/fixtures/databases/manifest.json
  modified:
    - src-tauri/src/state/mod.rs

key-decisions:
  - "APPLICATION_ID 与 CURRENT_SCHEMA_VERSION 由 migration registry 导出；生产 StateStore 不保留第二份 migration 名称、SQL、checksum 或 fingerprint。"
  - "v001 generator 对 DB 或 manifest 任一已存在的目标 fail-closed，并使用 create_new 锁与文件创建阻断并发覆盖。"
  - "fixture 固定 page_size、DELETE journal、UUID、UTC 时间和纯假 Key；schema/data canonical digest 与实际文件 SHA 分开记录。"
  - "StateStore 每次打开都先验证 registry 连续性、名称唯一性、SQL checksum 与 fingerprint，再进行任何状态目录或数据库写入。"

patterns-established:
  - "Append-only registry: 每个 migration 行内固定 version/name/sql/checksum/schema_fingerprint，历史只能追加。"
  - "Create-before-verify: committed fixture 由显式单次生成建立，所有验证只消费或在 TempDir 重生成。"

requirements-completed: [STATE-03]

coverage:
  - id: D1
    description: "create-once generator 生成字节与逻辑均确定的 v001 SQLite fixture，并拒绝覆盖 DB 或 manifest 任一既有输出。"
    requirement: STATE-03
    verification:
      - kind: integration
        ref: "src-tauri/tests/fixture_generation.rs#two_fresh_generations_are_byte_and_logically_deterministic"
        status: pass
      - kind: integration
        ref: "src-tauri/tests/fixture_generation.rs#generator_refuses_every_existing_output_without_modification"
        status: pass
      - kind: integration
        ref: "src-tauri/tests/fixture_generation.rs#committed_v001_is_the_create_once_generator_output"
        status: pass
    human_judgment: false
  - id: D2
    description: "生产 StateStore 与 fixture generator 消费同一 MIGRATIONS，并在启动时拒绝未注册账本行或历史 identity 漂移。"
    requirement: STATE-03
    verification:
      - kind: unit
        ref: "src-tauri/src/state/mod.rs#state_store_writes_the_exact_registry_identity"
        status: pass
      - kind: unit
        ref: "src-tauri/src/state/mod.rs#state_store_rejects_unregistered_ledger_rows"
        status: pass
      - kind: integration
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart --test state_persistence --test fixture_generation"
        status: pass
    human_judgment: false

# Metrics
duration: 1h 08m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 20: 确定性历史 fixture 与单一迁移注册表 Summary

**以自校验 append-only registry、create-once generator 和固定合成数据交付可永久验证的 v001 SQLite 历史输入**

## Performance

- **Duration:** 1 小时 08 分钟
- **Started:** 2026-08-07T06:52:36Z
- **Completed:** 2026-08-07T08:00:44Z
- **Tasks:** 2/2 TDD
- **Files modified:** 6

## Accomplishments

- `MIGRATIONS` 固化连续版本、唯一名称、内嵌 SQL、SHA-256 checksum 与 schema fingerprint，并在 StateStore 触碰状态目录前完成自校验。
- `generate_v001_fixture` 使用固定 SQLite 页/journal 设置、固定 UUID、固定 UTC 时间和两组明确的假凭据创建 v001；两个独立目录的数据库字节、schema/data/logical digest 完全一致。
- generator 在 DB 或 manifest 任一已存在时返回非零且保持原字节不变；committed fixture 不由测试重生成或覆盖。
- manifest 分开记录实际文件 SHA、schema/data/logical digest、application ID、user version、migration identity 与 synthetic-only 声明，不包含假 Key 字节或真实凭据。
- StateStore 初始化遍历同一 `MIGRATIONS` 写 SQL 与账本，重开时逐行精确比较完整账本并拒绝未注册版本。

## Task Commits

1. **Task 1 RED: 添加 v001 fixture 失败测试** - `690cb13` (`test`)
2. **Task 1 GREEN: 生成确定性 v001 历史 fixture** - `9da41c7` (`feat`)
3. **Task 2 RED: 添加状态迁移注册表失败测试** - `8125e0c` (`test`)
4. **Task 2 GREEN: 统一 StateStore 迁移注册表** - `9b0a228` (`feat`)

## Files Created/Modified

- `src-tauri/src/state/migrations/mod.rs` - append-only registry、checksum/fingerprint 算法与启动自校验。
- `src-tauri/src/state/mod.rs` - 从 registry 导出当前 identity，顺序初始化并精确验证完整迁移账本。
- `src-tauri/src/bin/generate_v001_fixture.rs` - 固定合成数据、canonical digest、create-new/no-overwrite fixture generator。
- `src-tauri/tests/fixture_generation.rs` - 双目录确定性、no-overwrite、只读身份与 committed fixture 一致性测试。
- `tests/fixtures/databases/v001/state.sqlite3` - 64 KiB 的永久 schema v1 SQLite 历史输入。
- `tests/fixtures/databases/manifest.json` - v001 文件与逻辑身份、migration identity 及假数据声明。

## Decisions Made

- registry 是 migration identity 的唯一事实源；`state/mod.rs` 只 re-export application/current version，且所有 SQL、名称、checksum 和 fingerprint 均从 `MIGRATIONS` 读取。
- registry validation 在 `create_dir_all` 和 coordinator 之前运行，使源历史漂移在任何本地状态写入之前 fail-closed。
- committed fixture 使用稳定的合成凭据而非空数据，以便未来 migration matrix 证明真实表间关联与敏感字段保存；manifest 只公开摘要和假数据分类。
- byte SHA 与 logical digest 分开：前者锁定 committed artifact，后者稳定表达 schema/data 语义并可供后续历史矩阵验证。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 缩短 migration ledger 查询临时借用生命周期**

- **Found during:** Task 2 GREEN 首次 unit test 编译。
- **Issue:** 块尾直接返回 `query_map(...).collect(...)` 使 `MappedRows` 临时析构晚于 statement，Rust 报 E0597。
- **Fix:** 将收集结果绑定到局部 `rows` 后再作为块结果返回，确保查询迭代器先于 statement 析构。
- **Files modified:** `src-tauri/src/state/mod.rs`
- **Verification:** `cargo test --manifest-path src-tauri/Cargo.toml --lib state::tests` 2/2 通过；Clippy `-D warnings` 通过。
- **Committed in:** `9b0a228`

---

**Total deviations:** 1 auto-fixed（Rule 3：1）
**Impact on plan:** 仅修正 Rust 借用作用域以完成既定 registry 比对，不改变 schema、fixture bytes 或公共行为。

## Issues Encountered

- Task 1 首次 Cargo/Tauri 冷构建耗时约 9 分 20 秒；等待到编译器明确报告缺少 `CARGO_BIN_EXE_generate_v001_fixture` 后才确认 RED，没有把等待或超时当作失败证据。
- Task 2 RED 通过注入 `0002_unregistered` 账本行得到真实断言失败，证明旧实现只匹配 v1 行而未验证完整 registry。
- Windows 测试链接继续输出既有的 DLL import library 信息 warning；Clippy `--all-targets -D warnings`、locked check 与全部计划测试均返回 0。

## TDD Gate Compliance

- Task 1：RED `690cb13` 明确因 generator binary 缺失而编译失败；GREEN `9da41c7` 后 fixture tests 3/3 通过。
- Task 2：RED `8125e0c` 明确因未注册账本行仍被接受而断言失败；GREEN `9b0a228` 后 registry unit tests 2/2 通过。
- Git 顺序为 `test → feat → test → feat`，两项任务均保留独立 RED/GREEN 提交。

## Authentication Gates

None。

## Known Stubs

None。修改文件中没有 TODO、FIXME、placeholder、跳过测试、空 UI 数据源或测试时重写 committed fixture 的旁路。

## User Setup Required

None - 本计划不需要真实供应商凭据、外部服务、签名资源或人工系统配置。

## Next Phase Readiness

- 01-21 可直接从 `tests/fixtures/databases/manifest.json` 枚举 v001，先验证 file/application/user_version/fingerprint/logical identity，再只复制到 TempDir 执行 migration matrix。
- 01-21 必须锁定已提交的 0001 SQL、fixture bytes 和 fingerprint；后续版本只能向 `MIGRATIONS` 末尾追加，不能修改 v001 历史字节。
- STATE-03 仍是 01-20/01-21 的共享 requirement；本计划建立历史输入与 registry，完整历史升级矩阵由 01-21 关闭。

## Self-Check: PASSED

- 5 个 created key files 与 `src-tauri/src/state/mod.rs` 均存在；TDD commits `690cb13`、`9da41c7`、`8125e0c`、`9b0a228` 均存在于 main。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib state::tests`：2 passed。
- `cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart --test state_persistence --test fixture_generation`：9 passed。
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 与 `cargo check --manifest-path src-tauri/Cargo.toml --locked --lib --bins` 均返回 0。
- 重复对 committed 目标运行 generator 返回 1，DB 与 manifest SHA-256 前后完全不变；双 TempDir 测试证明 file/schema/data/logical digest 一致。
- stub/skip 与 manifest forbidden-secret 扫描无命中；没有引入 plan threat model 之外的新网络、认证或用户路径 surface。
- 未暂存或修改 `.planning/config.json`、`.planning/research/.cache/*` 与既有 `.planning/HANDOFF.json` 删除。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 20*
*Completed: 2026-08-07*
