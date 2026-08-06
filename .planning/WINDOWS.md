---
schema_version: 1
open_count: 1
waived_count: 0
fixed_count: 14
total_count: 15
last_updated: 2026-08-06T08:41:56.928Z
---

# Broken Windows Ledger

> Cross-phase defect register. `/gsd-ship` blocks while `open_count > 0`.
> Waive with `gsd-tools windows waive <id> "<reason>"` (reason required).
> Mark fixed with `gsd-tools windows fixed <id>`.

| id | phase | kind | file | line | description | status | reason | recorded_at | resolved_at |
|----|-------|------|------|------|-------------|--------|--------|-------------|-------------|
| 1 | 01 | deviation | vite.config.ts |  | Vite 8 使用内置 Oxc minifier，避免 legacy esbuild 额外依赖并恢复 npm run build | open |  | 2026-08-06T03:56:51.191Z |  |
| 2 | 01 | deviation | scripts/contracts/test-windows-contract-probes.ps1 | 350 | 修正 Windows PowerShell 静态协议断言的属性语法 | fixed |  | 2026-08-06T04:36:54.042Z | 2026-08-06T04:37:06.000Z |
| 3 | 01 | deviation | scripts/contracts/probe-wsl2.ps1 | 374 | 修正 StrictMode 下单元素 WSL 输出的 Count 访问 | fixed |  | 2026-08-06T04:36:54.450Z | 2026-08-06T04:37:06.960Z |
| 4 | 01 | deviation | scripts/contracts/probe-codex.ps1 | 187 | 修正 schema 输出目录的 Windows 参数引用 | fixed |  | 2026-08-06T04:36:54.858Z | 2026-08-06T04:37:08.044Z |
| 5 | 01 | deviation | scripts/contracts/test-macos-wave0-contract.ps1 | 174 | 修正 RED 静态合同对 shell 转义和 YAML matrix 列表的误判 | fixed |  | 2026-08-06T05:04:40.045Z | 2026-08-06T05:04:59.302Z |
| 6 | 01 | deviation | scripts/contracts/probe-codex-macos.zsh | 629 | 修正 probe 内部摘要分隔符和 host child 退出码传播 | fixed |  | 2026-08-06T05:04:42.372Z | 2026-08-06T05:05:00.497Z |
| 7 | 01 | deviation | scripts/contracts/probe-codex-macos.zsh | 491 | 补齐 raw response 解析失败和 canary 所有权清理的 fail-closed 路径 | fixed |  | 2026-08-06T05:04:44.398Z | 2026-08-06T05:05:01.738Z |
| 8 | 01 | deviation | src-tauri/icons/icon.ico |  | Tauri Windows Resource 编译需要计划外补充确定性 icon.ico，已在 baed5d0 修复 | fixed |  | 2026-08-06T06:13:20.973Z | 2026-08-06T06:14:58.640Z |
| 9 | 01 | deviation | src-tauri/Cargo.toml |  | Tauri mock 测试需要 dev test feature 与 Windows test resource linking | fixed |  | 2026-08-06T07:07:57.218Z | 2026-08-06T07:09:20.264Z |
| 10 | 01 | deviation | src-tauri/src/path_smoke.rs |  | 公开 path smoke 入口在解析状态根前重复验证 opaque ID | fixed |  | 2026-08-06T07:07:58.156Z | 2026-08-06T07:09:21.079Z |
| 11 | 01 | deviation | scripts/contracts/test-macos-wave0-contract.ps1 |  | 扩展 Wave 0 静态合同以支持独立 package 脚本集并匹配计划级验证入口 | fixed |  | 2026-08-06T08:40:36.864Z | 2026-08-06T08:41:27.977Z |
| 12 | 01 | deviation | tests/fixtures/contracts/runner-cli-matrix.json |  | canonical PackagingSelfTest 接入完整 macOS package/evidence contract 并刷新只读 digest lock | fixed |  | 2026-08-06T08:41:33.031Z | 2026-08-06T08:41:36.597Z |
| 13 | 01 | deviation | scripts/contracts/run-macos.zsh |  | 解包最终 attested archive 并与签名 app 建立 fail-closed 关联 | fixed |  | 2026-08-06T08:41:39.654Z | 2026-08-06T08:41:42.914Z |
| 14 | 01 | deviation | scripts/contracts/assert-macos-job-lifecycle.zsh |  | 补齐 runner 身份、调用者所有权、半创建账户回滚与 finalized cleanup 硬化 | fixed |  | 2026-08-06T08:41:46.893Z | 2026-08-06T08:41:49.935Z |
| 15 | 01 | deviation | .github/workflows/phase1-macos-evidence.yml |  | 将当前用户 Gatekeeper 与 path smoke OS/原生架构事实纳入 strict predicate | fixed |  | 2026-08-06T08:41:53.671Z | 2026-08-06T08:41:56.928Z |

