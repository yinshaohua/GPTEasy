---
name: spike-findings-gpteasy
description: GPTEasy Spike 实验形成的实现蓝图，包含不可妥协的需求、已验证模式、限制和陷阱。实现 GPTEasy 功能时自动加载。
---

<context>
## Project: GPTEasy

GPTEasy 使用 Tauri 2 与 Rust 管理当前用户的原生 Codex 配置，统一覆盖 ChatGPT 桌面应用中的 Codex 与本机 Codex CLI，并提供供应商验证、安全配置写入、托盘与进程生命周期管理，以及 Windows/macOS 当前用户范围的安装和更新。

Spike sessions wrapped: 2026-08-05
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
</requirements>

<findings_index>
## Feature Areas

| Area | Reference | Key Finding |
|------|-----------|-------------|
| Codex 与供应商兼容 | `references/codex-provider-compatibility.md` | 原生配置契约可直接驱动 Responses provider，但保存前必须完成带 nonce 的两轮工具闭环验证。 |
| 安全配置写入 | `references/safe-config-editing.md` | 首次接管使用结构化迁移，之后使用 dotted-key 管理区块；全程配合备份、并发检查和平台原子替换。 |
| 桌面运行生命周期 | `references/desktop-runtime-lifecycle.md` | 进程分类必须结合路径和父子关系；桌面进程可自动重启，CLI 只能提示人工重启。 |
| 安装与更新 | `references/install-and-update.md` | Windows NSIS 当前用户安装已验证；更新检查与安装必须分离，macOS 严格用户级安装仍需真实机器验证。 |

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
</metadata>
