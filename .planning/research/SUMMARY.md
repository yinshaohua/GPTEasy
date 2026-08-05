# GPTEasy 项目研究总结

**Project:** GPTEasy  
**Domain:** Windows/macOS 跨平台 Codex 供应商管理桌面伴侣，包含 Windows WSL2 管理与独立 Linux 切换脚本导出  
**Researched:** 2026-08-05  
**Confidence:** MEDIUM

## Executive Summary

GPTEasy 是一个完全本地运行、托盘优先、由 Rust 后端保持权威状态的配置编排器，而不是一个由 React 直接调用文件或命令插件的桌面脚本。产品核心是让非技术用户完成供应商验证，并在保留既有 Codex 配置且可恢复的前提下，为原生 Codex 环境、各 WSL2 环境和独立 Linux 用户级配置选择供应商。专家实现此类产品时，会把供应商目录、受管环境的期望状态、磁盘实际状态和运行中进程状态明确分离，并让所有外部副作用遵循“计划 → 执行 → 复读校验 → 对账/恢复”的流程。

推荐采用锁定的 Tauri 2 + Rust + TypeScript/React 架构：Rust 独占 SQLite、Codex 配置、供应商网络请求、进程、WSL2、剪贴板、诊断、更新和平台 API；React 只通过窄而强类型的 Tauri command 使用脱敏投影。内部状态以不可变供应商 ID、验证成功后的修订版本、环境期望/已应用状态、操作 journal 和待重启状态组织。SQLite 迁移、配置备份、同目录暂存、平台原子替换、文件指纹复核、启动恢复和凭据脱敏必须先于正式 UI、托盘、WSL2 批量切换和 Linux 切换脚本。

最大风险不是吞吐量，而是跨 SQLite、Codex 文件、进程、WSL2 和更新安装器的状态分裂，以及明文供应商凭据从日志、错误、临时文件、诊断导出或脚本生成路径间接泄露。路线图必须设置不可绕过的安全与发布门禁：历史数据库迁移与故障注入通过后才能开放写入；外部配置保留、并发冲突和崩溃恢复通过后才能接入切换 UI；真实 Windows/macOS/WSL2/Bash/Zsh 验收通过后才能进入发布候选；Windows x64/ARM64、macOS Intel/Apple Silicon、双语、无障碍、签名、公证和 N-1/N-2 更新全部通过后，才能一次性发布完整首版。

## 基线裁决与研究对齐

以下结论以 `PROJECT.md`、`CONTEXT.md`、ADR 0001–0008 和 `docs/ui/UI-SPEC.md` 为锁定基线。研究文件中的建议只解释如何实现，不改变产品范围、领域语言或 UI 决策。

1. **直接配置模式保持不变。** 架构研究提出的“版本化凭据槽位 + 配置提交点”是候选实现模式，不是新的产品决策。是否存在独立凭据文件、Codex 实际支持哪些 `env_key`/认证字段，必须由早期 Codex 配置兼容性 spike 确认；不得先假定某个未验证的文件布局。
2. **供应商修订版本是内部实现模型。** 用户侧仍使用 `CONTEXT.md` 中的“供应商”“已验证供应商”“验证后替换”“供应商配置传播”等术语。修订版本只用于保证验证失败时保留旧配置、传播时可识别期望/已应用内容以及崩溃恢复。
3. **供应商验证与配置写入引擎是两个能力，但路线图先稳定写入契约。** 网络验证本身可以并行开发；然而首次供应商验证成功后必须自动切换原生 Codex 环境，因此“供应商与供应商验证闭环”的阶段出口依赖已确认的 Codex 配置物化契约。
4. **备份保留数量按对象区分。** SQLite 迁移前完整备份默认保留最近三份；原生 Codex 环境、每个 WSL2 环境和 Linux 用户级配置的配置备份默认保留最近五份。
5. **WSL2 被动检测不允许偷偷启动发行版。** `CONTEXT.md` 同时要求展示 WSL2 默认用户并保证检测不启动发行版。停止的 WSL2 环境如何无启动取得可展示用户名仍未验证，是路线图 Phase 0/WSL2 阶段的阻断性问题，不能通过静默启动或猜测用户名规避。
6. **完整首版一次交付。** 下面的阶段是内部实施与风险收敛顺序，不是可分别对外发布的功能版本。

## Key Findings

### Recommended Stack

采用 Rust workspace 承载领域、应用和基础设施边界，Tauri 只作为桌面壳与 IPC 装配层，React 只作为展示与交互层。SQLite 选择 `rusqlite` 的单所有者模型；Codex 配置采用 `toml_edit` 保留未知字段、注释和格式；供应商验证采用 `reqwest` + SSE 解码器直接实现协议状态机；平台能力由 Windows/macOS 窄适配器封装。前端不安装或开放通用 `fs`、`shell`、`http`、`sql`、`clipboard` 权限。

**核心技术与版本基线：**

