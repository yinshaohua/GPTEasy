---
phase: 01-trusted-local-state-contract
plan: 07
subsystem: contracts
tags: [powershell, planning-audit, sha256, topology, runner-cli, security]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-03 的 canonical Scope/Target/Mode runner CLI、退出码合同与 PhaseComplete dispatch 骨架"
provides:
  - "只读解析 ROADMAP、REQUIREMENTS、28 份 PLAN、PATTERNS、VALIDATION、SOURCE-AUDIT 与 runner matrix 的机器审计器"
  - "排除自身、规范化执行状态、显式更新且普通运行只比较的 34 文件 SHA-256 digest lock"
  - "需求缺失、CLI 漂移、波次冲突、路径漂移、缺计划、通配符、digest 篡改和静态 COVERED 的临时副本负例"
  - "PhaseComplete 在聚合前强制执行来源审计并 fail-closed"
affects: [01-28 PhaseComplete, phase-1 planning integrity, contract gates]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "规划门禁仅从实时 parser 与当前 digest 得出结论，SOURCE-AUDIT 的静态状态字段不授予通过"
    - "digest lock 通过显式 -UpdateLock 生成，-ReadOnly 模式不创建、覆盖或修复任何输入；ROADMAP/REQUIREMENTS 仅规范化执行状态字段"
    - "负例只在系统临时目录的完整输入副本中变异，并对生产输入执行前后 SHA-256 快照比较"

key-files:
  created:
    - scripts/contracts/audit-phase1-plan-source.ps1
    - scripts/contracts/test-phase1-plan-source-audit.ps1
    - tests/fixtures/contracts/phase1-plan-audit-lock.json
  modified:
    - scripts/contracts/run-phase1-contracts.ps1

key-decisions:
  - "digest lock 固定 ROADMAP、REQUIREMENTS、PATTERNS、VALIDATION、SOURCE-AUDIT、28 份 PLAN 与 runner matrix，并明确排除 lock 自身；ROADMAP/REQUIREMENTS 的执行进度状态先规范化。"
  - "requirement、来源映射、拓扑、任务标签、路径、CLI、key link 与 high/critical threat disposition 必须实时重算；PLANNED 或 COVERED 文本不参与 pass。"
  - "PhaseComplete 在任何 aggregate 判定前执行 audit-phase1-plan-source.ps1 -ReadOnly；审计非零直接使最终门禁失败。"

patterns-established:
  - "只读审计模式：读取 UTF-8 输入、稳定聚合全部错误、非零退出，且不产生缓存或修复写入。"
  - "副本负例模式：每个 case 使用独立临时仓库布局，校验目标诊断后安全删除，生产 hash 始终保持不变。"

