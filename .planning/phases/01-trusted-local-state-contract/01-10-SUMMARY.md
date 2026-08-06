---
phase: 01-trusted-local-state-contract
plan: 10
subsystem: local-state-path
tags: [rust, tauri, apphandle, app-local-data, atomic-write, subprocess-test]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-09 的最小 Tauri composition root、current-user bundle contract 与 exact Cargo 工程"
provides:
  - "只接受 1–64 位 ASCII 字母数字或连字符的 phase1-path-smoke CLI"
  - "固定 app_local_data_dir/contract-smoke/path 状态根与原子 marker 写入"
  - "真实 Tauri mock AppHandle 的独立进程 reopen 与脱敏边界测试"
affects: [sqlite-state-store, installed-path-smoke, windows-packaging, macos-packaging]

# Tech tracking
tech-stack:
  added:
    - "Tauri test feature（仅 dev-dependency）"
  patterns:
    - "AppHandle PathResolver 是产品状态根的唯一来源，调用者不能传入路径"
    - "固定目录内先写同根临时文件、sync_all 后 rename 提交非敏感 JSON"
    - "integration test executable 以独立子进程复用生产 path smoke predicate"

key-files:
  created:
    - src-tauri/src/path_smoke.rs
    - src-tauri/tests/path_smoke.rs
  modified:
    - src-tauri/src/lib.rs
    - src-tauri/Cargo.toml
    - src-tauri/build.rs

key-decisions:
  - "path smoke 状态固定为 app_local_data_dir/contract-smoke/path/{opaque-id}.json，CLI 与生产函数都不接受路径、配置正文或环境值。"
  - "marker 只保存 run_id、OS、arch 与 schema；运行报告仅额外包含 reopened，错误与输出不暴露绝对用户路径。"
  - "跨进程测试使用 Tauri mock Context 的 identifier 将真实 AppHandle PathResolver 隔离到临时根，并在 Windows 测试目标链接 tauri-build 生成的 Common Controls v6 resource。"

patterns-established:
  - "路径型诊断入口先验证 opaque ID，再解析 AppHandle 状态根，并只在固定产品子目录执行文件操作。"
  - "重开语义必须由当前 integration-test executable 的独立子进程证明，不能只在同一进程重复调用。"

requirements-completed: [STATE-01, STATE-02]

coverage:
  - id: D1
    description: "phase1-path-smoke 只接受严格 opaque ID，并通过真实应用 wiring 使用 AppHandle 的 app_local_data_dir 固定根"
    requirement: STATE-02
    verification:
      - kind: unit
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --lib path_smoke"
        status: pass
      - kind: integration
        ref: "cargo check --manifest-path src-tauri/Cargo.toml --locked --lib --bin gpteasy"
        status: pass
    human_judgment: false
  - id: D2
    description: "同一 opaque ID 的两个独立测试进程分别返回 reopened=false 与 reopened=true"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "src-tauri/tests/path_smoke.rs#independent_processes_reopen_only_the_fixed_temp_root"
        status: pass
    human_judgment: false
  - id: D3
    description: "marker 与报告仅含非敏感合同字段，且测试临时根的 outside canary 保持不变"
    requirement: STATE-02
    verification:
      - kind: unit
        ref: "src-tauri/src/path_smoke.rs#report_serialization_has_only_non_sensitive_contract_fields"
        status: pass
      - kind: integration
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --test path_smoke"
        status: pass
    human_judgment: false

# Metrics
duration: 40min
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 10: 固定当前用户状态根 Path Smoke Summary

**以真实 Tauri AppHandle、严格 opaque ID 和独立子进程测试建立固定当前用户状态根的原子 path smoke contract**

## Performance

- **Duration:** 40 分钟
- **Started:** 2026-08-06T06:25:42Z
- **Completed:** 2026-08-06T07:06:02Z
- **Tasks:** 2/2
- **Files modified:** 5

## Accomplishments

- 新增 `phase1-path-smoke <opaque-id>` 应用入口，只允许 1–64 位 ASCII 字母、数字或连字符，不接受路径、配置正文、环境值或额外参数。
- 通过 `AppHandle::path().app_local_data_dir()` 获取唯一状态根，并在固定 `contract-smoke/path` 子目录以同根临时文件、`sync_all` 和 `rename` 原子提交非敏感 marker。
- marker 只保存 run ID、OS、arch 与 schema；报告仅额外返回 `reopened`，测试确认 stdout/stderr 不包含临时绝对路径或用户 profile。
- integration test 使用真实 Tauri mock AppHandle，并由当前测试可执行文件的两个独立子进程证明首次 `reopened=false`、再次 `reopened=true`，同时验证越界输入不创建状态目录。
- 通过 unit、integration、locked production check 与 Clippy `-D warnings`。

## Task Commits

Each task was committed atomically:

1. **Task 1 RED: 添加路径冒烟失败测试** - `fad8086` (`test`)
2. **Task 1 GREEN: 实现固定根路径冒烟与 CLI wiring** - `94fa208` (`feat`)
3. **Task 2: 验证真实 AppHandle 的跨进程重开** - `d3645c3` (`test`)
4. **Threat hardening: 在状态根解析前验证 opaque ID** - `64530ab` (`fix`)

**Plan metadata:** 最终元数据提交包含本 SUMMARY、STATE、ROADMAP 与必要的 REQUIREMENTS 同步。

## Files Created/Modified

