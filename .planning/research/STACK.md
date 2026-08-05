# Stack Research

**Domain:** Windows/macOS 跨平台 Codex 供应商管理桌面伴侣（Tauri 2 + Rust + TypeScript/React）
**Researched:** 2026-08-05
**Confidence:** MEDIUM

> 置信度说明：版本号已通过官方注册表、官方发布记录或平台文档交叉核验；但该栈更新频繁，且 Windows ARM64 runner、Codex 配置格式、Tauri WebDriver 等能力仍在快速演进，所以整体按研究 seam 的 `MEDIUM` 标记。以下建议只实现已锁定决策，不改变 `CONTEXT.md`、ADR 或 UI-SPEC。

## Executive Recommendation

采用 **Rust 1.97.1 + Tauri 2.11.x + Node.js 24.18.0 LTS + pnpm 11.20.0 + React 19.2.8 + Vite 8.2.0**。Rust 后端持有全部文件、数据库、网络、进程、WSL2、剪贴板、更新和平台 API 权限；WebView 只安装 `@tauri-apps/api`，通过小粒度、强类型的 `invoke` 封装访问后端，不授予通用 `fs`、`shell`、`http`、`sql`、`clipboard` 能力。

SQLite 选择 `rusqlite` 而非 SQLx；配置编辑选择 `toml_edit` 加自研的“读取—验证—最小修改—并发复核—备份—原子替换”事务边界；供应商验证选择 `reqwest` + `sse-stream`，不引入第三方 OpenAI SDK。平台能力以窄适配器封装：Windows 使用 `windows` crate 和官方 WSL/Win32 API，macOS 使用 `objc2-app-kit` 与 AppKit。

发布采用 **Windows NSIS current-user 安装包**、**macOS universal DMG**、双重签名（操作系统代码签名 + Tauri updater artifact 签名）。完整测试必须覆盖 Rust/React、历史数据库迁移、故障注入的原子写入、真实 Bash/Zsh、Windows/macOS Tauri E2E、四个目标架构的安装与签名验收。

## Recommended Stack

### Core Technologies

| Technology | Version | Purpose | Why Recommended | 置信度 |
|---|---:|---|---|---|
| Rust | `1.97.1`，edition `2024` | 系统与领域后端 | 当前 stable；满足 Tauri 和本研究所选 crates 的最高 MSRV；用 `rust-toolchain.toml` 精确固定 | MEDIUM |
| Tauri | `2.11.5` | 桌面壳、窗口、托盘、菜单、IPC、bundler | 符合锁定 ADR；内建 tray/menu/window API，插件权限模型适合最小化 WebView 权限 | MEDIUM |
| `tauri-build` | `2.6.3` | Rust build script | 与当前 Tauri 2 发布线配套，生成资源与配置 | MEDIUM |
| `@tauri-apps/cli` | `2.11.4` | 开发、构建、签名、打包 | 使用 Tauri 官方 CLI；本地与 CI 必须相同版本 | MEDIUM |
| `@tauri-apps/api` | `2.11.1` | WebView 的最小 IPC/窗口 API | 前端只需要 Tauri 核心 API；不要安装通用系统插件的 JS 客户端 | MEDIUM |
| Node.js | `24.18.0` LTS | 前端构建与测试运行时 | Node 24 为 Active LTS；满足 Vite 8、pnpm 11、Vitest 4、jsdom 30 | MEDIUM |
| pnpm | `11.20.0` | JS 包管理 | workspace/lockfile 严格，适合 `--frozen-lockfile` 可复现构建 | MEDIUM |
| React / React DOM | `19.2.8` | 设置窗口 UI | 当前稳定线，生态依赖均声明 React 19 支持 | MEDIUM |
| Vite | `8.2.0` | 前端 dev/build | Tauri 官方推荐的轻量前端集成；Vitest 4 同版本生态兼容 | MEDIUM |
| TypeScript | **`6.0.3`** | UI 与 command DTO 类型安全 | 不采用当前最新 `7.0.2`：`typescript-eslint 8.66.0` 只支持 `<6.1.0`；先固定最后兼容的 6.0.x | MEDIUM |
| SQLite（bundled） | `rusqlite 0.40.1` | 应用内部状态 | 同步单连接模型贴合 Rust 后端独占访问；`bundled` 保证 Win/mac SQLite 行为一致；`backup` 支持迁移前在线备份 | MEDIUM |

### Tauri Capabilities and Plugins

全部插件由 Rust 初始化并包在受控 commands 后面。除 `@tauri-apps/api` 外，不在前端安装相应 JS 包，也不在 capability 文件中开放插件通配权限。

| Component | Version | Purpose | Required Rule | 置信度 |
|---|---:|---|---|---|
| Tauri built-in tray/menu | `tauri 2.11.5` + `tray-icon` feature | 托盘、原生菜单、待重启图标状态 | 用 `TrayIconBuilder`/menu API；菜单事件进入 application service，不直接改文件 | MEDIUM |
| `tauri-plugin-single-instance` | `2.4.3` | 保证单实例、第二次启动聚焦设置窗口 | **必须在所有插件中最先注册** | MEDIUM |
| `tauri-plugin-autostart` | `2.5.1` | 当前用户登录启动 | 默认关闭；只由设置 command 启停并回读实际状态 | MEDIUM |
| `tauri-plugin-dialog` | `2.7.2` | 原生确认、保存文件、恢复决策 | 只调用 Rust API；按钮语义由领域层先构造 | MEDIUM |
| `tauri-plugin-notification` | `2.3.3` | 窗口不可见时的系统通知 | 通知内容必须先脱敏；安装包内验证 Windows AppUserModelID/macOS 权限 | MEDIUM |
| `tauri-plugin-updater` | `2.10.1` | 每日最多一次检查、用户确认后更新 | 使用默认 `rustls-tls`；Rust 侧 `check`/download/install；不静默安装 | MEDIUM |
| `tauri-plugin-process` | `2.3.1` | 更新后重启 GPTEasy 自身 | **只用于当前应用**，不能用于外部 Codex/ChatGPT 进程 | MEDIUM |
| `tauri-plugin-opener` | `2.5.4` | 打开官网、帮助、诊断目录 | command 端白名单 URL scheme/目标，不给 WebView 任意 opener 权限 | MEDIUM |