- **Rust `1.97.1` / edition 2024**：领域、系统和平台后端；用 `rust-toolchain.toml` 精确固定。
- **Tauri `2.11.x`**：窗口、托盘、菜单、单实例、通知、登录启动、更新和打包；插件由 Rust 初始化，前端只使用核心 API。
- **Node.js `24.18.0` LTS + pnpm `11.20.0`**：可复现的前端构建与测试环境；提交 `pnpm-lock.yaml` 并在 CI 使用 `--frozen-lockfile`。
- **React `19.2.8` + Vite `8.2.0` + TypeScript `6.0.3`**：设置窗口 UI；TypeScript 暂不升级到 7，避免当前 lint 生态不兼容。
- **`rusqlite 0.40.1`（bundled + backup）**：Rust 后端独占的 SQLite；使用一致快照备份、永久顺序迁移和未来版本拒写。
- **`toml_edit 0.25.13` + 平台安全文件写入器**：只修改 GPTEasy 管理字段或 GPTEasy 管理区块，保留其他 Codex 配置。
- **`reqwest 0.13.4` + `sse-stream 0.2.5`**：模型发现、Responses API 流式响应和工具调用闭环；不引入隐藏协议细节的第三方 OpenAI SDK。
- **`ts-rs 12.0.1`**：从 Rust command DTO 生成 TypeScript 类型，并在 CI 检查类型漂移。
- **`react-aria-components`、React Hook Form、Zod、TanStack Query、i18next**：无障碍表单、局部草稿、边界解析、异步 command 状态和双语 UI；写操作不自动重试。
- **Windows `windows` crate / macOS `objc2-app-kit`**：文件替换、进程身份、桌面应用优雅关闭与重新激活、WSL2 等平台能力。

**关键版本与发布约束：**

- 本地和 CI 必须使用相同的 Tauri CLI、Rust、Node 和 pnpm 版本，并提交 `Cargo.lock` 与 `pnpm-lock.yaml`。
- `tauri-plugin-single-instance` 必须最先注册。
- Tauri WebDriver 插件只能存在于 debug 或显式 E2E 构建，生产二进制必须证明不包含调试服务。
- Windows 正式分发使用 NSIS `currentUser`，分别生成 x64/ARM64 工件。
- macOS 使用支持 Intel/Apple Silicon 的 universal DMG，但必须满足当前用户安装与 updater 写权限。
- 操作系统代码签名/公证与 Tauri updater 内容签名是两条独立信任链。
- 研究中的版本号是 2026-08-05 的冻结建议。项目初始化时按该兼容组合锁定；后续升级走独立变更和完整矩阵，不在业务阶段顺手追新。

### Expected Features

#### Must have（完整首版基线）

- **供应商目录与供应商生命周期**：标准供应商字段、不可变供应商 ID、DayWay、未配置推荐供应商、已验证供应商、验证后替换、供应商配置传播和供应商删除保护。
- **供应商验证**：默认模型确认、Responses API 流式响应和工具调用闭环按固定顺序执行，任一失败停止；只有全部成功的供应商才能保存和参与切换。
- **原生 Codex 环境管理**：将统一 ChatGPT 桌面应用中的 Codex 与本机原生 Codex CLI 视为同一个原生 Codex 环境，支持代理 API 模式、原厂登录模式、外部配置和当前用户默认配置。
- **安全配置写入**：配置保留、带时间戳备份、同目录临时文件、平台原子替换、替换前指纹复核、替换后复读、崩溃恢复和待重启。
- **进程感知切换**：运行中切换前选择立即重启、稍后重启或取消；取消发生在写入前；CLI 不被强制终止；“已写入”和“已生效”是不同状态。
- **托盘优先体验**：托盘只快捷切换原生 Codex 环境的已验证供应商；设置窗口、托盘、通知和页面共享同一后端状态。
- **WSL2 独立管理**：Windows 上被动发现 WSL2 环境，只管理 WSL2 默认用户，支持逐个/批量切换、WSL2 临时启动、原停止状态恢复和逐环境结果。
- **Linux 切换脚本**：分别生成 Bash 4+ 与 Zsh 5+ 自包含 function，包含全部已验证供应商及明文凭据，只修改 GPTEasy 管理区块，可脱离 GPTEasy 长期使用。
- **本地状态与运行维护**：SQLite 永久顺序迁移、七天脱敏诊断日志、用户主动诊断导出、每日最多一次更新检查、用户确认安装、登录启动和托盘驻留。
- **发布与可访问性**：Windows 10 22H2+ x64/ARM64、macOS 14+ Intel/Apple Silicon、简体中文/英语、键盘、屏幕阅读器、200% 缩放、高对比度和减少动态效果。

#### Should have（锁定范围内的差异化）

- **三项供应商验证闭环**：保存的是 Codex 实际可用的供应商，而不是一次普通 HTTP 请求成功的地址。
- **一个供应商目录、多个独立受管环境**：供应商资料只维护一份，原生 Codex 环境与每个 WSL2 环境各自拥有当前供应商。
- **直接配置而非本地代理网关**：GPTEasy 不成为每次请求的持续依赖，Linux 切换脚本也能独立工作。
- **非破坏的外部配置识别**：无法唯一匹配的状态展示为外部配置，不自动接管、不静默覆盖。
- **可恢复的供应商配置传播**：已验证供应商更新后逐环境传播，每个环境独立成功、失败或待重启。
- **明文凭据的可理解性与严格脱敏边界**：用户可以完整查看、复制和导出 API Key，但所有非必要输出通道必须脱敏。
- **WSL2 停止状态保留**：GPTEasy 不把原本停止的发行版永久留在运行状态，也不在出现用户并发活动时误终止。

