---
phase: 01-trusted-local-state-contract
plan: 13
subsystem: windows-packaging-evidence
tags: [powershell, windows, authenticode, nsis, github-actions, provenance, lifecycle]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-04 的 evidence/provenance fail-closed 边界、01-06 的 gh preflight 批准、01-10 的固定路径重开合同与 01-11 的 Windows/WSL 探针"
provides:
  - "Authenticode、PE 架构、current-user、路径重开与账户生命周期组合 predicate"
  - "绑定 run/attempt/job、runner tracking、SID/profile 的一次性 Windows 账户创建与销毁证明"
  - "Windows x64/ARM64 exact-checkout、签名构建、清理、上传与 provenance attestation workflow"
  - "会实际执行 Windows/macOS 本地 package self-test 的 canonical PackagingSelfTest dispatch"
affects: [01-15 windows evidence, 01-17 provenance verification, 01-26 freeze, 01-28 phase completion]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "fixture 与 live facts 复用同一 package predicate；fixture 成功永远 strict_gate_eligible=false"
    - "disposable 证明必须同时绑定 GitHub job/runner identity 与 SID/profile 销毁结果，marker 只能作为相关性字段"
    - "敏感账户名、原始 SID、profile 绝对路径和 DPAPI 密码状态只留在 runner 私有目录，不进入 evidence bundle"
    - "strict-pass manifest 仅在 finalized lifecycle 与 live package predicate 均通过后创建"

key-files:
  created:
    - scripts/contracts/assert-windows-job-lifecycle.ps1
    - scripts/contracts/verify-windows-package.ps1
    - scripts/contracts/test-windows-package-verifier.ps1
    - tests/fixtures/contracts/packaging/windows-positive-control.json
    - .github/workflows/phase1-windows-evidence.yml
  modified:
    - tests/fixtures/contracts/runner-cli-matrix.json
    - tests/fixtures/contracts/phase1-plan-audit-lock.json

key-decisions:
  - "Windows package strict pass 同时要求 Authenticode Valid、目标 PE machine、currentUser/LOCALAPPDATA、跨进程 reopen、profile digest 绑定和 finalized cleanup attestation。"
  - "GitHub-hosted runner 以平台 ephemeral identity 通过；self-hosted fallback 只有在显式 ephemeral 且完整 baseline hash 恢复时才允许通过。"
  - "workflow 的签名材料只通过 WINDOWS_AUTHENTICODE_PFX_BASE64/WINDOWS_AUTHENTICODE_PFX_PASSWORD secrets 注入，PFX、密码和证书在 build step finally 清理。"
  - "PackagingSelfTest Local 从仅检查文件存在改为运行 Windows 正负例和既有 macOS Wave 0 静态合同。"