### Rust Application Libraries

| Library | Version | Purpose | Features / When to Use | 置信度 |
|---|---:|---|---|---|
| `serde` / `serde_json` | `1.0.229` / `1.0.151` | command DTO、数据库映射、API wire types | `serde = { features = ["derive"] }` | MEDIUM |
| `ts-rs` | `12.0.1` | 从 Rust DTO 生成 TypeScript 类型 | `uuid-impl`、`url-impl`；CI 检查生成文件无 drift；不用 RC 状态的 `tauri-specta 2` | MEDIUM |
| `tokio` | `1.53.1` | 网络、子进程、超时和后台任务 | 只启用 `macros, rt-multi-thread, process, time, sync, io-util`；复用 Tauri runtime | MEDIUM |
| `reqwest` | `0.13.4` | 模型发现、Responses API 验证、更新元数据之外的供应商网络 | `default-features = false`；启用 `rustls, http2, json, stream, system-proxy` | MEDIUM |
| `sse-stream` | `0.2.5` | 解码 Responses API SSE | 当前维护的 `http-body` SSE decoder；必须增加单事件、总响应和总时长上限 | MEDIUM |
| `url` | `2.5.8` | 服务地址解析与安全策略 | 结构化判断 scheme/host/port；禁止字符串前缀式 HTTPS 判断 | MEDIUM |
| `rusqlite` | `0.40.1` | SQLite | `default-features = false`；启用 `bundled, backup`；UUID/时间按稳定文本或整数格式保存 | MEDIUM |
| `toml_edit` | `0.25.13+spec-1.1.0` | Codex TOML 最小修改并保留注释/格式 | 只编辑 GPTEasy 管理字段；解析歧义立即停止 | MEDIUM |
| `uuid` | `1.24.0` | 不可变供应商 ID | `v4, serde`；显示名/地址变化不得生成新 ID | MEDIUM |
| `thiserror` | `2.0.19` | 领域与基础设施错误类型 | 错误分层；面向 UI 的错误 DTO 不包含秘密或原始正文 | MEDIUM |
| `time` | `0.3.55` | UTC 时间戳、备份命名、保留策略 | `formatting, parsing, serde, macros`；持久化统一 UTC | MEDIUM |
| `tracing` | `0.1.44` | 结构化诊断事件 | span 字段禁止记录 key、Authorization、完整 URL query | MEDIUM |
| `tracing-subscriber` | `0.3.23` | 日志过滤与格式 | `env-filter, fmt, registry`；生产过滤器不得通过环境变量开启敏感 dump | MEDIUM |
| `tracing-appender` | `0.2.5` | 每日日志文件与 non-blocking writer | 应用启动时自研清理最近 7 天；保留 `WorkerGuard` 到退出 | MEDIUM |
| `secrecy` | `0.10.3` | 进程内 API Key 包装 | `SecretString` 防止意外 `Debug`；仅在网络/配置/复制边界显式 expose | MEDIUM |
| `tempfile` | `3.27.0` | 同目录临时文件、故障安全测试 | 临时文件必须建在目标目录，确保同卷原子替换 | MEDIUM |
| `rustix` | `1.1.4` | macOS/POSIX 文件 `fsync`、权限 | macOS 原子提交后 `fsync` 父目录；不要只调用 `std::fs::rename` | MEDIUM |
| `windows` | `0.62.2` | Win32、WSL、文件替换、进程与应用激活 | 仅启用所需 Win32 features，见“平台 API” | MEDIUM |
| `objc2` / `objc2-app-kit` | `0.6.4` / `0.3.2` | macOS 运行中应用检测、退出、重启 | 仅启用 `NSRunningApplication`、`NSWorkspace` 等需要的 feature | MEDIUM |
| `sysinfo` | `0.39.6` | 跨平台 CLI 进程快照的辅助层 | `default-features = false, features = ["system"]`；GUI 身份仍用平台原生 API 复核 | MEDIUM |
| `directories` | `6.0.0` | GPTEasy 自有数据/日志目录 | 只用于 GPTEasy 目录；Codex 默认路径由 compatibility adapter 决定 | MEDIUM |
| `arboard` | `3.6.1` | 后端控制的文本剪贴板 | `default-features = false`，避免图片依赖；复制 command 返回后不记录内容 | MEDIUM |
| `zip` | `8.6.0` | 用户主动诊断导出 | `default-features = false, features = ["deflate", "time"]`；先脱敏再写 ZIP | MEDIUM |

### Frontend Libraries

