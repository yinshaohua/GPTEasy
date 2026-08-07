---
phase: 01-trusted-local-state-contract
plan: 16
subsystem: database-contracts
tags: [sqlite, schema-v1, backup, recovery, human-approval]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-15 的 non_signing_contract Freeze 与独立 PhaseComplete 硬门"
provides:
  - "freeze-approved：只批准非签名技术合同进入本地磁盘决策"
  - "approve-schema-version-1：批准六张 STRICT 表与永久 schema v1 身份合同"
  - "approve-db-backup-contract：批准 verified backup、三份 retention 与 quarantine-first restore"
affects: [01-17 state-tracer, 01-20 migration-history, 01-23 database-backup, 01-24 database-recovery]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "单向磁盘合同必须在最新 non-signing Freeze 全绿后由用户明确批准"
    - "平台发布身份与本地 schema/backup 技术决策分离，签名延期不能晋升为 PhaseComplete"

key-files:
  created:
    - .planning/phases/01-trusted-local-state-contract/01-16-SUMMARY.md
  modified: []

key-decisions:
  - "freeze-approved：Freeze 仅冻结非签名技术合同；Windows Authenticode 与 macOS Developer ID/notary 继续延期，PhaseComplete 仍 blocked。"
  - "approve-schema-version-1：固定 APPLICATION_ID、user_version=1、database UUID、schema fingerprint、双账本与六张 STRICT 表；发布后只允许追加迁移。"
  - "approve-db-backup-contract：固定 UTC/schema/database UUID/backup UUID 身份、sidecar digest、三份 verified retention、同源复用、opaque restore 与 verified quarantine。"

patterns-established:
  - "Approval token：后续计划只接受 Summary 中的精确批准令牌作为 one-way 实现前置条件。"
  - "Recovery ordering：validate → coordinator → verified quarantine → same-directory atomic replace → preflight。"

requirements-completed: [STATE-01, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "用户在最新 Freeze Local Strict 六项非签名检查全绿后明确批准 freeze-approved，并保留四目标签名/公证延期。"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict"
        status: pass
    human_judgment: true
    rationale: "Freeze 范围与发布身份隔离属于明确的人类信任决定，机器结果不能替用户批准。"
  - id: D2
    description: "用户明确批准永久 schema version 1、六张 STRICT 表、数据库身份、schema fingerprint 与双账本。"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict"
        status: pass
    human_judgment: true
    rationale: "0001 migration 被用户数据库消费后只能追加演进，必须保留明确的 one-way 人工批准。"
  - id: D3
    description: "用户明确批准 verified SQLite backup identity、三份 retention、同源复用与 quarantine-first atomic restore。"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict"
        status: pass
    human_judgment: true
    rationale: "历史 snapshot 和 sidecar 会形成长期兼容面，恢复替换也涉及唯一用户数据库。"

# Metrics
duration: 1h 14m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 16: Schema 与 Backup One-Way 合同批准 Summary

**在非签名 Freeze 全绿且发布身份仍明确延期的边界内，批准永久 SQLite schema v1 与 verified backup/recovery 合同**

## Performance

- **Duration:** 1h 14m（从首次 Freeze 执行计，不含前置上下文恢复与人工等待）
- **Started:** 2026-08-07T01:08:29Z
- **Completed:** 2026-08-07T02:22:13Z
- **Tasks:** 3/3 blocking-human checkpoints
- **Files modified:** 1

## Accomplishments

- 最新 `Freeze Local Strict` 两次真实执行均返回 0；最终复核六项 checks 全部通过，`freeze_kind=non_signing_contract`、`release_ready=false`、`blocking_reasons=[]`。
- 用户明确输入 `freeze-approved`，确认本次结果只支持本地 schema/backup 决策，不代表 Windows/macOS 发布身份已验证。
- 用户明确输入 `approve-schema-version-1`，批准 APPLICATION_ID、schema v1、database UUID、schema fingerprint、双账本和六张 STRICT 表。
- 用户明确输入 `approve-db-backup-contract`，批准 Online Backup、sidecar identity、三份 verified retention、同源复用、opaque ID、verified quarantine 与原子恢复。

## Task Commits

三个任务均为不修改产品文件的阻断式人工 checkpoint，没有独立 task commit；本 Summary 由计划元数据提交记录。

## Files Created/Modified

- `.planning/phases/01-trusted-local-state-contract/01-16-SUMMARY.md` — 永久记录三个批准令牌、one-way 理由和发布签名延期边界。

## Decisions Made

### 1. freeze-approved

- Freeze 只覆盖 runner、provenance、path smoke、Windows/WSL2 contract 与跨平台 packaging 的非签名合同。
- `windows-x64-authenticode`、`windows-arm64-authenticode`、`macos-intel-developer-id-notarization`、`macos-apple-silicon-developer-id-notarization` 继续 deferred。
- 当前不读取、创建或替代 PFX、Developer ID 或 notarization 材料；PhaseComplete 仍 blocked。

### 2. approve-schema-version-1

- 固定 APPLICATION_ID、`PRAGMA user_version=1`、database UUID、schema fingerprint 与 `schema_migrations` 双账本。
- 固定 `providers`、`provider_verifications`、`managed_environments`、`app_settings`、`state_metadata`、`schema_migrations` 六张 STRICT 表。
- Provider/environment 使用不可变 ID；API Key 按 ADR 明文入库但不进入日志、公开投影或诊断材料。
- 运行中进程、配置 Saga、WSL 切换和诊断日志不进入 v1 schema；0001 发布后只能追加迁移。

### 3. approve-db-backup-contract

- snapshot 使用 SQLite Online Backup API，而不是普通文件复制；关闭后由统一只读 validator 验证。
- 备份身份固定绑定 UTC、source/target schema、database UUID、backup UUID、schema fingerprint 与实际 SHA-256 sidecar。
- 只有 verified backup 参与复用和 retention，按合同字段保留最近三份；同源未变且同一迁移目标可以复用。
- restore 只接受后端枚举的 opaque backup ID；替换前必须先创建并验证当前数据库 quarantine。
- 恢复使用同目录临时文件与平台原子替换，失败时不得丢失当前唯一数据库副本。

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

- 研究稿的示例 DDL 只展示五张表；最终执行计划已补入 `state_metadata`，后续 01-17/01-22 明确依赖其 database UUID 与 schema fingerprint，因此按最终六表合同批准并记录。
- 首次 Freeze 因 Cargo 缓存已清理耗时约 8 分 38 秒并生成可再生构建目录；最终复核运行约 1 分 23 秒。

## User Setup Required

None - 本计划不需要外部服务配置，也没有读取任何签名或供应商 secret。

## Next Phase Readiness

- 01-17 已获得创建 `0001_initial.sql` 和生产 Tauri command→SQLite→新进程 bootstrap tracer 的前置批准。
- 01-23/01-24 已获得 backup/recovery 合同批准，但仍需先完成 01-17～01-22 的依赖实现和自动验证。
- Windows PFX、macOS Developer ID 与 notarization 继续延期到 01-26/01-27；01-28 PhaseComplete 仍必须取得正式四目标证据。

## Self-Check: PASSED

- 三个精确批准令牌均由用户在独立 blocking checkpoint 中明确输入。
- 最终 Freeze 返回 0，六项 checks 全部 passed，`release_ready=false` 且四项 deferred evidence 完整。
- Summary 精确复制 PLAN requirements，并记录 schema/backup one-way 理由和签名延期边界。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 16*
*Completed: 2026-08-07*
