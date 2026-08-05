# Spike Manifest

## Idea

验证使用 Tauri 2 与 Rust 直接管理当前用户原生 Codex 配置的可行性，覆盖 Windows/macOS、统一 ChatGPT 桌面应用中的 Codex 与本机 Codex CLI、供应商配置与验证、原子修改和备份恢复、托盘与进程检测，以及当前用户范围的安装和显式确认更新。

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

## Spikes

| # | Name | Type | Validates | Verdict | Tags |
|---|------|------|-----------|---------|------|
| 001 | codex-native-config-contract | standard | Given Windows/macOS 当前用户可能同时使用桌面 Codex 与本机 Codex CLI，when 定位、读取并隔离测试原生配置，then 能确认默认路径、共享关系、供应商字段写法、凭据来源和重启生效边界 | PARTIAL | codex, config, windows, macos, desktop, cli |
| 002 | provider-validation-loop | standard | Given 标准供应商的服务地址、API Key 和默认模型，when 执行模型发现、Responses API 流式请求及工具调用闭环，then 能可重复地判定供应商是否满足 Codex 使用要求并提供脱敏诊断 | PARTIAL | responses-api, sse, tools, provider, validation |
| 003a | toml-structural-edit | comparison | Given 含未知字段、注释和不同换行的 Codex TOML，when 使用结构化 TOML 编辑执行供应商切换，then 能保留非受管配置并安全原子落盘、备份和恢复 | VALIDATED | rust, toml, atomic-write, backup, comparison |
| 003b | managed-block-edit | comparison | Given 含未知字段、注释、损坏或重复管理区块的 Codex TOML，when 只替换 GPTEasy 管理区块，then 能保留文件其余字节并在歧义时停止修改 | PARTIAL | rust, managed-block, atomic-write, backup, comparison |
| 004 | tauri-tray-process-restart | standard | Given Tauri 2 托盘程序运行且桌面 Codex 或 Codex CLI 可能持有旧配置，when 检测进程并切换供应商，then 能呈现立即重启、稍后重启、取消和明确退出语义且不误杀无关进程 | PARTIAL | tauri, tray, process, restart, windows, macos |
| 005 | desktop-install-update-matrix | standard | Given Windows/macOS x64/ARM64 目标，when 构建、安装和更新 Tauri 2 应用，then 能确认当前用户安装、权限、签名公证、更新包和显式确认更新的可交付方式 | PARTIAL | tauri, installer, updater, windows, macos |
