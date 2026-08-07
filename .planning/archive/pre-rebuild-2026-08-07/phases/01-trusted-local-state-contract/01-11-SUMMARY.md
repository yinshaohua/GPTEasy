---
phase: 01-trusted-local-state-contract
plan: 11
subsystem: contracts
tags: [powershell, codex-app-server, windows-appx, wsl2, security-boundary]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-04 的脱敏 contract evidence schema、退出码与 strict eligibility 边界"
provides:
  - "Codex 0.146.1 app-server initialize/initialized/config-read 脱敏探针"
  - "正式 OpenAI.Codex AppX bundled Codex 与 official CLI 的共享用户层一致性 predicate"
  - "只读 WSL2 host probe、运行集合不变门禁和重复 DistributionName fail-closed 解析"
affects: [windows-contract-runners, codex-host-freeze, wsl2-environment-discovery, phase-1-freeze]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "外部进程的原始配置、stdout/stderr 与完整命令行只在内存处理，证据只保留允许清单摘要"
    - "Windows 正式宿主以固定 AppX package family 和固定 bundled relative path 定位"
    - "WSL2 被动发现只允许四次固定只读调用，并以 Lxss registration GUID 作为稳定身份"

key-files:
  created:
    - scripts/contracts/probe-codex.ps1
    - scripts/contracts/probe-windows-host.ps1
    - scripts/contracts/probe-wsl2.ps1
    - scripts/contracts/test-windows-contract-probes.ps1
    - scripts/contracts/test-wsl2-probe.ps1
  modified: []

key-decisions:
  - "Codex probe 固定要求 0.146.1，执行 initialize、initialized、config/read(includeLayers=true)，但绝不把 raw response、配置正文或凭据值写入输出。"
  - "Windows 正式宿主身份固定为 OpenAI.Codex_2p2nqsd0c76g0，bundled Codex 固定从 app/resources/codex.exe 定位；调用方不能放宽 allowlist。"
  - "WSL2 探针只允许 --version、--list --quiet 与两次 --list --running --quiet；重复 DistributionName 始终 command_target_resolvable=false。"

patterns-established:
  - "Contract fixture 与真实探针复用同一生产 predicate；fixture 结果始终 test_only=true 且 strict_gate_eligible=false。"
  - "外部配置事实使用 SHA-256 摘要、根类别和布尔载体表达，不保存敏感原文。"

requirements-completed: [STATE-02, STATE-03, STATE-05]

coverage:
  - id: D1
    description: "Codex 0.146.1 official CLI probe 验证 binary/schema digest 与 app-server 三步协议，并只输出脱敏配置摘要"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-windows-contract-probes.ps1"
        status: pass
    human_judgment: false
  - id: D2
    description: "正式 Windows AppX bundled Codex 与 official CLI 必须读取同一默认用户层 canary，并对根、摘要和凭据载体 fail-closed"
    requirement: STATE-05
    verification:
      - kind: integration
        ref: "scripts/contracts/test-windows-contract-probes.ps1#positive parity and 6 fail-closed cases"
        status: pass
    human_judgment: false
  - id: D3
    description: "WSL2 host probe 保持运行集合不变、拒绝 guest command，并让重复显示名不可解析"
    requirement: STATE-03
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope ContractSelfTest -Target Wsl2 -Mode Strict"
        status: pass
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/probe-wsl2.ps1"
        status: pass
    human_judgment: false

# Metrics
duration: 约 38 分钟
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 11: Windows Codex 与 WSL2 宿主契约 Summary

**以脱敏 app-server 摘要冻结 official CLI/正式宿主共享用户层一致性，并用固定只读命令和注册 GUID 建立无副作用 WSL2 predicate**

## Performance

- **Duration:** 约 38 分钟
- **Started:** 2026-08-06T03:58:00Z
- **Completed:** 2026-08-06T04:35:34Z
- **Tasks:** 2/2
- **Files modified:** 5 个新脚本

## Accomplishments

- 新增 Codex `0.146.1` 探针：计算 binary/schema SHA-256，执行 app-server `initialize`、`initialized`、`config/read(includeLayers=true)`，并只输出配置根类别、model/provider/origin 摘要和凭据载体布尔值。
- 新增正式 Windows AppX 宿主 predicate：固定 `OpenAI.Codex_2p2nqsd0c76g0` 与 `app/resources/codex.exe`，在同一一次性用户默认配置根写入固定非敏感 canary，比较 CLI 与 bundled Codex 的共享用户层事实。
- 新增 WSL2 被动探针：只调用固定的 version/list/list-running 命令并只读 HKCU Lxss，验证运行集合前后相同、guest command 计数为 0，并让重复显示名不可解析。
- 两份 fixture self-test 覆盖宿主缺失、版本/配置根/provider/origin/credential carrier 错配、运行集合漂移、guest command 注入与重复显示名。

## Task Commits

Each task was committed atomically:

1. **Task 1 RED: Windows 契约探针失败用例** - `6fa9f36` (`test`)
2. **Task 1 GREEN: Windows Codex/宿主一致性探针** - `7540aa7` (`feat`)
3. **Task 2 RED: WSL2 被动探针失败用例** - `cb26f63` (`test`)
4. **Task 2 GREEN: 无副作用 WSL2 host probe** - `c2cac6d` (`feat`)

**Plan metadata:** final metadata commit（包含 SUMMARY、STATE、ROADMAP、REQUIREMENTS 与 broken-windows ledger）。

_Note: 两个任务均按 RED → GREEN 执行 TDD，因此各包含一份测试提交和一份实现提交。_

## Files Created/Modified

