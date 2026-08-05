# Spike Wrap-Up Summary

**Date:** 2026-08-05

**New spikes processed:** 3

**Total spikes packaged:** 16

**Feature areas:** Codex 与供应商兼容、安全配置写入、切换一致性与外部协调、桌面运行生命周期、桌面供应商切换端到端、WSL2 与 Linux 导出物、安装与更新、macOS 真实宿主契约

**Skill output:** `./.codex/skills/spike-findings-gpteasy/`

## Wrap-Up Sessions

| Session | Date | Newly Processed |
|---------|------|-----------------|
| Initial | 2026-08-05 | 001、002、003a、003b、004、005 |
| Append | 2026-08-05 | 006、007、008、009、010a、010b、011 |
| Append 2 | 2026-08-05 | 012、013、017 |

## Processed Spikes

| # | Name | Type | Verdict | Feature Area |
|---|------|------|---------|--------------|
| 001 | codex-native-config-contract | standard | PARTIAL | Codex 与供应商兼容 |
| 002 | provider-validation-loop | standard | PARTIAL | Codex 与供应商兼容 |
| 003a | toml-structural-edit | comparison | VALIDATED | 安全配置写入 |
| 003b | managed-block-edit | comparison | PARTIAL | 安全配置写入 |
| 004 | tauri-tray-process-restart | standard | PARTIAL | 桌面运行生命周期 |
| 005 | desktop-install-update-matrix | standard | PARTIAL | 安装与更新 |
| 006 | first-takeover-managed-block-transaction | standard | VALIDATED | 安全配置写入 |
| 007 | provider-switch-saga | standard | VALIDATED | 切换一致性与外部协调 |
| 008 | external-config-reconciliation | standard | VALIDATED | 切换一致性与外部协调 |
| 009 | wsl2-environment-lifecycle | standard | PARTIAL | WSL2 与 Linux 导出物 |
| 010a | linux-switch-functions-bash | comparison | VALIDATED | WSL2 与 Linux 导出物 |
| 010b | linux-switch-functions-zsh | comparison | VALIDATED | WSL2 与 Linux 导出物 |
| 011 | real-provider-compatibility-matrix | standard | VALIDATED | Codex 与供应商兼容 |
| 012 | desktop-provider-switch-e2e | standard | VALIDATED | 桌面供应商切换端到端 |
| 013 | wsl2-host-guest-switch-transaction | standard | VALIDATED | WSL2 与 Linux 导出物 |
| 017 | macos-real-host-contract | standard | PARTIAL | macOS 真实宿主契约 |

## Key Findings

### Codex 与供应商兼容

- Windows 默认环境中，统一 ChatGPT 桌面应用的 bundled Codex 与本机 Codex CLI 共享当前用户的 `~/.codex` 配置根。
- 供应商保存门禁必须完整覆盖模型发现、Responses SSE、strict function call 和 `function_call_output` nonce 回传。
- 真实供应商组合已在 2026-08-05 完成四阶段闭环；兼容结论只绑定当次地址、Key 和模型组合，任一变化都必须重新验证。
- SSE 解析器必须容忍附加事件和分段 arguments delta，只对完成事件、最终函数调用项及 nonce 结果设门禁。
- 首事件、流空闲、整体超时和 429 限流必须分开分类；诊断只记录事件类型、耗时、状态和正文长度。

### 安全配置写入

- 003a 与 003b 的接缝已由 006 闭合：首次接管必须在单个结构化事务中移除旧受管键、处理 `model_providers` 父表、建立唯一管理区块并重新解析。
- `model_providers` 父表只含 provider 子表时可转为 implicit；含直属未知值时必须在备份和写入前停止。
- 管理区块建立后，后续切换可保证区块外字节完全不变。
- Windows 使用同步临时文件加 `ReplaceFileW(..., flags = 0)`；不能依赖文档标记为不支持的 `REPLACEFILE_WRITE_THROUGH`。

### 切换一致性与外部协调

