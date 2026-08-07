---
phase: 01-trusted-local-state-contract
plan: 18
subsystem: local-state
tags: [rust, tauri, sqlite, domain-model, ipc, secrets]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-17 的 SQLite schema v1、StateStore、production builder 与跨进程 command tracer"
provides:
  - "供应商、明文 API Key、验证记录、受管环境关联和完整设置的权威 SQLite 持久化"
  - "不可变 UUID、组合指纹、FK/唯一性和内置/自定义供应商匹配的领域不变量"
  - "不暴露 Key、地址、模型或平台身份的完整状态脱敏投影与不可逆摘要"
affects: [01-19 installed-smoke, 01-20 migration-history, 01-22 validation, phase-2 secret-boundary]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "secret-bearing command input 不实现 Debug/Serialize，SecretString Debug 固定输出 <redacted>"
    - "完整 snapshot 在 IMMEDIATE transaction 内按 FK 顺序整体替换，提交后从 SQLite 权威重读"
    - "版本化 length-prefixed canonical hash 同时证明秘密内部状态完整恢复并保持公开不可逆"

key-files:
  created:
    - src-tauri/src/domain/mod.rs
    - src-tauri/src/state/repositories.rs
  modified:
    - src-tauri/src/state/mod.rs
    - src-tauri/src/commands.rs
    - src-tauri/src/lib.rs
    - src-tauri/tests/state_persistence.rs

key-decisions:
  - "ProviderId 与 EnvironmentId 只接受 UUID；显示名、地址、模型、Key 和 platform identity 均不承担主键语义。"
  - "组合指纹沿用 gpteasy-provider-combination-v1 域并绑定 base URL、默认模型与 API Key；任何不匹配的验证记录在写入前拒绝。"
  - "公开 state_digest 使用 gpteasy-state-snapshot-v1、big-endian 长度前缀、固定 enum/option/bool 编码和 UUID 排序，覆盖全部 secret-bearing 权威字段。"
  - "完整状态写入使用单个 SQLite IMMEDIATE transaction，公开 command 只返回 identity、status、计数、完整无秘密 settings 与不可逆摘要。"

requirements-completed: [STATE-01, STATE-02]

coverage:
  - id: D1
    description: "两个供应商及不同明文假 Key、两份验证记录、native 与 WSL2 环境的不同 current provider 和完整 settings 经两个独立 OS 子进程逐字段恢复。"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "src-tauri/tests/state_persistence.rs#complete_state_round_trips_across_processes_without_secret_output"
        status: pass
      - kind: integration
        ref: "cargo test --manifest-path src-tauri/Cargo.toml --test state_persistence -- --nocapture"
        status: pass
    human_judgment: false
  - id: D2
    description: "领域层拒绝非 UUID 身份、重复身份、悬空 provider 关联、built-in/custom 不匹配和与关键配置不一致的组合指纹。"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "src-tauri/tests/state_persistence.rs#full_state_write_child_process"
        status: pass
      - kind: other
        ref: "src-tauri/src/domain/mod.rs"
        status: pass
    human_judgment: false
  - id: D3
    description: "公开 snapshot、固定错误、domain Debug 与子进程 stdout/stderr 不包含两个 Key canary，测试只通过 production Tauri mock IPC 访问状态。"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "src-tauri/tests/state_persistence.rs#complete_state_round_trips_across_processes_without_secret_output"
        status: pass
      - kind: other
        ref: "rg StateStore/rusqlite/Connection::open/direct repository calls src-tauri/tests/state_persistence.rs"
        status: pass
    human_judgment: false

# Metrics
duration: 25m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 18: 完整权威状态持久化 Summary

**以 UUID 领域不变量、参数化 SQLite repositories、secret-safe commands 和两个真实 OS 子进程交付完整权威状态重开**

## Performance

- **Duration:** 25 分钟
- **Started:** 2026-08-07T03:53:00Z
- **Completed:** 2026-08-07T04:18:00Z
- **Tasks:** 1/1 TDD
- **Files modified:** 6

## Accomplishments

- 新增完整领域模型：两个独立 UUID 身份类型、脱敏 `SecretString`、供应商/验证/环境/settings 类型，以及唯一性、FK、供应商种类和组合指纹不变量。
- 新增参数化 repositories，在单个 IMMEDIATE transaction 内整体替换 provider/verification/environment/settings，并从 SQLite 按 UUID 稳定排序恢复完整内部 snapshot。
- 注册 `replace_state_snapshot` 与 `bootstrap_state_snapshot` production commands；secret-bearing 输入不支持 Debug/Serialize，公开 DTO 只含 identity/status/计数/无秘密 settings 和不可逆 state digest。
- 两个独立 OS 子进程分别通过 production Tauri mock IPC 写入与重开；两个假 API Key、验证记录、native/WSL2 不同关联和完整设置深比较一致，stdout/stderr 与公开 JSON canary 扫描为零。