#### Defer（v2+ 或明确排除）

- 产品账户、云端存储、云同步、自动跨设备迁移和 Linux 脚本回导。
- Linux 独立 GUI。
- 本地 API 代理网关。
- 机器级公共配置、其他用户配置、自定义配置路径。
- WSL2 自动跟随原生 Codex 环境或托盘 WSL2 快捷切换。
- Fish、Nushell、仅 POSIX sh 和依赖 Python/Node.js 的导出物。
- 自定义 Header、组织、项目标识等高级供应商认证字段。
- 旧版宿主应用的供应商管理。
- 自动终止 Codex 进程、持续供应商健康监控、自动修复歧义配置。
- 静默下载/安装更新、聊天应用式仪表盘和持续营销动效。

### Architecture Approach

推荐使用“领域核心 + 应用服务 + Ports/Adapters + 薄 Tauri 壳 + React 投影”的单向依赖结构。SQLite 是供应商身份和已验证配置的权威来源；原生 Codex 环境与 WSL2 环境的实际状态必须在操作前后从文件和进程重新读取；Linux 用户级配置完全由导出的 Linux 切换脚本在目标机器上管理，不回导也不同步。任何外部副作用都应有 operation ID、资源锁、短期计划 token、备份清单、提交点、复读校验和可恢复状态。

**主要组件：**

1. **TauriAppShell / CommandFacade** — 单实例、启动顺序、托盘、窗口生命周期、通知、更新桥、最小 capabilities 和窄 DTO；不承载供应商规则。
2. **ProviderCatalog / ProviderValidation** — 不可变供应商 ID、DayWay、已验证配置修订、验证后替换、删除约束和三项供应商验证状态机。
3. **OperationCoordinator** — 所有写操作的唯一入口；管理资源锁、计划 token、取消点、操作 journal、崩溃恢复和对账。
4. **SqliteStore** — 独占连接、迁移前一致备份、永久顺序迁移、未来版本拒写、短事务和领域不变量。
5. **CodexConfigModel / codex_compat** — 版本化读取实际状态、识别外部配置、生成最小 patch、保留未知配置和计算指纹。
6. **SafeFileWriter** — 同目录暂存、权限/ACL、刷盘、平台替换、五份配置备份和复读校验；不解析业务语义。
7. **EnvironmentSwitch / NativeCodexAdapter** — 原生 Codex 环境直接配置、代理 API/原厂登录模式、首次自动切换、后续切换和供应商配置传播。
8. **ProcessInventory / RestartCoordinator** — 以路径、签名、bundle identifier、PID 和启动时间识别进程，区分桌面宿主应用与本机 Codex CLI，输出真实待重启/重启结果。
9. **WslAdapter** — Windows-only 被动发现、WSL2 默认用户上下文、逐发行版锁、WSL2 临时启动租约、配置写入和原停止状态恢复。
10. **LinuxScriptGenerator** — Bash/Zsh 独立模板、两层编码、明文警告、GPTEasy 管理区块、目标侧备份和独立交互选择。
11. **DiagnosticsService / UpdateService** — 字段允许清单、七天日志、主动诊断导出、每日更新节流、签名元数据和用户控制更新。
12. **React feature boundaries** — `AppProjection`、页面局部草稿、验证进度、错误焦点、双语和无障碍；不保存后端权威状态或完整 API Key。

**必须遵循的架构模式：**

- React → typed command/DTO → application use case → domain/ports → adapter，禁止反向依赖。
- 计划 → 执行 → 复读校验 → 对账；用户确认、网络、进程等待期间不持有 SQLite 写事务。
- 供应商身份与可变配置分离；受管环境保存期望/已应用状态，不用名称或地址作为身份。
- “配置已写入”“受管环境当前供应商”“运行中进程已读取”分别建模。
- 托盘和设置窗口调用同一原生切换 use case。
- WSL2 批量切换是多个独立操作的编排，不是假装跨发行版原子事务。
- 错误 DTO、操作事件和日志只允许稳定错误码及脱敏字段，不透传底层错误正文或子进程输出。

### Critical Pitfalls

1. **SQLite 迁移备份不一致** — WAL 模式下不得复制活动主文件；必须使用 SQLite Online Backup API 或等价一致快照，备份可打开并通过检查后才迁移。历史 schema、失败迁移和未来版本拒写是发布门禁。
2. **把单文件原子替换当成完整事务** — SQLite 无法回滚外部文件。必须使用操作 journal、同目录暂存、平台替换、复读校验和启动恢复，保证崩溃后只能得到明确的旧状态、新状态或需用户决定的恢复状态。
3. **覆盖外部配置或破坏链接/优先级** — 保留未知字段和注释，提交前比较文件身份与内容指纹；符号链接、重解析点、重复区块、未知优先级或外部并发修改一律 fail-closed。
4. **供应商凭据间接泄露** — 明文保存不等于可进入日志。使用敏感值类型、字段允许清单、一次性 reveal、从空目录构造诊断包和 canary 扫描；数据库、备份、Codex 配置和 Linux 切换脚本不得进入诊断导出。
5. **把“已写入”误报成“已生效”** — 进程身份不能只看名称；切换前、提交前、提交后都要复核。桌面宿主应用可在用户确认后优雅重启，本机 Codex CLI 不能伪造原终端恢复，必要时保持待重启。
6. **WSL2 临时启动租约错误** — 原始运行状态和恢复责任必须持久化；只终止 GPTEasy 明确临时启动且无用户并发活动的目标发行版，禁止使用全局 `wsl --shutdown`。
7. **Linux 切换脚本把数据当代码** — Bash/Zsh 使用独立模板，先编码 TOML 值再编码 shell literal，禁止 `eval`；控制字符、损坏区块、权限、磁盘满和中断必须安全失败。
8. **更新/打包信任链只在开发构建通过** — OS 签名、公证与 updater 签名分别验证；N-1/N-2 正式安装版必须从正式端点升级；更新退出前必须完成配置操作与 WSL2 租约 cleanup barrier。

