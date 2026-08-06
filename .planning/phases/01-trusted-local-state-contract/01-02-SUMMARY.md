---
phase: 01-trusted-local-state-contract
plan: 02
subsystem: supply-chain
tags: [npm, package-identity, human-approval, tauri, react]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-01 的 7 项 exact npm allowlist、隔离公开 registry verifier 与污染/泄漏负例"
provides:
  - "7 个 package@version 的人工官方身份、repository 与官方 Tauri React TypeScript 模板来源批准记录"
  - "01-08 首次 npm install 前置 legitimacy 门禁的 approved 结论"
affects: [01-08 frontend scaffold, npm dependency installation]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "首次依赖安装前必须同时具备机器 exact allowlist 通过与人工官方来源 approved"
    - "checkpoint-only 计划只记录门禁结论，不安装依赖或创建前端脚手架"

key-files:
  created:
    - .planning/phases/01-trusted-local-state-contract/01-02-SUMMARY.md
  modified:
    - .planning/STATE.md
    - .planning/ROADMAP.md

key-decisions:
  - "01-02 checkpoint 结论为 approved：人工已核对全部 7 个 package@version 的官方身份、repository 与官方 create-tauri-app React TypeScript 模板来源。"
  - "批准范围严格等于 tests/fixtures/contracts/npm-package-allowlist.json；任一名称、版本或 repository 变化都必须重新阻断并核对。"
  - "本计划未执行 npm install、未安装依赖；approved 只解除 01-08 的首次安装前置条件。"

patterns-established:
  - "供应链 legitimacy 采用机器 metadata predicate 与不可自动替代的人工官方来源确认双重门禁。"

requirements-completed: [STATE-01, STATE-02]

coverage:
  - id: D1
    description: "7 个 exact package@version 的官方身份、repository 与 Tauri React TypeScript 模板来源批准"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-npm-package-allowlist.ps1"
        status: pass
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/verify-npm-package-allowlist.ps1 -Allowlist tests/fixtures/contracts/npm-package-allowlist.json"
        status: pass
      - kind: manual_procedural
        ref: "用户回复 approved：核对 npm 官方页面、repository 与官方 create-tauri-app React TypeScript 模板来源"
        status: pass
    human_judgment: true
    rationale: "发布者与官方模板来源确认是计划明确要求的 blocking-human 门禁，机器 metadata 结果不能替代人工判断。"
  - id: D2
    description: "批准记录不触发首次 npm install 或任何依赖安装"
    requirement: STATE-01
    verification:
      - kind: manual_procedural
        ref: "执行记录与 git diff 审查：未运行 npm install，未创建 package.json、lockfile 或 node_modules"
        status: pass
    human_judgment: false

# Metrics
duration: 约 22 分钟
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 2: npm 官方来源人工批准 Summary

**在首次依赖安装前，以 7/7 机器身份校验和人工官方来源核对共同记录 approved，并保持仓库未安装依赖**

## Performance

- **Duration:** 约 22 分钟（含 checkpoint 恢复与只读复核）
- **Started:** 2026-08-06T03:13:43Z
- **Completed:** 2026-08-06T03:35:28Z
- **Tasks:** 1/1
- **Files modified:** 3 个计划元数据文件（生产文件 0）

## Accomplishments

- 记录 blocking-human checkpoint 的明确结论：`approved`。
- 确认批准严格覆盖 allowlist 中全部 7 个 `package@version`，包括官方 repository 与官方 `create-tauri-app` React TypeScript 模板来源。
- 复跑生产 verifier 自测与真实公开 registry 校验，分别返回 0，真实校验报告 `7 package identities`。
- 未执行首次 `npm install`，未安装依赖，未创建或修改前端脚手架、lockfile 或 `node_modules`。

## Approved Package List

| Package | Version | Approved repository |
|---------|---------|---------------------|
| `@types/react` | `19.1.8` | `https://github.com/DefinitelyTyped/DefinitelyTyped` |
| `@types/react-dom` | `19.1.6` | `https://github.com/DefinitelyTyped/DefinitelyTyped` |
| `@vitejs/plugin-react` | `6.0.2` | `https://github.com/vitejs/vite-plugin-react` |
| `react` | `19.1.0` | `https://github.com/facebook/react` |
| `react-dom` | `19.1.0` | `https://github.com/facebook/react` |
| `typescript` | `6.0.3` | `https://github.com/microsoft/TypeScript` |
| `vite` | `8.0.16` | `https://github.com/vitejs/vite` |

**Template source:** 官方 `create-tauri-app` React TypeScript 模板来源已人工核对并纳入本次 `approved`。

## Task Commits

1. **Task 1: 批准官方 npm package 身份** - checkpoint 结论为 `approved`；本计划只记录人工门禁，没有生产任务 commit。

**Plan metadata:** `docs(01-02): 完成 npm 官方来源批准计划`

## Files Created/Modified

- `.planning/phases/01-trusted-local-state-contract/01-02-SUMMARY.md` - 记录 `approved`、精确包清单、机器复核与未安装依赖事实。
- `.planning/STATE.md` - 推进已完成计划计数、记录批准决策、执行指标和 session continuity。
- `.planning/ROADMAP.md` - 将 01-02 标记完成并把 Phase 1 进度更新为 6/28。
- `.planning/REQUIREMENTS.md` - 按 shared-ID gate 重新评估 STATE-01/STATE-02；既有 Complete 状态无需文本变更。

## Decisions Made

- 只接受精确字符串 `approved` 解除门禁；本次用户已明确给出该结论。
- 批准对象与 `tests/fixtures/contracts/npm-package-allowlist.json` 完全一致，不允许用相近包名、不同版本或不同 repository 替代。
- 后续若 7 项中任一 package 名称、版本或 repository 变化，01-08 必须重新进入人工阻断，而不能沿用本次批准。
- 本计划的完成不等于已经安装依赖；首次安装只允许在后续 01-08 按其计划执行。

## Deviations from Plan

None - plan executed exactly as written after the user supplied the required `approved` resume signal.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- 01-08 的 npm install 前置 legitimacy 门禁已解除，可以按 01-08 计划首次安装精确批准的依赖。
- 01-06 的 gh 环境人工批准仍是独立 checkpoint，不受本计划结论替代。
- 任一批准项变化都必须重新阻断；不得把本次 `approved` 扩展到 allowlist 之外。

## Self-Check: PASSED

- `01-02-SUMMARY.md` 已记录精确 `approved` 结论和全部 7 个 package/version/repository。
- allowlist self-test 返回 0。
- 真实公开 registry verifier 返回 0，并报告 7 项身份匹配。
- 未执行 `npm install`，未安装依赖。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 02*
*Completed: 2026-08-06*