| Library | Version | Purpose | When to Use | 置信度 |
|---|---:|---|---|---|
| `react-aria-components` | `1.20.0` | 无障碍表单、菜单、列表、对话框与焦点管理 | 作为无样式交互 primitive；样式由 CSS Modules/design tokens 控制 | MEDIUM |
| `react-hook-form` | `7.84.0` | 供应商完整单页表单 | 管理 dirty/touched、离开确认、字段依赖；Rust 仍是最终验证权威 | MEDIUM |
| `@hookform/resolvers` | `5.7.1` | RHF 与 Zod 集成 | 只做即时 UX 校验，不复制复杂领域规则 | MEDIUM |
| `zod` | `4.4.3` | 前端输入形状与 command response 防御性解析 | 对 invoke 返回做边界解析；与 Rust DTO 生成类型配合 | MEDIUM |
| `@tanstack/react-query` | `5.101.4` | commands 的异步状态、失效与重试策略 | 查询可缓存；写操作默认不自动重试，避免重复配置写入 | MEDIUM |
| `i18next` / `react-i18next` | `26.3.6` / `17.0.11` | 简中/英语本地化 | key 使用领域术语；CI 验证双语 key 完整 | MEDIUM |
| `lucide-react` | `1.28.0` | 窗口内功能图标 | 只用于 UI 图标；应用/托盘图标使用原创静态资产 | MEDIUM |
| CSS Modules + CSS custom properties | Vite 内建 | UI-SPEC 的主题与 8px token | 不引入 Tailwind/运行时 CSS-in-JS；使用 `prefers-color-scheme/contrast/reduced-motion` | MEDIUM |

### Frontend Quality Toolchain

| Tool | Version | Purpose | Notes | 置信度 |
|---|---:|---|---|---|
| ESLint | `9.39.5` | TS/React 静态检查 | 暂不升 10：`eslint-plugin-jsx-a11y 6.10.2` peer range 尚未覆盖 ESLint 10 | MEDIUM |
| `typescript-eslint` | `8.66.0` | Type-aware lint | 要求 TypeScript `<6.1.0`，因此固定 TS 6.0.3 | MEDIUM |
| `eslint-plugin-react-hooks` | `7.1.1` | Hooks 规则 | 启用推荐规则 | MEDIUM |
| `eslint-plugin-jsx-a11y` | `6.10.2` | 基础可访问性 lint | 不是替代 axe、Narrator、VoiceOver 的验收 | MEDIUM |
| Prettier | `3.9.6` | TS/JSON/CSS/Markdown 格式 | 普通文本保持 LF；不要格式化生成的 shell golden fixtures | MEDIUM |

## Database and Migration Prescription

1. 单个 `rusqlite::Connection` 由专用 repository/service 持有；不要让 React 或 Tauri SQL plugin 接触数据库。
2. 启动时依次执行：
   - 打开并读取 schema version；
   - 若版本高于当前应用支持版本，拒绝所有写入；
   - 使用 `rusqlite::backup::Backup` 创建完整迁移前备份，默认保留 3 份；
   - `BEGIN IMMEDIATE`，按永久编号依次执行 migration；
   - 写入 `schema_migrations(version, applied_at, app_version)`；
   - 完整 invariant 检查后提交。
3. 建议 PRAGMA：`foreign_keys=ON`、`journal_mode=WAL`、`synchronous=FULL`、有限 `busy_timeout`。本产品写入量低，优先耐久性而不是吞吐。
4. CI 保存每个正式 schema 的最小、典型、边界数据库 fixture；每次发布测试 `N -> latest`，并测试失败 migration 不改变原库。
5. 不使用“migration 失败则删除数据库”“自动修复高版本数据库”或运行时下载 migration。
6. 数据库及其 3 份迁移备份都包含明文凭据：macOS 设置 `0600`，Windows 使用当前用户目录继承的 user-only ACL 并在创建后复核；诊断导出绝不包含这些文件。

## Configuration and File-I/O Prescription

### Codex compatibility adapter

Codex 配置是外部、会变化的协议，不是 GPTEasy 数据库模型。建立独立 `codex_compat` 模块：

- `read_actual_state(path) -> External/Managed/OriginalLogin`；
- `plan_change(original_bytes, desired_state) -> ChangePlan`；
- `validate_plan` 确认只改变供应商字段/管理区块；
- `commit_change` 执行备份和原子替换；
- 按已支持 Codex 版本保存官方配置 fixture。

OpenAI 当前配置参考包含 `model_providers.<id>.base_url`、`env_key`、`wire_api`、`requires_openai_auth` 等字段，并明确把某些明文 bearer 配置标为实验/不推荐。由于 ADR 已锁定明文凭据与直接配置，roadmap 必须安排 **Codex 配置兼容性 spike**，用目标 Codex 版本的真实 fixture 确认写入位置；不要把供应商 DTO 直接序列化成整个 `config.toml`。

### Atomic commit algorithm

1. 读取原文件 bytes、权限和内容 hash。
2. `toml_edit` 生成候选 bytes；重新解析并做“非管理字段未变化”语义/快照校验。
3. 创建带时间戳备份并 `fsync`。
4. 在目标目录创建临时文件，写入、flush、同步，继承/收紧用户权限。
5. 替换前重新读取目标 hash；若外部已修改则停止，不覆盖。
6. Windows：已有文件用 `ReplaceFileW`；新文件/兼容回退用 `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`。
7. macOS：`fsync(temp)`、同目录 `rename`、`fsync(parent directory)`。
8. 替换后重新读取并验证；失败时提示从备份恢复，不静默继续。

不要把 advisory file lock 当并发安全保证：Codex 或用户编辑器不会遵守 GPTEasy 的锁。必须依靠 hash 复核和 fail-closed。

## Network and Provider Validation

- 使用单个配置好的 `reqwest::Client`，但每次验证有独立 cancellation token、连接超时、首字节超时、单步骤超时和总超时。
- 默认模型确认、SSE、工具闭环按三个显式 state machine step 顺序执行；失败即停止后续步骤。
- HTTPS 策略通过 `url` 结构化验证：远程只允许 `https`；`http` 仅允许 host literal `localhost`、`127.0.0.1`、`[::1]`。
- redirect policy 只允许安全的同源/明确规则跳转，避免 Authorization 泄漏到其他 host。
- `sse-stream` 解码后对单事件大小、累计字节、事件数量和响应总时长设上限。
- 不记录 request body、response body、Authorization、API Key 或完整服务地址；错误 DTO 只保留状态码、错误类别和脱敏技术摘要。
- 测试使用本地 `axum 0.8.9` fake provider 精确模拟 models、SSE 分片、工具调用、断流、慢响应、超限事件和错误状态；不要在 CI 依赖真实供应商。

