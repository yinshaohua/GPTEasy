# Architecture Research

**Domain:** 跨平台桌面伴侣、原生 Codex 环境配置管理、WSL2 配置切换与独立 Linux shell function 导出  
**Project:** GPTEasy  
**Researched:** 2026-08-05  
**Confidence:** MEDIUM（锁定的领域/技术决策为 HIGH；外部平台 API 与 Codex 当前配置契约为 MEDIUM，需在目标平台验收）

## 研究结论

GPTEasy 不应被实现成“React 调用几个文件操作 command”的桌面脚本，而应被实现成一个**本地、单实例、Rust 权威的配置编排器**。React 是展示和交互层，SQLite 是应用内部状态的唯一持久化来源，Codex 配置文件与导出的 Linux 切换脚本是外部副作用目标。所有副作用都经过 Rust use case、计划/执行/校验流程和可恢复的操作日志。

最重要的结构性决策是把“供应商目录状态”和“环境实际状态”分开。供应商使用不可变供应商 ID，并以验证成功后的供应商修订版本保存；每个受管环境分别记录期望修订版本、已应用修订版本、配置指纹和待重启状态。这样，原生 Codex 环境与每个 WSL2 环境可以独立成功或失败，不需要假装 SQLite 与多个配置文件之间存在分布式事务。

配置写入应采用**版本化凭据槽位 + 原子配置提交**。供应商 API Key 明文保存在 SQLite，也按锁定决策写入环境，但不应把 API Key 放在进程参数、日志或 Tauri event 中。对 Codex 的用户级 `config.toml` 和环境凭据文件先写入新的、版本化的环境变量，再原子替换引用该变量的配置；外部文件修改后重新读取并校验。更新供应商时，旧修订版本和旧备份在保留期内不可过早清理。

实现顺序必须先解决跨平台配置安全和恢复，再做 UI。React 页面、托盘、WSL2 批量切换和 Linux 脚本导出都应复用同一套领域模型与配置物化器。最早的 Windows spike 必须验证“停止的 WSL2 环境不启动时如何获得可展示的默认用户名称”，以及“立即重启”对桌面应用与 CLI 的真实边界；这两个问题不能通过静默启动或强制杀进程规避。

## Standard Architecture

### System Overview

```text
┌─────────────────────────────────────────────────────────────────────────┐
│ React / Tauri WebView                                                   │
│                                                                         │
│  App shell  Providers  Native  WSL2  Linux script  Settings  Diagnostics │
│       │          │        │      │          │            │         │      │
│       └──────────┴────────┴──────┴──────────┴────────────┴─────────┘      │
│                    typed invoke client / progress channels              │
├─────────────────────────────────────────────────────────────────────────┤
│ Tauri shell                                                             │
│  command DTO boundary · tray menu · window lifecycle · capabilities     │
│  notification bridge · updater bridge · single-instance bootstrap       │
├─────────────────────────────────────────────────────────────────────────┤
│ Rust application layer                                                  │
│  ProviderCatalog · ProviderValidation · NativeSwitch                    │
│  WslSwitch · LinuxExport · Diagnostics · Settings · UpdateCheck          │
│  OperationCoordinator · Recovery/Reconciliation                          │
├─────────────────────────────────────────────────────────────────────────┤
│ Rust domain/core                                                        │
│  Provider ID/revisions · validation state machine · environment state    │
│  restart policy · managed-field patch · script model · redaction rules   │
├─────────────────────────────────────────────────────────────────────────┤
│ Ports / adapters                                                        │
│  SqliteStore · CodexConfigAdapter · AtomicFileWriter · WslPort            │
│  ProcessInventory · DesktopRestart · PlatformPaths · LoginStart           │
│  HttpProviderClient · ShellScriptRunner · Updater · DiagnosticsSink       │
├─────────────────────────────────────────────────────────────────────────┤
│ External state                                                          │
│  SQLite (application state)       ~/.codex/config.toml + credential file  │
│  WSL2 user files                  Linux user config + exported function   │
│  Codex/host processes              signed update metadata/artifacts        │
└─────────────────────────────────────────────────────────────────────────┘
```

**单向依赖规则：**

```text
React → Tauri command/DTO → Application use case → Domain + Ports
                                                        ↓
                                  Adapters → SQLite/files/network/OS/WSL
```

- `gpteasy-core` 不依赖 Tauri、SQLite、React、平台 API 或网络客户端。
- `gpteasy-application` 只依赖领域类型和端口；它不直接调用 `std::process::Command`、文件系统或 HTTP。
- `src-tauri` 负责壳层装配和 DTO 转换，不承载供应商规则。
- React 不读 SQLite、不读取 Codex 文件、不执行 WSL 命令，不把配置写入浏览器存储。
- 托盘菜单和设置窗口调用同一 application use case；不复制一套托盘专用切换逻辑。

### Component Responsibilities

| Component | 所有权 | 责任 | 不负责什么 |
|-----------|--------|------|------------|
| `TauriAppShell` | `src-tauri` | 单实例、启动顺序、托盘、窗口关闭拦截、系统通知、能力声明、前端加载 | 供应商验证和文件编辑 |
| `CommandFacade` | `src-tauri/api` | 将窄 DTO 转成 use case 输入；返回脱敏投影、操作 ID 和进度 | 绕过 use case 直接操作 OS |
| `AppProjection` | application | 组合供应商目录、原生 Codex 环境、WSL2、设置和操作摘要供 UI 使用 | 保存完整 API Key |
| `ProviderCatalog` | application/domain | 内置推荐供应商、不可变供应商 ID、供应商修订版本、验证后替换、删除约束 | 修改任何 Codex 配置文件 |
| `ProviderValidation` | application/domain | 模型发现、默认模型确认、Responses API 流式响应、工具调用闭环状态机 | 将未验证草稿写入目录 |
| `ProviderHttpClient` | infra | HTTPS/回环 HTTP 策略、超时、重定向约束、流事件解码、大小限制 | 决定供应商是否可保存 |
| `CodexConfigModel` | core | 解析用户级 Codex 配置、识别 managed 字段、构造最小 patch、计算指纹 | 选择备份目录或执行替换 |
| `NativeCodexAdapter` | platform | 定位当前用户默认 Codex 配置，识别外部配置，应用代理 API/原厂登录模式 | 扫描项目配置或其他用户 |
| `WslAdapter` | platform/windows | 被动发现 WSL2 环境；用户确认后以 WSL2 默认用户执行读写；恢复原停止状态 | macOS/Linux 的 WSL2 空实现 |
| `EnvironmentSwitch` | application | 将供应商修订版本传播到一个环境，协调备份、原子替换、重读验证和待重启 | 让多个环境共享一个运行时状态 |
| `OperationCoordinator` | application | 资源锁、计划 token、操作日志、取消点、崩溃恢复和重试 | 长期保存明文错误正文 |
| `ProcessInventory` | platform | 识别桌面 Codex 与本机 Codex CLI，记录 PID、启动时间、可重启能力 | 按进程名杀掉所有匹配项 |
| `RestartCoordinator` | application/platform | 处理切换前确认、桌面应用优雅重启、CLI 待重启、轮询和结果 | 强制终止仍在运行的 Codex |
| `SafeFileWriter` | platform | 同目录临时文件、权限复制、sync、时间戳备份、平台原子替换、重读校验 | 解析 TOML 或判断业务字段 |
| `LinuxScriptGenerator` | application | 由全部已验证供应商生成 Bash 4+/Zsh 5+ 自包含 function，专用转义和警告文案 | 连接 GPTEasy 或修改导出目标 |
| `DiagnosticsService` | application/infra | 七天脱敏日志、环境摘要、用户主动诊断导出、API Key/URL 脱敏 | 自动上传 |
| `UpdateService` | application/infra | 每日最多一次检查、签名更新摘要、用户确认后的下载安装 | 静默安装或携带供应商数据 |
| `SqliteStore` | infra | 独占连接、顺序迁移、事务、数据库备份、版本拒写、关系约束 | 保存 Codex 外部文件内容 |
| React feature boundaries | `src/` | 页面结构、草稿状态、验证进度展示、可访问性、国际化、错误焦点 | 任何安全策略或系统集成 |

## Recommended Project Structure