requirements-completed: [STATE-01, STATE-02, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "同一 Windows package predicate 接受 test-only 正控制并拒绝 unsigned、wrong-arch、per-machine、marker-only 与 cleanup-missing"
    requirements: [STATE-01, STATE-02]
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet All"
        status: pass
    human_judgment: false
  - id: D2
    description: "生命周期 guard 绑定 GitHub runner/job/account identity，并在 app stop 后证明账户/profile 不存在或完整 baseline restore"
    requirements: [STATE-02, STATE-03, STATE-04, STATE-05]
    verification:
      - kind: static-contract
        ref: "powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet Workflow"
        status: pass
    human_judgment: false
  - id: D3
    description: "Windows x64/ARM64 workflow 只在清理与 live package predicate 通过后上传 signed installer/evidence 并生成 provenance attestation"
    requirements: [STATE-01, STATE-02, STATE-03, STATE-04, STATE-05]
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target Local -Mode Strict"
        status: pass
      - kind: static-contract
        ref: "workflow YAML parse、immutable action pins、cleanup → verify → upload → attestation 顺序"
        status: pass
    human_judgment: false

# Metrics
duration: 约 31 分钟
completed: 2026-08-06
tasks: 2
files: 7
status: complete
---

# Phase 1 Plan 13: Windows Package 与 Attested Lifecycle Summary

**以同一生产 predicate 组合 Authenticode、PE 架构、当前用户路径、跨进程重开和一次性账户销毁，并建立 x64/ARM64 签名 evidence workflow**

## Performance

- **Duration:** 约 31 分钟
- **Started:** 2026-08-06T07:15:09Z
- **Completed:** 2026-08-06T07:45:45Z
- **Tasks:** 2/2
- **Files modified:** 7

## Accomplishments

- 新增 Windows package verifier，live 路径读取实际 Authenticode、PE machine 和 installer SHA-256；fixture 注入复用相同 assertions，但任何 fixture 成功都保持 `test_only=true`、`strict_gate_eligible=false`。
- 正控制与负例覆盖 unsigned、wrong-arch、per-machine/system install、marker-only 和 cleanup 缺失，防止“所有 artifact 一律拒绝”的伪实现通过 success aggregation。
- 新增生命周期 guard：绑定固定仓库、run/attempt/job/commit、runner name/image/tracking digest、架构、SID/profile digest；创建一次性本地用户并使用 DPAPI 保护私有状态。
- `Finalize` 先停止 SID 所属进程，再删除账户与 `Win32_UserProfile` 并复核不存在；删除失败时只允许 self-hosted ephemeral runner 的完整 baseline hash restore。
- evidence 只保存摘要与布尔事实，不保存账户名、原始 SID、profile 绝对路径、密码、PFX 或 probe stderr。
- 新增 Windows x64/ARM64 workflow：exact commit checkout、固定 GitHub-hosted native runner、只读 gh preflight、locked npm/Cargo 构建、正式 Authenticode secrets、一次性用户下的 host parity/WSL/path smoke、统一 always cleanup、live package verify、artifact upload 与 provenance attestation。
- `strict-pass.json` 只在 finalized lifecycle 和 live package predicate 都通过后生成；cleanup 失败、签名缺失、架构错、路径错或身份错配都会阻止上传 strict pass evidence。

## Task Commits

1. **Task 1 RED: Windows package 正控制与拒绝分支** — `8754774` (`test(01-13): 添加 Windows 包验证失败测试`)
2. **Task 1 GREEN: 生命周期与 package 生产 predicate** — `51a9917` (`feat(01-13): 实现 Windows 包与账户生命周期门禁`)
3. **Task 2: Windows 双架构 attested evidence workflow** — `98861f9` (`feat(01-13): 建立 Windows 双架构证据工作流`)

## Verification

- `powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet All` — PASS。
- `powershell -NoProfile -File scripts/contracts/test-windows-package-verifier.ps1 -CaseSet Workflow` — PASS。
- `powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target Local -Mode Strict` — PASS，canonical dispatch 实际运行 Windows 与 macOS package contracts。
- `powershell -NoProfile -File scripts/contracts/test-run-phase1-cli.ps1 -ScanPlans .planning/phases/01-trusted-local-state-contract` — PASS，16 个 matrix combinations、28 个 plans、23 个 runner invocations。
- `powershell -NoProfile -File scripts/contracts/audit-phase1-plan-source.ps1 -PhaseDir .planning/phases/01-trusted-local-state-contract -ReadOnly` — PASS。
- 三个 PowerShell 脚本 AST parse — PASS。
- workflow YAML、正控制、runner matrix 与 source lock JSON parse — PASS。
- `git diff --check` — PASS。

## Requirements Coverage

- **STATE-01:** package/path smoke 绑定固定应用状态根的两次独立进程重开，并要求 current-user installer。
- **STATE-02:** 私有账户状态与签名材料不进入 evidence；上传内容只含允许清单摘要、布尔 assertions 和签名 artifact。
- **STATE-03:** runner/job/profile 生命周期和 WSL 被动探针进入同一 attested workflow，为迁移与发现合同提供 Windows 原生证据入口。
- **STATE-04:** package predicate 与 lifecycle cleanup 构成后续备份/恢复外部证据的不可自报前置条件。
- **STATE-05:** higher-schema/future state evidence 只能在当前用户路径、签名、架构和 finalized lifecycle 全部通过后获得 strict pass。

## TDD Gate Compliance

- RED commit `8754774` 先加入正控制和五类拒绝分支，执行时因生产脚本缺失按预期失败。
- GREEN commit `51a9917` 随后实现 lifecycle/package predicate，并使 `All` 与 `Workflow` 两个 case set 通过。
- Task 2 在 GREEN 基线之上完成 workflow expansion，没有跳过 RED → GREEN 顺序。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical Functionality] canonical PackagingSelfTest 原先只检查路径存在**

