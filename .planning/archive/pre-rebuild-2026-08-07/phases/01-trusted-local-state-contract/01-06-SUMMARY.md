---
phase: 01-trusted-local-state-contract
plan: 06
subsystem: contracts
tags: [github-cli, github-api, attestations, preflight, human-approval, powershell]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    plan: 05
    provides: "固定 gh 最低版本、认证、repository、Actions 与 attestations 只读权限检查"
provides:
  - "获准 GitHub CLI 身份对 yinshaohua/GPTEasy、Actions runs/artifacts 与 attestations 的人工授权确认"
  - "PowerShell 7.6.4 下真实只读 preflight 返回 0 的门禁记录"
  - "01-13 之后真实 evidence 流程必须继续对任何 preflight 非零结果 fail-closed"
affects: [01-13 workflows, 01-14 workflows, 01-15 evidence, 01-16 evidence]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "人工批准只记录脱敏结论，不记录登录身份、token、认证头或 raw gh config"
    - "门禁批准与 artifact 下载/验签分离；preflight 成功不声明 artifact_verified"

key-files:
  created:
    - .planning/phases/01-trusted-local-state-contract/01-06-SUMMARY.md
  modified: []

key-decisions:
  - "接受 PowerShell 7.6.4 下真实只读 preflight 的成功结果，解除 01-06 blocking-human checkpoint。"
  - "本次批准只证明固定 gh 环境门禁通过，不宣称 Windows PowerShell 5.1 已完成同等真实网络兼容性验证。"
  - "后续真实 evidence 路径仍必须运行固定 preflight；任何非零结果继续阻断下载与验签。"

