---
phase: 01-trusted-local-state-contract
plan: 09
subsystem: desktop-shell
tags: [rust, tauri, sqlite, nsis, macos, capability, reproducible-build]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-08 的 exact Tauri CLI/React/Vite 前端壳与可重复 npm 构建"
provides:
  - "精确锁定的 Rust/Tauri/SQLite Cargo 工程与 Cargo.lock"
  - "仅授权 main window core:default 的最小 Tauri capability"
  - "薄 main 入口、可测试 composition root 与当前用户 bundle contract"
affects: [phase-1-state-tracer, sqlite-state-store, windows-packaging, macos-packaging]

# Tech tracking
tech-stack:
  added:
    - "Rust edition 2021"
    - "tauri 2.11.5 / tauri-build 2.6.3"
    - "rusqlite 0.40.1（bundled, backup）"
    - "chrono 0.4.45 / serde 1.0.229 / serde_json 1.0.151"
    - "sha2 0.11.0 / thiserror 2.0.19 / uuid 1.24.0"
    - "windows-sys 0.61.2（仅 cfg(windows)）/ tempfile 3.27.0（仅 dev）"
  patterns:
    - "main 二进制只调用 gpteasy_lib::run()，Tauri composition root 保持在 library"
    - "WebView capability 只允许 main window 的 core:default，不注册业务 command 或高权限 plugin"
    - "Windows NSIS 固定 currentUser，macOS 固定 minimumSystemVersion 14.0"

key-files:
  created:
    - src-tauri/Cargo.toml
    - src-tauri/Cargo.lock
    - src-tauri/build.rs
    - src-tauri/capabilities/default.json
    - src-tauri/tauri.conf.json
    - src-tauri/src/main.rs
    - src-tauri/src/lib.rs
    - src-tauri/icons/icon.ico
  modified: []

key-decisions:
  - "Rust 直接依赖严格采用 01-RESEARCH 的 exact pins；rusqlite 关闭默认 features，仅启用 bundled 与 backup。"
  - "Tauri runtime 不注册 command、托盘、账户、网络或 updater，capability 仅保留 main window 的 core:default。"
  - "桌面 bundle identifier 固定为 com.gpteasy.desktop；Windows 使用 NSIS currentUser，macOS 最低版本固定为 14.0。"

patterns-established:
  - "后续 Rust 状态层从 gpteasy_lib composition root 接入，二进制入口不承载业务逻辑。"
  - "新增 Tauri 原生能力必须显式扩展 capability，并重新审计 WebView 到 runtime 的权限边界。"

requirements-completed: [STATE-01, STATE-02]

coverage:
  - id: D1
    description: "Cargo.toml 与 Cargo.lock 精确锁定 Rust/Tauri/SQLite 依赖图，且不包含高权限 Tauri plugin"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "cargo metadata --manifest-path src-tauri/Cargo.toml --locked --no-deps"
        status: pass
    human_judgment: false
  - id: D2
    description: "main 仅调用 library run，最小 Tauri composition root 可完成 library 测试编译与 debug no-bundle 构建"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --lib"
        status: pass
      - kind: integration
        ref: "npm run tauri -- build --debug --no-bundle"
        status: pass
    human_judgment: false
  - id: D3
    description: "main window 只拥有 core:default capability，bundle contract 固定严格 CSP、Windows currentUser 与 macOS 14"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "Tauri 2 build-time config/capability validation during npm run tauri -- build --debug --no-bundle"
        status: pass
    human_judgment: false

# Metrics
duration: 约 26 分钟
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 9: 最小 Tauri 当前用户应用壳 Summary

**以 exact Cargo 依赖、最小 core capability 和薄 Rust composition root 建立可编译的 Tauri 2 当前用户桌面进程**

## Performance

- **Duration:** 约 26 分钟
- **Started:** 2026-08-06T05:43:44Z
- **Completed:** 2026-08-06T06:09:31Z
- **Tasks:** 2/2
- **Files modified:** 8 个生产文件

## Accomplishments

- 创建 edition 2021 Cargo 工程，精确锁定 Tauri 2.11.5、rusqlite 0.40.1 及研究批准的 Rust 依赖，并提交可重复的 `Cargo.lock`。
- 将 WebView 权限限制为 `main` window 的 `core:default`，未添加 fs、shell、HTTP、SQL、updater 或其他高权限 plugin。
- 创建只调用 `gpteasy_lib::run()` 的薄二进制入口，以及不注册业务 command 的最小 Tauri composition root。
- 固定 `com.gpteasy.desktop`、严格 CSP、Windows NSIS `currentUser` 和 macOS 14.0 最低系统版本 contract。
- 通过 locked Cargo metadata、library test 编译和 Tauri debug no-bundle 构建。

