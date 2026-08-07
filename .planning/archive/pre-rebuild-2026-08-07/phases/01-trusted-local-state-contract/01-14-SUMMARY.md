---
phase: 01-trusted-local-state-contract
plan: 14
subsystem: contracts
tags: [macos, zsh, codesign, notarization, gatekeeper, github-actions, provenance]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-04 的脱敏 evidence schema 与 strict eligibility 边界"
  - phase: 01-trusted-local-state-contract
    provides: "01-06 的固定只读 GitHub evidence preflight"
  - phase: 01-trusted-local-state-contract
    provides: "01-10 的真实 AppHandle 当前用户 path smoke"
  - phase: 01-trusted-local-state-contract
    provides: "01-12 的 macOS Codex/正式宿主 probe 与双架构 Wave 0"
provides:
  - "同一生产 predicate 驱动的 macOS package test-only 正控制与 9 类 fail-closed 负例"
  - "绑定 GitHub job、runner、UID、home/profile 与 finalized cleanup 的一次性 macOS 账户生命周期证据"
  - "Developer ID、notarization/stapling、Gatekeeper、~/Applications、path smoke 与 archive correlation 组合门禁"
  - "显式依赖 Wave 0 的 Intel/Apple Silicon attested evidence workflow"
affects: [01-16 macos evidence, 01-17 provenance verification, 01-26 freeze, 01-28 phase completion]

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "fixture 与 live facts 通过同一 zsh predicate；fixture 永远 test_only=true 且 strict_gate_eligible=false"
    - "finalized lifecycle evidence 必须先于 package strict predicate、上传和 provenance attestation"
    - "最终上传 archive 必须解包并重新验证其中的签名 app，不能只验证旁路 build directory"
    - "Apple 凭据仅在单一 CI step 的私有目录和临时 keychain 中消费并在 trap/finally 清理"

key-files:
  created:
    - scripts/contracts/assert-macos-job-lifecycle.zsh
    - scripts/contracts/run-macos.zsh
    - scripts/contracts/test-macos-package-verifier.zsh
    - tests/fixtures/contracts/packaging/macos-positive-control.json
    - .github/workflows/phase1-macos-evidence.yml
  modified:
    - scripts/contracts/test-macos-wave0-contract.ps1
    - tests/fixtures/contracts/runner-cli-matrix.json
    - tests/fixtures/contracts/phase1-plan-audit-lock.json

key-decisions:
  - "macOS strict pass 同时要求原生 Darwin/目标架构、Developer ID codesign、公证 stapling、Gatekeeper、当前用户 HOME_APPLICATIONS、跨进程 reopen、archive/app 关联与 finalized cleanup。"
  - "一次性账户证据固定绑定 repository/run/attempt/job、runner name/tracking/architecture、UID 与 profile digest；状态文件不保存随机账户密码。"
  - "Intel 与 Apple Silicon 使用同一 matrix job，但每个展开后的 native job 都继承 needs: wave0、exact checkout 和独立账户生命周期。"
  - "本计划只建立并本地验证 workflow/contract；没有把 Windows 静态结果写成真实 macOS、Apple 签名、公证或远程 GitHub run 结果。"

patterns-established:
  - "current-user install evidence 只保存 root category、profile digest、Gatekeeper/path-smoke 布尔事实，不保存绝对用户路径。"
  - "lifecycle guard 在半创建失败时回滚账户/home，并把状态/证据文件归还 sudo 调用者所有权后再供后续 step 消费。"
  - "canonical PackagingSelfTest Local 同时执行 Windows package 正负例和完整 macOS Wave 0/evidence 静态合同。"

requirements-completed: [STATE-01, STATE-02, STATE-03, STATE-04, STATE-05]

coverage:
  - id: D1
    description: "macOS package 正控制通过同一生产 predicate，且 non-Darwin、wrong-arch、archive mismatch、codesign/notary/Gatekeeper、system install、marker-only 与 cleanup missing 均 fail closed"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/test-macos-wave0-contract.ps1 -Scripts <all-six-zsh-scripts> -Workflow .github/workflows/phase1-macos-wave0.yml -EvidenceWorkflow .github/workflows/phase1-macos-evidence.yml"
        status: pass
    human_judgment: true
    rationale: "当前执行主机是 Windows 且没有 zsh；PowerShell 静态合同已通过，但 zsh fixture 控制流只能由原生 macOS Wave 0 执行。"
  - id: D2
    description: "一次性 macOS 账户生命周期绑定 job/runner/UID/profile，并在 strict predicate 前证明账户与 home 删除或完整 ephemeral baseline restore"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "scripts/contracts/test-macos-wave0-contract.ps1#lifecycle source and workflow ordering contract"
        status: pass
    human_judgment: true
    rationale: "账户创建、sysadminctl/createhomedir、进程停止与 profile 删除必须在原生 macOS runner 上获得运行时证据。"
  - id: D3
    description: "Intel/Apple Silicon evidence matrix 依赖 reusable Wave 0，并在 cleanup 后才上传 app/evidence 与生成 provenance attestation"
    requirement: STATE-03
    verification:
      - kind: integration
        ref: "scripts/contracts/test-macos-wave0-contract.ps1#evidence workflow dependency, immutable action pins, and ordering"
        status: pass
    human_judgment: true
    rationale: "workflow 结构已静态验证；Developer ID、公证、Gatekeeper 和 attestation 仍需带真实 GitHub/Apple 凭据的远程 native run。"
  - id: D4
    description: "canonical PackagingSelfTest Local 实际分派 Windows package self-test 与完整 macOS package/evidence contract"
    requirement: STATE-05
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/run-phase1-contracts.ps1 -Scope PackagingSelfTest -Target Local -Mode Strict"
        status: pass
    human_judgment: false

