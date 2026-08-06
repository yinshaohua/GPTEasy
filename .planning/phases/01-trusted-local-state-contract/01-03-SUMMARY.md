---
phase: 01-trusted-local-state-contract
plan: 03
subsystem: contracts
tags: [powershell, cli, matrix, exit-codes, plan-scan, security]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "Phase 1 验证策略、28 份计划和已有 npm 合同"
provides:
  - "Scope/Target/Mode 合法调用矩阵与稳定退出码合同"
  - "matrix-driven Phase 1 canonical runner，输出机器可读 JSON"
  - "Phase 1 automated runner consumer parser 与旧调用形状负例"
affects: [01-04 evidence provenance, 01-05 gh preflight, 01-07 source audit, 01-11-to-01-17 evidence, 01-26-to-01-28 final gates]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "runner-cli-matrix.json 作为 Scope/Target/Mode、组合、dispatch 和退出码的唯一事实源"
    - "PowerShell runner 手动解析参数，未知/旧参数统一返回 64 并输出 JSON"
    - "AllowBlocked 资源阻断返回 0，但 outcome=blocked 且 strict_gate_eligible=false"
    - "PLAN consumer scan 只解析 verify/automated 内容并拒绝未声明组合"

# Key files
key-files:
  created:
    - tests/fixtures/contracts/runner-cli-matrix.json
    - scripts/contracts/run-phase1-contracts.ps1
    - scripts/contracts/test-run-phase1-cli.ps1
  modified: []

decisions:
  - "RunnerSelfTest 和 ProvenanceSelfTest 只允许 Local+Strict；Freeze 和 PhaseComplete 只允许 Local+Strict。"
  - "AllowBlocked 只允许具体目标的 ContractSelfTest/PackagingSelfTest，并且 blocked 结果永远不可严格通过。"
  - "下游脚本或 manifest 尚未创建时，matrix dispatch 将 Strict 标记为 exit 3 blocked，而 AllowBlocked 标记为 exit 0 blocked。"

metrics:
  duration: "约 35 分钟"
  completed: 2026-08-06
  tasks: 2
  files: 3
status: complete
---

# Phase 1 Plan 3: 唯一 runner CLI 合同 Summary

**用唯一 matrix 固定 Phase 1 runner 的合法 Scope/Target/Mode 组合、0/2/3/4/5/64 退出码和所有计划消费者的机器扫描边界**

## Performance

- **Duration:** 约 35 分钟
- **Completed:** 2026-08-06
- **Tasks:** 2/2
- **Files modified:** 3

## Accomplishments

- 创建 `runner-cli-matrix.json`，精确声明 6 个 Scope、6 个 Target、2 个 Mode、16 个合法 Scope/Target dispatch 以及固定退出码语义。
- 实现 `run-phase1-contracts.ps1`：手动解析必需参数，拒绝缺参、未知参数、旧式 `-Freeze`/`-PhaseComplete` 开关和未声明组合；每次调用输出统一 JSON。
- 实现 matrix-driven dispatch：RunnerSelfTest 不依赖下游脚本；缺少 Strict 前置资源返回 exit 3；AllowBlocked 缺少资源返回 exit 0，但强制 `outcome=blocked` 与 `strict_gate_eligible=false`。
- 创建 `test-run-phase1-cli.ps1`：覆盖正例、非法参数、AllowBlocked 升级负例、Strict blocked 语义、退出码声明和 28 份 Phase 1 PLAN 的 23 个 automated runner invocation。
- 保留现有 `.planning/config.json` 修改和 `.planning/research/.cache/` 未跟踪文件，未安装依赖或修改无关文件。

## Task Commits

1. **Task 1: 固定 Scope/Target/Mode 与退出码合同** — `8629fe5` (`feat(01-03): 固定 Phase 1 runner CLI 合同`)
2. **Task 2: 锁定 parser 与所有计划消费者合同** — `9e23a52` (`test(01-03): 锁定 runner parser 与计划消费者`)

## Verification

- `powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope RunnerSelfTest -Target Local -Mode Strict` — PASS，输出 `outcome=passed`、`strict_gate_eligible=true`、exit 0。
- `powershell -NoProfile -File scripts/contracts/test-run-phase1-cli.ps1 -ScanPlans .planning/phases/01-trusted-local-state-contract` — PASS，验证 16 个 matrix combinations、28 份 PLAN、23 个 runner invocation。
- `python -c "import json; json.load(open('tests/fixtures/contracts/runner-cli-matrix.json', encoding='utf-8'))"` — PASS。
- `git diff --check` — PASS。

## Requirements Coverage

- `STATE-01`、`STATE-02`、`STATE-03`、`STATE-04`、`STATE-05`：本计划建立其 Phase 1 runner/consumer 机器合同，实际状态实现仍由后续计划交付。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] 修正 Windows PowerShell 5.1 的脚本与 JSON 编码兼容性**

- **Found during:** Task 1 验证。
- **Issue:** Windows PowerShell 5.1 在无 BOM 的 UTF-8 脚本/JSON 中遇到中文文本时无法稳定解析 runner 或 matrix。
- **Fix:** runner 错误消息和 matrix 的机器描述改为 ASCII；JSON 仍保持 UTF-8/Unix 换行，结构与合同内容不变。
- **Files modified:** `scripts/contracts/run-phase1-contracts.ps1`、`tests/fixtures/contracts/runner-cli-matrix.json`。
- **Commit:** `8629fe5`。

**2. [Rule 1 - Bug] 修正 PowerShell 集合绑定与 Generic List 返回**

- **Found during:** Task 1/Task 2 自测。
- **Issue:** PowerShell 5.1 对 JSON 数组绑定到标量参数、数组比较和 `List[object]` 直接展开存在不稳定行为，导致合法 matrix 被误判或 consumer scan 抛出 `Argument types do not match`。
- **Fix:** 使用显式数组参数、顺序比较 helper 和 `ToArray()` 返回；补充完整 parser/consumer 自测覆盖。
- **Files modified:** `scripts/contracts/run-phase1-contracts.ps1`、`scripts/contracts/test-run-phase1-cli.ps1`。
- **Commits:** `8629fe5`、`9e23a52`。

**Total deviations:** 2 auto-fixed issues

## Issues Encountered

- 未来 provenance、platform manifest 和 packaging verifier 尚未创建；matrix 已将这些 dispatch 的缺失资源正确建模为 Strict exit 3 blocked，未把阻断伪装成通过。
- 01-02 的人工 npm 官方身份批准仍是独立 gate，本计划没有替代或批准该 gate。

## Deferred Issues

- `Freeze`、`PhaseComplete` 以及各具体目标的 Strict dispatch 需要后续计划创建的 evidence、manifest、source audit 和 packaging artifacts；这些是计划依赖，不是本计划遗漏。

## User Setup Required

None.

## Next Phase Readiness

- 01-04 可以直接消费 `ProvenanceSelfTest + Local + Strict` 唯一调用形状。
- 01-07 可以复用 `test-run-phase1-cli.ps1` 的 automated consumer 解析边界，并把 matrix 纳入只读 source audit。
- 01-02 的人工 package 来源批准和后续真实平台 evidence gate 仍按路线图顺序执行。

## Self-Check: PASSED

- 3 个计划产物均存在。
- `8629fe5`、`9e23a52` 均存在于当前 `main` 历史。
- 两个任务 verify 均以退出码 0 通过。
- 工作树中仅保留用户已有的 `.planning/config.json` 修改与 research cache 未跟踪文件。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 03*
*Completed: 2026-08-06*
