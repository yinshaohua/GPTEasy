# GPTEasy

## What This Is

GPTEasy 是统一 ChatGPT 桌面应用的伴侣程序，面向需要使用第三方 AI 服务、但不希望手工维护 Codex 配置的用户。它在 Windows 和 macOS 上管理已验证供应商，并安全切换当前用户的原生 Codex 环境；在 Windows 上还管理 WSL2 环境，并可导出能够脱离 GPTEasy 长期使用的 Linux 切换脚本。

项目以根目录 `CONTEXT.md`、`docs/adr/` 和 `docs/ui/UI-SPEC.md` 为已确认基线。这些文档中已经锁定的领域语言、产品行为、技术架构和界面决策不在项目规划阶段重新讨论。

## Core Value

非技术用户能够验证供应商，并在保留既有 Codex 配置且可恢复的前提下，可靠地切换各受管环境使用的 API 服务。

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] 用户可以配置标准供应商，并且只有完成默认模型确认、Responses API 流式响应和工具调用闭环验证的供应商才能保存和参与切换。
- [ ] 应用可以使用不可变供应商 ID 管理统一供应商目录，并正确处理内置推荐供应商 DayWay、供应商更新、删除和外部配置。
- [ ] 应用可以检测并管理当前用户的原生 Codex 环境，通过直接配置模式切换代理 API 模式或原厂登录模式，同时保留非 GPTEasy 管理的配置。
- [ ] 所有正式配置写入都具备备份、原子替换、失败恢复和待重启处理，不能因部分写入、迁移失败或进程仍在运行而静默破坏用户环境。
- [ ] Windows 用户可以分别管理各 WSL2 环境默认用户的当前供应商，并安全执行单个或批量切换。
- [ ] 用户可以导出包含全部已验证供应商的 Bash 4+ 或 Zsh 5+ 独立切换脚本，并在明确的 GPTEasy 管理区块内安全修改 Linux 用户级 Codex 配置。
- [ ] 托盘、设置窗口、首次使用、验证反馈、重启提示和空状态遵循 `docs/ui/UI-SPEC.md`，使非技术用户无需理解底层配置即可完成核心任务。
- [ ] 应用内部状态通过带永久顺序迁移的 SQLite 数据库保存，历史数据库可以跨版本升级，迁移失败不会通过清空用户数据继续运行。
- [ ] 应用提供本地脱敏诊断日志、用户主动触发的诊断导出、每日最多一次的更新检查，以及受用户控制的安装与更新流程。
- [ ] 首版支持 Windows 10 22H2+ 的 x64/ARM64、macOS 14+ 的 Intel/Apple Silicon，并提供简体中文和英语界面。
- [ ] 完整首版范围作为一个整体发布；内部阶段可以逐步完成，但不把功能不完整的中间阶段作为正式首版交付。

### Out of Scope

- 产品账户、云端存储、云同步和自动跨设备迁移 — 首版采用完全本地模式。
- Linux 独立 GUI 应用 — Linux 首版只提供导出的 shell function。
- 本地 API 代理网关 — 采用直接修改 Codex 配置的模式。
- 机器级公共配置、自定义配置路径和其他操作系统用户的配置 — 首版只管理当前用户的默认配置。
- Fish、Nushell、仅 POSIX sh 或依赖 Python、Node.js 的 Linux 导出物 — 首版仅支持无额外运行时依赖的 Bash 4+ 与 Zsh 5+。
- 自定义 Header、组织或项目标识等高级供应商认证字段 — 首版只支持服务地址、API Key 和默认模型。
- 对旧版宿主应用的供应商管理 — 首版只检测其存在并提示迁移。
- 自动终止仍在使用旧配置的 Codex 进程 — 应用提示并允许用户决定立即或稍后重启。
- 持续监控供应商健康状态 — 验证时间只表示最近一次完整验证成功的时间。

## Context