# Metrics
duration: 49min
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 14: macOS 双架构包证据与账户生命周期 Summary

**以一次性 macOS 用户、统一 package predicate 和 Wave 0 依赖 workflow，把 Developer ID、公证、Gatekeeper、当前用户路径、跨进程重开与 attested archive 绑定为同一 strict gate**

## Performance

- **Duration:** 49 分钟
- **Started:** 2026-08-06T07:53:50Z
- **Completed:** 2026-08-06T08:42:28Z
- **Tasks:** 2/2
- **Files modified:** 8

## Accomplishments

- 新增 macOS package 正控制与生产 predicate：fixture 复用 live predicate 但永不 strict eligible，并覆盖 9 类 fail-closed 分支。
- 新增一次性 macOS OS 用户 lifecycle guard：绑定 GitHub/runner/UID/profile，停止该 UID 的 run-scoped 进程，删除账户/home，并输出 finalized 脱敏 evidence。
- package strict pass 同时要求 Developer ID codesign、stapled notarization、Gatekeeper、`HOME_APPLICATIONS`、path smoke reopen、profile digest、cleanup attestation 与最终 archive 中 app 的重新验签/关联。
- 新增 Intel/Apple Silicon evidence matrix：先调用 reusable Wave 0，再 exact checkout、只读 gh preflight、签名构建、公证、当前用户安装/path smoke、cleanup、上传与 provenance attestation。
- 更新 canonical PackagingSelfTest 与 Phase 1 digest lock，使本地严格入口覆盖完整 macOS package/evidence contract。

## Task Commits

Each task was committed atomically:

1. **Task 1 RED: macOS package 正控制与拒绝分支** — `9dcf0d3` (`test`)
2. **Task 1 GREEN: 生命周期与 package 生产 predicate** — `1c8f6d9` (`feat`)
3. **Task 2: Wave 0 依赖的双架构 attested workflow** — `5c028a7` (`feat`)
4. **Threat hardening RED: archive/app 关联失败契约** — `8ae9e2d` (`test`)
5. **Threat hardening GREEN: attested archive 与签名 app 绑定** — `c8fda69` (`fix`)
6. **Integration hardening: lifecycle、Gatekeeper 与 path-smoke 身份收紧** — `f93aee7` (`fix`)

**Plan metadata:** 最终元数据提交包含本 SUMMARY、STATE、ROADMAP、REQUIREMENTS 与已解决 deviation ledger。

## Files Created/Modified

- `scripts/contracts/assert-macos-job-lifecycle.zsh` — GitHub/runner/UID/profile 绑定、sudo 调用者文件所有权、半创建回滚、run-scoped 进程停止、账户/home 删除与 baseline restore evidence。
- `scripts/contracts/run-macos.zsh` — fixture/live 共用的 codesign、notarization/stapling、Gatekeeper、当前用户安装、path smoke、lifecycle、identity 与 archive correlation predicate。
- `scripts/contracts/test-macos-package-verifier.zsh` — 对三个 package zsh 文件执行 `zsh -n`，运行正控制和 9 类负例，并扫描敏感输出。
- `tests/fixtures/contracts/packaging/macos-positive-control.json` — test-only Darwin/arm64、Developer ID、公证、Gatekeeper、当前用户路径与 finalized lifecycle 正控制。
- `.github/workflows/phase1-macos-evidence.yml` — Wave 0 dependent、双架构、Developer ID/notary/Gatekeeper/current-user/cleanup/upload/attestation workflow。
- `scripts/contracts/test-macos-wave0-contract.ps1` — 支持 probe/package 两套独立脚本集并静态验证 evidence workflow、immutable action pins 与 cleanup→verify→upload→attest 顺序。
- `tests/fixtures/contracts/runner-cli-matrix.json` — canonical PackagingSelfTest 传入六个 zsh 脚本和 evidence workflow。
- `tests/fixtures/contracts/phase1-plan-audit-lock.json` — 显式更新 runner matrix digest。