## Implications for Roadmap

建议使用 11 个内部阶段（Phase 0–10）。阶段可以产生可测试的中间工件，但正式 v1 只在 Phase 10 全部门禁通过后发布。

### Phase 0：实现契约与发布可行性 Spike

**Rationale:** Codex 配置、进程身份、停止 WSL2 环境的默认用户和 macOS 当前用户安装是后续设计的事实前提。先用实机和代表性 fixture 冻结适配器契约，避免在业务代码中固化猜测。  
**Delivers:**

- Windows/macOS 当前用户原生 Codex 配置路径、字段、认证模式和配置优先级事实矩阵。
- 代理 API 模式、原厂登录模式、供应商凭据物化方式和 GPTEasy 管理字段的可验证最小 patch。
- 统一 ChatGPT 桌面应用与本机 Codex CLI 的进程身份、优雅关闭、重新激活和不可自动恢复边界。
- 停止 WSL2 环境在不启动时取得 WSL2 默认用户显示名称的验证结果。
- 最小 Tauri Windows/macOS 工件：托盘驻留、当前用户安装、签名/公证和 updater 配置 smoke。
- 无真实秘密的 Codex、SQLite、WSL、进程和 shell fixture 基线。

**Addresses:** TS-06、TS-07、TS-08、TS-09、TS-11、TS-18 的实施契约。  
**Avoids:** Pitfall 2、3、5、6、10、14、15。  
**Exit gate:**

- 每个锁定行为都有已验证的平台实现路径或明确阻断项。
- 不得以“稍后再看”接受 Codex 凭据文件、WSL2 默认用户名或安装位置的隐式假设。
- 若无法满足锁定行为，必须在进入正式需求拆分前标记为项目 blocker，而不是降低产品语义。

### Phase 1：工具链、领域核心与窄 Command 契约

**Rationale:** 先固定可复现工具链、领域语言和状态机，后续 SQLite、文件、网络与 UI 才能共享同一套不变量。  
**Delivers:**

- Rust workspace、Tauri/React skeleton、锁定依赖和最小 capabilities。
- 不可变供应商 ID、内部供应商修订版本、环境期望/已应用状态、待重启和外部配置类型。
- DayWay 排序/模板更新规则、安全服务地址规则、三项验证状态机和重启结果模型。
- `Sensitive<T>`/`SecretString`、稳定错误码、脱敏 DTO、短期 validation/plan token。
- Rust DTO → TypeScript 类型生成和 drift gate。

**Addresses:** TS-02、TS-03、TS-04、TS-05、TS-14 的领域基础。  
**Avoids:** React 成为第二权威状态源、名称/地址身份匹配、通用前端系统权限和敏感对象调试输出。  
**Exit gate:**

- Core 不依赖 Tauri、SQLite、文件、网络或平台 API。
- 地址策略、DayWay、供应商生命周期、状态机、token 绑定和脱敏 canary 单元/属性测试通过。
- 生产 capabilities 不开放通用 fs/shell/http/sql/clipboard。

### Phase 2：SQLite、迁移、操作 Journal 与启动恢复

**Rationale:** 所有供应商和环境操作都依赖可恢复的本地状态；数据库安全必须在任何正式写入前完成。  
**Delivers:**

- Rust 独占 SQLite executor、`providers`、供应商修订、环境绑定、operations、settings 和 update state。
- SQLite Online Backup API 迁移前一致备份，默认保留最近三份。
- 永久顺序迁移、`BEGIN IMMEDIATE`、foreign keys、未来版本拒写和完整不变量检查。
- 启动恢复框架：未完成操作在开放写 command 前先观察、对账或进入恢复状态。
- 每个正式 schema 的最小、典型和边界 fixture。

**Addresses:** TS-01、TS-15、TS-16 的状态底座。  
**Avoids:** Pitfall 1、自动清库、WAL 不一致备份、长事务包住网络/用户确认。  
**Exit gate:**

- 每个历史 fixture 可直接升级到当前 schema。
- 每个 migration failpoint 回滚，原库和三份备份可打开，凭据与环境引用保持。
- 更高版本数据库拒绝写入。
- 数据库及备份权限/ACL 与明文供应商凭据等级一致，诊断导出无法收集。

### Phase 3：Codex 兼容层与可恢复配置事务