## Platform APIs

### Windows

`windows 0.62.2` 建议 features：

```toml
Win32_Foundation
Win32_Storage_FileSystem
Win32_System_Com
Win32_System_Diagnostics_ToolHelp
Win32_System_SubsystemForLinux
Win32_System_Threading
Win32_UI_Shell
Win32_UI_WindowsAndMessaging
```

| Need | API | Prescription |
|---|---|---|
| 原子替换 | `FlushFileBuffers`, `ReplaceFileW`, `MoveFileExW` | 同卷临时文件；`MOVEFILE_WRITE_THROUGH`；错误必须保留原文件和备份 |
| 进程枚举 | `CreateToolhelp32Snapshot`, `Process32First/Next`, `QueryFullProcessImageNameW` | 按路径/签名/包身份复核，不只按易冲突的进程名 |
| GUI 正常关闭 | `EnumWindows`, `GetWindowThreadProcessId`, `PostMessageW(WM_CLOSE)` | 用户明确选择“立即重启”后先请求正常退出，设超时，不默认强杀 |
| 等待退出 | `OpenProcess(SYNCHRONIZE)`, `WaitForSingleObject` | 不轮询文件或睡眠猜测 |
| 打包应用激活 | `IApplicationActivationManager::ActivateApplication` | 适用于 MSIX/AppUserModelID；普通可执行文件用受验证路径启动 |
| WSL2 枚举 | `wsl.exe --list --quiet` + `WslGetDistributionConfiguration` | 不解析本地化的 `--list --verbose` 状态文本 |
| WSL2 运行状态 | `wsl.exe --list --running --quiet` | 操作前记录，结束后只 `--terminate <distro>` 原先停止的发行版 |
| WSL2 执行 | `wsl.exe --distribution <name> --exec ...` | 参数数组调用；固定脚本从 stdin 读 payload；禁止拼 shell 命令 |

`wsl.exe` 名称参数必须作为独立 argv 传入。目标发行版内调用默认用户的固定 `/bin/sh` helper，provider 数据通过 stdin/临时文件传递并设 `umask 077`；不要把 API Key、地址或发行版名插进 shell source。

### macOS

| Need | API / crate | Prescription |
|---|---|---|
| GUI 应用检测 | `NSRunningApplication::runningApplicationsWithBundleIdentifier` via `objc2-app-kit` | 使用 bundle identifier；不要解析 `ps` 文本识别 GUI |
| 正常退出 | `NSRunningApplication::terminate` | 用户明确确认后调用并等待；超时作为可见错误 |
| 重启/激活 | `NSWorkspace::openApplicationAtURL:configuration:completionHandler:` | 用已验证 `.app` URL；处理异步 completion error |
| 原子文件提交 | `rustix` + `rename` + directory `fsync` | 保持 mode/owner，只修改当前用户配置 |
| 托盘优先行为 | Tauri window/tray + macOS activation policy | 关闭窗口时 hide；退出只来自托盘明确操作；在真机验收 Dock/Cmd-Tab 行为 |

## Testing Stack

### Tools and Versions

| Layer | Tool | Version | Purpose | 置信度 |
|---|---|---:|---|---|
| Rust runner | `cargo-nextest` | `0.9.143` | PR/CI 主测试 runner、重试隔离、JUnit | MEDIUM |
| Rust coverage | `cargo-llvm-cov` | `0.8.7` | 核心领域、migration、writer 分支覆盖 | MEDIUM |
| Supply chain | `cargo-deny` / `cargo-audit` | `0.20.2` / `0.22.2` | license、bans、advisory 检查 | MEDIUM |
| Snapshot | `insta` | `1.48.0` | TOML 最小 diff、错误 DTO、Bash/Zsh 输出 golden | MEDIUM |
| Property tests | `proptest` | `1.11.0` | 名称/路径/Unicode/管理区块/quote encoder 不变量 | MEDIUM |
| Test doubles | `mockall` | `0.15.0` | 平台 adapter 和 clock/network boundary | MEDIUM |
| HTTP fixture | `axum` | `0.8.9` | 可控 SSE/工具调用 fake provider | MEDIUM |
| Component tests | Vitest | `4.1.10` | React 逻辑、command hooks、i18n | MEDIUM |
| DOM | jsdom | `30.0.1` | Vitest DOM；要求 Node 24.15+ | MEDIUM |
| UI testing | RTL / user-event | `16.3.2` / `14.6.3` | 按用户角色和键盘交互断言 | MEDIUM |
| Browser E2E/visual/a11y | Playwright | `1.62.1` | Vite preview 的主题、双语、键盘、截图和 axe | MEDIUM |
| Accessibility | `@axe-core/playwright` | `4.12.1` | 自动 WCAG 检查；仍需屏幕阅读器手测 | MEDIUM |
| Tauri E2E | WebdriverIO | `9.30.1` | 已打包/测试构建的桌面交互 | MEDIUM |
| Tauri WDIO bridge | `@wdio/tauri-service` / `tauri-plugin-wdio` | `1.3.0` / `1.3.0` | Windows/macOS Tauri E2E | MEDIUM |
| Bash analysis | ShellCheck | `0.11.0` | Bash 静态检查 | MEDIUM |
| Bash tests | bats-core | `1.14.0` | Bash function 行为 | MEDIUM |
| Shell formatting | shfmt | `3.13.1` | 仅 Bash generator 输出/fixture | MEDIUM |

