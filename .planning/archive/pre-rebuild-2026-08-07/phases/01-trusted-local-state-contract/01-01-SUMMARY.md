---
phase: 01-trusted-local-state-contract
plan: 01
subsystem: supply-chain
tags: [npm, powershell, registry, package-identity, canary]

# Dependency graph
requires:
  - phase: none
    provides: "Phase 1 计划、研究与验证策略中的 npm legitimacy contract"
provides:
  - "7 个 SUS package@version 与官方 GitHub repository 的 exact allowlist"
  - "固定公开 registry、空 npm 配置和隔离工作目录的只读 metadata verifier"
  - "覆盖伪造 metadata、恶意 .npmrc、私有 registry、token canary 和查询失败的生产 predicate 自测"
affects: [01-02 npm legitimacy approval, 01-08 frontend scaffold]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "PowerShell verifier 使用 exact JSON schema、规范化 GitHub repository 和 fail-closed 比较"
    - "npm 查询只在固定公开 registry、空 user/global config 与无 .npmrc 工作目录中执行"
    - "测试通过生产 verifier 的 metadata fixture 模式与 npm wrapper 正控制复用同一 predicate"

key-files:
  created:
    - tests/fixtures/contracts/npm-package-allowlist.json
    - scripts/contracts/verify-npm-package-allowlist.ps1
    - scripts/contracts/test-npm-package-allowlist.ps1
  modified: []

key-decisions:
  - "allowlist 固定为 registry.npmjs.org、schemaVersion 1，并只允许 name/version/repository 三个 package 字段。"
  - "repository 统一规范化为 GitHub HTTPS canonical URL，兼容 npm 返回的 git+https 与 repository object，但比较仍为 exact。"
  - "临时 verifier 根目录优先使用不在用户 profile 下的公开/系统临时位置，避免祖先 .npmrc 通过 npm project config 污染查询。"

patterns-established:
  - "生产脚本不回显 npm stdout/stderr、token、Authorization 或配置正文；失败只返回统一非零结果。"
  - "测试脚本通过真实生产 verifier 覆盖正例、错名、错版本、错 repository、缺字段、重复项、额外项和查询失败。"

requirements-completed: [STATE-01, STATE-02]

coverage:
  - id: D1
    description: "7 个 SUS package@version 的固定公开 registry 身份门禁"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/contracts/verify-npm-package-allowlist.ps1 -Allowlist tests/fixtures/contracts/npm-package-allowlist.json"
        status: pass
    human_judgment: false
  - id: D2
    description: "npm 配置污染、伪造 metadata 与 canary 泄漏自测"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/contracts/test-npm-package-allowlist.ps1"
        status: pass
    human_judgment: false

# Metrics
duration: 约 45 分钟
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 1: 可信 npm 包身份门禁 Summary

**固定 7 个 SUS npm 包的官方身份，并用隔离 registry/config/cwd 与 canary 负例证明 package legitimacy 门禁 fail-closed**

## Performance

- **Duration:** 约 45 分钟
- **Started:** 2026-08-06T00:34:00Z
- **Completed:** 2026-08-06T01:18:55Z
- **Tasks:** 2/2
- **Files modified:** 3

## Accomplishments

- 创建 7 项 exact `package@version` allowlist，覆盖 React、React DOM、Vite、TypeScript、Vite React plugin 与两个 DefinitelyTyped 包。
- 实现只读 verifier：每次运行创建空 user/global npm config、无 `.npmrc` 的隔离工作目录，固定 `https://registry.npmjs.org/`，并清除 registry/auth/token 相关环境变量。
- 实现生产 predicate 自测：正控制通过，错名/版本/repository、缺字段、重复项、额外项和查询失败全部非零；恶意 `.npmrc`、私有 registry 与 token canary 未改变请求或进入输出。
- 确认 verifier 与自测没有创建或修改仓库 `package.json`、lockfile 或 `node_modules`。

## Task Commits

Each task was committed atomically:

1. **Task 1: 固定隔离 npm metadata verifier** - `94f44769fa8f2a51ccda0faea770c61fb88842a8` (`feat(01-01)`)
2. **Task 2: 证明恶意 npm 配置和伪造 metadata 不能绕过门禁** - `8ac37823c6d40d31e40261364fb2137d3e39b0d5` (`test(01-01)`)