```text
/
├── src/                                  # TypeScript/React presentation
│   ├── app/
│   │   ├── App.tsx                       # 路由、首次加载、projection 订阅
│   │   ├── navigation/                   # 供应商/WSL2/Linux 脚本/设置
│   │   └── shell/                        # 页面布局、托盘唤起后的窗口状态
│   ├── bridge/
│   │   ├── commands.ts                   # 唯一 invoke 入口
│   │   ├── channels.ts                   # 长操作进度订阅
│   │   └── dto.ts                        # 与 Rust command DTO 对齐的类型
│   ├── features/
│   │   ├── providers/                    # 列表、完整供应商页面、验证进度
│   │   ├── native-environment/           # 当前供应商、外部配置、待重启
│   │   ├── wsl2/                         # 行内选择、临时启动、批量结果
│   │   ├── linux-script/                 # Shell/语言选择、凭据警告、导出
│   │   ├── settings/                     # 登录模式、登录启动、主题、语言
│   │   └── diagnostics-update/           # 日志、导出、更新
│   ├── state/
│   │   ├── appProjection.ts              # Rust 权威状态缓存
│   │   └── operationState.ts             # 只保存 operation_id 和进度摘要
│   ├── i18n/                             # 简体中文/英语
│   └── components/                       # 无业务含义的可访问控件
├── crates/
│   ├── gpteasy-core/
│   │   ├── provider.rs                   # ID、修订版本、草稿、验证状态
│   │   ├── environment.rs                # 原生/WSL2 状态与绑定
│   │   ├── config_patch.rs               # managed field patch 与指纹
│   │   ├── validation.rs                 # 三阶段状态机
│   │   ├── script.rs                     # 方言无关脚本模型
│   │   ├── secret.rs                     # 明文值的受控包装和脱敏
│   │   └── error.rs                      # 稳定错误码，不携带凭据
│   ├── gpteasy-application/
│   │   ├── ports.rs                      # 所有外部能力 trait
│   │   ├── provider_catalog.rs
│   │   ├── provider_validation.rs
│   │   ├── environment_switch.rs
│   │   ├── restart.rs
│   │   ├── wsl.rs
│   │   ├── linux_export.rs
│   │   ├── diagnostics.rs
│   │   ├── updates.rs
│   │   └── recovery.rs
│   └── gpteasy-infra/
│       ├── sqlite/
│       ├── codex_toml/
│       ├── atomic_files/
│       ├── provider_http/
│       ├── diagnostics_log/
│       └── update_client/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                       # Tauri builder 与平台装配
│   │   ├── commands/                     # DTO、command handler
│   │   ├── tray.rs
│   │   ├── lifecycle.rs
│   │   ├── capabilities/
│   │   └── platform/
│   │       ├── windows.rs
│   │       └── macos.rs
│   ├── capabilities/                     # 每个窗口/插件的最小权限
│   └── tauri.conf.json
├── migrations/                           # 永久保留、按序编号
├── fixtures/
│   ├── codex/
│   ├── sqlite/
│   ├── wsl/
│   └── scripts/
└── .github/workflows/                    # 多平台测试、签名、打包
```

### Structure Rationale

- `gpteasy-core` 与 `gpteasy-application` 分离，确保配置安全、供应商验证和恢复逻辑可以在没有 Tauri WebView、真实 WSL2 或桌面进程的情况下测试。
- `gpteasy-infra` 只实现端口，不向上泄漏 `rusqlite::Error`、`reqwest::Error`、Windows 错误对象或命令行原文；application 层将它们映射成稳定错误码和技术详情。
- `src-tauri` 保持很薄。Tauri command 是进程内 RPC 边界，不能成为第二个领域层。
- React 按 UI-SPEC 的页面边界组织，而不是按 Rust 模块镜像。每个 feature 通过 `bridge/commands.ts` 请求 `AppProjection` 或启动一个有 operation ID 的长操作。
- `fixtures/` 是跨平台契约的一部分：历史数据库、真实 Codex 配置样本、外部配置样本、WSL 命令输出和脚本目标文件都应版本化；fixture 不得包含真实 API Key。

## Architectural Patterns

### Pattern 1: 后端权威的窄 Command API

**What:** React 只能使用显式的 query/mutation command。查询返回脱敏的 `AppProjection`；长操作返回 `operation_id`，进度通过 Tauri Channel 发送；低频状态变化通过事件通知前端刷新投影。

**When to use:** 所有 UI 功能，包括托盘切换、供应商验证、WSL2 批量切换、脚本生成和更新。

**Trade-offs:** command/DTO 数量比暴露一个通用 RPC 多，但权限、审计、错误处理和未来迁移都更清晰。不要让前端传入任意路径、任意命令或任意 TOML 片段。

建议的 command 族：

```text
read_app_projection()
read_provider(provider_id)
reveal_provider_key(provider_id)                 # 明确动作，返回一次性响应
discover_provider_models(draft, channel)
validate_provider(draft, channel)                # 成功后返回短期 validation_token
save_validated_provider(validation_token)
delete_provider(provider_id)

plan_native_switch(provider_id)                  # 返回 restart plan token
apply_native_switch(plan_token, restart_policy)
set_factory_login_mode(enabled)

refresh_wsl_inventory()
plan_wsl_switch(environment_id, provider_id)
apply_wsl_switch(plan_token, restart_policy)
plan_wsl_batch(provider_id, environment_ids)

preview_linux_script(shell, language)
copy_linux_script(shell, language)
save_linux_script(shell, language, destination)

export_diagnostics(destination)
check_for_update()
install_confirmed_update(update_token)
```

`validation_token` 和 `plan_token` 必须由 Rust 生成，短期有效，并绑定：

- 规范化后的供应商草稿哈希或供应商修订版本；
- 当前配置文件指纹；
- 目标环境 ID；
- 当前应用实例和操作；
- 过期时间；
- 允许的 restart policy。

执行 command 必须再次检查 token、数据库状态、文件指纹和进程快照，避免“用户打开确认对话框后文件被外部程序改写”的 TOCTOU 问题。

### Pattern 2: 领域核心 + Ports/Adapters

**What:** 领域层定义 `Provider`, `ProviderRevision`, `EnvironmentBinding`, `ValidationSession`, `RestartPlan`, `ConfigPatchPlan` 等类型；应用层定义端口；平台和基础设施只实现端口。

**When to use:** 所有涉及 Windows/macOS/WSL2/文件/网络/数据库的能力。

**Trade-offs:** 端口数量较多，但能把 Windows 的 WSL 命令细节、macOS 的应用重启细节和 SQLite 错误隔离在实现层。不要为没有不同测试行为的简单函数建立空泛抽象。

关键端口应保持能力导向：

```rust
trait CodexEnvironment {
    fn inspect(&self) -> Result<EnvironmentSnapshot>;
    fn prepare_switch(&self, target: &ProviderRevision) -> Result<ConfigPatchPlan>;
    fn apply(&self, plan: ConfigPatchPlan) -> Result<AppliedConfig>;
}

trait ProcessControl {
    fn snapshot(&self) -> Result<ProcessSnapshot>;
    fn request_desktop_restart(&self, target: &KnownProcess) -> Result<RestartRequest>;
    fn observe_until_changed(&self, snapshot: &ProcessSnapshot) -> Result<ProcessOutcome>;
}

trait AtomicFileOps {
    fn backup(&self, path: &Path, retention: usize) -> Result<BackupRef>;
    fn stage(&self, path: &Path, bytes: &[u8], mode: FileMode) -> Result<StagedFile>;
    fn commit(&self, staged: StagedFile, destination: &Path) -> Result<()>;
    fn verify(&self, path: &Path, expected: &ConfigFingerprint) -> Result<()>;
}
```

业务 use case 不应知道 `wsl.exe`、`ReplaceFileW`、`NSRunningApplication` 或 Tauri `AppHandle` 的存在。

### Pattern 3: 计划/执行/校验（Plan → Execute → Reconcile）

**What:** 每个外部副作用都分为：

1. `plan`：读取当前状态、验证前置条件、生成变更摘要；
2. `execute`：登记操作、备份、暂存、原子提交；
3. `reconcile`：重新读取外部状态，确认实际应用修订版本，更新 `applied_revision` 和待重启状态。

**When to use:** 原生 Codex 切换、供应商配置传播、WSL2 切换、Linux function 运行时切换。

**Trade-offs:** 代码比“写完就返回”多，但可以处理文件被外部改写、部分环境失败、进程仍在运行和应用崩溃。不要把数据库事务保持到网络请求、用户确认或进程重启结束。

### Pattern 4: 修订版本化供应商与期望/实际双状态

**What:** 供应商 ID 永久不变；每次验证成功产生新的 `ProviderRevision`。环境绑定同时记录：