- `scripts/contracts/probe-codex.ps1` - Codex 版本、binary/schema digest、app-server 协议与脱敏配置摘要 predicate。
- `scripts/contracts/probe-windows-host.ps1` - 固定 AppX 身份、一次性用户 canary 写入/清理和 host-vs-CLI parity。
- `scripts/contracts/probe-wsl2.ps1` - UTF-16 WSL 只读命令、Lxss 注册记录、运行集合和命令目标可解析性。
- `scripts/contracts/test-windows-contract-probes.ps1` - Windows CLI/host 正例、脱敏门禁与六个 fail-closed fixture 场景。
- `scripts/contracts/test-wsl2-probe.ps1` - WSL2 运行集合、guest command、重名与只读源码边界自测。

## Decisions Made

- Codex probe 只对非敏感 provider 字段、来源类型和凭据载体布尔值计算/输出摘要；`experimental_bearer_token`、原始 `config/read` 响应和 stderr 均不得进入证据。
- official CLI 和 bundled Codex 的 binary digest 可以不同，但两者必须都是精确 `0.146.1`，且各自 schema digest、协议步骤与用户层 canary 检查必须通过。
- Windows 正式宿主 allowlist 不开放调用方参数；只有固定 package family 与固定 bundled relative path 可以获得 host 身份。
- WSL2 命令序列是固定事实源，任何 `-d`、`--distribution`、`--exec`、`--user` 或其他未列入允许清单的调用都按 security boundary failure 拒绝。
- 发行版显示名不是稳定身份；数据库/证据身份使用 Lxss registration GUID，名称只在注册记录与 `wsl --list` 均唯一时才可成为 command target。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] 修正 Windows PowerShell 静态协议断言的属性语法**

- **Found during:** Task 1 GREEN（Windows probe 自测）。
- **Issue:** RED 测试错误地要求 PowerShell ordered hashtable 的 `method` 属性名带引号，导致正确的 `method = "initialize"` 实现被误报。
- **Fix:** 将三条源码断言收敛为 PowerShell 实际属性语法，同时仍精确约束 `initialize`、`initialized` 和 `config/read`。
- **Files modified:** `scripts/contracts/test-windows-contract-probes.ps1`
- **Verification:** `powershell -NoProfile -File scripts/contracts/test-windows-contract-probes.ps1` 返回 0。
- **Committed in:** `7540aa7`

**2. [Rule 1 - Bug] 修正 Windows PowerShell StrictMode 下单元素输出的 Count 访问**

- **Found during:** Task 2 GREEN（WSL2 positive fixture）。
- **Issue:** `wsl --version` 只有一行时，Windows PowerShell 会把结果解包为标量；StrictMode 下直接访问 `.Count` 失败并把正例误判为 probe unavailable。
- **Fix:** 所有数量判断使用 `@(...).Count` 显式数组化，保持单元素与多元素输出语义一致。
- **Files modified:** `scripts/contracts/probe-wsl2.ps1`
- **Verification:** WSL fixture self-test、runner Strict dispatch 和真实本机只读 probe 均返回 0。
- **Committed in:** `c2cac6d`

**3. [Rule 1 - Bug] 修正 schema 输出目录的 Windows 参数引用**

- **Found during:** Task 1 GREEN 后的生产路径审查。
- **Issue:** 初始引用函数会重复普通反斜杠，可能改变 `generate-json-schema --out` 的 Windows 路径文本。
- **Fix:** 只转义双引号并保留普通路径分隔符，继续由固定参数模板包裹输出目录。
- **Files modified:** `scripts/contracts/probe-codex.ps1`
- **Verification:** Windows contract self-test 返回 0，`git diff --check` 返回 0。
- **Committed in:** `7540aa7`

---

**Total deviations:** 3 auto-fixed（Rule 1：3）
**Impact on plan:** 三项均是测试或生产脚本的直接正确性修复，没有增加依赖、网络、认证、持久化 schema 或计划外产品范围。

## Issues Encountered

- 本机真实 WSL2 注册表存在两个同名 `Ubuntu` registration GUID；真实探针按合同输出两个 `command_target_resolvable=false`，同时确认运行集合保持 `Ubuntu` 不变。
- 当前工作树没有精确 `0.146.1` official CLI 与一次性 Windows OS 用户，因此本计划只执行 Windows fixture self-test；真实 x64/ARM64 runner 仍由后续既定 contract workflow 采集，fixture 不获得 strict eligibility。

## Known Stubs

None。五个脚本没有 TODO/FIXME、占位数据源或会阻止计划目标的硬编码空输出；fixture 路径明确标记 `test_only=true` 且永不授予 strict eligibility。

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Windows x64/ARM64 runner 可在一次性 OS 用户下调用 `probe-windows-host.ps1`，传入 official Codex `0.146.1` executable，采集正式 host-vs-CLI predicate。
- WSL2 runner 已可通过统一命令 `run-phase1-contracts.ps1 -Scope ContractSelfTest -Target Wsl2 -Mode Strict` 执行 fixture self-test。
- 当前真实 WSL2 同名 `Ubuntu` 状态已按预期 fail-closed；后续 Phase 5 不得猜测 registration GUID 到 `wsl.exe -d NAME` 的映射。

## Self-Check: PASSED

- 5 个计划脚本均已创建并存在。
- TDD 提交 `6fa9f36`、`7540aa7`、`cb26f63`、`c2cac6d` 均存在于当前 main 历史。
- Windows fixture self-test、WSL2 Strict runner 和真实只读 WSL2 probe 均返回 0。
- stub scan 无结果，普通文本均为 Unix LF。
- 工作树中的既有 `.planning/config.json` 修改与 `.planning/research/.cache/*` 未跟踪文件保持原样，未被本计划提交。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 11*
*Completed: 2026-08-06*