`tauri-plugin-wdio` 只允许在 `cfg(debug_assertions)` 或显式 `e2e` Cargo feature 下编译。它会开放 localhost WebDriver 服务，**生产构建必须通过二进制/依赖检查确认不包含该插件**。

### Required Test Matrix

| Gate | Windows x64 | Windows ARM64 | macOS Intel | macOS Apple Silicon | Linux CI |
|---|---|---|---|---|---|
| Rust unit/domain tests | 每 PR | nightly/release | 每 PR | 每 PR | 每 PR 快速反馈 |
| React/Vitest | 每 PR | 可复用构建结果 | 每 PR | 每 PR | 每 PR |
| DB historical migrations | 每 PR | release | 每 PR | release | 每 PR |
| Atomic writer fault injection | 每 PR | release 真机 | 每 PR | release 真机 | POSIX 参考 |
| Provider fake-server contract | 每 PR | release | 每 PR | release | 每 PR |
| Tauri WebDriver E2E | 每 PR 主路径 | release | release | 每 PR 主路径 | 不作为产品目标 |
| Installer/package smoke | release | release | universal DMG 验证 | universal DMG 验证 | — |
| WSL2 real integration | 专用 Win11+WSL2 runner | 专用 ARM64 真机/runner | — | — | — |
| Bash 4+/Zsh 5+ script | — | — | 可选 | 可选 | 每 PR 多版本容器 |
| Narrator/VoiceOver/200%/高对比度 | release 手测 | release 手测 | release 手测 | release 手测 | — |

Shell 兼容矩阵至少包含 Bash `4.4` + 当前稳定、Zsh `5.0`/`5.9` 或可获得的最低 5.x + 当前稳定。Bash 使用 ShellCheck+Bats；Zsh 使用 `zsh -n` 和真实执行测试。不要把 ShellCheck 的 Bash parser 当作 Zsh 兼容证明。

### Release Verification

- Windows：干净 Windows 10 22H2 x64、Windows 11 ARM64 安装/升级/卸载；验证 current-user、无管理员权限、WebView2 缺失路径、签名和 updater。
- Windows 签名：`signtool verify /pa /all /v`，并检查时间戳。
- macOS：macOS 14 Intel/Apple Silicon 安装 universal DMG、首次启动 Gatekeeper、托盘、登录启动、通知和更新。
- macOS 签名：`codesign --verify --deep --strict --verbose=2`、`spctl --assess --type execute`、`xcrun stapler validate`。
- 更新：用已签名的 `N-1` 正式构建更新到候选版；测试签名不匹配、下载中断、用户取消和待下次启动。
- 数据：复制真实旧版数据库/config fixture，升级失败后确认原始文件与备份可恢复。

## Packaging, Signing, and Release Tooling

### Windows

| Component | Choice | Configuration |
|---|---|---|
| Installer | Tauri NSIS；Tauri 2.11 当前固定 **NSIS 3.11** | `bundle.targets = ["nsis"]`，`nsis.installMode = "currentUser"`；不要把 MSI 作为首版主包 |
| WebView2 | Evergreen `downloadBootstrapper` | 体积小且适合联网更新；在干净 Win10 22H2 测试。若以后要求离线安装，再增加单独 offlineInstaller artifact |
| Architectures | `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` | 各自产出并签名，文件名含 arch |
| Authenticode | Microsoft Artifact Signing + Windows SDK SignTool | Tauri `signCommand` 调用仓库内 PowerShell wrapper；SHA256 + Artifact Signing timestamp |
| Artifact Signing client | 官方 Client Tools、`.NET 8`、`Azure.CodeSigning.Dlib.dll` | CI 用 OIDC/最小角色；不要把 PFX 长期放仓库或普通 secret 文件 |
| Update signature | Tauri updater signing key | 与 Authenticode 独立；private key 只存在 CI secret，public key 写入 app config |

Artifact Signing 官方要求时间戳；其证书有效期很短，缺少 timestamp 会导致随后验证失败。签名 wrapper 应同时签主 EXE、必要 DLL 和最终 NSIS installer，并在上传前执行验证。

### macOS

| Component | Choice | Configuration |
|---|---|---|
| App target | `universal-apple-darwin` | 一个 universal app/DMG 同时支持 Intel 与 Apple Silicon |
| Minimum OS | `bundle.macOS.minimumSystemVersion = "14.0"` | 同时设置构建环境 `MACOSX_DEPLOYMENT_TARGET=14.0` |
| Installer | Tauri universal DMG 作为传输载体 | 目标安装位置必须是 `~/Applications/GPTEasy.app`；标准 `/Applications` alias 不能作为唯一安装路径，不做 privileged helper |
| Code signing | `Developer ID Application` | hardened runtime 保持开启；entitlements 最小化，不申请无关能力 |
| Notarization | App Store Connect API key | 使用 `APPLE_API_ISSUER/KEY/KEY_PATH`；比 Apple ID app-specific password 更适合 CI |
| Verification | `codesign`, `spctl`, `notarytool`, `stapler` | 上传前后验证；DMG 必须 stapled |

### Update Publication

- `bundle.createUpdaterArtifacts = true`。
- 使用静态 `latest.json` + GitHub Releases/静态 HTTPS endpoint；应用每日最多请求一次。
- `latest.json` 同时包含 Windows x64/ARM64 与 macOS universal target 的 URL、version、notes、signature。
- 发布 job 先构建并完成 OS 签名/公证，再生成/上传 updater artifacts 和 `latest.json`，最后公开 release；不能先发布 metadata 再补文件。
- 使用 `tauri-apps/tauri-action` 当前不可变发布 `action-v1.0.0`，工作流仍固定到审核过的完整 commit SHA，而不是可移动分支/tag。

### CI Runner Prescription