```text
desired_provider_id / desired_revision
applied_provider_id / applied_revision
observed_mode
config_fingerprint
pending_restart
last_operation
```

**When to use:** 新增、编辑、传播、切换、恢复和重试。

**Trade-offs:** 需要清理旧修订版本和备份，但避免了更新供应商时覆盖旧 Key、把部分成功标成成功，或删除仍被环境引用的供应商。

### Pattern 5: 版本化凭据槽位，配置文件作为提交点

**What:** 对每个供应商修订版本生成稳定、不可猜测但不含明文的 env key，例如：

```text
GPTEASY_<PROVIDER_ID_COMPACT>_R<REVISION>_API_KEY
```

在受管凭据文件中先写入新槽位，再在 `config.toml` 中将该供应商的 `env_key` 指向新槽位。配置文件原子替换成功并重新读取后，才把操作标为已应用；旧槽位在备份保留期内保留。

**When to use:** 原生 Codex 环境、每个 WSL2 环境和 Linux 切换脚本。

**Trade-offs:** 需要同时处理 `config.toml` 与凭据文件，也会暂时保留旧 Key；但它避免了跨文件写入时“配置已指向新变量、变量还不存在”的窗口，并让回滚可以恢复同一修订版本。明文 Key 仍受 ADR 约束，只在受控存储/导出中出现。

### Pattern 6: 受控敏感值与脱敏错误

**What:** API Key 以 `SecretString`/`Sensitive<T>` 之类的 Rust 类型传递，禁止 `Debug`、`Display`、序列化到诊断 DTO。日志事件只允许 provider ID、修订版本、目标主机名、HTTP 状态、稳定错误码和 operation ID。

**When to use:** 验证、SQLite 读取、配置物化、WSL 写入、脚本预览、通知和诊断导出。

**Trade-offs:** 排障需要显式的技术详情白名单；这是预期成本。错误正文不能直接从 HTTP 客户端或子进程输出透传到 UI。

## Backend Component Decomposition

### 1. Tauri 壳层和启动编排

`TauriAppShell` 只做装配和生命周期：

1. 初始化平台路径和单实例锁；
2. 初始化脱敏日志；
3. 打开 SQLite，执行迁移前备份、迁移和版本检查；
4. 读取操作日志，执行安全恢复/重新校验；
5. 扫描原生 Codex 环境与 WSL2（Windows）；
6. 创建 application services、command state 和托盘菜单；
7. 创建设置窗口或响应第二实例唤起；
8. 在首屏可用后触发每日一次的更新检查。

窗口关闭事件只隐藏设置窗口并保持托盘驻留。显式退出才停止后台服务。退出前不能强制中止正在提交的原子写入；可以在安全取消点阻止新操作并等待当前 commit/reconcile 完成。

Tauri capabilities 只允许本地窗口需要的命令和插件能力。GPTEasy 不加载远程页面、不授予远程域名 IPC、不把 shell plugin 暴露给 React；Rust 后端可以直接使用受控的 `std::process::Command` 和平台 API。

### 2. 应用服务和并发控制

`OperationCoordinator` 是所有写操作的唯一入口。锁按资源分层：

| 锁 | 覆盖范围 | 目的 |
|----|----------|------|
| `catalog_lock` | 供应商验证成功保存、编辑、删除、DayWay 模板更新 | 保证 ID/修订版本和删除约束一致 |
| `native_lock` | 原生 Codex 环境的配置文件和重启计划 | 避免托盘与设置页面同时覆盖配置 |
| `wsl:<id>` | 单个 WSL2 环境 | 允许不同发行版独立失败，避免同一发行版并发启动/终止 |
| `script_export_lock` | 剪贴板/文件导出请求 | 防止前端重复导出和错误目的地 |
| `migration_lock` | 启动阶段 | 迁移时不开放任何写 command |

验证网络请求不持有 SQLite 写事务。完成验证后，使用短事务插入新的 `ProviderRevision`，再根据首次自动切换或传播规则创建环境操作。

`OperationCoordinator` 应区分：

- `CancelableBeforeCommit`：模型发现、流式验证、用户确认等待；
- `NonCancelableCommit`：备份已完成后的原子替换；
- `NeedsReconcile`：进程重启、WSL 临时启动恢复、跨文件提交后崩溃。

### 3. 供应商目录和供应商验证

#### 供应商目录

DayWay 是一个固定的内置推荐供应商记录或模板记录，具有发布者定义的稳定 ID。模板更新只改变模板元数据；未配置推荐供应商可以更新，已验证 DayWay 不得静默覆盖服务地址、API Key 或默认模型。

建议的持久化关系：

```text
providers
  provider_id (immutable primary key)
  kind (builtin_recommended | user_created)
  display_name
  is_builtin
  display_order
  deleted_at (仅允许未被环境引用时真正删除)

provider_revisions
  provider_id + revision (unique)
  base_url
  api_key_plaintext
  default_model
  verified_at
  source (user_validation | builtin_template)
```

未验证草稿只存在于 command 调用和 validation session 内，不进入 `providers` 或 `provider_revisions`。编辑已验证供应商时，旧修订版本保持可用；新地址、API Key 或默认模型必须在新验证成功后一次性成为新修订版本。

#### 验证状态机

```text
Draft
  ↓ 地址 + API Key 合法且通过安全服务地址策略
DiscoveringModels
  ↓ 找到可解析模型且用户选择默认模型
StreamingResponse
  ↓ Responses API 流事件闭环成功
ToolCallRoundTrip
  ↓ function_call / function_call_output / final response 成功
ValidatedCandidate
  ↓ save_validated_provider(validation_token)
CommittedProviderRevision
```

任何失败都停止后续步骤。验证 session 保存：

- 规范化地址、默认模型和 API Key 的摘要；
- 验证通过时间；
- 操作 ID；
- 仅用于后续保存的短期内存句柄；
- 非敏感技术详情。

验证客户端必须：

- 拒绝非回环 HTTP；远程 HTTPS 使用正常证书校验，不提供绕过开关；
- 禁止把 API Key 放在 URL、命令行或日志；
- 对重定向重新检查安全服务地址和目标主机，最好默认不自动跨主机重定向；
- 设置连接、整体、流式空闲和响应大小限制；
- 只保留需要的事件类型，不保存完整响应；
- 对工具调用使用固定的无副作用测试工具，例如返回固定 JSON 的本地函数，不执行供应商返回的任意命令；
- 将供应商名称、完整服务地址、请求正文和响应正文从错误/诊断中排除。

### 4. Codex configuration adapters

#### 统一模型

所有目标环境通过同一个无平台依赖的 `CodexConfigModel` 表示：

```text
CodexConfigSnapshot
  file set:
    config.toml
    credential file (.env or equivalent)
  root model / model_provider
  model_providers.gpteasy_<provider-id>
  GPTEasy-managed credential slots
  unmanaged fields preserved as source text/AST
  fingerprint
```

Rust 端应使用保留未知字段、注释和格式信息的 TOML 编辑器（例如 `toml_edit` 类库），而不是反序列化到一个只包含 GPTEasy 字段的 struct 后重写整个文件。解析失败、重复表、重复 managed block、未知的冲突字段或文件指纹在确认后变化时，必须停止并要求用户处理，不得自动清空或“修复”。

#### 原生 Codex 环境

`NativeCodexAdapter` 只定位当前操作系统用户的默认 `~/.codex/config.toml` 和 Codex 使用的用户级凭据文件。它：

- 只管理默认用户，不扫描项目级配置、机器级配置或自定义路径；
- 使用以 immutable provider ID 命名的 provider alias，不能依赖显示名称、地址指纹或数组顺序；
- 使用 `model_provider` 和 `model` 选择代理 API 模式的当前供应商；
- 进入原厂登录模式时不修改 Codex auth 存储，不读取或复制登录令牌；
- 退出原厂登录模式时只恢复 GPTEasy 以前记录的受管选择，发现外部修改则呈现外部配置；
- 首次纳入外部配置前要求地址/模型唯一匹配且用户重新验证；无法唯一匹配则保持外部配置；
- 每次操作前读取真实文件，而不是把 SQLite 的上次状态当作事实。

保存“原生 Codex 环境基线”时只记录受管字段的存在性和值、指纹和时间，不保存整份配置作为数据库副本。完整文件由配置备份保留。

#### 多文件安全提交

配置操作采用以下顺序：

