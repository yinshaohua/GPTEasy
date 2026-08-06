---
phase: 01-trusted-local-state-contract
plan: 12
subsystem: contracts
tags: [macos, zsh, codex-app-server, github-actions, security-boundary]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-04 的脱敏 contract evidence schema、退出码与 strict eligibility 边界"
provides:
  - "macOS Codex 0.146.1 app-server/config-read 脱敏 probe"
  - "Codex.app/ChatGPT.app bundled Codex 与 official CLI 默认用户层 canary parity predicate"
  - "Intel/Apple Silicon 可复用 zsh Wave 0 workflow 与本地 dependency contract"
affects: [macos-evidence, macos-packaging, codex-host-freeze, phase-1-freeze]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "原生 zsh 先以 zsh -n 解析全部合同脚本，再运行 test-only fixture predicate"
    - "app-server 原始响应只在权限受限临时文件中处理，证据仅输出摘要、类别与布尔事实"
    - "macOS evidence workflow 必须先以 reusable job 依赖双架构 Wave 0"

key-files:
  created:
    - scripts/contracts/probe-codex-macos.zsh
    - scripts/contracts/probe-macos-host.zsh
    - scripts/contracts/test-macos-contract-probes.zsh
    - .github/workflows/phase1-macos-wave0.yml
    - scripts/contracts/test-macos-wave0-contract.ps1
  modified: []

key-decisions:
  - "macOS Codex probe 与 Windows schema 对齐，固定 0.146.1、initialize/initialized/config-read(includeLayers=true)，但不输出 raw response、配置正文、用户绝对路径或凭据值。"
  - "正式宿主候选固定覆盖系统级和当前用户级 Codex.app/ChatGPT.app，bundled Codex 只从 Contents/Resources/codex 定位。"
  - "fixture 通过同一 probe/parity predicate，但始终 test_only=true 且 strict_gate_eligible=false；只有 Darwin 14+ 和目标原生架构可获得真实 strict eligibility。"
  - "Wave 0 使用 exact actions/checkout commit 与 github.sha，并在 macos-15 arm64、macos-15-intel x86_64 上先执行全部 zsh 语法检查和 fixture tests。"

patterns-established:
  - "共享默认用户层 canary 只保存固定非敏感值，清理前重新核对 SHA-256，所有输出以允许清单摘要表达。"
  - "Windows 开发主机只验证 workflow/dependency 静态合同，不冒充原生 zsh 或 macOS runner 结果。"

requirements-completed: [STATE-02, STATE-03, STATE-05]

coverage:
  - id: D1
    description: "macOS official CLI 与 bundled Codex probe 对齐 Windows evidence schema，并比较默认用户层 model/provider/origin/carrier 摘要"
    requirement: STATE-02
    verification:
      - kind: other
        ref: "powershell -NoProfile -File scripts/contracts/test-macos-wave0-contract.ps1 -Scripts scripts/contracts/probe-codex-macos.zsh,scripts/contracts/probe-macos-host.zsh,scripts/contracts/test-macos-contract-probes.zsh -Workflow .github/workflows/phase1-macos-wave0.yml"
        status: pass
    human_judgment: true
    rationale: "当前执行主机是 Windows 且没有 zsh；真实协议与原生宿主行为必须由 macOS Wave 0/evidence run 证明，本文不把静态合同升级为原生结果。"
  - id: D2
    description: "可复用 Wave 0 以 exact checkout 覆盖 macos-15 arm64 与 macos-15-intel x86_64，并在 evidence 前运行 zsh 语法和合同测试"
    requirement: STATE-03
    verification:
      - kind: integration
        ref: "scripts/contracts/test-macos-wave0-contract.ps1#reusable dual-architecture workflow and exact checkout contract"
        status: pass
    human_judgment: false
  - id: D3
    description: "zsh fixture tests 覆盖 positive、host missing、wrong arch、root/origin/provider/carrier mismatch，且 fixture 永不 strict eligible"
    requirement: STATE-05
    verification:
      - kind: other
        ref: "scripts/contracts/test-macos-wave0-contract.ps1#fixture matrix and syntax-test dependency contract"
        status: pass
    human_judgment: true
    rationale: "fixture 场景与调用顺序已由 Windows 静态合同验证；zsh 控制流本身需在原生 Wave 0 job 中执行后才能获得运行时证明。"

# Metrics
duration: 约 20 分钟
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 12: macOS 原生 Wave 0 与共享宿主配置契约 Summary

