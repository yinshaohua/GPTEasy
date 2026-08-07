---
phase: 01-trusted-local-state-contract
plan: 04
subsystem: contracts
tags: [powershell, json-schema, provenance, attestations, github-actions, security]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-03 的 canonical runner、稳定退出码与 ProvenanceSelfTest 调用形状"
provides:
  - "外部 contract manifest 与 provenance 的严格 allowlist schema"
  - "敏感字段、非原生证据、生命周期自报和 strict eligibility 自报的 fail-closed validator"
  - "按 run/attempt/job/artifact 重新查询、下载、验签和 digest 交叉验证的 provenance verifier"
  - "无真实网络的 transcript 正反自测与 AllowBlocked 防升级覆盖"
affects: [01-05 gh preflight, 01-06 approval, 01-11-to-01-17 external evidence, 01-26-to-01-28 final gates]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "manifest validator 只证明结构与脱敏边界，永远不自行授予 strict eligibility"
    - "API artifact archive digest 与下载 subject digest/attestation subject digest 分开验证"
    - "Transcript 注入仅用于生产 predicate 自测，test_only=true 且 strict_gate_eligible=false"
    - "真实路径先运行 gh preflight，再在全新临时目录查询、下载和执行 gh attestation verify"

# Key files
key-files:
  created:
    - tests/fixtures/contracts/schema/contract-manifest.schema.json
    - tests/fixtures/contracts/schema/provenance.schema.json
    - tests/fixtures/contracts/provenance-negative-cases.json
    - scripts/contracts/validate-contract-evidence.ps1
    - scripts/contracts/verify-evidence-provenance.ps1
    - scripts/contracts/test-evidence-provenance.ps1
  modified:
    - tests/fixtures/contracts/runner-cli-matrix.json

decisions:
  - "manifest 中的 hash、runner 标签、verified 或 strict_gate_eligible 自报均不能关闭门禁；只有真实重新取回与 attestation 验证可授予严格通过。"
  - "provenance 同时保留 GitHub artifact archive digest 与 attestation subject digest，前者匹配 API metadata，后者匹配重新下载文件。"
  - "认证、网络、preflight 缺失、过期或不可取回返回 blocked；身份、commit、workflow、job、digest 或 attestation 错配返回 provenance invalid。"

metrics:
  duration: "约 22 分钟"
  completed: 2026-08-06
  tasks: 2
  files: 7
status: complete
---

# Phase 1 Plan 4: 外部 evidence provenance 核心 Summary

**用严格 schema、脱敏 validator 和独立重新取回/验签 verifier，阻止自报、陈旧、错配或敏感 evidence 关闭 freeze/final gate**

## Performance

- **Duration:** 约 22 分钟
- **Completed:** 2026-08-06
- **Tasks:** 2/2
- **Files modified:** 7

## Accomplishments

- 创建两份 JSON Schema，固定 immutable workflow ref、run attempt、数值 run/job/artifact ID、40-hex commit、artifact archive digest 与 attestation subject digest。
- 实现 manifest allowlist validator：只允许摘要 digest、布尔 assertions、计数、redactions、origin 类型、runner/account lifecycle 和 provenance；敏感字段、canary、synthetic/partial/blocked、marker-only disposable、自报 `verified`/`strict_gate_eligible` 全部拒绝。
- 明确 validator 的安全边界：schema 合法只输出 `outcome=validated`，但始终 `strict_gate_eligible=false`，避免本地 manifest 自证。
- 实现真实 provenance 路径：先要求 01-05/01-06 的 gh preflight，通过 run/attempt/job/artifact API 重新查询，在新临时目录按 run 与 artifact name 下载，并对 evidence bundle 与 subject artifact 分别执行 `gh attestation verify`。
- 对查询到的 workflow、run attempt、job、commit、artifact ID/name/archive digest、下载文件 SHA-256、attestation subject/predicate 逐项交叉验证；没有本地同名文件回退。
- 建立 17 个负例，覆盖缺字段、错格式、non-native、自报、敏感字段、陈旧 run、错 attempt/job/workflow/commit/digest、缺 attestation、过期和不可取回。

## Task Commits

1. **Task 1: 固定 manifest/provenance schema 与脱敏 validator** — `dbeb237` (`feat(01-04): 固定外部证据结构与脱敏边界`)
2. **Task 2: 实现重新取回与签名 provenance 验证** — `e1f8332` (`feat(01-04): 实现独立取回与签名来源验证`)

## Verification

