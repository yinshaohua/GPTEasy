---
schema_version: 1
open_count: 1
waived_count: 0
fixed_count: 3
total_count: 4
last_updated: 2026-08-06T04:37:08.044Z
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
  }
]
````