**以脱敏 app-server 摘要和固定默认用户层 canary 建立 macOS CLI/正式宿主 parity，并让 Intel 与 Apple Silicon 的原生 zsh 语法和 fixture tests 成为后续 evidence 的先行门禁**

## Performance

- **Duration:** 约 20 分钟
- **Started:** 2026-08-06T04:42:53Z
- **Completed:** 2026-08-06T05:02:44Z
- **Tasks:** 1/1
- **Files modified:** 5 个文件（全部新建；TDD GREEN 同步收敛 RED 契约）

## Accomplishments

- 新增 macOS Codex `0.146.1` probe：验证 Darwin 14+、目标原生架构、binary/schema SHA-256 与 app-server `initialize`、`initialized`、`config/read(includeLayers=true)`，只输出与 Windows schema 对齐的脱敏事实。
- 新增正式 macOS 宿主 parity probe：覆盖 `/Applications` 与 `~/Applications` 下的 `Codex.app`/`ChatGPT.app`，从固定 `Contents/Resources/codex` 定位 bundled binary，并与 official CLI 比较同一默认用户层 canary 摘要。
- 新增原生 zsh self-test：首先对自身和两个 probe 执行 `zsh -n`，随后覆盖 positive、宿主缺失、架构错误以及 root/origin/provider/carrier 错配。
- 新增 `workflow_call` + `workflow_dispatch` Wave 0：使用 exact checkout，在 Apple Silicon 与 Intel runner 上验证原生身份、解析全部 zsh 文件、执行合同 fixture，并在后续 package verifier 出现时自动运行其 zsh tests。
- 新增 PowerShell dependency contract：从 Windows 主机静态验证全部脚本和 workflow 的可复用性、双架构、exact checkout、语法/测试顺序与未来 evidence dependency seam。

## Task Commits

Each task was committed atomically:

1. **Task 1 RED: macOS Wave 0 失败契约** - `f6ab8dc` (`test`)
2. **Task 1 GREEN: macOS probes、zsh fixtures 与双架构 Wave 0** - `3c1eaa5` (`feat`)

**Plan metadata:** final metadata commit（包含 SUMMARY、STATE、ROADMAP、REQUIREMENTS 与已解决 deviation ledger）。

_Note: 本任务按 RED → GREEN 执行 TDD，因此包含测试提交和实现提交。_

## Files Created/Modified

- `scripts/contracts/probe-codex-macos.zsh` - Darwin/arch、Codex 版本与 digest、app-server 三步协议和脱敏 config/read 摘要 predicate。
- `scripts/contracts/probe-macos-host.zsh` - 正式 bundle 候选、固定 bundled relative path、同用户层 canary 写入/清理和 host-vs-CLI parity。
- `scripts/contracts/test-macos-contract-probes.zsh` - 原生 zsh 语法入口、正例、六类 fail-closed 场景和输出脱敏扫描。
- `.github/workflows/phase1-macos-wave0.yml` - Intel/Apple Silicon 可复用 Wave 0，exact checkout 后先执行 zsh syntax/tests。
- `scripts/contracts/test-macos-wave0-contract.ps1` - Windows 可运行的 workflow、脚本和未来 evidence dependency 静态合同。

## Decisions Made

- app-server 原始 `config/read` 只在权限受限临时文件和进程内变量中处理；输出永远不包含 canary、完整配置、完整进程参数或用户绝对路径。
- CLI 与 bundled Codex 的 binary digest 可以不同，但两者必须精确为 `0.146.1`，读取同一个默认 `~/.codex` 用户层，并在 model/provider/origin/carrier 摘要上完全一致。
- bundle 名只允许 `Codex.app` 或 `ChatGPT.app`，bundled binary 固定为 `Contents/Resources/codex`；调用方不能传入另一条 relative path 放宽边界。
- Wave 0 只证明原生 zsh 语法与 fixture 控制流；真实 bundle、签名、公证、Gatekeeper 与一次性 OS 用户 evidence 继续由 01-14 依赖本 workflow 完成。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] 修正 RED 静态合同对 shell 转义和 YAML matrix 列表的误判**