**Rationale:** 这是核心数据完整性边界，也是原生 Codex 环境、WSL2 和 Linux 切换脚本共同依赖的配置契约。  
**Delivers:**

- `codex_compat`：读取 `Managed / External / Unavailable`，生成最小变更计划，校验只改 GPTEasy 管理字段。
- 由 Phase 0 事实决定的供应商凭据物化方案；不得硬编码未经验证的独立凭据文件。
- Windows/macOS `SafeFileWriter`：五份配置备份、同目录 temp、权限/ACL、刷盘、平台替换和复读。
- 文件身份、mtime、大小、内容 hash、符号链接/重解析点和提交前并发复核。
- 多工件 operation journal、明确提交点、崩溃后 reconcile 和用户恢复分支。

**Addresses:** TS-07、TS-08、TS-14；为 TS-11、TS-12、TS-13 提供共享内核。  
**Avoids:** Pitfall 2、3，整份 TOML 重写、跨卷 temp、Windows 直接套用 Unix rename、静默修复歧义配置。  
**Exit gate（安全基础 Gate A）:**

- 外部修改发生时停止而不覆盖。
- MCP/profiles/features/注释/未知键/CRLF/LF/Unicode 等 corpus 的非受管语义保持。
- 在 backup、stage、commit 前后、verify 前后强制结束应用，重启后只得到明确旧状态、明确新状态或用户可决定的恢复状态。
- 替换失败、磁盘满、只读目录、文件占用、权限复制失败和符号链接场景均不产生假成功。
- Gate A 未通过，不得接入真实切换 UI。

### Phase 4：供应商目录与供应商验证闭环

**Rationale:** 只有已验证供应商才能成为任何切换或导出的合法输入。先用 fake provider 固定协议与错误边界，再连接真实供应商。  
**Delivers:**

- 标准供应商草稿、模型发现、默认模型选择、Responses API 流式响应和工具调用闭环。
- 连接/首字节/步骤/总超时、取消、事件/响应大小上限、安全重定向和回环 HTTP 规则。
- validation token、验证后保存、编辑关键字段失败时保留旧的已验证配置。
- DayWay 的未配置推荐供应商、验证后排序、模板更新确认和供应商删除保护。
- 首次供应商验证成功后触发原生 Codex 环境自动切换所需的后端意图；实际切换由 Phase 5 完成。

**Addresses:** TS-02、TS-03、TS-04、TS-05、D-01、D-06、D-08。  
**Avoids:** 未验证草稿进入供应商目录、跨主机重定向泄露 Authorization、记录请求/响应正文、失败验证覆盖旧配置。  
**Exit gate:**

- fake provider 覆盖模型缺失、SSE 分片/断流、工具 call ID 错误、慢响应、超限事件、错误状态和重定向。
- 任一步失败均停止后续步骤且旧的已验证配置不变。
- 日志、错误 DTO、通知、临时目录和诊断测试中不存在 canary API Key。
- 真实供应商只用于受控验收，不成为 CI 依赖。

### Phase 5：原生 Codex 环境、外部配置与重启协调

**Rationale:** 该阶段首次形成 GPTEasy 的核心价值闭环：已验证供应商真正写入原生 Codex 环境，并准确表达进程是否已读取。  
**Delivers:**

- 宿主应用/本机 Codex CLI 检测，统一为一个原生 Codex 环境。
- 宿主应用缺失、原生 Codex 环境缺失、外部配置、代理 API 模式和原厂登录模式。
- 首次自动切换、后续供应商切换、供应商配置传播、供应商删除引用检查。
- 设置页/托盘共享的 plan/apply use case 和切换预确认。
- 桌面宿主应用优雅重启、本机 Codex CLI 保守待重启、真实 RestartOutcome。

**Addresses:** TS-06、TS-07、TS-09、TS-13、TS-14、D-02、D-03、D-05、D-09。  
**Avoids:** Pitfall 5，按进程名强杀、保存 PID 后长期复用、配置写成功即显示已生效、访问拒绝视为无进程。  
**Exit gate（核心价值 Gate B）:**

- 仅宿主应用、仅 CLI、两者都有、两者都没有、多 CLI、同名未知进程和访问拒绝均有正确状态。
- 取消发生在写入前；稍后重启不终止进程；立即重启只报告可验证结果。
- 配置内容、SQLite 环境绑定、进程快照、设置页和托盘状态一致。
- 外部配置不会被自动绑定或覆盖。

### Phase 6：React 设置窗口、托盘与 UI Contract

**Rationale:** 后端 command、投影和核心切换稳定后再构建 UI，避免由前端乐观状态定义错误业务语义。  
**Delivers:**

- UI-SPEC 锁定的供应商页、原生 Codex 环境状态、首次使用、空状态、验证进度、重启确认和设置页。
- 托盘只列已验证供应商并只切换原生 Codex 环境；原厂登录模式、外部配置和待重启可见。
- 单实例、唯一设置窗口、关闭隐藏、明确退出、更新退出和系统退出状态机。
- 简体中文/英语动态切换，托盘、原生对话框、通知和未来后台结果同步语言。
- 浅色/深色/系统主题、键盘、焦点管理、ARIA、200% 缩放、高对比度和减少动态效果。