requirements-completed: [STATE-01, STATE-02, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "28 份计划的 requirement、artifact/task path、depends_on/wave、同波冲突、runner CLI、key link、threat disposition 与当前 digest 被只读实时审计"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/audit-phase1-plan-source.ps1 -PhaseDir .planning/phases/01-trusted-local-state-contract -ReadOnly"
        status: pass
    human_judgment: false
  - id: D2
    description: "缺映射、错 CLI、同波冲突、路径/glob/计划漂移、digest 篡改与静态 COVERED 均在临时副本中 fail-closed"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-phase1-plan-source-audit.ps1"
        status: pass
    human_judgment: false
  - id: D3
    description: "PhaseComplete Local Strict 强制执行只读来源审计，不能只检查审计脚本是否存在"
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PhaseComplete -Target Local -Mode Strict"
        status: pass
    human_judgment: false

# Metrics
duration: 36min
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 7: 只读计划与来源审计 Summary

**以实时解析、拓扑/CLI/path 交叉检查和 34 文件 SHA-256 lock 取代静态 COVERED 声明，并将审计 fail-closed 接入 PhaseComplete**

## Performance

- **Duration:** 36 分钟
- **Started:** 2026-08-06T02:45:11Z
- **Completed:** 2026-08-06T03:20:25Z
- **Tasks:** 2/2
- **Files modified:** 4

## Accomplishments

- 实现 `audit-phase1-plan-source.ps1`，只读检查 28 个顺序计划、STATE-01..05、ROADMAP/REQUIREMENTS/SOURCE-AUDIT 映射、task 标签、Artifacts 等式、依赖波次、同波文件冲突和具体路径。
- 解析 `runner-cli-matrix.json` 并校验计划中的 23 个 runner invocation 均显式声明合法 Scope/Target/Mode，拒绝旧参数、未知组合和未声明参数。
- 检查 PATTERNS concrete path 分类、已存在 key link 的真实引用，以及所有 high/critical threat 都使用 `mitigate` disposition。
- 创建包含 34 个输入的 SHA-256 lock；lock 排除自身并规范化 ROADMAP/REQUIREMENTS 的执行状态字段，只有显式 `-UpdateLock` 写入，正常 `-ReadOnly` 运行只重算和比较。
- 建立 9 类独立临时副本负例与 PhaseComplete 集成负例，并证明真实规划、runner matrix 和 lock 在测试前后 hash 不变。
- 将来源审计接入 `PhaseComplete Local Strict`，审计失败时在 aggregate 之前返回非零，避免“脚本存在”等同于“审计已执行”。

## Task Commits

1. **Task 1: 实现只读多来源与计划拓扑审计** — `f1b685f` (`feat(01-07): 实现只读计划来源审计`)
2. **Task 2: 证明静态文本和漂移不能关闭审计** — `2c423c7` (`test(01-07): 覆盖计划来源审计负例`)
3. **Rule 2 补全: 将审计真实接入 PhaseComplete** — `10989f3` (`fix(01-07): 将来源审计接入最终门禁`)
4. **Rule 1 修复: 规范化可变执行状态摘要** — `1a927e2` (`fix(01-07): 规范化可变执行状态摘要`)

## Verification

- `powershell -NoProfile -File scripts/contracts/audit-phase1-plan-source.ps1 -PhaseDir .planning/phases/01-trusted-local-state-contract -ReadOnly` — PASS，28 个计划、5 个 requirements 与 topology/path/CLI/threat/digest 一致。
- `powershell -NoProfile -File scripts/contracts/test-phase1-plan-source-audit.ps1` — PASS，基线、执行状态规范化、10 类负例、PhaseComplete 正负集成和生产 hash 不变检查全部通过。
- `powershell -NoProfile -File scripts/contracts/test-run-phase1-cli.ps1 -ScanPlans .planning/phases/01-trusted-local-state-contract` — PASS，16 个 matrix combinations 与 23 个计划 runner invocation 通过。
- `powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PhaseComplete -Target Local -Mode Strict` — PASS，输出 `outcome=passed`、`strict_gate_eligible=true`、`exit_code=0`。
- `git diff --check` 与 LF/BOM 检查 — PASS，普通文本保持 LF；含中文的 Windows PowerShell 脚本使用 UTF-8 BOM 兼容 Windows PowerShell 5.1。

## Requirements Coverage

- **STATE-01..STATE-05:** 每项必须至少由一个 Phase 1 计划 frontmatter 声明，且 SOURCE-AUDIT 的映射只能引用存在的 01-01..01-28 计划。
- **STATE-03 / STATE-04 / STATE-05:** 迁移、备份与 higher-schema/recovery 计划的依赖拓扑、wave 顺序、关键路径和 high/critical 威胁处置现在可机器重算。
- 本计划证明的是 Phase 1 计划与来源合同的完整性，不替代后续 SQLite 行为、平台 evidence 或最终人工批准。

## Decisions Made

- lock 不包含自身，避免生成后立即自漂移；ROADMAP/REQUIREMENTS 的 checkbox、完成数和状态先规范化，语义正文仍参与 digest；更新必须显式使用 `-UpdateLock`，门禁始终使用 `-ReadOnly`。
- SOURCE-AUDIT 只作为映射输入，`PLANNED`、`PLANNED-COVERAGE` 或伪造 `COVERED` 不影响最终判定。
- key link 不只验证 `pattern` 文本；当 from/to 文件均存在时，from 文件必须真实引用目标路径或文件名。
- PhaseComplete 必须先执行来源审计，再进入原 aggregate dispatch；审计失败不能被 AllowBlocked 或人工文字覆盖。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical] PhaseComplete 只检查审计脚本存在，未真实执行审计**
- **Found during:** 全部任务后的 key link 验证
- **Issue:** `phase-complete-local` 的 aggregate 仅验证 `audit-phase1-plan-source.ps1` 路径存在，静态脚本占位仍可得到 pass，不满足计划的最终 gate 依赖。
- **Fix:** 在 `run-phase1-contracts.ps1` 的 PhaseComplete dispatch 前执行 `audit-phase1-plan-source.ps1 -ReadOnly`；补充 key link 实体引用检查与 PhaseComplete 正负集成测试。
- **Files modified:** `scripts/contracts/run-phase1-contracts.ps1`, `scripts/contracts/audit-phase1-plan-source.ps1`, `scripts/contracts/test-phase1-plan-source-audit.ps1`
- **Verification:** PhaseComplete 正控制返回 0；临时副本删除 STATE-05 映射并伪造 COVERED 后返回非零。
- **Committed in:** `10989f3`