```text
读取并指纹校验
  ↓
创建 config.toml 与凭据文件备份
  ↓
写入新版本凭据槽位到同目录临时文件并 sync
  ↓
写入引用新 env_key 的 config.toml 临时文件并 sync
  ↓
原子替换 config.toml（业务提交点）
  ↓
重新读取 config.toml + 凭据文件并验证 provider/revision/指纹
  ↓
更新 applied_revision / pending_restart
  ↓
仅在备份保留期外清理旧凭据槽位
```

如果配置提交点之前失败，删除临时文件并保留旧配置。如果提交点之后应用崩溃，启动恢复读取操作日志和实际文件：匹配目标指纹则完成操作，不匹配且旧文件仍未被外部修改则用备份恢复，否则标为“需要用户决定的恢复”，不静默覆盖。

#### 配置保留与外部编辑

`CodexConfigAdapter` 返回三种结果：

```text
Managed(provider_id, revision)
External(summary, fingerprint)
Unavailable(reason)
```

托盘和设置页展示 `External` 的地址/模型摘要，但不能把它当作已验证供应商。任何 GPTEasy 写入都携带读取时的指纹和 managed field precondition；外部程序改动后，操作失败而不是覆盖。

### 5. 进程和重启协调

`ProcessInventory` 与 `RestartCoordinator` 必须分开。进程快照包含：

- PID 和启动时间（避免 PID 复用）；
- 进程类型：桌面宿主应用、本机 Codex CLI、未知；
- 可验证的 executable path/bundle identifier；
- 是否使用原生 Codex 环境；
- 是否可请求优雅关闭；
- 当前配置读取时机未知/已启动。

匹配不能只依赖 `codex.exe` 或 `codex` 名称。Windows 应结合可执行文件路径、文件签名/产品信息和进程树；macOS 应结合 bundle identifier、bundle path 和进程类型；CLI 只作为已知命令路径/启动参数模式的观测结果。快照不应把完整命令行写入日志，因为命令行可能包含用户秘密。

切换流程：

```text
plan_native_switch
  → 读取当前供应商/外部配置
  → 枚举 Codex 进程
  → 生成当前→目标摘要与 RestartPlan
  → React 显示重启确认

apply_native_switch(plan_token, policy)
  → 重新校验文件指纹和进程快照
  → 备份并提交配置
  → policy=Later: 标记待重启
  → policy=Immediate:
       请求桌面宿主应用优雅关闭并重新激活
       不强制终止 CLI；等待其退出或提示用户在终端重启
  → 重新扫描并返回每个进程的 RestartOutcome
```

“切换成功”和“所有进程已经读取新配置”是两个状态。CLI 仍然存在时，配置切换可以成功，但原生 Codex 环境仍标记为待重启。GPTEasy 不应凭空重启 CLI，也不应杀掉未确认的进程；如果用户选择立即重启，后端可以执行桌面应用的可验证优雅重启，并把 CLI 的剩余动作明确报告为用户操作。

### 6. WSL2 集成

WSL2 只在 Windows 编译和注册 `WslPort`。所有调用使用参数数组，不经过 `cmd.exe` 或 `sh -c` 拼接；供应商数据通过 stdin 或受控临时文件传输，不出现在 `wsl.exe` 参数中。

#### 被动发现

`refresh_wsl_inventory` 只能做：

1. 读取发行版名称集合；
2. 读取运行集合；
3. 读取 WSL2 版本并过滤 WSL1；
4. 获取默认 UID/平台元数据；
5. 读取 SQLite 中上次观察到的当前供应商、待重启和默认用户缓存。

不能因为打开 WSL2 页面而启动发行版。`--list --verbose` 的输出可能受本地化影响，适配器不应依赖“Running/Stopped”等英文词；可以用 `--list --running --quiet` 得到运行集合，并对版本字段做明确的解析测试。发行版名称必须作为单独参数传递。

**需要早期实机 spike 的风险：** 官方 WSL 配置 API 可提供默认 UID，但“停止发行版且不启动它时取得默认用户名”不是同一件事。不要通过检测阶段执行 `id -un` 来掩盖这一点，因为那会违反“未配置 WSL2 环境”的被动检测语义。实施前必须验证官方 API 与目标 Windows 版本是否能获得用户名；否则应在 WSL2 阶段将该状态显式建模为未解析，而不是静默启动或显示猜测值。

#### 用户确认后的切换

```text
WSL inventory row
  → user selects verified provider
  → if stopped: display WSL2 临时启动 confirmation
  → acquire wsl:<distro> lock
  → start only for this operation, using default user
  → read default user's $HOME and Codex files
  → host Rust computes TOML/env patch
  → send bytes to a minimal stdin-driven shell writer
  → backup, stage, atomic replace, reread through wsl.exe
  → inspect Codex process state without killing it
  → if originally stopped and no user activity appeared: --terminate
  → persist applied/pending/recovery state
```

写入助手只接收已打开的文件路径、临时文件名和 stdin 字节；路径通过 shell positional parameters 传入并始终双引号引用。不能把供应商名称、地址、Key 或模型拼入 shell source。临时启动恢复必须有明确的 `started_by_gpteasy` 标志；若用户在操作期间启动了其他活动，GPTEasy 应停止自动终止并报告“需要用户处理”。

批量“应用到全部 WSL2”先固定目标发行版快照和用户确认结果，再按发行版顺序执行独立 operation。一个发行版失败不回滚其他已经成功的发行版；结果以每发行版的成功、失败、待重启和恢复需要呈现。

### 7. Linux Script Generator

`LinuxScriptGenerator` 输入只来自已验证 `ProviderRevision` 集合、导出 Shell 和导出语言，绝不读取目标 Linux 当前状态。导出内容始终包含全部已验证供应商，DayWay 验证后排在第一；当前供应商由目标 Linux 上的 function 从配置文件读取，不从 GPTEasy 的原生/WSL2 当前状态继承。

脚本格式应包含：

```text
header: GPTEasy version / export time / shell / warning
metadata: provider ID + revision + display name + host + model
config.toml managed block
credential file managed block
interactive switch function
backup/retention helper
recovery marker helper
```

建议约束：

- `config.toml` 与凭据文件分别使用唯一的 begin/end marker；
- managed block 位置固定在配置文件开头、任何 TOML table 之前，避免 root `model`/`model_provider` 落入错误 table；
- 首次安装若发现 root 字段、marker 或 table 结构不能唯一迁移，备份后停止，不做模糊的正则替换；
- shell-specific quote encoder 只输出单行安全字面量；名称、地址、模型和 Key 中出现 NUL、换行或控制字符时导出失败；
- 使用 `umask 077`、同目录临时文件、`trap` 清理和 `mv`/平台可用的原子替换；
- 修改前每个文件保留最近五份带时间戳备份；
- 使用无秘密的恢复 marker 记录 prepare/staged/config-committed 状态；
- function 只有用户选择供应商后才写入，直接退出不改变任何文件；
- 复制/保存前由 React 展示明文凭据警告，脚本正文只有用户主动查看才显示。

导出物脱离 GPTEasy，因此不能依赖数据库、网络、GPTEasy 可执行文件、Python、Node.js、第三方 TOML 解析器或未来在线同步。脚本的“解析”范围必须被限制在自己生成的 marker 区块和固定格式；不能试图成为通用 TOML 编辑器。

### 8. SQLite application state

SQLite 由 Rust 后端独占访问。推荐一个专用数据库执行器/线程拥有连接，所有 query 和事务通过消息进入，避免 Tauri async command 在 UI 调用期间直接阻塞或多个线程争用连接。

建议的核心表：

| 表 | 关键字段 | 用途 |
|----|----------|------|
| `providers` | `provider_id`, `kind`, `display_name`, `is_builtin`, `deleted_at` | 供应商身份与排序；ID 永不改变 |
| `provider_revisions` | `provider_id`, `revision`, `base_url`, `api_key_plaintext`, `default_model`, `verified_at` | 只保存验证成功的配置修订版本 |
| `environments` | `environment_id`, `kind`, `platform_key`, `display_name`, `last_seen` | 原生 Codex 环境或 WSL2 环境 |
| `environment_bindings` | `environment_id`, desired/applied provider/revision, `mode`, `fingerprint`, `pending_restart` | 分离期望状态与实际状态 |
| `operations` | `operation_id`, `kind`, `target`, `phase`, `intent`, `backup_manifest`, `error_code` | 外部副作用的恢复日志，不存秘密 |
| `settings` | key/value/version | 语言、主题、登录启动、托盘提示等 |
| `update_state` | last check, ignored/version metadata | 每日更新检查节流 |