## Task Commits

1. **Task 1 RED: 添加完整状态跨进程失败测试** - `f69f14c` (`test`)
2. **Task 1 GREEN: 持久化完整权威状态** - `23e0149` (`feat`)

## Files Created/Modified

- `src-tauri/src/domain/mod.rs` - UUID 身份、秘密类型、完整状态模型、不变量、组合指纹和 canonical state digest。
- `src-tauri/src/state/repositories.rs` - provider/verification/environment/settings 的参数化事务写入与稳定顺序读取。
- `src-tauri/src/state/mod.rs` - 完整 snapshot transaction/read API，并保留 01-17 settings tracer。
- `src-tauri/src/commands.rs` - 不可调试的秘密输入 DTO、固定错误和脱敏完整状态 commands。
- `src-tauri/src/lib.rs` - 在 production invoke handler 注册两个完整状态 commands 并公开 domain module。
- `src-tauri/tests/state_persistence.rs` - command-name mock IPC、两个子进程 round-trip、固定摘要与 canary 扫描。

## Decisions Made

- API Key 按 ADR-0001/0006 明文留在 SQLite 与内部 domain model；`SecretString` 不实现 Serialize，Debug 永远为 `<redacted>`，公开投影不包含 Key、base URL、model 或 platform identity。
- provider verification 只有在版本化 SHA-256 组合指纹与同一 provider 的 base URL、默认模型、API Key 完全匹配时才可进入权威状态。
- state digest 使用版本化域、8-byte big-endian 长度/数量、显式 option/bool/enum 字节和 UUID 排序；它覆盖秘密但不可逆，可由公开 DTO 比较跨进程完整性。
- snapshot replacement 删除与插入均位于同一 IMMEDIATE transaction，任何约束或完整性检查失败都会回滚，不暴露部分新状态。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 在 production composition root 注册新增 commands**

- **Found during:** Task 1 GREEN command wiring。
- **Issue:** 计划的 `files_modified` 漏列 `src-tauri/src/lib.rs`，但 RED 已证明仅实现函数而不注册 invoke handler 时 production IPC 仍会返回 command not found。
- **Fix:** 在现有 `configure_builder` 的唯一 invoke handler 中注册 `replace_state_snapshot` 与 `bootstrap_state_snapshot`，并公开新增 domain module。
- **Files modified:** `src-tauri/src/lib.rs`
- **Verification:** `state_persistence` 与既有 `state_command_restart` 各 3 个测试通过，Clippy `-D warnings` 通过。
- **Committed in:** `23e0149`

---

**Total deviations:** 1 auto-fixed（Rule 3：1）
**Impact on plan:** 仅补齐 production command 的必要 composition wiring；没有增加 test-only 数据库入口、旁路 repository 或公开秘密字段。

## Issues Encountered

- 清理后的 Windows/Tauri 依赖首次构建较慢，第一次 RED 在 2 分钟命令超时前没有产出结论；复用已生成缓存并放宽超时后得到明确的 `Command replace_state_snapshot not found` RED，随后全目标 Clippy 首次构建约 5 分钟通过。

## Known Stubs

None。没有 TODO、FIXME、placeholder、跳过测试或直接数据库测试旁路。

## User Setup Required

None - 本计划不需要真实供应商凭据、外部服务或签名 PFX 文件。

## Next Phase Readiness

- 01-19 可以在完整 `StateStore` 与脱敏 command seam 上补齐 truthful installed state smoke、local-only 静态证据和跨进程 coordinator。
- 数据库 migration history、备份/恢复和 higher-schema 拒写仍由 01-20 至 01-24 交付；本计划没有提前扩大 schema version 1。
- Windows/macOS 正式签名与公证继续延期到 01-26/01-27，当前不需要 PFX。

## Self-Check: PASSED

- `cargo test --manifest-path src-tauri/Cargo.toml --test state_persistence -- --nocapture`：3 passed；write/read 子进程的完整公开响应与固定 digest 一致。
- `cargo test --manifest-path src-tauri/Cargo.toml --test state_command_restart`：3 passed；01-17 commands 无回归。
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`：返回 0。
- `cargo check --manifest-path src-tauri/Cargo.toml --locked --lib --bin gpteasy`：返回 0。
- test source 静态扫描没有 `StateStore`、`rusqlite`、`Connection::open` 或直接 repository call。
- RED/GREEN commits `f69f14c`、`23e0149` 均存在，且未提交 `.planning/config.json`、`.planning/research/.cache/*` 或构建产物。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 18*
*Completed: 2026-08-07*