- Windows x64：固定 `windows-2025`，不要使用漂移的 `windows-latest`。
- Windows ARM64：`windows-11-arm` 当前仍为 public preview；可用于常规构建，但正式发布必须有一台自托管 Windows 11 ARM64 后备并保留真机安装证据。
- macOS Apple Silicon：固定 `macos-15`。
- macOS Intel：固定 `macos-15-intel` 做架构测试；universal release 可在 Apple Silicon runner 安装两个 Rust targets 后构建。
- WSL2：使用专用、已启用 Hyper-V/WSL2 的 Windows runner；普通 unit CI 不能替代真实发行版状态恢复测试。
- 所有 actions 固定完整 commit SHA；上传 release 前使用 `pnpm --frozen-lockfile`、`cargo --locked`。

macOS 没有像 NSIS `currentUser` 一样的 Tauri 一键配置。实施阶段应尽早做一个签名/公证 spike：DMG 中提供明确的“安装到个人 Applications”入口，或由已签名应用在用户确认后复制自身到 `~/Applications`、验证复制后的 code signature、重启新副本。不能把默认拖到 `/Applications` 的系统级安装当成满足“当前用户安装”。

## Installation

建议先通过精确版本初始化，再提交 `pnpm-lock.yaml` 与 `Cargo.lock`。后续依赖更新走单独 PR 和完整跨平台测试。

```bash
# Toolchain
corepack enable
corepack prepare pnpm@11.20.0 --activate

# Frontend runtime
pnpm add -E \
  @tauri-apps/api@2.11.1 \
  react@19.2.8 react-dom@19.2.8 \
  react-aria-components@1.20.0 \
  react-hook-form@7.84.0 @hookform/resolvers@5.7.1 zod@4.4.3 \
  @tanstack/react-query@5.101.4 \
  i18next@26.3.6 react-i18next@17.0.11 \
  lucide-react@1.28.0

# Frontend build/lint/test
pnpm add -DE \
  @tauri-apps/cli@2.11.4 \
  typescript@6.0.3 vite@8.2.0 @vitejs/plugin-react@6.0.5 \
  @types/node@24.13.3 @types/react@19.2.18 @types/react-dom@19.2.4 \
  eslint@9.39.5 typescript-eslint@8.66.0 \
  eslint-plugin-react-hooks@7.1.1 eslint-plugin-jsx-a11y@6.10.2 \
  prettier@3.9.6 \
  vitest@4.1.10 @vitest/coverage-v8@4.1.10 jsdom@30.0.1 \
  @testing-library/dom@10.4.1 @testing-library/react@16.3.2 \
  @testing-library/user-event@14.6.3 @testing-library/jest-dom@7.0.0 \
  @playwright/test@1.62.1 @axe-core/playwright@4.12.1 \
  webdriverio@9.30.1 @wdio/cli@9.30.1 @wdio/local-runner@9.30.1 \
  @wdio/mocha-framework@9.30.1 @wdio/spec-reporter@9.30.1 \
  @wdio/tauri-service@1.3.0

# Rust CI tools
cargo install cargo-nextest --version 0.9.143 --locked
cargo install cargo-llvm-cov --version 0.8.7 --locked
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-audit --version 0.22.2 --locked
```

`rust-toolchain.toml`：

```toml
[toolchain]
channel = "1.97.1"
profile = "minimal"
components = ["clippy", "rustfmt", "llvm-tools-preview"]
```

`Cargo.toml` 采用显式版本与 feature；以下为关键依赖集合，不表示所有代码都放进同一 crate：

```toml
[workspace.package]
edition = "2024"
rust-version = "1.97.1"

[dependencies]
tauri = { version = "2.11.5", features = ["tray-icon"] }
tauri-plugin-autostart = "2.5.1"
tauri-plugin-dialog = "2.7.2"
tauri-plugin-notification = "2.3.3"
tauri-plugin-opener = "2.5.4"
tauri-plugin-process = "2.3.1"
tauri-plugin-single-instance = "2.4.3"
tauri-plugin-updater = "2.10.1"
tauri-plugin-wdio = { version = "1.3.0", optional = true }

serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
thiserror = "2.0.19"
uuid = { version = "1.24.0", features = ["v4", "serde"] }
url = { version = "2.5.8", features = ["serde"] }
time = { version = "0.3.55", features = ["formatting", "parsing", "serde", "macros"] }
ts-rs = { version = "12.0.1", features = ["uuid-impl", "url-impl"] }

tokio = { version = "1.53.1", features = ["macros", "rt-multi-thread", "process", "time", "sync", "io-util"] }
reqwest = { version = "0.13.4", default-features = false, features = ["rustls", "http2", "json", "stream", "system-proxy"] }
sse-stream = "0.2.5"

rusqlite = { version = "0.40.1", default-features = false, features = ["bundled", "backup"] }
toml_edit = "0.25.13"
tempfile = "3.27.0"
secrecy = "0.10.3"
directories = "6.0.0"
arboard = { version = "3.6.1", default-features = false }
zip = { version = "8.6.0", default-features = false, features = ["deflate", "time"] }

tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", features = ["env-filter", "fmt", "registry"] }
tracing-appender = "0.2.5"
sysinfo = { version = "0.39.6", default-features = false, features = ["system"] }

[target.'cfg(windows)'.dependencies]
windows = { version = "0.62.2", features = [
  "Win32_Foundation",
  "Win32_Storage_FileSystem",
  "Win32_System_Com",
  "Win32_System_Diagnostics_ToolHelp",
  "Win32_System_SubsystemForLinux",
  "Win32_System_Threading",
  "Win32_UI_Shell",
  "Win32_UI_WindowsAndMessaging",
] }

[target.'cfg(target_os = "macos")'.dependencies]
rustix = { version = "1.1.4", features = ["fs"] }
objc2 = "0.6.4"
objc2-app-kit = { version = "0.3.2", default-features = false, features = [
  "std",
  "NSRunningApplication",
  "NSWorkspace",
] }

[dev-dependencies]
axum = "0.8.9"
insta = "1.48.0"
mockall = "0.15.0"
proptest = "1.11.0"

[features]
e2e = ["dep:tauri-plugin-wdio"]

[build-dependencies]
tauri-build = "2.6.3"
```