`provider_revisions.api_key_plaintext` 明确承接 ADR-0001；数据库文件应放在当前用户应用数据目录，文件权限/ACL 由平台适配器设置。它不应被写入诊断导出或普通 projection。

数据库启动顺序：

```text
open user database
  → 确认 application_id
  → 读取 user_version
  → 更高版本：只读拒绝写入
  → 备份到独立迁移备份目录（保留最近三份）
  → BEGIN IMMEDIATE
  → 顺序执行永久迁移
  → 更新 user_version
  → COMMIT
  → 重新打开并运行完整一致性检查
```

迁移失败必须回滚并保留备份，应用进入恢复/只读错误页；绝不能删除数据库、创建空库后继续运行。CI 应保存每个历史版本的数据库样本，并测试从每个样本直接升级到当前版本。

### 9. Diagnostics、更新和安装

#### Diagnostics

诊断日志使用结构化事件和字段白名单。允许字段包括 operation ID、稳定错误码、目标环境类别、发行版名的安全摘要、服务主机名、HTTP 状态、耗时和平台版本；禁止完整 URL path/query、API Key、请求/响应正文、模型输出和完整进程命令行。

`DiagnosticsService` 负责：

- 默认最近七天滚动日志；
- 在写入前和导出前双重脱敏；
- 用户主动选择保存路径；
- 生成环境摘要、应用版本、数据库 schema version、受管环境状态和脱敏日志；
- 对诊断导出做文件大小限制和临时目录清理。

脱敏不能只依赖“从日志中搜索字符串”。根本措施是让敏感类型不可 `Debug`/`Serialize`，网络客户端只返回白名单错误，command DTO 从类型上不返回 Key。

#### Update / packaging

更新检查是 `UpdateService` 的一个普通 application use case：

```text
startup after projection ready
  → check update_state.last_checked_at
  → if < 24h: skip
  → fetch signed metadata only
  → persist check time/result
  → notify only if newer version exists
  → explicit user confirmation
  → download + signature verification
  → install/relaunch through updater plugin
```

发布流水线必须分别验证：

- Windows 10 22H2+ x64/ARM64 的当前用户安装与更新；
- macOS 14+ Intel/Apple Silicon 的代码签名、notarization、更新和用户可写安装位置；
- Tauri updater 工件签名、公钥配置和错误更新元数据；
- 安装升级后 SQLite 迁移备份、待重启状态和旧备份保留。

更新检查不得发送供应商目录、API Key、当前供应商或诊断日志。更新服务失败只影响更新提示，不影响本地供应商管理。

## Data Flow

### Request Flow

```text
[React user action]
        ↓
[typed bridge: DTO + channel]
        ↓
[Tauri CommandFacade]
        ↓
[application use case]
        ↓
[domain validation + resource lock + operation journal]
        ↓
[port adapter: SQLite / HTTP / Codex file / WSL / process]
        ↓
[re-read external state + reconcile]
        ↓
[redacted Result DTO / progress Channel]
        ↓
[AppProjection refresh + page-local UI feedback]
```

### Startup and Reconciliation Flow

```text
Tauri bootstrap
  → singleton and paths
  → redacted logger
  → SQLite backup/migration/version gate
  → recover operations marked staged/committed/needs_reconcile
  → inspect native Codex config
  → passive WSL inventory on Windows
  → build AppProjection
  → build tray from the same projection
  → render React
  → optional daily update check
```

启动恢复的原则是“先观察，再决定写入”。只在操作日志、备份和当前文件指纹都证明安全时自动完成；任何外部修改或歧义进入需要用户决定的恢复状态。

### Provider Validation and Save

```text
React draft (page-local, includes key)
  → validate URL/key locally and in Rust
  → model discovery request
  → user selects default model
  → streaming Responses request
  → tool call request
  → fixed local tool output
  → final response
  → Rust creates short-lived validation_token
  → React requests save(validation_token)
  → SQLite transaction inserts provider revision
  → if first verified provider:
       native switch operation is scheduled
    else:
       existing environment bindings remain unchanged
  → projection refresh
```

验证失败不会改变旧供应商修订版本。编辑页面的 API Key 只在该页面内存中保存；打开已保存供应商默认隐藏，显式 reveal command 才返回完整 Key，离开页面清除草稿。

### Native Codex Switch

```text
tray/settings choose verified provider
  → read native files and current mode
  → inspect process snapshot
  → plan token
  → UI restart pre-confirmation if needed
  → revalidate plan token + file fingerprint
  → SQLite operation = prepared
  → backup config.toml + credential file
  → stage new revisioned credential slot
  → atomically replace config.toml
  → reread and match provider ID/revision
  → applied_revision updated
  → immediate or later restart coordination
  → tray/projection shows current or pending restart
```

托盘切换与设置页面切换必须调用同一 `NativeSwitchUseCase`。托盘不提供 WSL2 快捷操作，也不为已验证供应商之外的记录生成菜单项。

### Provider Update Propagation

```text
validated edit
  → catalog transaction commits new ProviderRevision
  → find environment_bindings using provider_id
  → each environment gets desired_revision = new revision
  → enqueue independent switch operation
  → apply/reconcile per environment
  → applied_revision or failure/recovery state
```

这不是跨环境原子事务。一个 WSL2 环境失败不能把其他已经成功的环境回滚成旧供应商；UI 必须逐项展示结果，并提供重试/恢复。

### WSL2 Flow

```text
refresh_wsl_inventory
  → list names/version/running set/default UID
  → persist observation only
  → user confirms one/batch switch
  → start stopped distro only for the operation
  → execute as WSL2 默认用户
  → host computes patch; stdin transports bytes
  → backup/stage/replace/reread
  → detect Codex process presence
  → terminate only if GPTEasy started it and safe recovery conditions hold
  → persist per-distro result
```

### Linux Script Flow

```text
verified provider revisions
  → generator sorts DayWay first
  → shell/language-specific rendering + quote validation
  → preview/copy/save command
  → explicit plaintext-key warning
  → user manually sources/installs function
  → user invokes function on Linux
  → function reads its own managed block
  → user selects provider or exits
  → backup → credential block stage → config block atomic replace
  → reread marker/revision → report current/pending restart
```

## Persistence and External State Boundaries

### State Ownership Matrix

| 状态 | 权威来源 | SQLite 保存什么 | 读取时机 |
|------|----------|------------------|----------|
| 供应商身份/已验证配置 | SQLite | ID、修订版本、地址、明文 Key、模型、验证时间 | 每次 projection/操作 |
| 原生 Codex 实际配置 | 用户级 Codex 文件 | 期望/已应用修订、指纹、外部配置摘要 | 启动、窗口聚焦、托盘操作前后 |
| WSL2 实际配置 | 各发行版默认用户文件 | 发行版观察值、期望/已应用修订、运行/待重启 | 页面打开、操作前后 |
| Linux 目标配置 | 目标机器文件 | 不回导、不同步 | 仅由导出 function 读取 |
| 运行进程 | OS process table | 最近操作摘要，不持久化 PID | 切换计划/重启轮询 |
| 诊断日志 | 本地日志文件 | 导出索引/设置，不存完整日志 | 操作期间与用户导出 |
| 更新状态 | SQLite | 最近检查时间和版本摘要 | 启动/设置页 |

### 外部文件写入协议

`SafeFileWriter` 的平台实现必须：

1. 确认父目录存在且属于目标用户级路径；
2. 读取原文件并记录文件大小、mtime、内容 hash 和必要权限；
3. 生成时间戳备份，保留最近五份；
4. 在同目录创建随机临时文件，复制必要权限并使用受限权限；
5. 写 bytes、`sync_all`，关闭文件；
6. 在平台封装中替换目标文件；
7. 重新读取并校验预期 managed projection；
8. 只在校验成功后删除超出保留期的旧槽位和临时文件。

Windows 不能假设 Rust `rename` 对已存在目标的语义与 Unix 相同；使用 Windows `ReplaceFileW` 适配器。macOS/Unix 使用同目录 `rename`，并在需要的目录 sync 策略上做平台测试。不要把“先删除目标再 rename”作为原子替换。

## Trust and Failure Boundaries

