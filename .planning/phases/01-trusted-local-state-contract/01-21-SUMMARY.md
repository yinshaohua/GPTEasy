---
phase: 01-trusted-local-state-contract
plan: 21
subsystem: database
tags: [rust, sqlite, migrations, history-lock, powershell, policy-lint]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-20 的 append-only migration registry、确定性 v001 fixture 与 manifest"
provides:
  - "从 manifest 动态枚举、验证身份并只升级 TempDir 副本的全历史迁移矩阵"
  - "锁定 migration SQL、fixture bytes/logical digest 与 schema fingerprint 的 history lock"
  - "只读比较工作树及已合并 v* tag 的历史漂移门禁"
  - "拒绝非事务 SQL 与 Rust 文件/进程/网络能力的迁移策略 lint"
affects: [01-22 database-validator, 01-23 backup-rollback, STATE-03]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "历史 fixture 只从 manifest 枚举，先验证完整身份，再复制到 TempDir 升级"
    - "history lock 与 registry/manifest/tag 三向比较，缺少 release tag 时仍完整验证本地历史"
    - "SelfTest 与真实扫描复用同一文件级 policy predicate，fixture 永不授予 strict eligibility"

key-files:
  created:
    - .gitignore
    - src-tauri/tests/migration_matrix.rs
    - tests/fixtures/databases/history-lock.json
    - scripts/contracts/verify-migration-history.ps1
    - scripts/contracts/verify-migration-policy.ps1
    - tests/fixtures/migrations/forbidden-migration-cases.json
  modified: []

key-decisions:
  - "migration matrix 不保存 fixture 版本数组；manifest 是历史样本集合的唯一枚举入口，仓库数据库始终只读。"
  - "history lock 固定 SQL、fixture 与 fingerprint 身份；无本地 v* tag 不构成跳过，仍必须通过 registry/manifest/文件一致性。"
  - "policy SelfTest 固定 test_only=true、strict_gate_eligible=false；只有真实 repository scan 可以产生正式 lint 通过结果。"
  - "Rust migration 源按能力 token fail closed，transform_* 只能接收单个 &Transaction 参数。"

patterns-established:
  - "Manifest-driven migration matrix: 身份验证发生在复制前，所有升级写入仅落在临时副本。"
  - "Release history drift: 使用 git cat-file 原始字节比较 tag 中实际存在的同版本 SQL/fixture。"
  - "Migration capability lint: SQL 状态机 lexer 与遮蔽注释/字符串后的 Rust token allowlist。"

requirements-completed: [STATE-03]

coverage:
  - id: D1
    description: "manifest 中每个正式历史数据库在身份校验后顺序应用生产 migration 与 test-only v2/v3，且原件 hash/mtime 不变。"
    requirement: STATE-03
    verification:
      - kind: integration
        ref: "src-tauri/tests/migration_matrix.rs#manifest_drives_every_historical_fixture_through_the_upgrade_matrix"
        status: pass
    human_judgment: false
  - id: D2
    description: "history lock 与工作树 registry/manifest/文件一致，并拒绝 SQL、fixture、fingerprint 或已合并 release tag 漂移。"
    requirement: STATE-03
    verification:
      - kind: contract
        ref: "powershell -NoProfile -File scripts/contracts/verify-migration-history.ps1"
        status: pass
      - kind: negative
        ref: "临时 Git 仓库中的 fingerprint、SQL、fixture bytes 与 merged v* tag 四类漂移"
        status: pass
    human_judgment: false
  - id: D3
    description: "迁移策略拒绝 VACUUM/ATTACH/DETACH/journal_mode 及 Rust 文件、路径、进程、网络、Connection 能力。"
    requirement: STATE-03
    verification:
      - kind: contract
        ref: "scripts/contracts/verify-migration-policy.ps1 -SelfTest"
        status: pass
      - kind: contract
        ref: "scripts/contracts/verify-migration-policy.ps1 -RepositoryRoot ."
        status: pass
    human_judgment: false

# Metrics
duration: 35m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 21: 历史迁移矩阵与安全门禁 Summary

**以 manifest-driven 临时升级矩阵、跨工作树/tag history lock 和非事务副作用 lint 交付可执行的 SQLite 永久历史约束**

## Performance

- **Duration:** 约 35 分钟
- **Started:** 2026-08-07T08:12:15Z
- **Completed:** 2026-08-07T08:47:29Z
- **Tasks:** 3/3 TDD
- **Files modified:** 6