- **Found during:** Task 2 的 canonical verify。
- **Issue:** `packaging-self-test-local` 使用 aggregate dispatch，只要测试脚本存在就返回 strict pass，不会运行正控制、负例或 workflow contract。
- **Fix:** 改为 command dispatch，执行 Windows `-CaseSet All` 与既有 macOS Wave 0 静态合同；使用显式 `-UpdateLock` 刷新 source audit digest lock。
- **Files modified:** `tests/fixtures/contracts/runner-cli-matrix.json`、`tests/fixtures/contracts/phase1-plan-audit-lock.json`
- **Commit:** `98861f9`

**2. [Rule 1 - Bug] 初始化中途失败可能留下无状态文件的一次性账户**

- **Found during:** Task 2 workflow lifecycle review。
- **Issue:** 初始实现先完成 profile warmup 再写 state；若 warmup/发现 profile 失败，always cleanup 无法读取账户 SID 与 DPAPI credential state。
- **Fix:** 创建账户和 SID 后立即持久化私有 partial state，再 warm profile 并更新；Finalize 异常也写 `cleanup_attested=false` 的脱敏 lifecycle evidence。
- **Files modified:** `scripts/contracts/assert-windows-job-lifecycle.ps1`
- **Commit:** `98861f9`

**3. [Rule 1 - Bug] Windows PowerShell 兼容性与单产物断言**

- **Found during:** Task 2 workflow 静态集成。
- **Issue:** 初始随机密码使用 Windows PowerShell 5.1 不保证可用的静态 RNG API，workflow 还使用不存在的 `Select-Object -Single`。
- **Fix:** 改用实例 RNG `GetBytes` 并强制复杂度前缀；对 installer 与 installed executable 使用显式数组计数 `Count -eq 1`。
- **Files modified:** `scripts/contracts/assert-windows-job-lifecycle.ps1`、`.github/workflows/phase1-windows-evidence.yml`
- **Commit:** `98861f9`

**Total deviations:** 3 auto-fixed（Rule 1：2；Rule 2：1）

## Issues Encountered

- 未触发或伪造任何远程 GitHub Actions run。计划指定的 automated verification 是本地 predicate/workflow contract；真实 Windows x64/ARM64 签名、账户生命周期、artifact upload 与 attestation 结果必须由后续 evidence 计划引用实际 run。
- workflow 对 official Codex `0.146.1` CLI、正式 `OpenAI.Codex_2p2nqsd0c76g0` host 与 signing secrets 均 fail-closed；缺失时不会生成 strict pass manifest。

## Authentication Gates

None。未执行需要新增认证的远程 workflow 或 artifact 操作；本地仅验证既有只读 preflight 调用合同。

## User Setup Required

- Repository/Environment secrets：`WINDOWS_AUTHENTICODE_PFX_BASE64`、`WINDOWS_AUTHENTICODE_PFX_PASSWORD`。
- 原生 runner 必须能解析精确 `codex.exe 0.146.1`，并为 disposable 用户提供固定 package family `OpenAI.Codex_2p2nqsd0c76g0` 的正式宿主；缺失会按合同阻断。

## Known Stubs

None。fixture 明确是 test-only 正控制，不是 production data stub；workflow 不含占位实现、跳过测试或空 strict-pass 路径。

## Next Phase Readiness

- 01-15 可触发 Windows evidence workflow，在具备 official host 与签名 secrets 的原生 x64/ARM64 环境中取得真实 run/job/artifact IDs、installer 与 evidence bundle。
- 01-17 可复用 01-04 provenance verifier 对实际上传 artifact、subject digest 与 attestation 做独立重新取回和交叉验证。
- PhaseComplete 的 read-only source audit 已重新通过，runner matrix 的行为与 digest lock 一致。

## Self-Check: PASSED

- 5 个计划产物、2 个必要 contract 元数据文件和本 SUMMARY 均存在。
- TDD/任务提交 `8754774`、`51a9917`、`98861f9` 均存在于当前 `main` 历史。
- SUMMARY frontmatter 包含 `status: complete`，stub scan 与 `git diff --check` 均通过。
- 工作树只保留用户既有 `.planning/config.json` 修改、research cache 未跟踪文件及待提交的计划元数据。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 13*
*Completed: 2026-08-06*