| 边界 | 不可信/可能改变的对象 | 强制保护 | 失败结果 |
|------|----------------------|----------|----------|
| React → Rust | WebView 输入、草稿、token、路径字符串 | command 白名单、Rust 重验、短期 token、不信任前端状态 | 拒绝 command，不写文件 |
| 供应商网络 | URL、TLS、重定向、HTTP 状态、流事件、工具输出 | 地址策略、证书校验、超时/大小限制、事件状态机、无副作用固定工具 | 验证失败，旧修订保留 |
| SQLite | 历史库、未来版本、迁移错误、连接锁 | application_id、顺序迁移、迁移前备份、事务、版本拒写 | 只读恢复错误，不清空数据 |
| Codex 文件 | 外部编辑、格式歧义、重复表/marker、权限 | 指纹/precondition、保留 AST、最小 patch、备份、原子替换 | 停止写入并提示恢复 |
| 进程 | PID 复用、同名程序、CLI 终端上下文 | PID+启动时间+路径/签名/bundle、优雅关闭、CLI 不强杀 | 配置已切换但标记待重启 |
| WSL2 | 本地化输出、发行版名称、默认用户、启动状态 | 参数数组、被动发现、逐发行版锁、started_by_gpteasy 标记 | 该发行版失败/恢复，不影响其他发行版 |
| 导出脚本 | 用户复制、公开仓库、目标文件损坏 | 明文警告、无云上传、唯一 managed blocks、备份、shell quote | function 停止，不修改歧义文件 |
| 更新 | 更新元数据、工件、安装权限 | 签名校验、HTTPS、用户确认、独立 updater 权限 | 更新失败，不影响本地核心功能 |
| 日志/诊断 | 底层错误正文、子进程输出、截图 | Sensitive 类型、字段白名单、二次脱敏、用户主动导出 | 丢弃危险字段，不阻塞业务 |

**恢复决策：**

- 可确认没有提交：删除临时文件，保持旧配置；
- 可确认已提交并匹配目标：完成 operation；
- 状态未知但旧备份和目标文件都可验证：进入恢复对话框，由用户选择恢复或保留；
- 发现外部修改、重复 marker、未来数据库版本或目标路径不在允许范围：拒绝自动恢复；
- 绝不以删除数据库、覆盖外部配置或强制终止 Codex 作为默认恢复。

## React Frontend Boundaries

### 组件分层

```text
AppShell
 ├── Sidebar / WindowFrame
 ├── ProviderPage
 │    ├── ProviderList
 │    ├── ProviderCard
 │    └── ProviderEditor
 ├── NativeEnvironmentBanner
 ├── Wsl2Page
 ├── LinuxScriptPage
 └── SettingsPage
```

- 页面数据来自 `AppProjection`，不要在多个 feature 中各自查询并拼接当前供应商。
- `ProviderEditor` 维护 page-local draft、dirty 状态、错误焦点和 API Key reveal 状态；保存按钮只在 Rust 返回 `validation_token` 后可用。
- `OperationProgress` 只消费 operation ID 和白名单步骤：默认模型确认、Responses API 流式响应、工具调用闭环；不消费原始事件正文。
- WSL2 行组件只渲染每发行版结果，不在 React 中推断停止状态或批量成功。
- Linux 脚本页可请求预览和复制，但默认不把包含 Key 的完整脚本放入全局状态；页面卸载清理。
- React 的 Toast、系统通知和模态框由返回的结果类型驱动，避免一个结果同时 Toast 和通知。

### 前端状态规则

```text
Rust AppProjection (authoritative)
        ↓ cache/subscription
React render state

Page-local draft (non-authoritative)
        ↓ command input
Rust validation/session

Rust operation channel
        ↓ redacted progress
React progress UI
```

不要在 localStorage、IndexedDB、URL、错误边界或 analytics 中保存 API Key。不要把 `AppProjection` 设计成包含完整供应商对象的调试 dump；前端只收到列表所需摘要，显式 reveal 才获取 Key。

## Cross-Platform Test Seams

### 测试层次

| 层 | 测试内容 | 环境 |
|----|----------|------|
| Core unit | ID/revision、验证状态机、地址策略、重启策略、错误码、shell quote、managed block 模型 | 任意 Rust CI |
| Property/fuzz | TOML 最小 patch 保留未知字段；marker 重复/损坏；特殊字符转义；指纹冲突 | Rust CI |
| SQLite integration | 历史库逐版本迁移、未来版本拒写、备份保留、事务回滚、崩溃恢复 fixture | Windows/macOS CI |
| Fake adapter contract | `CodexEnvironment`、`AtomicFileOps`、`ProcessControl`、`WslPort` 的成功/失败/中断矩阵 | Rust CI |
| Provider validator | fake HTTPS/event server、模型缺失、流中断、工具 call_id 不匹配、超时、重定向、超大响应 | Rust CI |
| Tauri mock | command 注册、managed state、Channel/event 转换、错误 DTO、权限配置 | Tauri mock runtime |
| React | 页面草稿、禁用条件、三步验证展示、重启确认、可访问性、i18n | Vitest/Testing Library 类工具 |
| Shell runtime | `bash -n`、Bash 4+、Zsh 5+；交互选择、退出不写、五份备份、断电 marker、特殊值 | Linux VM/container |
| Windows integration | WSL inventory/switch、Windows ReplaceFileW、宿主/CLI 进程识别、当前用户安装 | Windows 10 22H2+ x64/ARM64 |
| macOS integration | Intel/Apple Silicon 配置路径、Unix replace、桌面应用关闭/重新激活、菜单栏图标 | macOS 14+ |
| Packaging smoke | clean user install、升级、签名、notarization、数据库迁移、托盘驻留 | 真实签名工件 |

### 必须建立的测试缝

1. **文件缝：** `AtomicFileOps` 支持故障注入点：backup 后、credential commit 后、config commit 前后、verify 前后、权限复制失败。
2. **进程缝：** `ProcessControl` 使用固定 PID/启动时间/路径 fixture；测试 PID 复用和同名未知进程不能被选中。
3. **WSL 缝：** `WslPort` 使用命令 runner fake 记录精确参数、stdin 和预期是否启动/终止；另有 Windows 实机测试验证真实输出本地化。
4. **网络缝：** provider HTTP client 通过 fake server 注入流事件、状态码、断连和重定向；测试不得把真实供应商 API Key 放进 fixture。
5. **数据库缝：** 每个 migration fixture 都以只读副本执行；模拟 migration 中途错误，确认原库和备份仍可打开。
6. **脚本缝：** generator 输出 golden files；runtime harness 在真实 Bash/Zsh 中执行，验证 config.toml 其他字段字节/语义保留。
7. **Tauri 缝：** mock runtime 验证 command facade，不把真实 WebView 作为领域逻辑测试前提。

### 跨平台验收重点

- Windows 和 macOS 的用户级路径、文件权限、换行、临时文件替换和备份清理不能只在开发机测试。
- Windows 测试必须覆盖 WSL2 未安装、无发行版、WSL1 与 WSL2 混合、发行版已运行、发行版停止、名称含空格和命令失败。
- macOS 测试必须覆盖桌面应用缺失但 CLI 存在、两者都存在、两者都缺失、外部配置和原厂登录模式。
- 进程测试不能只验证“进程名匹配”；要验证路径/签名/bundle 与 PID 启动时间。
- 任何包含明文 Key 的 preview/copy/save 测试都使用明显的合成值，并检查日志、通知和诊断导出没有泄露。

## Integration Points

### External Services and Platforms

| Service/platform | Integration pattern | 关键注意事项 |
|------------------|---------------------|--------------|
| OpenAI Codex config | Rust `CodexConfigAdapter` 读取/最小 patch 用户级文件 | Codex 版本变化必须由 fixture/实机验证；不管理项目级配置 |
| Provider Responses API | Rust HTTP client + streaming state machine | 不执行远程工具；重定向、事件大小和完整响应均受限 |
| Windows WSL2 | Windows-only `WslPort` 调用官方 WSL CLI/API | 不解析本地化状态文本；不在被动发现时启动 |
| Windows desktop process | native process inventory + WM_CLOSE/relaunch adapter | 不按名字杀进程；CLI 不保证可自动重启 |
| macOS desktop process | bundle/process adapter + `NSRunningApplication` | 请求优雅结束；签名/bundle 路径必须校验 |
| SQLite | single-owner Rust executor + ordered migrations | 迁移前备份，未来版本拒写，错误不清空 |
| Linux export target | generated Bash/Zsh function | 自包含、明文警告、marker 唯一、无运行时依赖 |
| Tauri updater | signed artifact metadata and explicit install command | 每日节流，用户控制，不发送供应商数据 |

### Internal Boundaries