- `src-tauri/src/path_smoke.rs` - opaque ID validator、固定 AppLocalData 状态根、原子 marker、脱敏 report 与 unit tests。
- `src-tauri/tests/path_smoke.rs` - Tauri mock AppHandle、独立测试子进程、临时目录树和越界输入验证。
- `src-tauri/src/lib.rs` - `phase1-path-smoke` 参数解析、真实 Tauri App 构造、AppHandle 调用和 JSON 输出。
- `src-tauri/Cargo.toml` - 仅测试启用 Tauri mock feature，并声明 path_smoke integration target。
- `src-tauri/build.rs` - Windows 测试目标链接 tauri-build 生成的 application resource，保证 Common Controls v6 loader contract。

## Decisions Made

- 调用者永远不能指定状态根；生产路径只来自当前 Tauri `AppHandle` 的 `app_local_data_dir`，之后追加固定产品子目录与已验证 opaque ID 文件名。
- 同一 ID 的已存在 marker 必须完整匹配 run ID、OS、arch 与 schema 才能报告 `reopened=true`；损坏或不匹配内容 fail-closed，不覆盖既有文件。
- 跨进程 contract 使用 integration-test executable 自身作为子进程宿主，避免用同进程缓存或测试替身冒充 reopen。
- Windows test executable 必须链接与应用相同的 resource manifest；该设置只作用于显式 test targets，不重复链接生产 binary resource。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 补齐 Tauri mock feature 与 Windows test resource linking**

- **Found during:** Task 1 RED 与 Task 2 integration target 首次编译。
- **Issue:** `tauri::test` 由 `test` feature 门控，但计划文件清单未包含 `Cargo.toml`；启用后 Windows unit test executable 因未链接 Common Controls v6 manifest，以 `STATUS_ENTRYPOINT_NOT_FOUND` 无法启动。将 resource 泛化到所有 targets 又导致生产 binary 重复 resource。
- **Fix:** 在 dev-dependency 启用 exact `tauri = 2.11.5` 的 `test` feature，声明显式 `path_smoke` test target，并让 `build.rs` 仅通过 `rustc-link-arg-tests` 链接 tauri-build 生成的 `resource.lib`。
- **Files modified:** `src-tauri/Cargo.toml`, `src-tauri/build.rs`
- **Verification:** unit test executable 正常启动；`cargo test --manifest-path src-tauri/Cargo.toml --test path_smoke` 返回 0；production `cargo check --locked --lib --bin gpteasy` 返回 0。
- **Committed in:** `fad8086`, `d3645c3`

**2. [Rule 2 - Missing Critical] 在解析任何状态路径前重复验证 opaque ID**

- **Found during:** 计划级威胁面复核。
- **Issue:** CLI parser 已先验证 ID，但公开生产函数最初在解析 AppLocalData 根后才由内部 helper 验证，未对非 CLI 调用方形成同样严格的前置边界。
- **Fix:** `run_path_smoke` 入口先执行相同 validator，再调用 `AppHandle` PathResolver；内部 helper 保留重复验证作为纵深防御。
- **Files modified:** `src-tauri/src/path_smoke.rs`
- **Verification:** unit 与 integration path smoke suites 全部通过，无效 ID 运行后临时状态树与 baseline 完全一致。
- **Committed in:** `64530ab`

---

**Total deviations:** 2 auto-fixed（Rule 3：1，Rule 2：1）
**Impact on plan:** 两项修复均为完成真实 Tauri mock 测试和收紧路径信任边界所必需，没有新增产品路径、凭据、网络、账户或 WebView 权限。

## Issues Encountered

- 第一次全量 Rust 测试编译超过初始命令超时；延长超时后完成依赖构建并进入预期 RED。
- libtest 的 `--nocapture` 会把子进程标记输出接在测试状态行之后；integration parser 改为定位固定非敏感前缀，不依赖行首格式。
- Windows integration build 仍输出“创建 import library”的非阻断 linker 消息；测试返回 0，`cargo clippy --all-targets -- -D warnings` 返回 0。

## Known Stubs

None。未发现 TODO、FIXME、placeholder、跳过测试或未接线数据路径。

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- 当前用户固定状态根、opaque ID 与跨进程 reopen contract 已可被后续 SQLite `StateStore`、installed package smoke 和平台 evidence workflow 复用。
- 下一未完成计划为 `01-13-PLAN.md`；既有 01-11 Windows/WSL 探针与本计划 path smoke 可供 Windows package lifecycle 消费。
- `.planning/config.json` 的既有修改和 `.planning/research/.cache/*` 未跟踪缓存保持原样，未被本计划修改或提交。

## Self-Check: PASSED

- `src-tauri/src/path_smoke.rs`、`src-tauri/tests/path_smoke.rs` 与 `src-tauri/src/lib.rs` 均存在，Cargo/build test wiring 已提交。
- 任务提交 `fad8086`、`94fa208`、`d3645c3`、`64530ab` 均存在于当前 main 历史。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib path_smoke`：4 passed。
- `cargo test --manifest-path src-tauri/Cargo.toml --test path_smoke`：2 passed，包括两个独立子进程的 reopen 验证。
- `cargo check --manifest-path src-tauri/Cargo.toml --locked --lib --bin gpteasy` 与 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 均返回 0。
- 生成的 `src-tauri/target` 与 `src-tauri/gen` 内容已清理，Git 工作树只保留用户既有 config 修改和 research cache。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 10*
*Completed: 2026-08-06*