- SQLite、Codex TOML 和进程不存在跨资源 ACID；必须先持久化 `prepared` 意图、旧/新配置哈希和备份路径，再替换配置。
- 恢复时旧哈希回滚、新哈希前滚、未知哈希进入 `needs_attention`，不得覆盖外部编辑。
- 不可变供应商 ID 保存在管理区块注释中；地址、Key 或模型变化需要重新验证，而不是修改原身份后静默接受。
- 最终有效配置通过 Codex app-server `config/read(cwd, includeLayers=true)` 获取，只在内存中提取 model/provider 和来源摘要。
- 用户层配置正确但被项目或会话层覆盖时应展示 `managed_overridden`，不得自动改写用户文件争夺优先级。

### 桌面供应商切换端到端

- UI 不应编排松散的验证、保存和写入 command；单个 Rust 后端调用持有 `VerifiedProvider` 并完成全部安全门禁。
- 版本化 SHA-256 组合指纹绑定地址、Key 和模型；SQLite 只保存指纹，不保存 Key。
- 固定链路为“验证 → 准备候选与备份 → 持久化 `prepared` → 原子替换 → 提交状态 → app-server 协调 → 重启收尾”。
- 012 的 15/15 确定性矩阵和真实供应商链路通过；真实进程只参与扫描，实验没有终止桌面宿主或 CLI。
- Windows app-server cwd 与 trust 路径必须去除 `\\?\` 前缀；项目层覆盖只形成 `managed_overridden`。
- 含目标配置与备份的 operational workspace 不属于可导出诊断，泄漏扫描只覆盖 evidence、SQLite 和事件日志。

### WSL2 与 Linux 导出物

- WSL2 检测使用全部发行版、运行中发行版的集合差和当前用户 Lxss 注册表，不进入或启动发行版。
- 数据库身份使用注册 GUID；显示名称重复且无法解歧时必须停止管理。
- 已停止发行版只在用户明确切换后临时启动，并在成功、写入失败和管理区块损坏路径恢复原停止状态。
- Windows Rust 在内存中完成 TOML 迁移，把完整候选通过 `wsl.exe` stdin 交给无凭据 guest writer；Key、地址和模型不得进入两侧进程参数。
- 013 在一次性 Ubuntu Base 24.04.3 amd64 WSL2 中通过 10/10 真实矩阵，覆盖停止/运行生命周期、并发哈希冲突、`0600` 权限和五份备份。
- Bash 4.4 与 Zsh 5.9 的 12 项矩阵均通过；source 和取消零写入，后续切换保留区块外内容、权限、最近五份备份和恢复语义。
- 正式产品应由同一个 Rust 生成器共享 provider、管理区块和备份模板，只分叉 Bash/Zsh 的交互与选项隔离层。

### macOS 真实宿主契约

- `~/Applications/GPTEasy.app` 是严格当前用户安装目标，默认 `/Applications` DMG 不能单独满足要求。
- macOS 证据必须区分非 macOS 契约测试、原生 Intel/Apple Silicon CI 构建和真实用户 Mac；只有最后一层能验证托盘、LaunchServices 和真实 Codex 拓扑。
- 应同时发现 `Codex.app` 与 `ChatGPT.app`，并结合 bundle 路径、父子关系、resource 路径和 Electron `--type=` 分类进程。
- `codesign`、Gatekeeper、公证/Stapling 与 Tauri updater 内容签名是四个独立门禁。
- 两版本 updater 必须在 app data 目录保存 canary，验证原地替换后用户状态仍存在。
- 017 仍为 PARTIAL；截至 2026-08-05 没有真实 macOS 14+、Developer ID、公证或签名 updater 证据。

### 保留的既有结论

- 桌面进程分类必须结合名称、路径、父子关系和 Electron `--type=` 参数；桌面宿主可自动重启，CLI 必须人工重启。
- Windows x64 NSIS 当前用户安装与 updater 签名产物已验证；macOS 严格 `~/Applications` 安装及完整签名、公证、更新链路仍需真实机器验证。