- **Found during:** Task 1 GREEN（首次运行 PowerShell dependency contract）。
- **Issue:** 初始测试只接受未转义的 JSON method 字面量、把 verifier 中用于拒绝扫描的敏感字段名误判为泄漏，并遗漏 YAML `include` 项前的 `-`。
- **Fix:** method 断言改为同时接受 shell 转义形式；源码泄漏扫描只约束生产 probes；matrix runner pattern 精确包含列表标记。
- **Files modified:** `scripts/contracts/test-macos-wave0-contract.ps1`
- **Verification:** 计划指定的 PowerShell verify 命令返回 0。
- **Committed in:** `3c1eaa5`

**2. [Rule 1 - Bug] 修正 probe 内部摘要通道和子进程退出码传播**

- **Found during:** Task 1 GREEN（host-vs-CLI parity 源码审查）。
- **Issue:** 初始内部 TSV 输出使用了字面 `\t`，且 host child exit code 以 blocked 默认值初始化，可能让正确 fixture 无法进入 parity。
- **Fix:** 输出真实 tab 分隔符；每次 child probe 调用前显式把退出码初始化为 0，并只在子进程非零时覆盖。
- **Files modified:** `scripts/contracts/probe-codex-macos.zsh`, `scripts/contracts/probe-macos-host.zsh`
- **Verification:** PowerShell dependency contract 返回 0；真实 zsh 运行留给已创建的原生 Wave 0，不在 Windows 上虚构结果。
- **Committed in:** `3c1eaa5`

**3. [Rule 2 - Missing Critical] 补齐 raw response 解析失败和 canary 所有权清理的 fail-closed 路径**

- **Found during:** Task 1 GREEN（T-01-27 信息泄漏与临时配置清理审查）。
- **Issue:** 原始实现需要显式保证解析失败时删除临时响应，并且只在 canary digest 未变化时删除配置。
- **Fix:** config/read 摘要派生失败立即删除临时响应并返回 blocked；host 清理前重新计算 canary SHA-256，无法证明所有权时保留文件并拒绝 strict pass。
- **Files modified:** `scripts/contracts/probe-codex-macos.zsh`, `scripts/contracts/probe-macos-host.zsh`
- **Verification:** 静态 dependency contract、stub scan 与 `git diff --check` 均通过。
- **Committed in:** `3c1eaa5`

---

**Total deviations:** 3 auto-fixed（Rule 1：2，Rule 2：1）
**Impact on plan:** 三项均直接保证测试准确性、parity 正确性或敏感临时数据/配置清理安全，没有新增依赖、网络接口、持久化 schema 或计划外产品范围。

## Issues Encountered

- 当前执行环境为 Windows，未安装原生 `zsh`，WSL2 发行版中也没有 zsh；严格按计划 deviation rules 运行 PowerShell fixture/workflow dependency contract，没有宣称真实 macOS、Intel/Apple Silicon runner 或 zsh 运行结果。
- `actionlint` 与 `shellcheck` 在当前环境不可用，未安装计划外依赖；workflow 结构由仓库内 PowerShell contract 检查，并将在实际 GitHub macOS Wave 0 job 中接受平台解析与执行。

## Known Stubs

None。五个文件没有 TODO/FIXME、占位数据源或阻止目标的硬编码空输出；fixture 明确标记 `test_only=true` 且永不授予 strict eligibility。

## Authentication Gates

None。

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- 01-14 可通过 job-level reusable workflow 调用 `.github/workflows/phase1-macos-wave0.yml`，并让 Intel/Apple Silicon evidence jobs 显式 `needs: wave0`。
- 后续 macOS package verifier 只需创建 `scripts/contracts/test-macos-package-verifier.zsh`；Wave 0 已包含“文件出现后先 syntax-check 再执行”的入口。
- 当前仅关闭 workflow/dependency 与 fixture contract 的本地实现工作；真实 macOS host、签名、公证、Gatekeeper 和一次性用户生命周期仍保持既定后续门禁。

## Self-Check: PASSED

- 5 个计划文件均已创建并存在，三个 zsh 文件在 Git index 中为 `100755`。
- TDD 提交 `f6ab8dc` 与 `3c1eaa5` 均存在于当前 main 历史。
- 计划指定的 PowerShell dependency contract 返回 0，stub scan 无结果，`git diff --check` 返回 0。
- 当前 Windows 主机没有 zsh，因此没有记录或暗示任何真实 macOS/native zsh pass。
- 工作树中的既有 `.planning/config.json` 修改与 `.planning/research/.cache/*` 未跟踪文件保持原样，未被本计划提交。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 12*
*Completed: 2026-08-06*