## Accomplishments

- migration matrix 从 `manifest.json` 动态枚举全部 fixture，重算 file/schema/data/logical digest，并校验 application ID、user version、schema fingerprint 与 migration ledger 后才复制到 TempDir。
- 每个临时副本先顺序补齐生产 registry，再应用 test-only v2/v3；最终 ledger 每版本恰好一行，checksum/fingerprint/user_version、FK 与 quick_check 全部一致，仓库 DB 的字节与 mtime 保持不变。
- `history-lock.json` 固定 0001 SQL、v001 fixture 与 schema fingerprint；verifier 同时核对 registry/manifest/实际文件，并以原始 git blob 字节比较所有已合并 `v*` tag 中存在的历史。
- migration policy lexer 在 SQL 注释/字符串之外拒绝 VACUUM、ATTACH、DETACH 与 `PRAGMA journal_mode`；Rust lint 遮蔽注释/字符串后拒绝文件、路径、进程、网络与 Connection，并限制 transform 只接收 Transaction。
- policy fixture 覆盖 8 个 SQL 与 9 个 Rust 正反 case；SelfTest 明确不可获得 strict eligibility，真实仓库扫描独立通过。

## Task Commits

1. **Task 1 RED: 添加历史迁移矩阵失败测试** - `7ba4599` (`test`)
2. **Task 1 GREEN: 建立历史迁移矩阵** - `2ca56ba` (`feat`)
3. **Task 2 RED: 添加迁移历史漂移失败门禁** - `429ecf1` (`test`)
4. **Task 2 GREEN: 固定迁移历史锁与漂移门禁** - `7ce3833` (`feat`)
5. **Task 3 RED: 添加迁移禁止项失败用例** - `7a57a78` (`test`)
6. **Task 3 GREEN: 阻断迁移非事务副作用** - `902a40c` (`feat`)
7. **Task 3 FIX: 阻断 Rust 文件能力别名绕过** - `e034b13` (`fix`)

## Files Created/Modified

- `.gitignore` - 忽略 Cargo target 与 Tauri 生成 schema，避免测试构建产物污染工作树。
- `src-tauri/tests/migration_matrix.rs` - manifest identity、只读原件、临时 v2/v3 顺序升级与完整性验证。
- `tests/fixtures/databases/history-lock.json` - migration SQL、fixture 与 schema fingerprint 的不可变身份锁。
- `scripts/contracts/verify-migration-history.ps1` - registry/manifest/文件/tag 原始字节漂移门禁。
- `scripts/contracts/verify-migration-policy.ps1` - SQL lexer、Rust capability lint、SelfTest 与真实扫描入口。
- `tests/fixtures/migrations/forbidden-migration-cases.json` - 非事务 SQL、外部文件/进程/网络副作用及正控制 fixture。

## Decisions Made

- fixture 集合不在 Rust 测试中手写；后续新增历史样本只更新 manifest/lock，矩阵自动覆盖。
- history verifier 不调用 generator，也不因当前没有 merged release tag 而跳过；本轮 tag 数为 0，但本地 lock/registry/manifest/文件全部实时重算通过。
- tag 比较使用 `git cat-file blob` 的重定向原始字节，而不是 PowerShell 文本管道，避免 SQLite 二进制内容被编码转换。
- policy lexer 把 SQL 双引号、方括号与反引号内容当作标识符处理，单引号字符串和注释不参与禁止 token 判定；未闭合词法状态直接失败。
- Rust capability 采用代码 token 级拒绝，覆盖 grouped import 别名；fixture SelfTest 只证明 predicate 行为，不升级为正式门禁证据。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 忽略首次 Cargo/Tauri 构建产物**

- **Found during:** Task 1 RED 冷编译。
- **Issue:** 仓库没有根级 `.gitignore`，Cargo target 与 Tauri schema 以未跟踪生成目录留在工作树。
- **Fix:** 新增精确忽略规则 `/src-tauri/target/` 与 `/src-tauri/gen/schemas/`。
- **Files modified:** `.gitignore`
- **Verification:** Task 1 后 `git status --short` 不再列出两个生成目录。
- **Committed in:** `2ca56ba`

**2. [Rule 3 - Blocking] 兼容 Windows PowerShell 5.1 的 git 原始字节读取**