````json
[
  {
    "id": 1,
    "kind": "deviation",
    "phase": "01",
    "file": "vite.config.ts",
    "line": null,
    "description": "Vite 8 使用内置 Oxc minifier，避免 legacy esbuild 额外依赖并恢复 npm run build",
    "status": "open",
    "reason": "",
    "recorded_at": "2026-08-06T03:56:51.191Z",
    "resolved_at": null
  },
  {
    "id": 2,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/test-windows-contract-probes.ps1",
    "line": 350,
    "description": "修正 Windows PowerShell 静态协议断言的属性语法",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T04:36:54.042Z",
    "resolved_at": "2026-08-06T04:37:06.000Z"
  },
  {
    "id": 3,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/probe-wsl2.ps1",
    "line": 374,
    "description": "修正 StrictMode 下单元素 WSL 输出的 Count 访问",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T04:36:54.450Z",
    "resolved_at": "2026-08-06T04:37:06.960Z"
  },
  {
    "id": 4,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/probe-codex.ps1",
    "line": 187,
    "description": "修正 schema 输出目录的 Windows 参数引用",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T04:36:54.858Z",
    "resolved_at": "2026-08-06T04:37:08.044Z"
  },
  {
    "id": 5,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/test-macos-wave0-contract.ps1",
    "line": 174,
    "description": "修正 RED 静态合同对 shell 转义和 YAML matrix 列表的误判",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T05:04:40.045Z",
    "resolved_at": "2026-08-06T05:04:59.302Z"
  },
  {
    "id": 6,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/probe-codex-macos.zsh",
    "line": 629,
    "description": "修正 probe 内部摘要分隔符和 host child 退出码传播",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T05:04:42.372Z",
    "resolved_at": "2026-08-06T05:05:00.497Z"
  },
  {
    "id": 7,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/probe-codex-macos.zsh",
    "line": 491,
    "description": "补齐 raw response 解析失败和 canary 所有权清理的 fail-closed 路径",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T05:04:44.398Z",
    "resolved_at": "2026-08-06T05:05:01.738Z"
  },
  {
    "id": 8,
    "kind": "deviation",
    "phase": "01",
    "file": "src-tauri/icons/icon.ico",
    "line": null,
    "description": "Tauri Windows Resource 编译需要计划外补充确定性 icon.ico，已在 baed5d0 修复",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T06:13:20.973Z",
    "resolved_at": "2026-08-06T06:14:58.640Z"
  },
  {
    "id": 9,
    "kind": "deviation",
    "phase": "01",
    "file": "src-tauri/Cargo.toml",
    "line": null,
    "description": "Tauri mock 测试需要 dev test feature 与 Windows test resource linking",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T07:07:57.218Z",
    "resolved_at": "2026-08-06T07:09:20.264Z"
  },
  {
    "id": 10,
    "kind": "deviation",
    "phase": "01",
    "file": "src-tauri/src/path_smoke.rs",
    "line": null,
    "description": "公开 path smoke 入口在解析状态根前重复验证 opaque ID",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T07:07:58.156Z",
    "resolved_at": "2026-08-06T07:09:21.079Z"
  },
  {
    "id": 11,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/test-macos-wave0-contract.ps1",
    "line": null,
    "description": "扩展 Wave 0 静态合同以支持独立 package 脚本集并匹配计划级验证入口",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T08:40:36.864Z",
    "resolved_at": "2026-08-06T08:41:27.977Z"
  },
  {
    "id": 12,
    "kind": "deviation",
    "phase": "01",
    "file": "tests/fixtures/contracts/runner-cli-matrix.json",
    "line": null,
    "description": "canonical PackagingSelfTest 接入完整 macOS package/evidence contract 并刷新只读 digest lock",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T08:41:33.031Z",
    "resolved_at": "2026-08-06T08:41:36.597Z"
  },
  {
    "id": 13,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/run-macos.zsh",
    "line": null,
    "description": "解包最终 attested archive 并与签名 app 建立 fail-closed 关联",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T08:41:39.654Z",
    "resolved_at": "2026-08-06T08:41:42.914Z"
  },
  {
    "id": 14,
    "kind": "deviation",
    "phase": "01",
    "file": "scripts/contracts/assert-macos-job-lifecycle.zsh",
    "line": null,
    "description": "补齐 runner 身份、调用者所有权、半创建账户回滚与 finalized cleanup 硬化",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T08:41:46.893Z",
    "resolved_at": "2026-08-06T08:41:49.935Z"
  },
  {
    "id": 15,
    "kind": "deviation",
    "phase": "01",
    "file": ".github/workflows/phase1-macos-evidence.yml",
    "line": null,
    "description": "将当前用户 Gatekeeper 与 path smoke OS/原生架构事实纳入 strict predicate",
    "status": "fixed",
    "reason": "",
    "recorded_at": "2026-08-06T08:41:53.671Z",
    "resolved_at": "2026-08-06T08:41:56.928Z"
  }
]
````