**Addresses:** TS-10、TS-17、TS-18、D-10；承载 Phase 4/5 的可观察验收。  
**Avoids:** Pitfall 9、12、13，重复窗口/tray/listener、只翻译 React、隐藏 API Key 仍进入可访问名称、验证状态仅靠颜色。  
**Exit gate:**

- show/hide/close/reopen/quit 循环不增长窗口、tray 或 listener。
- 语言切换同步 React、托盘、对话框和通知。
- 核心供应商流程可纯键盘完成；自动 a11y 与 Windows/NVDA、macOS/VoiceOver 手工脚本通过。
- API Key 离开供应商页面后恢复隐藏，且不进入全局前端状态、localStorage、错误边界或调试快照。

### Phase 7：Windows WSL2 独立管理

**Rationale:** WSL2 复用配置事务与环境绑定，但具有独立的被动发现、默认用户、临时启动和恢复风险，必须单独收敛。  
**Delivers:**

- WSL1/WSL2 识别、发行版名称/运行集合、WSL2 默认用户和未配置 WSL2 环境。
- 逐发行版锁、单发行版切换、WSL2 待重启和逐项错误。
- WSL2 临时启动确认、持久化租约、失败/取消/崩溃恢复和用户并发活动检测。
- “应用到全部 WSL2”的确认快照、默认串行执行和逐发行版结果。
- 主机 Rust 生成配置变更，供应商数据通过 stdin/受控文件传递，不进入命令行。

**Addresses:** TS-11、TS-13、D-02、D-04、D-11。  
**Avoids:** Pitfall 6、10，解析本地化表格、猜 `/home/<name>`、launcher 依赖、命令字符串拼接、`wsl --shutdown`、批量全局成功布尔值。  
**Exit gate:**

- 简中/英文 Windows、WSL1/2 混合、Store/imported 发行版、默认 root、空格/Unicode 名称均覆盖。
- 检测不启动停止的发行版。
- 每个状态机步骤 kill 注入后，遗留租约可见且可恢复。
- 操作期间出现用户活动时不自动终止发行版。
- Windows x64/ARM64 的真实 WSL2 核心路径通过。

### Phase 8：独立 Linux 切换脚本

**Rationale:** 导出物脱离 GPTEasy，不能依赖 Rust 运行时恢复；必须把脚本当作含明文凭据的独立配置工具验收。  
**Delivers:**

- Bash 4+ 与 Zsh 5+ 两套生成模板，包含全部已验证供应商，DayWay 验证后排第一。
- Shell/界面语言选择、预览、复制、保存和固定明文凭据警告。
- 目标 Linux 上的 Linux 供应商选择、直接退出不写入、当前供应商读取和待重启提示。
- `config.toml`/实际凭据载体的唯一 GPTEasy 管理区块、五份备份、`umask 077`、同目录 temp、trap 清理和复读。
- TOML 编码与 shell literal 编码分层，拒绝 NUL、换行和不支持控制字符。

**Addresses:** TS-12、D-04、D-07、D-08。  
**Avoids:** Pitfall 7、11，`eval`、单一 polyglot 模板、`sed -i`、`/tmp` 固定临时文件、损坏区块自动修复、宽松权限。  
**Exit gate（环境扩展 Gate C）:**

- Bash 4.4/当前稳定和 Zsh 5.x/当前稳定真实执行通过，而非只有语法检查。
- 恶意字段与 Unicode 属性测试通过，写入后结构化解析值完全一致。
- 重复、嵌套、缺失结束 marker、磁盘满、SIGINT、只读目录和 `mv` 失败均保留原配置。
- 脚本、临时文件和备份权限符合明文供应商凭据要求，无残留 temp。

### Phase 9：诊断、更新、登录启动与发布基础设施

**Rationale:** 运行维护依赖稳定的 operation/error model；签名、安装和 updater 又必须在正式验收前提前跑通，不能留到发布最后一天。  
**Delivers:**

- 七天结构化脱敏日志、从允许清单构造的用户主动诊断导出和 canary 扫描。
- 每日最多一次更新检查、持久化节流、用户确认下载/安装和失败恢复。
- 更新 cleanup barrier：停止新操作、等待安全提交点、刷盘、处理 WSL2 租约后再允许 updater 退出。
- 登录启动默认关闭、回读实际状态和更新/移动应用后的校验。
- Windows NSIS currentUser x64/ARM64、WebView2 策略、Authenticode 与 updater 工件。
- macOS universal DMG、`~/Applications` 或等价当前用户安装流程、Developer ID、公证/staple 和 updater 权限。
- updater 私钥恢复、旧客户端信任和密钥轮换预案。

**Addresses:** TS-16、TS-17、TS-18 的运行维护和发布部分。  
**Avoids:** Pitfall 4、8、14、15，诊断目录递归打包、检查即安装、更新中断配置事务、工件签名后再修改、架构/URL/`.sig` 错配。  
**Exit gate（发布候选 Gate D）:**

- 诊断 zip 不含数据库、数据库备份、Codex 配置、配置备份、Linux 切换脚本或 canary。
- N-1/N-2 正式签名安装版可经候选正式端点升级；错误签名、错误架构、404、中断、磁盘满和用户取消可恢复。
- Windows 普通用户安装无 UAC，x64/ARM64 工件和 updater target 匹配。
- macOS 最终下载物在 Intel/Apple Silicon 上通过完整签名、公证、Gatekeeper、当前用户更新和登录启动。

