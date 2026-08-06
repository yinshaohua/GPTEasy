---
phase: 01-trusted-local-state-contract
plan: 15
subsystem: contracts
tags: [powershell, runner-cli, freeze, provenance, source-audit]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-07 的只读来源审计、01-10 的真实 path smoke，以及 01-11～01-14 的平台非签名合同自测"
provides:
  - "不读取签名凭据即可严格通过的 non_signing_contract Freeze"
  - "与 Freeze 隔离、缺任一四目标正式证据即返回 3 的 PhaseComplete"
  - "修订后 28 份计划与 runner matrix 的当前 digest lock"
affects: [01-16 schema-freeze, 01-26 windows-evidence, 01-27 macos-evidence, 01-28 phase-complete]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Freeze 只聚合本地非签名合同，并以 release_ready=false 明示发布身份仍未满足"
    - "正式 evidence-set 先校验 manifest，再执行实时 provenance verifier，test-only override 永不获得 strict eligibility"

key-files:
  created: []
  modified:
    - tests/fixtures/contracts/runner-cli-matrix.json
    - scripts/contracts/run-phase1-contracts.ps1
    - scripts/contracts/test-run-phase1-cli.ps1
    - scripts/contracts/test-phase1-plan-source-audit.ps1
    - tests/fixtures/contracts/phase1-plan-audit-lock.json

key-decisions:
  - "Freeze 固定执行 runner/provenance/path/Windows/WSL/macOS/package 六类非签名检查；Windows Authenticode 与 macOS Developer ID/notary 只记录为 deferred。"
  - "PhaseComplete 独立要求八个四目标 evidence-set，并要求实时 provenance 返回 strict_gate_eligible=true、test_only=false；Freeze 结果不能晋升为正式证据。"
  - "-Matrix 仅用于负例自测，任何 override 结果都强制 test_only=true、strict_gate_eligible=false、release_ready=false。"

patterns-established:
  - "Composite dispatch：逐项记录 checks，任一子检查 blocked/failed 即保留原退出码并停止。"
  - "PhaseComplete 先证明来源审计通过，再以 exit 3 诚实报告尚未取得的正式签名/公证 evidence。"

requirements-completed: [STATE-01, STATE-02, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "Freeze 在没有 PFX/Apple 凭据或正式 manifests 时执行六类非签名合同并严格通过，输出精确 deferred 列表与 release_ready=false。"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope Freeze -Target Local -Mode Strict"
        status: pass
      - kind: integration
        ref: "scripts/contracts/test-run-phase1-cli.ps1#Test-FreezeAndPhaseComplete"
        status: pass
    human_judgment: false
  - id: D2
    description: "PhaseComplete 不接受 Freeze、fixture 或 test-only override，正式四目标 evidence 缺失时稳定 fail-closed。"
    verification:
      - kind: integration
        ref: "scripts/contracts/test-run-phase1-cli.ps1#Test-FreezeAndPhaseComplete"
        status: pass
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PhaseComplete -Target Local -Mode Strict"
        status: pass
    human_judgment: false
  - id: D3
    description: "28 份修订计划、来源映射、拓扑、runner 调用和摘要锁可重算，普通门禁保持只读。"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-phase1-plan-source-audit.ps1"
        status: pass
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/audit-phase1-plan-source.ps1 -PhaseDir .planning/phases/01-trusted-local-state-contract -ReadOnly"
        status: pass
    human_judgment: false

# Metrics
duration: 1h 25m
completed: 2026-08-07
status: complete
---

# Phase 1 Plan 15: 非签名 Freeze 与 PhaseComplete 隔离 Summary

**六类本地非签名合同可以独立冻结，同时八个正式平台 evidence-set 继续由实时 provenance、签名与公证硬门保护**

## Performance

- **Duration:** 1h 25m
- **Started:** 2026-08-06T17:53:12Z
- **Completed:** 2026-08-06T19:18:10Z
- **Tasks:** 2/2
- **Files modified:** 5

## Accomplishments

- `Freeze Local Strict` 真实执行 runner、provenance、Cargo path smoke、Windows、WSL2 与跨平台 packaging 合同，六项全部通过后仍明确 `release_ready=false`。
- `PhaseComplete Local Strict` 与 Freeze 使用独立 evidence 集合；当前来源审计通过后因 Windows x64 正式 evidence 缺失返回 3，没有读取、创建或替代任何 PFX/Apple 凭据。
- matrix override、Freeze 冒充正式 evidence、unsigned fixture、缺少任一非签名检查均有 fail-closed 负例；28 份计划和 20 个 runner invocation 已重新扫描。
- source-audit 负例套件通过后显式刷新 digest lock，随后 `-ReadOnly` 对 28 个计划、5 个 requirements、拓扑、路径、CLI、threat 与摘要全部通过。

## Task Commits

1. **Task 1 RED: 添加非签名冻结失败契约** — `728e20a`
2. **Task 1 GREEN: 收紧非签名 Freeze 与 PhaseComplete** — `e3ce5b2`
3. **Task 2: 刷新计划来源摘要锁** — `ce487d1`

## Files Created/Modified

- `tests/fixtures/contracts/runner-cli-matrix.json` — 声明 composite Freeze、独立 PhaseComplete、process 与严格 evidence-set。
- `scripts/contracts/run-phase1-contracts.ps1` — 执行五类新增 dispatch，输出冻结/发布元数据，并隔离 test-only matrix。
- `scripts/contracts/test-run-phase1-cli.ps1` — 覆盖 Freeze/PhaseComplete 正反例、fixture 拒绝与 28 份计划消费者扫描。
- `scripts/contracts/test-phase1-plan-source-audit.ps1` — 将旧 PhaseComplete 正控制更新为“来源审计通过、正式 evidence 缺失返回 3”。
- `tests/fixtures/contracts/phase1-plan-audit-lock.json` — 固定本次审阅后的计划与 matrix 摘要。

## Decisions Made

- 非签名 Freeze 只批准继续本地 SQLite schema/backup 工作，不代表发布就绪。
- 正式 evidence 除 schema 校验外必须实时重验 provenance；结构正确的 fixture 也不能获得 strict eligibility。
- 当前不配置或读取 Windows PFX、Developer ID 和 notarization 值；它们继续由 01-26/01-27 的人工前置门处理。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] PowerShell 将 Cargo linker warning 当作终止错误**

