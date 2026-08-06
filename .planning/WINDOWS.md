---
schema_version: 1
open_count: 1
waived_count: 0
fixed_count: 0
total_count: 1
last_updated: 2026-08-06T03:56:51.191Z
---

# Broken Windows Ledger

> Cross-phase defect register. `/gsd-ship` blocks while `open_count > 0`.
> Waive with `gsd-tools windows waive <id> "<reason>"` (reason required).
> Mark fixed with `gsd-tools windows fixed <id>`.

| id | phase | kind | file | line | description | status | reason | recorded_at | resolved_at |
|----|-------|------|------|------|-------------|--------|--------|-------------|-------------|
| 1 | 01 | deviation | vite.config.ts |  | Vite 8 使用内置 Oxc minifier，避免 legacy esbuild 额外依赖并恢复 npm run build | open |  | 2026-08-06T03:56:51.191Z |  |

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
  }
]
````