## Task Commits

Each task was committed atomically:

1. **Task 1: 锁定 Rust/Tauri 工程与最小 capability** - `8082c2b` (`chore`)
2. **Task 2: 建立薄入口与当前用户 bundle contract** - `baed5d0` (`feat`)

**Plan metadata:** 最终元数据提交包含本 SUMMARY、STATE、ROADMAP 与 REQUIREMENTS 更新。

## Files Created/Modified

- `src-tauri/Cargo.toml` - edition 2021 crate、library/bin targets 与 exact Rust/Tauri/SQLite dependencies。
- `src-tauri/Cargo.lock` - 可重复的 Cargo 依赖图。
- `src-tauri/build.rs` - `tauri_build::build()` 入口。
- `src-tauri/capabilities/default.json` - 仅允许 `main` window 使用 `core:default`。
- `src-tauri/tauri.conf.json` - frontendDist、主窗口、严格 CSP、bundle identifier 与当前用户平台 contract。
- `src-tauri/src/main.rs` - 仅调用 `gpteasy_lib::run()` 的薄入口。
- `src-tauri/src/lib.rs` - 最小且可测试的 Tauri composition root。
- `src-tauri/icons/icon.ico` - Windows Tauri build resource 所需的确定性应用图标。

## Decisions Made

- Rust 依赖全部采用研究中已审计的 exact versions；`rusqlite` 明确关闭默认 features，仅启用 `bundled` 与 `backup`。
- 保持 composition root 最小：不注册业务 command、托盘、账户、网络、数据库 plugin 或 updater，后续状态层只能从 Rust library 边界接入。
- capability 仅允许 `main` window 的 `core:default`；任何新增原生能力都必须通过独立计划和威胁审计显式授权。
- Windows 打包目标固定为 NSIS `currentUser`，macOS 最低系统版本固定为 14.0；不把管理员安装或旧 macOS 纳入 contract。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 添加 Windows Resource 编译所需的应用图标**

- **Found during:** Task 2（建立薄入口与当前用户 bundle contract）的首次 `cargo test --lib`。
- **Issue:** `tauri-build` 在 Windows 上生成 Resource 文件时要求 `src-tauri/icons/icon.ico`；计划文件清单未包含该必要资源，构建以缺少图标失败。
- **Fix:** 添加确定性的 32×32 ICO 应用图标，不引入新 crate、plugin 或运行时权限。
- **Files modified:** `src-tauri/icons/icon.ico`
- **Verification:** 修复后 `cargo test --manifest-path src-tauri/Cargo.toml --lib` 与 `npm run tauri -- build --debug --no-bundle` 均返回 0。
- **Committed in:** `baed5d0`

---

**Total deviations:** 1 auto-fixed（Rule 3：1）
**Impact on plan:** 仅补充 Tauri Windows 编译所需的静态资源，没有扩展产品功能、依赖图、权限或网络范围。

## Issues Encountered

- 工作树按上一计划清理了 `node_modules`，首次调用 Tauri CLI 时命令不可用；按已批准的 `package-lock.json` 执行隔离 registry 的 `npm ci --ignore-scripts --no-audit --no-fund` 后完成验证。
- Windows linker 输出了创建 import library 的非阻断提示；构建返回 0，未发现代码或权限问题。
- 验证生成的 `node_modules`、`dist` 与 `src-tauri/gen` 已在提交后清理，未进入 Git。

## Known Stubs

None。composition root 的无业务 command 状态是本计划明确的最小权限目标，不是未接线的产品 stub。

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Rust/Tauri 进程、前端构建和当前用户 bundle contract 已可被后续 SQLite `StateStore` 与平台 smoke 计划消费。
- 后续计划应继续保持 `main.rs` 只调用 library `run()`，并从 `gpteasy_lib` 内接入状态初始化。
- 新增 command 或 plugin 前必须同步扩展 capability、测试本地模式边界并重新审计 WebView → Tauri runtime 威胁面。

## Self-Check: PASSED

- 8 个计划生产文件及必要的 `src-tauri/icons/icon.ico` 均已创建并存在。
- 任务提交 `8082c2b`、`baed5d0` 均存在于当前 main 历史。
- `cargo metadata --manifest-path src-tauri/Cargo.toml --locked --no-deps`、`cargo test --manifest-path src-tauri/Cargo.toml --lib` 与 `npm run tauri -- build --debug --no-bundle` 均返回 0。
- `.planning/config.json` 的既有修改与 `.planning/research/.cache/*` 未跟踪文件保持原样，未被本计划提交。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 09*
*Completed: 2026-08-06*