## Decisions Made

- live predicate 验证最终上传 ZIP 解包后的 `GPTEasy.app`，并与 build-directory app 的 bundle ID 和主可执行文件摘要关联；不能只验旁路 app 后 attestation 另一份 archive。
- lifecycle state 只保存一次性账户名、UID、home、runner/job identity 与 baseline digest，不保存随机账户密码；Apple 证书/API key 也不进入 lifecycle 或 evidence。
- GitHub-hosted runner 必须显式报告 `RUNNER_ENVIRONMENT=github-hosted`，`RUNNER_ARCH` 必须与 `uname -m` 归一化一致；缺失或错配直接 blocked。
- 当前用户 Gatekeeper 在 disposable user 的 `~/Applications/GPTEasy.app` 上执行；path smoke 还必须证明 `os=macos` 与目标 Rust 架构一致后才写入 `marker_correlated=true`。
- Apple secrets 仅作为 workflow_call/repository secrets 注入单一 signing step，并在私有目录与临时 keychain 中消费；trap 和最终 private-dir cleanup 均会清理。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] 计划级 Task 1 verify 与旧 Wave 0 静态合同输入不兼容**

- **Found during:** Task 1 RED。
- **Issue:** 既有 PowerShell contract 强制要求 01-12 的三个 probe 脚本，但计划的 Task 1 verify 明确只传入三个新 package 脚本，因此即使实现完成也会错误失败。
- **Fix:** 让静态合同识别完整 probe set、完整 package set 或两者同时出现，并分别验证对应 public seam。
- **Files modified:** `scripts/contracts/test-macos-wave0-contract.ps1`
- **Verification:** Task 1 package-only、01-12 probe-only 与 Task 2 all-six 三种命令均返回 0。
- **Committed in:** `9dcf0d3`

**2. [Rule 2 - Missing Critical] canonical PackagingSelfTest 未消费新 package/evidence contract**

- **Found during:** Task 2 canonical verify。
- **Issue:** runner matrix 仍只传入 01-12 的三个 probe 脚本且没有 EvidenceWorkflow，后续本地严格入口可能遗漏 01-14 回归。
- **Fix:** 将六个 zsh 脚本、Wave 0 和 evidence workflow 全部接入 command dispatch，并显式刷新 Phase 1 digest lock。
- **Files modified:** `tests/fixtures/contracts/runner-cli-matrix.json`, `tests/fixtures/contracts/phase1-plan-audit-lock.json`
- **Verification:** PackagingSelfTest Local、CLI/plan scan 和只读 source audit 均返回 0。
- **Committed in:** `5c028a7`

**3. [Rule 2 - Missing Critical] 最终 attested archive 与已验证 app 缺少生产 predicate 关联**

- **Found during:** 计划级 T-01-31 威胁面复核。
- **Issue:** 初始实现验证 build-directory app 后只计算最终 ZIP digest，无法证明上传/attest 的 archive 仍包含同一个签名 app。
- **Fix:** 新增 archive mismatch 负例；live predicate 将 ZIP 解包到私有临时目录，重新读取 bundle/executable、执行 codesign/stapler/spctl，并与 source app identity/digest 关联。
- **Files modified:** `scripts/contracts/run-macos.zsh`, `scripts/contracts/test-macos-package-verifier.zsh`, `tests/fixtures/contracts/packaging/macos-positive-control.json`, `scripts/contracts/test-macos-wave0-contract.ps1`
- **Verification:** 静态合同要求 `artifact_correlated`，archive mismatch 分支固定 exit 5；完整 PackagingSelfTest 返回 0。
- **Committed in:** `8ae9e2d`, `c8fda69`

**4. [Rule 1 - Bug] root lifecycle 文件所有权和半创建失败会阻断后续 job step 或遗留账户**

- **Found during:** Task 2 workflow integration review。
- **Issue:** `sudo` guard 创建的 0600 state/evidence 默认归 root，后续 runner step 无法读取；`createhomedir` 已完成但返回非零、state write 失败或账户创建半成功时也需要回滚。
- **Fix:** 验证 `SUDO_UID/SUDO_GID` 后把 state/stdout/stderr/evidence 归还调用者；账户创建各失败点统一删除账户/home和半写 state。
- **Files modified:** `scripts/contracts/assert-macos-job-lifecycle.zsh`, `scripts/contracts/test-macos-wave0-contract.ps1`
- **Verification:** Bash secondary parse、PowerShell source contract、workflow ordering 和 PackagingSelfTest 均通过。
- **Committed in:** `5c028a7`, `f93aee7`

