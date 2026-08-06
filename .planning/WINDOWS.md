---
schema_version: 1
open_count: 1
waived_count: 0
fixed_count: 7
total_count: 8
last_updated: 2026-08-06T06:14:58.640Z
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
  }
]
````