- **Found during:** Task 2 GREEN 首次 history verifier 运行。
- **Issue:** Windows PowerShell 5.1 的 `ProcessStartInfo` 没有 `ArgumentList` 属性，tag blob 读取在比较前失败。
- **Fix:** 增加 Windows native argument 编码并设置 `Arguments`，仍直接从重定向 `BaseStream` 读取 blob bytes。
- **Files modified:** `scripts/contracts/verify-migration-history.ps1`
- **Verification:** 正例通过；临时 Git 仓库中四类漂移均返回非零。
- **Committed in:** `7ce3833`

**3. [Rule 1 - Bug] 阻断 grouped std::fs alias 绕过**

- **Found during:** Task 3 GREEN 后的威胁面复查。
- **Issue:** `use std::{fs as disk}; disk::write(...)` 不含 `fs::`，原 capability regex 会误放行。
- **Fix:** 文件/进程能力改为遮蔽后代码 token 级拒绝，并增加 grouped import alias 负例。
- **Files modified:** `scripts/contracts/verify-migration-policy.ps1`, `tests/fixtures/migrations/forbidden-migration-cases.json`
- **Verification:** 新负例修复前明确失败，修复后 SQL 8/8、Rust 9/9 与真实扫描全部通过。
- **Committed in:** `e034b13`

---

**Total deviations:** 3 auto-fixed（Rule 1：1，Rule 3：2）
**Impact on plan:** 均加强既定验证路径的可运行性与 fail-closed 边界，没有修改 production schema、fixture bytes 或公共状态 API。

## Issues Encountered

- Task 1 RED 首次 Rust/Tauri 冷构建耗时 502 秒；等待编译器明确报告缺少 `upgrade_fixture_copy` 后才确认 RED，没有把耗时或超时当成失败证据。
- Clippy/check 全 target 回归耗时约 4 分钟；全部返回 0，仅保留既有 Windows DLL import library linker stdout warning。

## TDD Gate Compliance

- Task 1：RED `7ba4599` 明确因 `upgrade_fixture_copy` 缺失而 E0425；GREEN `2ca56ba` 后 migration matrix 1/1 通过。
- Task 2：RED `429ecf1` 明确因 `history-lock.json` 缺失返回 1；GREEN `7ce3833` 后当前历史与四类漂移负例通过。
- Task 3：RED `7a57a78` 明确因 SQL predicate 缺失返回 1；GREEN `902a40c` 后 SelfTest/真实扫描通过，`e034b13` 追加别名绕过负例与修复。
- Git 顺序保持 `test → feat → test → feat → test → feat → fix`。

## Authentication Gates

None。

## Known Stubs

None。修改文件中没有 TODO、FIXME、placeholder、跳过测试、空 UI 数据源或以 fixture 结果授予正式 eligibility 的旁路。

## User Setup Required

None - 本计划不需要真实供应商凭据、网络访问、签名资源或人工系统配置。

## Next Phase Readiness

- 01-22 可以复用 history lock、manifest identity 与 migration policy gate 构建数据库 validator，不需要维护第二份 fixture 版本列表。
- 后续 migration 只能追加 registry/SQL/fixture/lock；任何已存在版本的 SQL、fingerprint 或已发布 fixture 改写都会返回非零。
- STATE-03 的历史升级链与 migration 单事务能力边界已有可执行 backstop。

## Self-Check: PASSED

- 6 个 key files 全部存在；TDD/fix commits `7ba4599`、`2ca56ba`、`429ecf1`、`7ce3833`、`7a57a78`、`902a40c`、`e034b13` 均存在于 main。
- `cargo test --manifest-path src-tauri/Cargo.toml --test migration_matrix`：1 passed。
- history gate：migration=1、fixture=1、merged release tags=0，返回 0；临时 tag/漂移负例 4/4 被拒绝。
- policy gate：SelfTest SQL 8/Rust 9 与真实 SQL 1/Rust 1 扫描均返回 0；SelfTest 固定 strict_gate_eligible=false。
- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与 `cargo check --locked --lib --bins` 均返回 0。
- stub/skip 扫描无命中；没有引入 plan threat model 之外的网络 endpoint、认证路径、数据库 schema 或用户文件写入 surface。
- 未暂存或修改 `.planning/config.json`、`.planning/research/.cache/*` 与既有 `.planning/HANDOFF.json` 删除。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 21*
*Completed: 2026-08-07*
