# Spike Manifest

## Idea

验证使用 Tauri 2 与 Rust 管理 GPTEasy 当前用户环境的可行性，覆盖 Windows/macOS 原生 Codex、统一 ChatGPT 桌面应用中的 Codex 与本机 Codex CLI、供应商配置与验证、原子修改和备份恢复、托盘与进程检测、当前用户范围的安装和显式确认更新，以及 WSL2、独立 Linux 切换脚本和跨资源状态协调。

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

## Spikes

| # | Name | Type | Validates | Verdict | Tags |
|---|------|------|-----------|---------|------|
| 001 | codex-native-config-contract | standard | Given Windows/macOS 当前用户可能同时使用桌面 Codex 与本机 Codex CLI，when 定位、读取并隔离测试原生配置，then 能确认默认路径、共享关系、供应商字段写法、凭据来源和重启生效边界 | PARTIAL | codex, config, windows, macos, desktop, cli |
| 002 | provider-validation-loop | standard | Given 标准供应商的服务地址、API Key 和默认模型，when 执行模型发现、Responses API 流式请求及工具调用闭环，then 能可重复地判定供应商是否满足 Codex 使用要求并提供脱敏诊断 | PARTIAL | responses-api, sse, tools, provider, validation |
| 003a | toml-structural-edit | comparison | Given 含未知字段、注释和不同换行的 Codex TOML，when 使用结构化 TOML 编辑执行供应商切换，then 能保留非受管配置并安全原子落盘、备份和恢复 | VALIDATED | rust, toml, atomic-write, backup, comparison |
| 003b | managed-block-edit | comparison | Given 含未知字段、注释、损坏或重复管理区块的 Codex TOML，when 只替换 GPTEasy 管理区块，then 能保留文件其余字节并在歧义时停止修改 | PARTIAL | rust, managed-block, atomic-write, backup, comparison |
| 004 | tauri-tray-process-restart | standard | Given Tauri 2 托盘程序运行且桌面 Codex 或 Codex CLI 可能持有旧配置，when 检测进程并切换供应商，then 能呈现立即重启、稍后重启、取消和明确退出语义且不误杀无关进程 | PARTIAL | tauri, tray, process, restart, windows, macos |
| 005 | desktop-install-update-matrix | standard | Given Windows/macOS x64/ARM64 目标，when 构建、安装和更新 Tauri 2 应用，then 能确认当前用户安装、权限、签名公证、更新包和显式确认更新的可交付方式 | PARTIAL | tauri, installer, updater, windows, macos |
| 006 | first-takeover-managed-block-transaction | standard | Given 已有受管键、未知字段、注释和不同换行的 Codex TOML，when 首次接管在一个事务中完成结构化迁移并建立 dotted-key 管理区块，then 最终配置有效、非受管配置保留、后续切换区块外字节不变且故障可恢复 | VALIDATED | rust, toml, migration, managed-block, atomic-write, integration |
| 007 | provider-switch-saga | standard | Given SQLite 中的已验证供应商、当前 Codex 配置和相关进程，when 在验证、状态写入、配置替换及重启边界发生失败或崩溃，then 系统重启后能收敛到完整旧状态或完整新状态且不静默终止 CLI | VALIDATED | rust, sqlite, config, saga, recovery, restart, integration |
| 008 | external-config-reconciliation | standard | Given 用户层配置被外部修改、存在覆盖层或供应商身份匹配歧义，when GPTEasy 启动或重新扫描，then 能识别受管供应商、展示外部配置和层级差异且不自动覆盖 | VALIDATED | codex, config-layer, provider-id, reconciliation, external-config, integration |
| 009 | wsl2-environment-lifecycle | standard | Given 多个运行中或已停止的 WSL2 发行版及其默认用户，when 检测、单独切换或批量切换供应商，then 检测不启动发行版、显式切换才临时启动、只修改默认用户并恢复原停止状态 | PARTIAL | wsl2, windows, process, config, lifecycle, backup |
| 010a | linux-switch-functions-bash | comparison | Given 只有 Bash 4+ 且无额外运行时的 Linux 环境，when source 导出脚本并交互选择、取消或重复切换供应商，then 只有明确选择后才安全替换管理区块、备份并保留其他配置 | VALIDATED | bash, linux, shell, managed-block, backup, comparison |
| 010b | linux-switch-functions-zsh | comparison | Given 只有 Zsh 5+ 且无额外运行时的 Linux 环境，when source 导出脚本并交互选择、取消或重复切换供应商，then 只有明确选择后才安全替换管理区块、备份并保留其他配置 | VALIDATED | zsh, linux, shell, managed-block, backup, comparison |
| 011 | real-provider-compatibility-matrix | standard | Given Git 忽略的项目本地私密文件中的真实供应商地址、API Key 和模型，when 在分阶段截止时间、限流和协议差异下运行完整 nonce 工具闭环，then 能形成真实兼容结论、稳定失败分类和脱敏证据 | VALIDATED | provider, responses-api, sse, tools, timeout, rate-limit, live |
| 012 | desktop-provider-switch-e2e | standard | Given Tauri UI、真实已验证供应商、SQLite 状态、Codex 配置及运行中的桌面和 CLI，when 用户完成验证并选择立即重启、稍后重启或取消，then 验证、迁移、Saga、协调和进程语义的数据交接完整且最终状态可解释 | VALIDATED | tauri, provider, sqlite, config, reconciliation, process, integration, e2e |
| 013 | wsl2-host-guest-switch-transaction | standard | Given Windows 上运行中或已停止的真实 WSL2 用户发行版，when GPTEasy 从宿主机向默认用户安全传递供应商并执行切换，then 凭据不进入命令行或日志、配置协议成立且所有路径恢复原生命周期状态 | VALIDATED | wsl2, windows, linux, credential, config, lifecycle, integration |
| 017 | macos-real-host-contract | standard | Given macOS 14+ 的 Intel 或 Apple Silicon 真实环境，when 执行配置探针、Codex 进程识别、托盘关闭、应用激活、当前用户安装和 updater 原地替换，then Windows 上的跨平台推断得到真实验证或明确否定 | PARTIAL | macos, tauri, codex, process, install, updater, integration, live |