### Phase 10：完整首版跨平台验收与发布放行

**Rationale:** 完整 v1 是一个整体；单个平台、单架构或部分能力通过不构成正式发布。  
**Delivers:**

- Windows 10 22H2+ x64、Windows 11 ARM64、macOS 14+ Intel/Apple Silicon 的发布证据。
- 旧数据库、配置写入故障、宿主/CLI、WSL2、Linux 切换脚本、托盘、双语、无障碍、安装和更新的端到端矩阵。
- Release checklist、已知限制、恢复手册和 go/no-go 记录。

**Addresses:** 所有 TS-01–TS-18 与锁定的完整首版要求。  
**Avoids:** “能构建即完成”“开发机可运行即完成”“只测 happy path”“把部分范围当正式 v1”。  
**Exit gate（v1 Gate E）:**

- 数据完整性、供应商凭据、进程/待重启、WSL2、shell、签名、安装、更新和无障碍门禁全部通过。
- 任何 CRITICAL 风险无未关闭例外；HIGH 风险若不能关闭则阻止发布。
- 发布工件、update metadata、签名和文档来自同一不可变候选。
- 只有 Gate E 通过后才发布正式首版。

### Phase Ordering Rationale

```text
实现契约 spike
  → 工具链/领域/安全类型
  → SQLite/迁移/operation journal
  → Codex 兼容与配置事务
  → 供应商目录/供应商验证
  → 原生 Codex 环境/重启协调
  → React/托盘核心 UI
  → WSL2
  → Linux 切换脚本
  → 诊断/更新/安装/签名
  → 完整首版发布验收
```

- Phase 0 先冻结外部事实，防止后续架构建立在 Codex、WSL2、进程或安装行为的猜测上。
- Phase 2 先于任何外部写入，因为崩溃恢复需要永久 journal 和可靠迁移。
- Phase 3 是原生 Codex 环境、WSL2 和 Linux 用户级配置共同的数据完整性内核，不能由三个阶段分别实现近似版本。
- Phase 4 只产生已验证供应商；Phase 5 才把它们应用到原生 Codex 环境并完成首次自动切换。
- Phase 6 使用稳定 projection 和 command，不让 React/托盘复制领域规则。
- Phase 7/8 都复用前置领域和配置契约，但分别拥有 Windows/WSL2 与 Bash/Zsh 的独立故障面。
- Phase 9 收敛运行维护与发布设施，但最小打包/签名可行性必须在 Phase 0 提前验证。
- Phase 10 是完整首版唯一对外发布门。

### Research Flags

#### 规划时需要 `$gsd-plan-phase --research-phase`

- **Phase 0：必须研究。** 需要当时目标 Codex 版本、正式宿主应用、WSL2 和签名环境的一手验证。
- **Phase 3：必须研究。** Codex 配置字段、认证/凭据载体、配置优先级、Windows 替换与 macOS 刷盘语义会直接决定数据完整性。
- **Phase 5：必须研究。** Windows/macOS 正式宿主应用身份、优雅关闭/重新激活和 CLI 边界不能靠进程名推断。
- **Phase 7：必须研究。** 停止 WSL2 环境的默认用户名、临时启动所有权和并发用户活动需要真实环境 spike。
- **Phase 9：必须研究。** macOS 当前用户安装、updater 写权限、Windows ARM64、双签名链和密钥轮换属于发布基础设施事实。
- **Phase 10：针对发布候选做验证型研究。** 重点是目标平台、架构和正式签名工件的差异，不是重新设计功能。

#### 模式成熟，可不单独运行 research-phase

- **Phase 1：** 领域建模、Ports/Adapters、窄 command、Secret 类型和 DTO 生成属于成熟模式；按锁定基线实施即可。
- **Phase 2：** SQLite Online Backup、顺序迁移、事务和 fixture 测试有成熟官方模式；重点是执行与故障注入。
- **Phase 4：** HTTP/SSE 状态机可由 fake provider 和官方协议契约驱动；如 Phase 0 已冻结目标协议，不需要重复广泛研究。
- **Phase 6：** React/Tauri 投影、表单、i18n 和无障碍模式成熟；需要真实验收但不需要重新研究 UI 方向。
- **Phase 8：** Bash/Zsh 两套模板、编码与原子文件模式可直接实施；需要高强度属性/运行时测试，而不是扩大范围研究。

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| 锁定产品与领域基线 | HIGH | `PROJECT.md`、`CONTEXT.md`、ADR 0001–0008 和 UI-SPEC 对范围、术语与行为定义明确 |
| Stack | MEDIUM | 版本和官方能力已核对，但依赖更新频繁，Windows ARM64 runner、Tauri macOS E2E 和发布账号仍需实证 |
| Features | HIGH | 研究主要把已锁定 v1 转为依赖、状态和可观察验收，没有依赖竞品猜测 |
| Architecture | MEDIUM | Rust 权威、Ports/Adapters、operation journal 和状态分离可信；Codex 凭据载体、进程身份和 WSL2 默认用户仍未验证 |
| Pitfalls | MEDIUM | 主要风险由 SQLite、Tauri、Windows、Apple、WSL 和 shell 一手资料支撑，但平台组合行为必须在正式工件上验证 |

