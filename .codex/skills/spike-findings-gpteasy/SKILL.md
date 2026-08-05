---
name: spike-findings-gpteasy
description: GPTEasy Spike 实验形成的实现蓝图，包含不可妥协的需求、已验证模式、限制和陷阱。实现 GPTEasy 功能时自动加载。
---

<context>
## Project: GPTEasy

GPTEasy 使用 Tauri 2 与 Rust 管理当前用户的原生 Codex、WSL2 与独立 Linux 环境，统一覆盖 ChatGPT 桌面应用中的 Codex 和本机 Codex CLI，并提供供应商验证、安全配置写入、跨资源切换恢复、外部配置协调、托盘进程生命周期，以及 Windows/macOS 当前用户范围的安装和更新。

Spike sessions wrapped: 2026-08-05（001–013、017）
</context>

<requirements>
## Requirements

- 使用 Tauri 2 与 Rust 实现桌面应用。
- 首版支持 Windows 10 22H2 或更高版本的 x64/ARM64，以及 macOS 14 或更高版本的 Intel/Apple Silicon。
- 直接修改当前用户的 Codex 配置，不在请求链路中运行本地代理。
- 原生 Codex 环境必须同时考虑统一 ChatGPT 桌面应用中的 Codex 与本机 Codex CLI。
- 供应商配置至少包含服务地址、API Key 和默认模型。
- 供应商保存前必须验证模型发现、Responses API 流式响应和工具调用闭环。
- 配置修改必须保留非 GPTEasy 字段，采用原子写入，并在修改前创建带时间戳的备份。
- 每个受管环境默认只保留最近五份配置备份，并支持失败恢复。
- 首次接管已有 Codex 配置时使用结构化 TOML 迁移；管理区块建立后，后续切换只替换 dotted-key 管理区块。
- 检测到相关 Codex 进程时不得静默强制终止；用户切换前可选择立即重启、稍后重启或取消。
- “立即重启”只自动重启桌面宿主进程树；本机 Codex CLI 必须提示用户在原终端退出并重新运行。
- 应用默认托盘驻留，只有托盘中的明确退出操作才结束程序。
- 只提供当前用户安装；更新必须由用户确认，不进行静默安装。
- macOS 严格当前用户安装以 `~/Applications/GPTEasy.app` 为目标，默认指向 `/Applications` 的 DMG 不能作为唯一正式安装路径。
- 远程供应商必须使用 HTTPS；仅回环地址允许 HTTP。
- 供应商使用不可变 ID；地址、凭据或默认模型变化时必须验证后替换，失败保留旧配置。
- 供应商验证结果必须以不可逆组合指纹绑定服务地址、API Key 和默认模型；只有同一已验证组合可以进入保存或切换 Saga。
- SQLite 与 Codex 配置文件之间的跨资源切换必须能在失败或崩溃后恢复到一致状态。
- WSL2 检测不得启动发行版；用户明确切换已停止发行版时才临时启动，并在处理结束后恢复原停止状态。
- WSL2 首版只管理发行版默认用户的 Codex 配置，不主动终止其中运行的 Codex。
- Linux 导出物分别支持 Bash 4+ 与 Zsh 5+，不依赖 Python、Node.js、第三方解析器或 GPTEasy 可执行文件。
- Linux 切换脚本 source 时不得修改配置；只有用户调用交互式 function 并选择供应商后才写入。
- 真实供应商凭据只从 Git 忽略的 `.planning/spikes/.secrets/provider.json` 读取，不进入命令行、日志、诊断或 Git。
</requirements>

<findings_index>
## Feature Areas

| Area | Reference | Key Finding |
|------|-----------|-------------|
| Codex 与供应商兼容 | `references/codex-provider-compatibility.md` | 原生配置可直接驱动 Responses provider；保存前必须完成真实可观测的模型发现、SSE、strict function call 与 nonce 回传闭环。 |
| 安全配置写入 | `references/safe-config-editing.md` | 首次接管已验证为单事务结构化迁移并建立 dotted-key 区块；后续只替换区块并配合备份、并发检查和平台原子替换。 |
| 切换一致性与外部协调 | `references/switch-consistency-reconciliation.md` | SQLite 与配置文件通过持久化 Saga、旧/新哈希和不可变供应商 ID 收敛；覆盖层或外部修改只展示，不自动争夺。 |
| 桌面运行生命周期 | `references/desktop-runtime-lifecycle.md` | 进程分类必须结合路径和父子关系；桌面进程可自动重启，CLI 只能提示人工重启。 |
| 桌面供应商切换端到端 | `references/desktop-provider-switch-e2e.md` | 验证、组合指纹、首次接管、Saga、app-server 协调和重启计划必须由单个 Rust 后端流程串成不可绕过的链路。 |
| WSL2 与 Linux 导出物 | `references/wsl-linux-environments.md` | 检测 WSL2 不得启动发行版；受管切换以 Rust 内存渲染加 stdin guest writer 完成，Bash/Zsh 导出函数则保持 source 零写入。 |
| 安装与更新 | `references/install-and-update.md` | Windows NSIS 当前用户安装已验证；更新检查与安装必须分离，macOS 严格用户级安装仍需真实机器验证。 |
| macOS 真实宿主契约 | `references/macos-host-contract.md` | `~/Applications`、托盘、LaunchServices、签名公证和两版本 updater 必须在原生 CI 与真实 Mac 分层验证，非 macOS 结果不能冒充宿主证据。 |

## Source Files

原始 Spike 的 README、核心源码、脚本和配置保存在 `sources/`，可用于完整追溯和移植。
</findings_index>

<metadata>
## Processed Spikes

- 001-codex-native-config-contract
- 002-provider-validation-loop
- 003-a-toml-structural-edit
- 003-b-managed-block-edit
- 004-tauri-tray-process-restart
- 005-desktop-install-update-matrix
- 006-first-takeover-managed-block-transaction
- 007-provider-switch-saga
- 008-external-config-reconciliation
- 009-wsl2-environment-lifecycle
- 010-a-linux-switch-functions-bash
- 010-b-linux-switch-functions-zsh
- 011-real-provider-compatibility-matrix
- 012-desktop-provider-switch-e2e
- 013-wsl2-host-guest-switch-transaction
- 017-macos-real-host-contract
</metadata>