**Plan metadata:** `docs(01-01): 完成可信本地状态计划`

## Files Created/Modified

- `tests/fixtures/contracts/npm-package-allowlist.json` - 7 个精确 package/version/repository 身份与固定 registry。
- `scripts/contracts/verify-npm-package-allowlist.ps1` - allowlist schema 校验、GitHub repository 规范化、隔离 npm view 和 fail-closed predicate。
- `scripts/contracts/test-npm-package-allowlist.ps1` - 生产 verifier 正反 fixture、npm wrapper、恶意配置与泄漏扫描。

## Decisions Made

- 用 canonical GitHub HTTPS URL 作为 repository 比较值，兼容 npm metadata 常见的 `git+https://...git` 和 object 载体后再 exact compare。
- 不使用用户 profile 下的默认临时目录作为 npm cwd；临时根目录优先选择没有祖先 `.npmrc` 的公开/系统目录，防止用户级配置被 npm 当作 project config 读取。
- 测试不安装任何依赖，直接用 metadata fixture 与一次性 npm wrapper 验证生产 predicate，保留 01-02 人工批准作为首次 install 前的独立门禁。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] 修正 PowerShell 集合展开与 repository `.git` 规范化**
- **Found during:** Task 1（固定隔离 npm metadata verifier）
- **Issue:** 初版使用 generic list 作为 JSON package 数组时在 PowerShell 中不能稳定枚举；repository 正则可能把末尾 `.git` 纳入路径，导致真实 npm metadata 无法与 fixture 匹配。
- **Fix:** 改用普通数组构建 contract，并在 GitHub repository 规范化时显式去除 `.git`。
- **Files modified:** `scripts/contracts/verify-npm-package-allowlist.ps1`
- **Verification:** metadata 正控制、真实 registry verifier 和完整 self-test 均通过。
- **Committed in:** `94f44769fa8f2a51ccda0faea770c61fb88842a8`

**2. [Rule 3 - Blocking] 隔离用户 profile 祖先 `.npmrc` 对 npm project config 的污染**
- **Found during:** Task 1（真实公开 metadata 校验）
- **Issue:** 本机用户 `.npmrc` 位于临时目录祖先路径，包含 `prefix` 配置；即使显式传入空 user/global config，npm 仍拒绝查询并报告 project config 冲突。
- **Fix:** verifier 临时根目录优先选择 `C:\Users\Public\GPTEasy\Temp`、`ProgramData` 或系统临时目录，并拒绝带祖先 `.npmrc` 的候选路径；工作目录本身保持无 `.npmrc`。
- **Files modified:** `scripts/contracts/verify-npm-package-allowlist.ps1`
- **Verification:** 真实公开 registry 7/7 查询通过；恶意项目 `.npmrc`、私有 registry 和 token canary self-test 通过。
- **Committed in:** `94f44769fa8f2a51ccda0faea770c61fb88842a8`

---

**Total deviations:** 2 auto-fixed（1 Rule 1、1 Rule 3）
**Impact on plan:** 两项调整都直接服务于真实 metadata 身份校验的正确性和 npm 配置隔离；没有安装依赖、没有扩展业务范围。

## Issues Encountered

- 真实查询第一次暴露了用户级 `.npmrc` 的祖先路径污染；已按 Rule 3 修正为不在用户 profile 下且检查祖先配置的临时 cwd。
- 初始 `STATE.md` 使用 `Plan: 0 of TBD`，导致 `state.advance-plan` 无法解析；已将当前 Phase 1 计划总数规范化为 28 后重跑状态更新，并确认推进到 Plan 1。
- PowerShell 任务脚本和生产脚本均使用 Unix 换行；`git diff --check` 通过。

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- 01-01 的机器身份门禁已完成，后续 01-02 可以在首次 npm install 前执行人工官方来源批准。
- 01-02 仍必须人工核对 7 个 SUS 包的发布者、repository 和官方 Tauri React TypeScript 模板来源；本计划没有替代该 checkpoint。

## Self-Check: PASSED

- 所有 3 个计划产物存在。
- `94f4476` 与 `8ac3782` 均存在于当前 `main` 分支历史。
- 真实 registry verifier 和生产 predicate self-test 均以退出码 0 通过。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 01*
*Completed: 2026-08-06*