**Overall confidence:** MEDIUM

### Gaps to Address

- **Codex 配置兼容性：** 目标 Codex 版本的实际路径、字段、认证/凭据载体、配置优先级和原厂登录模式必须在 Phase 0/3 冻结；这是直接配置模式的首要 blocker。
- **宿主应用身份与重启：** Windows executable/package identity、macOS bundle identifier、正式安装路径和可重启能力未确认；必须用正式签名版本验证。
- **WSL2 默认用户被动发现：** 已知默认 UID 不等于可展示用户名；必须证明不启动停止发行版也能满足锁定 UI 行为。
- **WSL2 临时启动所有权：** 操作期间出现外部活动时能否可靠判定“仍由 GPTEasy 所有”未解决；无法证明时必须保守不终止并清楚报告。
- **macOS 当前用户安装：** DMG 如何引导到用户可写位置、应用被移动后 updater/登录启动如何工作，需要发布 spike。
- **Windows ARM64：** GitHub-hosted runner 仍不是充分证据，正式发布需要 ARM64 真机或稳定自托管 runner。
- **Updater 密钥生命周期：** 私钥恢复、旧客户端信任、轮换桥接版本和正式 metadata endpoint 尚未冻结。
- **GPTEasy 管理区块精确定义：** 注释边界、允许修改字段、首次遇到已有配置的无歧义迁移规则应由 Phase 0/3 固定，并供 Rust、WSL2 和 Linux 切换脚本共享 fixture。
- **诊断脱敏清单：** 允许字段、URL 摘要规则、嵌套错误和脚本生成错误的 canary corpus 需要在 Phase 1/2 建立并持续扩展。
- **供应商搜索阈值：** UI 已锁定“较多时可搜索”，但具体数量未定义；需求阶段应给出可自动验收的阈值，不改变 UI 方向。
- **版本冻结时点：** 研究版本是 2026-08-05 的建议组合；开始实现时应验证 registry 可获取性和 lockfile 解算，但不得把依赖升级演变为重新选择技术栈。

## Sources

### Primary（HIGH confidence）

- `C:/src/GPTEasy/.planning/PROJECT.md` — Core Value、完整首版范围、约束、阶段优先级。
- `C:/src/GPTEasy/CONTEXT.md` — 精确领域术语、供应商生命周期、受管环境、备份、待重启、凭据、平台和发布行为。
- `C:/src/GPTEasy/docs/adr/0001-plaintext-provider-credentials.md` — 明文供应商凭据与脱敏边界。
- `C:/src/GPTEasy/docs/adr/0002-tauri-rust-react.md` — Tauri 2 + Rust + TypeScript/React 架构边界。
- `C:/src/GPTEasy/docs/adr/0003-standalone-linux-switch-functions.md` — 独立 Linux 切换脚本、GPTEasy 管理区块和无运行时依赖。
- `C:/src/GPTEasy/docs/adr/0004-direct-codex-configuration.md` — 直接配置模式、当前用户默认配置、备份和原子替换。
- `C:/src/GPTEasy/docs/adr/0005-immutable-provider-identity.md` — 不可变供应商 ID 和外部配置匹配。
- `C:/src/GPTEasy/docs/adr/0006-sqlite-application-state.md` — SQLite 所有权、永久顺序迁移、备份和降级边界。
- `C:/src/GPTEasy/docs/adr/0007-local-only-product.md` — 本地模式和跨设备边界。
- `C:/src/GPTEasy/docs/adr/0008-native-codex-environment.md` — 原生 Codex 环境模型。
- `C:/src/GPTEasy/docs/ui/UI-SPEC.md` — 托盘、设置窗口、验证、重启、WSL2、Linux 脚本、主题与无障碍。

### External Primary（MEDIUM confidence）

- Rust、Node.js、npm、crates.io 官方版本与 registry 元数据 — 工具链和依赖版本。
- Tauri 2 官方文档 — command、capabilities、system tray、single instance、autostart、updater、Windows/macOS 分发、签名和 WebDriver。
- SQLite 官方文档 — Online Backup API、WAL、事务、原子提交、locking 和 PRAGMA。
- OpenAI Codex 官方配置参考与 Responses API 文档 — 配置字段、认证、流式响应和工具调用协议。
- Microsoft 官方文档 — WSL CLI/API、`ReplaceFileW`、`MoveFileExW`、进程枚举、应用激活、Artifact Signing 和 SignTool。
- Apple 官方文档 — `NSRunningApplication`、`NSWorkspace`、Developer ID、公证和 Gatekeeper。
- GNU Bash、Zsh、POSIX 官方规范 — function、引用、文件替换和 shell 行为。
- WCAG 2.2、WAI-ARIA Authoring Practices — 无障碍验收基线。

### Research Inputs（本总结直接综合）

- `.planning/research/STACK.md`
- `.planning/research/FEATURES.md`
- `.planning/research/ARCHITECTURE.md`
- `.planning/research/PITFALLS.md`

---
*Research completed: 2026-08-05*  
*Ready for roadmap: yes*