| Boundary | Communication | Notes |
|----------|---------------|-------|
| React ↔ Rust | typed command DTO + Channel/event | DTO 默认脱敏；Key 只能显式 reveal/导出 |
| CommandFacade ↔ application | direct typed calls | Facade 不包含业务分支 |
| application ↔ core | domain values and state machines | core 不依赖平台 |
| application ↔ ports | trait calls with operation context | 所有副作用都带 operation ID |
| application ↔ SQLite | short transactions | 不跨网络/用户确认/重启保持事务 |
| application ↔ file adapter | plan/stage/commit/reconcile | 外部文件不是数据库事务的一部分 |
| application ↔ process adapter | snapshot/request/poll | restart outcome 独立于 config outcome |
| application ↔ WSL adapter | passive inventory / explicit active session | 每发行版独立锁和独立结果 |
| application ↔ diagnostics | structured allowlist events | 日志 sink 不能回读完整秘密 |

## Scalability Considerations

GPTEasy 是本地应用，不需要服务端水平扩展。主要规模是供应商数量、WSL2 发行版数量、日志和备份数量。

| 规模 | 架构处理 |
|------|----------|
| 1–10 个供应商、0–3 个 WSL2 环境 | 单进程、单 SQLite 执行器、串行环境操作；优先可恢复性 |
| 10–100 个供应商、3–20 个 WSL2 环境 | 列表 projection 分页不需要引入；供应商搜索在内存投影完成；WSL 批量使用有界串行/并发和逐项结果 |
| 更大本地目录 | 保留最近修订/备份，导出仍是全部已验证供应商；不要为不存在的云端规模引入服务端或代理网关 |

### Scaling Priorities

1. **首先会出问题的是副作用一致性，而不是吞吐量。** 先实现操作日志、文件指纹、备份和 reconcile，再考虑并发 WSL2。
2. **其次是 Codex/WSL/宿主应用版本漂移。** 通过 fixture 版本矩阵和外部配置状态处理，而不是增加全局重写逻辑。
3. **最后才是 UI projection 性能。** 使用稳定的 projection 和局部刷新即可，不能因为列表规模把明文供应商对象复制到多个前端 store。

## Anti-Patterns

### Anti-Pattern 1: React 直接操作文件、SQLite 或 WSL

**What people do:** 为了快速做 UI，在前端调用 shell plugin、读取配置文件或把 SQLite 暴露为通用 query。

**Why it's wrong:** 绕过 Rust 的安全策略、备份、原子替换、错误码和审计；还会把 API Key 放进 WebView 或插件边界。

**Do this instead:** React 只调用窄 command；所有路径、环境 ID、provider ID 和 restart policy 在 Rust 重新验证。

### Anti-Pattern 2: 反序列化后重写整份 `config.toml`

**What people do:** 把 Codex 配置解析成自定义 struct，修改三个字段后重新序列化整份文件。

**Why it's wrong:** 丢失未知字段、注释、格式和用户设置；Codex 版本增加字段后容易造成破坏性覆盖。

**Do this instead:** 用保留 AST/文本的 TOML editor 做最小 patch；无法唯一定位时停止。

### Anti-Pattern 3: 把多文件写入当成原子事务

**What people do:** 先写 `.env`，再写 `config.toml`，发生错误时认为 SQLite 事务会自动回滚文件。

**Why it's wrong:** SQLite 事务不能回滚外部文件；崩溃会留下半更新状态。

**Do this instead:** 版本化凭据槽位、同目录临时文件、操作日志、备份、提交点后重读和启动 reconcile。

### Anti-Pattern 4: 用供应商名称/地址猜身份

**What people do:** 用显示名称、服务地址或地址+模型把外部配置自动绑定到供应商。

**Why it's wrong:** 同一服务多个 API Key、同名供应商和地址变更都会误绑定凭据。

**Do this instead:** 首选不可变供应商 ID；只有地址和模型唯一匹配且用户重新验证时才纳入 GPTEasy，否则展示外部配置。

### Anti-Pattern 5: 用进程名匹配并强制 kill

**What people do:** 找到所有名为 `codex` 的进程，切换后直接终止。

**Why it's wrong:** 会杀掉未知程序、误伤其他用户任务、丢失终端工作；PID 还可能已经复用。

**Do this instead:** 路径/签名/bundle + PID 启动时间的身份匹配；桌面应用请求优雅重启，CLI 保持待重启并提示用户。

### Anti-Pattern 6: 解析 `wsl --list --verbose` 的英文状态

**What people do:** 查找 `Running`/`Stopped` 字符串，或在发现阶段运行 `id -un`。

**Why it's wrong:** 输出可能本地化；执行发行版命令会启动停止的发行版，违反被动检测语义。

**Do this instead:** 组合 quiet/verbose/running 命令与 WSL API，解析稳定字段；默认用户名名称问题在 Windows 实机 spike 中验证，不能静默启动。

### Anti-Pattern 7: 在 shell function 中做通用 TOML 正则替换

**What people do:** 用一条 `sed` 替换所有 `model`、`model_provider` 或 URL。

**Why it's wrong:** TOML table 作用域、注释、重复 key 和用户自定义内容会导致误改或生成非法配置。

**Do this instead:** 只操作自己生成且位置固定的唯一 managed block；marker 异常或首次迁移不唯一时停止并保留备份。

### Anti-Pattern 8: 更新检查和更新安装耦合

**What people do:** 启动时发现新版本就自动下载、安装、重启。

**Why it's wrong:** 违背用户控制的更新策略，并可能在配置写入/WSL 操作期间中断应用。

**Do this instead:** 检查、下载、签名验证、用户确认、安装分成独立 command；更新服务不能阻塞配置核心路径。

## Suggested Dependency-Driven Build Order

### Phase 0 — 实现契约 spike（先于大规模 UI）

**目标：** 不重新讨论产品决策，只验证锁定决策在真实平台上可落地。

- 用代表性 Codex `config.toml` fixture 验证用户级路径、`model_provider`、自定义 provider、`env_key` 和原厂登录模式的读取/最小 patch；
- 在 Windows/macOS 实机注入文件替换失败、权限失败和进程外部编辑；
- 验证桌面应用识别/优雅关闭/重新激活，以及 CLI 只能进入待重启的边界；
- 在停止的 WSL2 环境验证不启动时的默认 UID/用户名获取；
- 生成最小 Tauri Windows/macOS 工件，验证当前用户安装、签名/updater 配置和托盘驻留；
- 建立无真实秘密的 Codex、SQLite、WSL 输出和脚本 fixture。

**出口：** 形成平台事实矩阵和已确认的适配器契约。任何无法验证的项转为该阶段的明确风险，而不是隐藏在业务代码中。

### Phase 1 — Rust workspace、领域核心和 command DTO

**依赖：** Phase 0 的配置/平台事实。  
**产出：** `gpteasy-core`、`gpteasy-application` 的领域类型、稳定错误码、Secret 包装、provider ID/revision、环境绑定状态机、计划 token、前端 DTO。

先实现纯函数：

- 地址安全策略；
- DayWay 排序和模板更新决策；
- 验证状态机；
- provider revision 与 credential slot 命名；
- 配置 patch 预期；
- restart outcome；
- shell quote 和导出 metadata。

**不要在此阶段做：** 真实文件写入、网络请求、React 页面和 WSL 命令。

### Phase 2 — SQLite、迁移和启动恢复

**依赖：** Phase 1 的领域 schema。  
**产出：** 独占 SQLite executor、永久顺序迁移、迁移前三份备份、未来版本拒写、provider/environment/operation/update 表、启动一致性检查。

重点测试历史数据库直升、迁移中途错误、连接锁、损坏库、未完成 operation。启动恢复必须在开放任何写 command 前完成。

### Phase 3 — Codex 文档模型与安全文件事务

**依赖：** Phase 1 领域 patch；Phase 2 operation journal。  
**产出：** `CodexConfigModel`、原生路径解析、TOML 保留式最小 patch、凭据文件 adapter、Windows/macOS `SafeFileWriter`、备份保留和 crash reconcile。

这是首个最高风险垂直切片。先用 fixture 完成：

```text
read → plan → backup → stage → replace → reread → reconcile
```

再接真实用户级 `~/.codex` 目录。未通过“外部修改停止、部分写入恢复、其他字段保留”测试前，不进入 UI。

### Phase 4 — 供应商验证和目录闭环

**依赖：** Phase 1 验证状态机、Phase 2 provider revision、Phase 3 配置物化器。  
**产出：** 模型发现、流式 Responses 验证、工具调用闭环、validation token、验证后保存、编辑保留旧修订、DayWay 模板更新和删除约束。