## Alternatives Considered

| Recommended | Alternative | Why Not for GPTEasy / When Alternative Fits |
|---|---|---|
| `rusqlite` | SQLx | SQLx 适合多连接 async 服务；本项目是单进程本地 DB，pool、runtime、query metadata 增加复杂度且无收益 |
| `toml_edit` + native atomic writer | 全文件 `toml::to_string` | 全量重写会破坏用户注释/格式并扩大冲突面，违反配置保留边界 |
| `reqwest` wire client | 第三方 OpenAI SDK | 验证需要自定义 base URL、精确 SSE/工具闭环、限流与脱敏；SDK 会隐藏关键协议细节 |
| React Aria Components | MUI/Ant Design | 大型视觉体系难以匹配已锁定的原生、克制 UI；RAC 提供行为与无障碍而不强加视觉 |
| CSS Modules/tokens | Tailwind | UI 规模有限，固定设计 contract；不需要额外 utility DSL 与构建约束 |
| TanStack Query + React local state | Redux/Zustand | 状态主要来自 Rust commands；全局 client store 会复制后端权威状态 |
| `ts-rs` | `tauri-specta 2.0.0-rc` | 后者仍为 RC；首版不应把 IPC 契约建立在预发布依赖上 |
| NSIS currentUser | WiX MSI | 首版锁定当前用户、无管理员安装；NSIS 对 per-user 模式更直接 |
| universal DMG | 分离 Intel/ARM DMG | 单一下载更适合非技术用户；代价仅是包体积增加 |
| Artifact Signing | 仓库/CI 中长期 PFX | 托管短期证书、OIDC 和审计更适合自动发布；PFX 只作为无法使用服务时的后备 |
| `tauri-plugin-wdio` | 传统 `tauri-driver` | 新 WDIO 插件覆盖 macOS；传统方案在 WKWebView 上有平台限制 |

## What NOT to Use

| Avoid | Why | Use Instead |
|---|---|---|
| `tauri-plugin-sql` | 让 WebView 直接接触数据库，违背 Rust 后端独占 SQLite | `rusqlite` repository + typed commands |
| `tauri-plugin-store` | 与锁定 SQLite 状态源重复，造成双写 | SQLite settings table |
| `tauri-plugin-fs` / `shell` / `http` 的前端权限 | XSS/前端 bug 会升级为任意文件、命令或网络访问 | 窄 Rust commands |
| `tauri-plugin-log` 作为唯一日志方案 | 难以保证所有字段先经过领域级秘密脱敏和 7 天策略 | `tracing` + redaction layer + startup retention cleanup |
| OS keychain / `keyring` | 与明文凭据 ADR 冲突，也会让导出/完整显示语义复杂化 | SQLite 明文 + `secrecy` 防意外日志 |
| 自动数据库 reset | migration 失败会丢用户凭据和环境关联 | 迁移前备份、事务回滚、拒绝继续 |
| 仅 `std::fs::rename` | 未覆盖 Windows 替换语义和父目录耐久性 | Win32 Replace/Move + POSIX fsync sequence |
| 正则表达式重写整个 TOML | 容易误改用户字段、重复区块和注释 | `toml_edit` + 管理边界验证 |
| 解析本地化 `wsl --list --verbose` | 状态文本随系统语言变化 | `--quiet`/`--running --quiet` + WSL API |
| `cmd /c`/PowerShell 拼接 WSL 命令 | 发行版名、路径、API Key 产生注入与转义错误 | `Command` argv + stdin payload |
| 仅按进程名识别宿主应用 | 易误判同名进程且无法确认打包身份 | exe path/package ID/bundle ID 复核 |
| 在生产包编译 WDIO plugin | localhost WebDriver 是高危调试入口 | dev-only/e2e Cargo feature |
| TypeScript 7.0.2 | 当前 lint 生态尚未完整兼容 | TypeScript 6.0.3，等 `typescript-eslint` 支持后升级 |
| ESLint 10.8.0 | `eslint-plugin-jsx-a11y` peer range 未覆盖 | ESLint 9.39.5 maintenance line |
| Tauri 当前源码之外的系统 NSIS 3.12 覆盖 | Tauri 2.11 bundler 测试的是其固定 NSIS 3.11 | 使用 Tauri 自动下载的 toolchain |
| 静默 updater 或自动安装 | 违反用户控制的更新行为 | 每日检查 + 显式确认 |
| 自动终止/重建 CLI 终端会话 | 无法可靠恢复原终端、cwd、TTY 和任务上下文 | 平台 adapter 按锁定重启流程实现并在阶段内专项验证 |

## Version Compatibility

