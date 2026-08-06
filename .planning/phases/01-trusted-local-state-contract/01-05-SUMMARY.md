---
phase: 01-trusted-local-state-contract
plan: 05
subsystem: contracts
tags: [powershell, github-cli, github-api, attestations, preflight, security]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-03 的 canonical runner/退出码合同，以及 01-04 在真实 evidence 取回前调用 gh preflight 的固定接线"
provides:
  - "固定 gh >= 2.49.0、attestation verify 命令、github.com 认证与 yinshaohua/GPTEasy 仓库读取的只读前置门禁"
  - "Actions runs/artifacts 和 repository attestations digest endpoint 的类型化权限判定"
  - "11 个 fail-closed transcript 负例、404 授权缺对象正例和敏感 stderr 零泄漏自测"
affects: [01-06 approval, 01-11-to-01-17 external evidence, 01-26-to-01-28 final gates]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "所有 gh 命令输出先丢弃或裁剪为 jq 白名单字段，再生成稳定 JSON 摘要"
    - "fixture transcript 复用生产命令形状和 parser，但 test_only=true 且 strict_gate_eligible=false"
    - "attestation digest probe 的 404 只证明读取路径可达，不声明任何 artifact 已验证"

key-files:
  created:
    - tests/fixtures/contracts/gh-preflight-cases.json
    - scripts/contracts/preflight-gh-evidence.ps1
    - scripts/contracts/test-gh-preflight.ps1
  modified: []

key-decisions:
  - "gh evidence preflight 固定 github.com、yinshaohua/GPTEasy 和最低版本 2.49.0；调用方不能放宽这些策略参数。"
  - "repository、Actions runs、Actions artifacts 和 attestations API 均使用显式 GET；任何非预期失败都阻断，attestation 仅对固定不存在 digest 的 404 例外放行。"
  - "preflight 永远输出 artifact_verified=false；只有后续 verify-evidence-provenance.ps1 对真实下载工件执行 attestation 验证。"

patterns-established:
  - "稳定错误码：权限或工具失败只暴露 GH_* code，不回显 gh stdout/stderr、认证头或配置正文。"
  - "离线 contract 自测：-GhFixture 与 -FixtureCase 注入 transcript，并逐参数核对真实 gh 命令形状。"

requirements-completed: [STATE-02, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "gh 版本、attestation 命令、认证、仓库、Actions 和 attestation endpoint 的只读前置门禁"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/preflight-gh-evidence.ps1 -SelfTest"
        status: pass
    human_judgment: false
  - id: D2
    description: "401/403、旧版本、未认证和仓库错配均 fail-closed，404 probe 不冒充 artifact 验证"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-gh-preflight.ps1"
        status: pass
    human_judgment: false
  - id: D3
    description: "既有 provenance verifier 在新增 preflight 文件后保持 transcript 回归全绿"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-evidence-provenance.ps1"
        status: pass
    human_judgment: false

# Metrics
duration: 14min
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 5: gh Evidence Preflight Summary

**固定 gh 2.49.0、GitHub 认证和最小只读权限门禁，并用离线 transcript 证明 401/403 与敏感输出全部 fail-closed**

## Performance

- **Duration:** 14 分钟
- **Started:** 2026-08-06T02:25:36Z
- **Completed:** 2026-08-06T02:38:58Z
- **Tasks:** 2/2
- **Files modified:** 3

## Accomplishments

- 实现 `preflight-gh-evidence.ps1`，固定 `github.com`、`yinshaohua/GPTEasy` 和 gh `2.49.0` 最低版本，并确认 `gh attestation verify --help` 可用。
- 对 repository metadata、Actions runs、Actions artifacts 和 repository attestations digest endpoint 执行显式 GET；401/403 与其他异常稳定阻断。
- 将固定不存在 digest 的 attestation 404 表示为 `GH_ATTESTATION_NOT_FOUND_AUTHORIZED`，同时保持 `artifact_verified=false`，避免把权限探针冒充真实验签。
- 建立 11 个 transcript 负例，覆盖旧版本、attestation 命令缺失、未认证、repo 403、Actions runs/artifacts 401/403、attestation 401/403 和仓库身份错配。
- 建立恶意 stderr 测试，注入 token canary、Authorization header 和伪 gh 配置片段，证明进程输出只包含稳定 `GH_*` 错误码。

## Task Commits

1. **Task 1: 固定 gh 版本/认证/权限 preflight** — `0a25836` (`feat(01-05): 固定 gh 证据前置门禁`)
2. **Task 2: 证明权限缺失和敏感输出 fail-closed** — `63f0cd3` (`test(01-05): 证明 gh 权限缺失安全阻断`)

## Verification

- `powershell -NoProfile -File scripts/contracts/preflight-gh-evidence.ps1 -SelfTest` — PASS，正控制和 11 个类型化负例通过生产 parser。
- `powershell -NoProfile -File scripts/contracts/test-gh-preflight.ps1` — PASS，404 授权探针、全部权限负例和恶意认证输出零泄漏。
- `powershell -NoProfile -File scripts/contracts/test-evidence-provenance.ps1` — PASS，01-04 provenance 正控制与 17 个 fail-closed case 保持全绿。
- `git diff --check HEAD~2 HEAD` — PASS。
- `git ls-files --eol` — PASS，三个计划文件均为 LF。

## Requirements Coverage

- **STATE-02:** gh 认证状态、token/header/config 内容不会进入摘要或错误；fixture 模式不访问网络，也不能获得 strict eligibility。
- **STATE-03 / STATE-04 / STATE-05:** 为后续迁移、备份和 higher-schema 外部平台证据提供工具/身份/读取权限前置门禁；实际 SQLite 行为仍由对应后续计划交付和验证。

## Decisions Made

- 固定策略参数而非接受可放宽的任意仓库或最低版本，防止调用方绕过可信 evidence 前置条件。
- API 调用使用 `--method GET` 和 `--jq` 白名单，认证状态输出完全丢弃；最终 JSON 仅包含版本、布尔状态、检查名和稳定错误码。
- 404 仅作为“固定不存在对象但 endpoint 可读”的 preflight 结果，真实 artifact 验证仍由 provenance verifier 独占。

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## Authentication Gates

None - 本计划只运行离线 transcript 自测，未访问真实 GitHub 账户或资源。

## User Setup Required

None.

## Next Phase Readiness

- 01-04 的真实 provenance 路径现在可在下载或验签前调用完整 gh preflight。
- 01-06 可在 blocking-human 批准后运行真实 GitHub 权限与 evidence 流程；未认证、权限不足或工具过旧会稳定返回 blocked。
- 后续外部 runner evidence 计划可复用同一固定仓库、错误码和脱敏输出合同。

## Self-Check: PASSED

- 三个计划产物和本 SUMMARY 均存在。
- `0a25836`、`63f0cd3` 均存在于当前 `main` 历史。
- 两个任务 verify 与相邻 provenance regression 均通过。
- 未留下 stub、TODO、FIXME、skipped test 或未运行的计划 verify。
- 工作树中仅保留用户已有的 `.planning/config.json` 修改、research cache 未跟踪文件，以及待提交的本计划规划元数据。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 05*
*Completed: 2026-08-06*