- **Found during:** Task 1 Freeze 正控制
- **Issue:** Rust 测试实际返回 0，但 Windows PowerShell 在 `$ErrorActionPreference=Stop` 下把原生 stderr warning 提升为异常。
- **Fix:** 捕获子进程 stdout/stderr，仅使用受控 `$LASTEXITCODE` 判断 command/process/evidence verifier 成败。
- **Files modified:** `scripts/contracts/run-phase1-contracts.ps1`
- **Verification:** `path_smoke` 2 个测试通过，完整 Freeze 六项 checks 返回 0。
- **Committed in:** `e3ce5b2`

**2. [Rule 1 - Bug] 来源审计自测仍把旧 aggregate PhaseComplete 当作正控制**

- **Found during:** Task 2 source-audit 负例套件
- **Issue:** 新合同要求正式 evidence 缺失返回 3，旧测试仍要求 PhaseComplete 返回 0。
- **Fix:** 结果 checks 显式记录 `source-audit`，测试改为要求审计先通过、随后正式 evidence 缺失 fail-closed。
- **Files modified:** `scripts/contracts/run-phase1-contracts.ps1`, `scripts/contracts/test-phase1-plan-source-audit.ps1`
- **Verification:** source-audit 全部正负例通过，静态 COVERED 仍不能掩盖实时缺口。
- **Committed in:** `e3ce5b2`

---

**Total deviations:** 2 auto-fixed（2 个 Rule 1 bug）
**Impact on plan:** 两项修复都用于让新门禁语义可重复执行；没有放宽证据要求，也没有扩大到签名凭据配置。

## Issues Encountered

- Cargo 首次验证生成约 3.7 GiB 可再生构建缓存和 Tauri schema；验证后已清理，未纳入 Git。

## User Setup Required

None - 本计划不需要外部服务配置，也没有读取任何 secret 值。

## Next Phase Readiness

- 01-16 可据 `freeze_kind=non_signing_contract` 的通过结果冻结 SQLite schema/backup 单向决策。
- Windows Authenticode PFX 和 macOS Developer ID/notarization 仍明确延期到 01-26/01-27；PhaseComplete 当前按设计返回 3。

## Self-Check: PASSED

- Task 1/2 的三个提交均存在于 `main`，五个计划产物存在且 LF/BOM 合同正确。
- Freeze 返回 0；PhaseComplete 返回 3；runner scan、source-audit 负例与只读重算全部通过。
- `.planning/config.json` 与 research cache 未纳入任何提交。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 15*
*Completed: 2026-08-07*
