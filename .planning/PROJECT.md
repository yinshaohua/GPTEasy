# GPTEasy

## What This Is

GPTEasy 是面向 Windows 个人用户的本地桌面工具。用户提供供应商名称、服务地址、API Key 和默认模型，GPTEasy 完成真实兼容性验证，并在不要求用户手工编辑 Codex 配置的前提下安全切换当前用户 Codex 环境。

当前仓库是重建后的 greenfield 项目：旧产品源码已经删除，旧 GSD 阶段和研究已归档。旧 Spike 的通过结果只能提供实现证据，不能继承旧需求、执行计划、完成状态或锁定声明。

## Core Value

用户可以验证并切换供应商，同时确信验证失败、配置冲突、应用崩溃或运行中的 Codex 不会静默破坏其当前环境。

## Current Milestone

**v0.1 Windows x64 可验收闭环**

- Windows 10 22H2 或更高版本，x64，当前用户安装。
- Tauri 2 + Rust + TypeScript/React；Rust 是文件、SQLite、网络和进程的唯一权威。
- 已验证供应商目录、SQLite 明文 API Key、供应商模式和 OpenAI 登录模式。
- 当前用户默认 Codex 环境的明确接管、切换、备份、恢复、冲突和待重启处理。
- 只有“供应商”和“Codex 环境”两页的简体中文设置窗口，以及托盘快捷切换。
- 自动化故障矩阵和真实供应商、真实 Codex CLI、桌面 Codex UAT。
- 可验收构建可以未签名；正式对外发布必须 Authenticode。

## Out of Scope

- Windows ARM64、macOS、WSL2 管理和 Linux 切换脚本。
- DayWay 或其他内置推荐供应商。
- 自动更新、应用内更新检查、登录启动和 updater。
- 持久诊断日志、诊断导出、遥测和崩溃上传。
- 产品账户、云同步、跨设备迁移和供应商云存储。
- 搜索、分页、供应商数量上限、多语言和手动主题选择。
- 本地 API 代理网关、环境变量切换、机器级配置、其他用户或自定义 Codex 路径。
- 自动关闭、终止、重启或恢复桌面 Codex 与 Codex CLI。
- 为未来能力预建数据库表、配置字段或 UI 入口。

## Evidence Policy

- `CONTEXT.md` 定义领域语言。
- `docs/adr/README.md` 与当前 ADR 定义难以逆转的决策。
- `docs/ui/UI-SPEC.md` 定义首个可验收版本的界面合同。
- `.planning/spikes/` 与 `.codex/skills/spike-findings-gpteasy/` 保存已验证实现证据；实现时仍须验证目标 Codex 当前版本。
- `.planning/archive/pre-rebuild-2026-08-07/` 只保存旧研究和执行记录，不参与需求覆盖或进度计算。

## Constraints

- 供应商验证与环境切换严格分离；未验证输入不持久化。
- API Key 明文保存在当前用户 SQLite 中，但不得进入日志、普通错误、通知、诊断资料或非必要进程参数。
- 配置与凭据载体是环境实际状态；SQLite 保存供应商目录、最后应用证据和恢复意图，不能单方面覆盖磁盘。
- 对 Codex 的每次写入都必须先读取、验证、备份、复核指纹、原子替换并复读。
- 用户明确操作之前不创建缺失的 Codex 配置，也不接管外部配置。
- 所有普通文本使用 Unix 换行；Git 在主干管理，提交说明使用中文。

## Key Decisions

| Decision | Rationale | Reference |
|----------|-----------|-----------|
| 证据继承、决策重审 | 保留验证价值，丢弃旧范围和完成幻觉 | ADR-0009 |
| Windows x64 垂直切片 | 先证明核心用户价值 | ADR-0010 |
| 管理当前用户 Codex 环境 | 桌面与 CLI 是消费者，不是所有者 | ADR-0011 |
| 验证与切换分离 | 网络验证不能产生隐藏配置副作用 | ADR-0012 |
| API Key 明文保存在 SQLite | 减少凭据存储协调和用户重复输入 | ADR-0013 |
| 不保存草稿或供应商历史 | 控制状态数量并保留清晰不变量 | ADR-0015、0016 |
| 外部配置明确接管 | 避免自动匹配和静默覆盖 | ADR-0017、0019 |
| 只恢复最近配置 | 提供可理解的撤销而非备份管理器 | ADR-0018 |
| 不自动重启 Codex | 不打断用户任务或误杀进程 | ADR-0020 |
| 单个未完成操作 | 在多工件间获得最小崩溃恢复能力 | ADR-0023 |
| 数据库异常停止写入 | 不用空库掩盖凭据或状态丢失 | ADR-0024 |