- 当前仓库处于项目规划和设计完成、尚无产品代码的 greenfield 状态。
- `CONTEXT.md` 定义领域词汇、支持平台、受管环境、供应商生命周期、配置安全边界和首版产品行为。
- `docs/adr/0001-0008` 已锁定明文凭据、Tauri 2 + Rust + TypeScript/React、独立 Linux function、直接配置模式、不可变供应商 ID、SQLite 状态、本地模式和原生 Codex 环境模型。
- `docs/ui/UI-SPEC.md` 已锁定托盘优先的桌面工具体验、窗口结构、供应商页面、验证反馈、重启提示、WSL2 页面、Linux 脚本导出、主题与无障碍要求。
- 首版采用一次完整交付策略。实施阶段按依赖与风险推进，但最终发布必须覆盖已锁定的完整首版范围。
- 实施优先级依次为：数据和配置安全基础、供应商与验证闭环、原生环境切换、托盘与设置界面、WSL2、Linux 脚本、运行维护能力、跨平台发布验收。

## Constraints

- **领域基线**: 实现和规划必须使用 `CONTEXT.md` 中定义的术语，不重新解释或替换已锁定领域决策。
- **架构基线**: 使用 Tauri 2、Rust 和 TypeScript/React；系统及领域能力由 Rust 实现，React 只通过受控 Tauri command 访问后端。
- **数据持久化**: 供应商、明文 API Key、验证状态、环境选择和设置保存在 Rust 后端独占访问的用户级 SQLite 数据库。
- **数据库升级**: 使用永久保留的顺序迁移；迁移前备份、事务内执行、失败回滚，并验证历史数据库到当前版本的升级路径。
- **凭据策略**: API Key 可明文保存、完整显示并写入 WSL2 和 Linux 脚本，但日志、错误、截图辅助和诊断导出必须脱敏。
- **配置修改**: 只修改 GPTEasy 管理的供应商字段或明确管理区块；写入前备份，使用临时文件和原子替换，发现歧义时停止。
- **网络安全**: 远程供应商必须使用 HTTPS；只有 `localhost`、`127.0.0.1` 和 `[::1]` 允许 HTTP，且不提供绕过开关。
- **本地模式**: 不建设产品账户、云端存储或供应商同步服务，不默认上传供应商数据或诊断信息。
- **平台支持**: Windows 10 22H2+ x64/ARM64、macOS 14+ Intel/Apple Silicon；WSL2 管理仅适用于 Windows。
- **Linux 导出**: 支持 Bash 4+ 和 Zsh 5+，只依赖受支持 Shell 与常见基础命令。
- **语言与无障碍**: 首版提供简体中文和英语；核心功能支持键盘、屏幕阅读器、系统缩放、高对比度和减少动态效果。
- **发布策略**: 不发布范围不完整的正式首版；所有锁定能力完成并通过 Windows/macOS 验收后统一交付。

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| 现有领域、ADR 和 UI 文档作为锁定基线 | 避免在实施规划中重复讨论已经确认的产品与架构决策 | — Pending |
| 使用 Tauri 2、Rust 和 TypeScript/React | 平衡原生系统集成、跨平台能力和 UI 开发效率 | — Pending |
| 采用直接配置模式而非本地 API 代理网关 | 避免 GPTEasy 成为持续请求依赖，并支持独立 Linux function | — Pending |
| 使用 SQLite 和不可变供应商 ID 管理本地状态 | 保持供应商身份与跨环境关联稳定，并支持事务与迁移 | — Pending |
| 允许明文保存及导出供应商凭据 | 对非技术用户优先保证可理解、可复制和可独立使用 | — Pending |
| 首版完全本地运行 | 避免凭据云存储和服务端运营负担 | — Pending |
| 完整首版一次交付 | 用户明确选择所有锁定能力完成后统一发布 | — Pending |
| 内部采用风险与依赖驱动的阶段顺序 | 先建立数据和配置安全基础，再逐步扩展环境与发布能力 | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `$gsd-transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `$gsd-complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-08-05 after initialization*