先用 fake provider server 完成全状态机，再做真实供应商验收。React 仍可以用临时开发面板或 command fixture，正式页面不应反向定义验证逻辑。

### Phase 5 — 原生 Codex 环境与重启协调

**依赖：** Phase 3 文件事务、Phase 4 已验证供应商、Phase 2 environment binding。  
**产出：** 原生环境检测、代理 API/原厂登录模式、首次自动切换、后续切换、外部配置、供应商传播、托盘与设置共用的 native switch use case、待重启状态。

此阶段必须覆盖桌面应用缺失、CLI 单独存在、两者同时存在和外部配置。立即重启只能报告真实 `RestartOutcome`，不能把配置写成功误报为进程已重启。

### Phase 6 — React 设置窗口、托盘和 UI contract

**依赖：** Phase 5 稳定 command DTO/projection；Phase 4 operation progress。  
**产出：** UI-SPEC 的供应商页、首次使用、验证反馈、重启确认、托盘菜单、设置页、主题/语言/可访问性。

React 只实现投影、草稿和反馈。托盘菜单从 Rust projection 生成；设置页和托盘点击不各自实现切换流程。此阶段才接入真实 Tauri window close prevention、通知和 tray platform differences。

### Phase 7 — Windows WSL2

**依赖：** Phase 3 复用的 Codex patch/backup/reconcile；Phase 5 的环境绑定/待重启模型。  
**产出：** 被动 inventory、默认用户处理、单发行版切换、WSL2 临时启动恢复、逐项批量切换和 WSL2 技术详情。

先做单发行版成功/失败/恢复，再做批量。批量默认串行，直到实机确认 WSL 启动/终止和文件权限行为后再考虑有限并发。

### Phase 8 — Linux 切换脚本

**依赖：** Phase 1 provider revision/script model、Phase 3 Codex 字段契约、Phase 4 已验证目录。  
**产出：** Bash 4+ 与 Zsh 5+ generator、preview/copy/save、唯一 managed block、五份备份、断点 marker、交互选择和重启提示。

此阶段不能复用 Rust 二进制或 Python/Node；必须在真实 Bash/Zsh 中运行 golden script。将“首次遇到已有自定义配置”的歧义路径作为显式失败，不把脚本变成通用 TOML 编辑器。

### Phase 9 — 诊断、更新、登录启动和发布

**依赖：** 所有核心 operation 和错误码。  
**产出：** 七天脱敏日志、诊断导出、每日更新检查、用户确认安装、登录启动、当前用户安装、Windows/macOS 签名/updater/notarization。

更新和诊断只能依赖 projection/日志白名单，不得读取供应商完整记录后打包。发布验收在 clean user profile 中执行，并包含迁移失败恢复和待重启场景。

### Phase 10 — 完整首版跨平台验收

**依赖：** Phase 1–9 全部出口条件。  
**产出：** Windows 10 22H2+ x64/ARM64、macOS 14+ Intel/Apple Silicon 的完整矩阵；旧数据库样本、真实托盘、WSL2、脚本、更新和可访问性验收证据。

按照项目约束，内部阶段可以逐步完成，但不把未覆盖完整范围的中间阶段作为正式首版发布。

### Phase Ordering Rationale

```text
平台契约 spike
  → 领域/端口
  → SQLite/恢复
  → Codex 文件事务
  → 验证/目录
  → 原生切换/重启
  → React/托盘
  → WSL2
  → Linux function
  → 维护/发布
```

- SQLite 和操作日志先于任何外部写入，否则失败恢复只能靠临时变量。
- Codex 配置适配器先于 provider UI，因为“验证后替换”和“配置保留”是 UI 的可用性基础。
- 供应商验证先于原生切换，因为只有已验证供应商可以进入环境绑定。
- 原生切换先于 React/托盘，因为两者必须共享稳定的 command/projection 契约。
- WSL2 复用配置事务和环境绑定，但具有 Windows 启动/默认用户/恢复风险，不能与原生切换同时首次实现。
- Linux script 复用供应商修订模型，但有独立的 shell parser/写入安全边界，应在 Codex 字段契约稳定后实现。
- 诊断、更新和安装依赖稳定的 operation/error model，最后接入可减少发布阶段的并发变量。

## Phase-Specific Research Flags

| Phase | 需要深入验证的点 | 为什么不能靠抽象假设 |
|-------|------------------|----------------------|
| 0/3 | Codex 当前桌面应用与 CLI 共用配置的真实路径、字段和版本兼容 | ADR 锁定直接配置，但上游配置 schema 会演进 |
| 0/5 | Windows/macOS 对桌面宿主应用的身份、优雅关闭和重新启动 | 进程名不能作为安全身份；CLI 无原始终端重启语义 |
| 0/7 | 停止 WSL2 环境中无启动取得默认用户名 | WSL 官方配置容易取得 UID，但 UI 需要可展示的默认用户 |
| 3/7/8 | Codex 配置与凭据文件的实际加载、权限和多文件失败恢复 | 两个文件不能由单个文件系统操作原子提交 |
| 8 | Bash 4+/Zsh 5+ 对特殊值、`mv`、权限和中断的行为矩阵 | 导出物脱离 GPTEasy，不能依赖 Rust 恢复 |
| 9 | macOS 当前用户安装位置、notarization 和 updater 替换权限 | 包签名与安装位置直接影响用户控制更新 |
| 10 | ARM64 工件和原生 Codex/WSL 行为 | 交叉编译成功不等于运行时路径和进程行为正确 |

## Sources

### 锁定基线（HIGH）

- `C:/src/GPTEasy/CONTEXT.md`
- `C:/src/GPTEasy/docs/adr/0001-plaintext-provider-credentials.md`
- `C:/src/GPTEasy/docs/adr/0002-tauri-rust-react.md`
- `C:/src/GPTEasy/docs/adr/0003-standalone-linux-switch-functions.md`
- `C:/src/GPTEasy/docs/adr/0004-direct-codex-configuration.md`
- `C:/src/GPTEasy/docs/adr/0005-immutable-provider-identity.md`
- `C:/src/GPTEasy/docs/adr/0006-sqlite-application-state.md`
- `C:/src/GPTEasy/docs/adr/0007-local-only-product.md`
- `C:/src/GPTEasy/docs/adr/0008-native-codex-environment.md`
- `C:/src/GPTEasy/docs/ui/UI-SPEC.md`

### 外部主来源（MEDIUM；截至 2026-08-05 检索）

- Tauri 2 calling Rust / frontend、system tray、capabilities、updater、testing：  
  `https://v2.tauri.app/develop/calling-rust/`  
  `https://v2.tauri.app/develop/calling-frontend/`  
  `https://v2.tauri.app/learn/system-tray/`  
  `https://v2.tauri.app/security/capabilities/`  
  `https://v2.tauri.app/plugin/updater/`  
  `https://v2.tauri.app/develop/tests/`
- SQLite transactions、atomic commit、backup、locking、application/user version：  
  `https://www.sqlite.org/lang_transaction.html`  
  `https://www.sqlite.org/atomiccommit.html`  
  `https://www.sqlite.org/backup.html`  
  `https://www.sqlite.org/lockingv3.html`  
  `https://www.sqlite.org/pragma.html`
- OpenAI Codex configuration/authentication 与 Responses API：  
  `https://developers.openai.com/codex/config-reference/`  
  `https://developers.openai.com/codex/auth/`  
  `https://developers.openai.com/api/docs/guides/streaming-responses`  
  `https://developers.openai.com/api/docs/guides/function-calling`
- Microsoft WSL 与 Windows process/file APIs：  
  `https://learn.microsoft.com/en-us/windows/wsl/basic-commands`  
  `https://learn.microsoft.com/en-us/windows/wsl/wsl-config`  
  `https://learn.microsoft.com/en-us/windows/win32/api/wslapi/nf-wslapi-wslgetdistributionconfiguration`  
  `https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-close`  
  `https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew`
- Apple process APIs：  
  `https://developer.apple.com/documentation/appkit/nsrunningapplication`  
  `https://developer.apple.com/documentation/appkit/nsrunningapplication/terminate()`  
  `https://developer.apple.com/documentation/appkit/nsrunningapplication/activate(options:)`
- Rust file semantics、Bash/Zsh shell primitives：  
  `https://doc.rust-lang.org/std/fs/fn.rename.html`  
  `https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all`  
  `https://www.gnu.org/software/bash/manual/bash.html`  
  `https://zsh.sourceforge.io/Doc/Release/Functions.html`

---
*Architecture research for: GPTEasy*
*Researched: 2026-08-05*
