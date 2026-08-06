---
phase: 01-trusted-local-state-contract
plan: 08
subsystem: ui
tags: [react, typescript, vite, tauri, npm, reproducible-build]

# Dependency graph
requires:
  - phase: 01-trusted-local-state-contract
    provides: "01-02 的 npm 官方来源 approved checkpoint 与 7 项 exact package allowlist"
provides:
  - "精确锁定的 React/Vite/TypeScript/Tauri 前端依赖与 package-lock"
  - "可执行 tsc --noEmit 与 vite build 的 Tauri/Vite 配置"
  - "不持有权威状态的单一 React root mount"
affects: [phase-1-state-tracer, tauri-bootstrap, frontend-builds]

# Tech tracking
tech-stack:
  added:
    - "@tauri-apps/api 2.11.1"
    - "@tauri-apps/cli 2.11.4"
    - "React 19.1.0 / react-dom 19.1.0"
    - "Vite 8.0.16 / @vitejs/plugin-react 6.0.2"
    - "TypeScript 6.0.3"
  patterns:
    - "所有直接 npm 依赖使用 exact version，不使用 caret、tilde 或 latest"
    - "React 仅提供单一 mount 与构建标识，不保存状态、不调用网络、不访问浏览器持久化"
    - "Vite 8 使用内置 Oxc minifier，避免额外引入未批准的 esbuild 依赖"

key-files:
  created:
    - package.json
    - package-lock.json
    - vite.config.ts
    - tsconfig.json
    - index.html
    - src/main.tsx
    - src/App.tsx
    - src/global.css
  modified: []

key-decisions:
  - "依赖版本严格采用研究与 01-02 approved checkpoint 指定的 exact pins，并将 Tauri API/CLI 固定为 2.11.1/2.11.4。"
  - "Vite 8 的生产压缩改用内置 Oxc；不为 legacy esbuild transform 额外安装未在计划 allowlist 中的包。"
  - "App 只渲染 phase-01-react-root 构建标识，权威状态仍由后续 Rust/Tauri 后端提供。"

patterns-established:
  - "前端壳只负责静态入口，后续阶段通过受控 Tauri command 连接后端，不建立第二状态源。"

requirements-completed: [STATE-01, STATE-02]

coverage:
  - id: D1
    description: "package.json 与 package-lock.json 形成 exact、可重复的 React/Vite/TypeScript/Tauri 依赖图"
    requirement: STATE-02
    verification:
      - kind: integration
        ref: "powershell -NoProfile -File scripts/contracts/verify-npm-package-allowlist.ps1 -Allowlist tests/fixtures/contracts/npm-package-allowlist.json"
        status: pass
      - kind: integration
        ref: "npm ci --ignore-scripts --no-audit --no-fund --registry=https://registry.npmjs.org/"
        status: pass
    human_judgment: false
  - id: D2
    description: "最小 React root 可以完成严格 TypeScript 检查与生产构建，且不引入业务状态或网络范围"
    requirement: STATE-01
    verification:
      - kind: integration
        ref: "npm run build"
        status: pass
    human_judgment: false

# Metrics
duration: 约 15 分钟
completed: 2026-08-06
status: complete
---

# Phase 1 Plan 8: React/Vite 前端壳 Summary

**以 exact npm 依赖和可重复 lockfile 建立 Tauri/Vite React 入口，并将 React 限制为无业务状态的静态 root**

## Performance

- **Duration:** 约 15 分钟
- **Started:** 2026-08-06T03:40:20Z
- **Completed:** 2026-08-06T03:54:09Z
- **Tasks:** 2/2
- **Files modified:** 8 个生产文件

## Accomplishments

- 创建 `package.json` 与 `package-lock.json`，精确锁定已批准的 React、Vite、TypeScript 和 Tauri API/CLI 版本。
- 添加严格 TypeScript 配置、Tauri/Vite 开发服务器约定，以及 `tsc --noEmit && vite build` 构建脚本。
- 创建单一 `root` mount、严格模式 bootstrap 和仅显示内部构建标识的无业务 React root。
- 通过 npm allowlist verifier、干净 `npm ci` 与 `npm run build`。