- `powershell -NoProfile -File scripts/contracts/validate-contract-evidence.ps1 -SelfTest` — PASS，验证两份 schema、正控制和 8 个结构/敏感边界负例。
- `powershell -NoProfile -File scripts/contracts/test-evidence-provenance.ps1` — PASS，验证独立正控制和全部 17 个 fail-closed case。
- `powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ProvenanceSelfTest -Target Local -Mode Strict` — PASS，canonical runner 返回 exit 0、`outcome=passed`。
- `powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ProvenanceSelfTest -Target Local -Mode AllowBlocked` — PASS（预期拒绝），返回 exit 64、`outcome=usage_error`、`strict_gate_eligible=false`。
- `powershell -NoProfile -File scripts/contracts/test-run-phase1-cli.ps1 -ScanPlans .planning/phases/01-trusted-local-state-contract` — PASS，16 个 matrix combinations、28 份 PLAN、23 个 runner invocations。
- PowerShell AST parse — PASS，validator/verifier/self-test 三个脚本均无语法错误。
- Python JSON parse — PASS，两个 schema、negative fixture 与 runner matrix 均可解析。
- `git diff --check` — PASS。

## Requirements Coverage

- **STATE-02:** evidence allowlist、canary 与 transcript 输出边界防止本地敏感状态进入外部证据。
- **STATE-03 / STATE-04 / STATE-05:** 为后续升级、备份、恢复和 higher-schema 平台证明建立不可自报的外部来源验证门禁；实际 SQLite 行为仍由后续 Phase 1 计划交付。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical Functionality] canonical ProvenanceSelfTest 原先未执行 provenance verifier 自测**

- **Found during:** Task 2 的 TDD 红灯与 runner 检查。
- **Issue:** 01-03 的 `provenance-self-test` dispatch 只调用 validator；即使 verifier/test 文件缺失，计划指定的 canonical verify 仍会错误通过。
- **Fix:** 将 `scripts/contracts/test-evidence-provenance.ps1` 加入同一 matrix dispatch，并运行 runner consumer regression。
- **Files modified:** `tests/fixtures/contracts/runner-cli-matrix.json`。
- **Commit:** `e1f8332`。

**2. [Rule 1 - Bug] 区分 GitHub artifact archive digest 与 attested subject file digest**

- **Found during:** Task 2 真实下载路径设计。
- **Issue:** 初始正控制把 API artifact digest 与 attestation subject digest 设为同值，会掩盖 GitHub artifact archive 和下载后被验签文件是两个 digest 边界。
- **Fix:** fixture 使用独立 digest；verifier 分别匹配 API metadata、下载文件 SHA-256 与 attestation subject。
- **Files modified:** `tests/fixtures/contracts/provenance-negative-cases.json`。
- **Commit:** `e1f8332`。

**3. [Rule 1 - Bug] 修正 PowerShell 自测对多行 command source 的扫描**

- **Found during:** Task 2 self-test。
- **Issue:** 初始 `run.+download` 正则不跨行，错误拒绝实际存在的安全下载调用。
- **Fix:** 改为明确匹配相邻 `"run"` / `"download"` 参数的 dotall 正则。
- **Files modified:** `scripts/contracts/test-evidence-provenance.ps1`。
- **Commit:** `e1f8332`。

**Total deviations:** 3 auto-fixed issues

## Issues Encountered

- 真实 GitHub 网络路径按计划依赖尚未执行的 01-05 preflight 和 01-06 blocking-human approval；当前计划只运行 transcript 自测，且 transcript 结果强制 `test_only=true`、`strict_gate_eligible=false`。
- 未触发认证 gate，也未访问真实 GitHub artifact；这符合 01-04 不得越过 01-06 的依赖边界。

## Deferred Issues

- 真实 `gh api`、artifact 下载与 `gh attestation verify` 将在 01-06 获得人工批准后，由 01-13 及后续外部 evidence 计划执行。

## User Setup Required

None.

## Next Phase Readiness

- 01-05 可直接实现并测试 `preflight-gh-evidence.ps1`；真实 verifier 已固定调用路径和最低版本参数。
- 01-06 批准前，真实 provenance verification 会 fail-closed 为 blocked，不会回退 transcript 或本地同名文件。
- 后续 Windows/macOS/WSL2/package manifests 可直接复用 schema 与 provenance verifier。

## Self-Check: PASSED

- 6 个计划产物和本 SUMMARY 均存在。
- `dbeb237`、`e1f8332` 均存在于当前 `main` 历史。
- 两个任务 verify、计划级 AllowBlocked 防升级验证和 runner consumer regression 均通过。
- 未留下 stub、TODO、FIXME 或 skipped test。
- 工作树中仅保留用户已有的 `.planning/config.json` 修改与 research cache 未跟踪文件，以及尚待元数据提交的规划文档。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 04*
*Completed: 2026-08-06*