**2. [Rule 1 - Bug] 正常 GSD 进度更新导致 ROADMAP raw digest 永久漂移**
- **Found during:** SUMMARY 后的 STATE/ROADMAP 元数据更新
- **Issue:** ROADMAP 的已执行数量、计划 checkbox 和进度表会在每个计划完成后合法变化；直接 hash 原始字节会让下一次 `-ReadOnly` 审计立即失败。
- **Fix:** 对 ROADMAP 的 Phase 1 执行状态和 REQUIREMENTS 的完成状态做窄范围 canonicalization，再计算 SHA-256；目标、计划描述、波次、路径和其他语义文本仍保持摘要保护。
- **Files modified:** `scripts/contracts/audit-phase1-plan-source.ps1`, `scripts/contracts/test-phase1-plan-source-audit.ps1`, `tests/fixtures/contracts/phase1-plan-audit-lock.json`
- **Verification:** 临时副本只改变执行状态仍通过；修改 ROADMAP Goal 语义正文稳定产生 digest drift。
- **Committed in:** `1a927e2`

---

**Total deviations:** 2 auto-fixed（1 missing critical, 1 bug）
**Impact on plan:** 两项修复都用于保证只读门禁真实可执行且不会被正常执行元数据变化自锁；未放宽任何规划语义检查。

## Issues Encountered

- Windows PowerShell 5.1 对无 BOM UTF-8 脚本中的中文解析不可靠；三个含中文的新/修改 PowerShell 脚本统一保存为 UTF-8 BOM，同时保持 LF。
- `ConvertTo-Json` 在 Windows PowerShell 中产生 CRLF；lock 写入前显式归一化为 LF。

## Authentication Gates

None - 本计划只读取本地仓库文件并运行本地临时副本测试。

## Known Stubs

None.

## User Setup Required

None.

## Next Phase Readiness

- 01-28 可直接依赖 `PhaseComplete Local Strict` 的实时 plan/source/path/CLI/digest 审计，不再信任静态 COVERED 文本。
- 后续若有意修改任何 lock 输入，必须在审阅变化后显式运行 `-UpdateLock` 并重新执行完整负例自测。
- 当前审计不会写 PLAN、ROADMAP、REQUIREMENTS、PATTERNS、VALIDATION、SOURCE-AUDIT 或 runner matrix。

## Self-Check: PASSED

- 四个实现/集成文件与本 SUMMARY 均存在。
- `f1b685f`、`2c423c7`、`10989f3`、`1a927e2` 均存在于当前 `main` 历史。
- coverage classifier 识别 3 个 deliverables，全部由通过的自动化集成验证覆盖。
- 未留下 stub、TODO、FIXME、skipped test 或未运行的计划 verify。
- 工作树中仅保留用户已有的 `.planning/config.json` 修改、research cache 未跟踪文件，以及待提交的本计划规划元数据。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 07*
*Completed: 2026-08-06*