## Task Commits

Each task was committed atomically:

1. **Task 1: 安装批准后的 exact npm 工具链** - `c6ba7d5` (`chore`)
2. **Task 1 自动修复：适配 Vite 8 压缩器** - `e679e00` (`fix`)
3. **Task 2: 建立无业务范围的 React root** - `6983646` (`feat`)

**Plan metadata:** final metadata commit（包含 SUMMARY、STATE、ROADMAP 与 broken-windows ledger）。

## Files Created/Modified

- `package.json` - private package、exact dependencies 和构建脚本。
- `package-lock.json` - lockfileVersion 3 的可重复 npm 依赖图。
- `vite.config.ts` - React plugin、Tauri dev host/HMR、环境变量前缀和生产构建设置。
- `tsconfig.json` - strict、Bundler module resolution 和 JSX 配置。
- `index.html` - 单一 `root` mount 入口。
- `src/main.tsx` - React StrictMode bootstrap，并在 mount 缺失时 fail fast。
- `src/App.tsx` - 无业务状态的可访问应用容器与内部构建标识。
- `src/global.css` - 仅提供 mount 所需的基础布局、颜色和深色模式样式。

## Decisions Made

- 保持直接依赖 exact pin，禁止 caret、tilde 和 latest；npm 安装始终固定到公开 registry verifier 通过的图谱。
- 不安装账户、遥测、云同步或高权限 Tauri plugin；前端不暴露数据库、文件系统或网络入口。
- 采用 Vite 8 内置 Oxc minifier，避免把 legacy esbuild transform 作为新的直接依赖带入 allowlist。

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Vite 8 legacy esbuild minifier 阻断生产构建**

- **Found during:** Task 2（建立无业务范围的 React root）的 `npm run build` 验证。
- **Issue:** 初始 Tauri/Vite 配置使用 `minify: "esbuild"`；Vite `8.0.16` 会调用已弃用且需要单独安装的 `esbuild`，导致精确 allowlist 下构建失败。
- **Fix:** 将 `vite.config.ts` 的生产压缩器改为 Vite 8 内置的 `oxc`，不新增计划外 npm 包。
- **Files modified:** `vite.config.ts`
- **Verification:** 修复后 `npm run build` 返回 0；随后完整重跑 verifier、`npm ci` 和 `npm run build` 均返回 0。
- **Committed in:** `e679e00`

---

**Total deviations:** 1 auto-fixed（Rule 3：1）
**Impact on plan:** 仅修正 Vite 8 配置兼容性，没有扩展依赖、权限、产品范围或状态边界；计划成功标准全部满足。

## Issues Encountered

- 首次构建暴露 Vite 8 的 legacy esbuild 配置不兼容，已按上面的 Rule 3 自动修复。
- `npm ci` 生成的 `node_modules` 与 `vite build` 生成的 `dist` 已在验证后清理，未进入提交。

## Known Stubs

None。React root 没有空数据源、占位业务组件或后续产品功能 stub；其无业务范围是本计划的明确目标。

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- 前端入口、精确 npm 依赖和生产构建命令已可被后续 Tauri bootstrap 与状态 tracer 消费。
- React 不持有权威状态；后续实现应继续通过受控 Tauri command 连接 Rust `StateStore`。
- 01-02 的 npm 官方来源批准只覆盖既定 exact allowlist；任何依赖名称、版本或来源变更都必须重新进入人工门禁。

## Self-Check: PASSED

- 8 个计划生产文件均已创建并存在。
- 任务提交 `c6ba7d5`、修复提交 `e679e00`、任务提交 `6983646` 均存在于当前 main 历史。
- 最终 verifier、`npm ci` 和 `npm run build` 均返回 0。
- 工作树中的 `.planning/config.json` 修改与 `.planning/research/.cache/*` 未跟踪文件保持原样，未被本计划提交。

---
*Phase: 01-trusted-local-state-contract*
*Plan: 08*
*Completed: 2026-08-06*