**5. [Rule 2 - Missing Critical] 当前用户 Gatekeeper 与 path-smoke OS/架构事实未进入 strict predicate**

- **Found during:** Task 2 integration/security review。
- **Issue:** 仅验证 build app Gatekeeper 和 path smoke reopen，尚不足以证明 disposable profile 中安装副本被 Gatekeeper 接受且 marker 来自目标 macOS/Rust 架构。
- **Fix:** 在 disposable user 的 `HOME_APPLICATIONS` 安装副本上执行 Gatekeeper；验证两次 path smoke 的 `os=macos`、目标 Rust arch、schema/run ID/reopened，并把事实加入统一 predicate。
- **Files modified:** `.github/workflows/phase1-macos-evidence.yml`, `scripts/contracts/run-macos.zsh`, `tests/fixtures/contracts/packaging/macos-positive-control.json`, `scripts/contracts/test-macos-wave0-contract.ps1`
- **Verification:** evidence workflow 静态合同、YAML 解析、10 个 embedded zsh block secondary parse 与 PackagingSelfTest 均通过。
- **Committed in:** `f93aee7`

---

**Total deviations:** 5 auto-fixed（Rule 1：1，Rule 2：3，Rule 3：1）
**Impact on plan:** 全部修复都直接收紧 T-01-31/T-01-32/T-01-33 或保证 canonical verification 真正覆盖 01-14；没有新增产品功能、依赖、网络 endpoint 或持久化 schema。

## Issues Encountered

- 当前执行环境为 Windows，未安装原生 `zsh`、`actionlint` 或 `shellcheck`；严格按计划运行 PowerShell contract，并额外使用 Git Bash `bash -n`、PyYAML 和 embedded zsh block secondary parsing。没有把这些结果表述为真实 macOS runtime pass。
- 没有运行 `.github/workflows/phase1-macos-evidence.yml`，也没有消费 Apple/GitHub 认证材料；因此本计划没有生成或声称 Developer ID、公证、Gatekeeper、Intel/Apple Silicon 或远程 attestation 的真实 evidence。
- native evidence workflow 运行时仍要求目标 runner 能发现 exact Codex `0.146.1` official CLI 和 allowlisted `Codex.app`/`ChatGPT.app` bundled host；缺失时既有 host predicate 会 fail closed。

## Known Stubs

None。未发现 TODO、FIXME、placeholder、跳过测试或可使目标失效的硬编码空值。

## Authentication Gates

None。未尝试触发远程 workflow 或访问 Apple secrets，因此没有发生认证交互。

## User Setup Required

执行真实 macOS evidence workflow 前，需要在 GitHub Actions secrets 中配置：

- `APPLE_CERTIFICATE`：Developer ID `.p12` 的 Base64 内容。
- `APPLE_CERTIFICATE_PASSWORD`：证书密码。
- `APPLE_SIGNING_IDENTITY`：精确 Developer ID Application identity。
- `APPLE_API_ISSUER`、`APPLE_API_KEY`、`APPLE_API_KEY_ID`：App Store Connect API notarization 凭据。

目标 native runner 还必须提供 exact Codex `0.146.1` official CLI 和 allowlisted `Codex.app`/`ChatGPT.app` bundled host；本计划没有替用户配置、下载或验证这些外部前置条件。

## Next Phase Readiness

- 01-16 可以在满足 Apple secrets 与正式宿主前置条件后，分别获取 Intel/Apple Silicon 独立 attested freeze evidence。
- 01-17 可消费上传的 app/evidence bundle 和 provenance attestation，并交叉验证 strict-pass、archive digest 与 GitHub identity。
- 当前只完成 workflow 与本地 contract；真实 macOS evidence、Apple Developer ID、公证与 Gatekeeper 结果仍保持后续计划的外部门禁。
- `.planning/config.json` 的既有修改和 `.planning/research/.cache/*` 未跟踪缓存保持原样，未被本计划修改或提交。

## Self-Check: PASSED

- 5 个计划产物和 3 个必要 verification/dispatch 元数据文件均存在，三个 zsh 文件在 Git index 中为 `100755`。
- `9dcf0d3`、`1c8f6d9`、`5c028a7`、`8ae9e2d`、`c8fda69`、`f93aee7` 均存在于当前 `main` 历史。
- 计划指定的 macOS Wave 0/evidence contract 与 canonical PackagingSelfTest 均返回 0。
- 16 个 canonical matrix combinations、28 个 plans、23 个 runner invocations 和 Phase 1 只读 source audit 均通过。
- workflow YAML、PowerShell AST、JSON、Git Bash secondary parse、10 个 embedded zsh blocks、LF 与 stub scan 均通过。
- 当前 Windows 主机没有原生 zsh/macOS/Apple 凭据，因此没有记录或暗示任何真实 macOS remote pass。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 14*
*Completed: 2026-08-06*