| Package A | Compatible With | Notes |
|---|---|---|
| Node `24.18.0` | pnpm `11.20.0`, Vite `8.2.0`, Vitest `4.1.10`, jsdom `30.0.1` | jsdom 30 要求 Node 24.15+，因此不能退回早期 Node 24 |
| TypeScript `6.0.3` | `typescript-eslint 8.66.0` | peer 要求 `<6.1.0`；TS 7 暂缓 |
| ESLint `9.39.5` | jsx-a11y `6.10.2`, typescript-eslint `8.66.0` | ESLint 10 暂缓 |
| React `19.2.8` | RTL `16.3.2`, RHF `7.84.0`, TanStack Query `5.101.4`, RAC `1.20.0` | 官方 peer ranges 覆盖 React 19 |
| Vite `8.2.0` | plugin-react `6.0.5`, Vitest `4.1.10` | 使用同一 Node LTS |
| Rust `1.97.1` | Tauri `2.11.5`, reqwest `0.13.4`, toml_edit `0.25.13`, sysinfo `0.39.6` | sysinfo MSRV 1.95 是关键下限之一 |
| Tauri `2.11.5` | CLI `2.11.4`, API `2.11.1` | Tauri 包独立发布，不要求 patch 相同；以官方当前版本 + lockfile 为准 |
| WebdriverIO `9.30.1` | `@wdio/tauri-service 1.3.0`, `tauri-plugin-wdio 1.3.0` | 只用于 e2e 构建 |
| `rusqlite 0.40.1` | bundled `libsqlite3-sys 0.38.1` | 不依赖操作系统 SQLite |

## Roadmap Implications

1. **先固定工具链和安全边界**：建立 rust-toolchain、pnpm、capability、typed command、redaction 和 fake clock/network。
2. **先做 SQLite + config transaction kernel**：这是后续供应商、切换、WSL2、更新传播的共同风险底座；必须同时建立历史 migration 与故障注入测试。
3. **再做供应商验证协议**：reqwest/SSE/tool loop 完整闭环，先用本地 axum fixture，不依赖 UI。
4. **随后实现原生环境 adapter**：Codex 配置格式需要阶段性 spike；Windows/macOS process identity 与重启也必须在真机验证。
5. **WSL2 单独成高风险阶段**：使用官方 WSL API/参数，不解析本地化表格；必须有专用 WSL2 runner。
6. **Linux 脚本 generator 与 shell matrix 独立验收**：生成器在 Rust，产物无运行时依赖。
7. **打包/签名/updater 不应留到最后一天**：早期建立 unsigned preview，随后尽快跑通 Artifact Signing、Developer ID/notarization、Tauri updater 双签名。

## Confidence Assessment

| Area | Level | Reason |
|---|---|---|
| Core versions | MEDIUM | 官方 npm/crates/Rust/Node 元数据已核验，但 patch 版本会继续变化 |
| Tauri plugins/API | MEDIUM | 官方文档与 registry 一致；插件版本独立、需 lockfile |
| SQLite/config stack | MEDIUM | crates 能力明确；Codex 外部配置格式仍需目标版本 fixture 验证 |
| Windows APIs/WSL2 | MEDIUM | Microsoft API 稳定；宿主应用包身份与 WSL2 真机行为需实施阶段验证 |
| macOS APIs/signing | MEDIUM | Apple/Tauri 流程明确；证书、notarization 与 universal 包需真实账号验证 |
| Test stack | MEDIUM | 版本和平台支持已核验；Tauri macOS WebDriver 是较新的测试路径 |
| Windows ARM64 CI | MEDIUM | 官方 runner 可用但仍为 public preview，已建议自托管后备 |

## Sources

### Toolchain and registries

- https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/
- https://nodejs.org/en/blog/release/v24.18.0
- https://registry.npmjs.org/@tauri-apps%2fcli/latest
- https://registry.npmjs.org/react/latest
- https://registry.npmjs.org/vite/latest
- https://registry.npmjs.org/typescript/latest
- https://crates.io/api/v1/crates/tauri
- https://crates.io/api/v1/crates/rusqlite
- https://crates.io/api/v1/crates/reqwest
- https://crates.io/api/v1/crates/toml_edit

### Tauri

- https://v2.tauri.app/release/tauri/v2.11.0/
- https://v2.tauri.app/learn/system-tray/
- https://v2.tauri.app/plugin/single-instance/
- https://v2.tauri.app/plugin/autostart/
- https://v2.tauri.app/plugin/dialog/
- https://v2.tauri.app/plugin/notification/
- https://v2.tauri.app/plugin/updater/
- https://v2.tauri.app/distribute/windows-installer/
- https://v2.tauri.app/distribute/sign/windows/
- https://v2.tauri.app/distribute/sign/macos/
- https://v2.tauri.app/distribute/dmg/
- https://v2.tauri.app/develop/tests/webdriver/
- https://v2.tauri.app/reference/config/
- https://raw.githubusercontent.com/tauri-apps/tauri/dev/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs

### Microsoft / Windows / WSL

- https://learn.microsoft.com/windows/wsl/basic-commands
- https://learn.microsoft.com/windows/win32/api/wslapi/nf-wslapi-wslgetdistributionconfiguration
- https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew
- https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw
- https://learn.microsoft.com/windows/win32/toolhelp/taking-a-snapshot-and-viewing-processes
- https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-enumwindows
- https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-iapplicationactivationmanager
- https://learn.microsoft.com/azure/artifact-signing/how-to-signing-integrations
- https://learn.microsoft.com/windows/win32/seccrypto/signtool

### Apple / macOS

- https://developer.apple.com/documentation/appkit/nsrunningapplication
- https://developer.apple.com/documentation/appkit/nsworkspace
- https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution
- https://developer.apple.com/help/account/certificates/create-developer-id-certificates
- https://docs.rs/objc2-app-kit/latest/objc2_app_kit/struct.NSRunningApplication.html
- https://docs.rs/objc2-app-kit/latest/objc2_app_kit/struct.NSWorkspace.html

### CI, tests, and external protocol

- https://docs.github.com/actions/reference/runners/github-hosted-runners
- https://github.com/tauri-apps/tauri-action/releases/tag/action-v1.0.0
- https://github.com/koalaman/shellcheck/releases/tag/v0.11.0
- https://github.com/bats-core/bats-core/releases/tag/v1.14.0
- https://developers.openai.com/codex/config-reference/

---
*Stack research for: GPTEasy*
*Researched: 2026-08-05*