requirements-completed: [STATE-02, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "固定最低版本、登录、repository、Actions runs/artifacts 与 attestations 只读能力均通过真实 preflight"
    verification:
      - kind: integration
        ref: "PowerShell 7.6.4: scripts/contracts/preflight-gh-evidence.ps1 -Repository yinshaohua/GPTEasy -MinimumVersion 2.49.0"
        status: pass
    human_judgment: true
  - id: D2
    description: "获准身份由用户确认，规划记录不包含身份、token、认证头或 raw gh config"
    verification:
      - kind: human
        ref: "用户回复“确认”"
        status: pass
    human_judgment: true

# Metrics
duration: 约 10 分钟（不含等待人工确认）
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 6: gh Evidence Preflight Approval Summary

**用户确认获准 GitHub CLI 身份具备固定仓库与 evidence API 的最小只读能力，并接受 PowerShell 7.6.4 下真实 preflight 成功结果**

## Performance

- **Duration:** 约 10 分钟（不含 blocking-human checkpoint 的等待时间）
- **Completed:** 2026-08-06
- **Tasks:** 1/1
- **Files modified:** 1 个规划产物

## Accomplishments

- 记录用户已回复“确认”：当前 GitHub CLI 登录身份是获准身份，可读取 `yinshaohua/GPTEasy`、Actions runs/artifacts 与 attestations。
- 接受 PowerShell 7.6.4 下真实只读 preflight 返回 0 的结果；版本、认证、repository、Actions 和 attestations 检查均为 pass。
- 保持脱敏边界：本记录不包含登录身份、token、认证头、原始 GitHub CLI 配置或命令原始响应。
- 明确 01-06 只解除执行依赖，不执行或冒充真实 artifact 下载、digest 交叉验证或 attestation 验签。
- 固定后续 fail-closed 规则：01-13 之后任何真实 evidence 路径若 preflight 非零，必须在下载或验签前阻断。

## Task Commits

1. **Task 1: 确认 gh 安装、认证与仓库权限** — checkpoint 无生产文件变更，由本计划最终规划元数据提交统一记录。

## Verification

- PowerShell 7.6.4 下真实只读 preflight — PASS，退出码为 0。
- 固定最低版本门禁 — PASS，当前 gh 满足 `>= 2.49.0`。
- GitHub CLI 登录授权 — PASS，由用户确认当前身份获准。
- Repository read — PASS，目标固定为 `yinshaohua/GPTEasy`。
- Actions read — PASS，覆盖 runs 与 artifacts。
- Attestations read — PASS，权限探针可读；此结论不表示任何具体 artifact 已验证。
- 脱敏人工复核 — PASS，SUMMARY 不记录身份、token、认证头、raw gh config 或原始 API 响应。

## PowerShell 兼容性注记

- 本次获准的真实只读结果来自 **PowerShell 7.6.4**；用户明确接受该运行时下的成功结果作为 01-06 门禁证据。
- 计划示例中的 `powershell` 在 Windows 上通常解析为 Windows PowerShell 5.1。本次批准**不宣称**已在 Windows PowerShell 5.1 下完成同等真实网络 preflight，也不把 7.6.4 的成功外推为 5.1 兼容性证明。
- 后续自动化应继续使用已验证的 PowerShell 7.6.4 路径；如果改用 Windows PowerShell 5.1，必须独立运行并通过同一固定只读 preflight，且不得放宽仓库、最低版本或权限检查。
- 该兼容性边界不阻断本计划完成，因为本计划门禁的目标是确认当前获准 gh 环境，而用户已明确接受 7.6.4 下的真实成功结果。

## Requirements Coverage

- **STATE-02:** 远程权限门禁只保存脱敏布尔结论，不保存 GitHub 身份或认证材料，也不触发默认上传。
- **STATE-03 / STATE-04 / STATE-05:** 为后续迁移、备份和 higher-schema 的真实外部 evidence 提供显式 gh 前置门禁；这些要求的 SQLite 行为仍由对应后续计划实现和验证。

## Decisions Made

- 将用户“确认”视为 blocking-human checkpoint 的明确批准，并从 Task 1 checkpoint 之后完成计划收口。
- 接受 PowerShell 7.6.4 的真实只读 preflight 成功结果，不要求为了本门禁重复执行 artifact 下载或验签。
- 将 Windows PowerShell 5.1 兼容性保留为独立运行时约束，不把它误写为本次已验证事实。
- 后续依赖计划即使已解除 01-06 依赖，也不能绕过各自运行时 preflight。

## Deviations from Plan

None - plan executed exactly as written. PowerShell 5.1 注记仅界定本次获准证据的运行时范围，没有改变门禁策略。

## Issues Encountered

None.

## Authentication Gates

- **Task 1 blocking-human checkpoint:** 已解决。
- **所需确认:** 当前 GitHub CLI 登录身份获准读取固定仓库、Actions runs/artifacts 与 attestations。
- **结果:** 用户回复“确认”，并接受 PowerShell 7.6.4 下真实只读 preflight 成功结果。
- **脱敏:** 未记录身份值、token、认证头或 raw gh config。

## User Setup Required

None - 当前门禁已批准。若未来认证、权限或工具版本变化，真实 evidence 流程会通过固定 preflight 重新 fail-closed。

## Next Phase Readiness

- 01-13/01-14 workflows 与 01-15/01-16 evidence 计划的 `depends_on: 01-06` 门禁已解除。
- 本计划没有下载 evidence、验证 digest 或执行 attestation；这些动作仍由后续计划按其自身门禁执行。
- 后续任何 preflight 非零结果都必须阻断真实 evidence，不得以本次历史批准覆盖当前失败。

## Known Stubs

None.

## Self-Check: PASSED

- `01-06-SUMMARY.md` 已创建，状态为 `complete`。
- ROADMAP 已标记 01-06 完成并按磁盘 SUMMARY 数量更新为 10/28。
- STATE 已按当前 main 的既有完成情况更新为 10/28（36%），下一未完成执行位置仍为 01-09。
- STATE-02、STATE-03、STATE-04、STATE-05 在 REQUIREMENTS 中此前已为 Complete，本计划确认无需重复改写。
- 本计划只有 blocking-human checkpoint，没有独立生产 task commit；最终规划元数据提交在 SUMMARY 自检后执行。
- 未执行真实 evidence 下载、digest 交叉验证或 attestation 验签。
- 未记录身份、token、认证头、raw gh config 或原始 API 响应。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 06*
*Completed: 2026-08-06*
