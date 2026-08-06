# Roadmap: GPTEasy

## Overview

GPTEasy v1 采用一次完整发布策略，内部按用户选择的 Horizontal Layers 模式推进：先冻结外部实现契约并建立 SQLite 与配置安全底座，再完成供应商、原生 Codex 环境、WSL2 和 Linux 切换脚本等依赖层，随后统一落实托盘与设置界面，最后通过更新、签名和跨平台发布门禁。所有阶段都只是内部可测试边界；只有 Phase 8 的完整首版放行条件全部满足后，才发布正式 v1。

## Dependency and Risk Rationale

- **外部契约先于正式写入**：目标 Codex 版本的配置路径、字段、认证模式、供应商凭据载体和优先级必须由 Windows/macOS 实机与 fixture 验证；不能把未经证实的文件布局固化到直接配置模式。
- **SQLite 先于跨资源操作**：永久顺序迁移、一致备份、未来版本拒写和启动恢复是供应商目录、环境绑定与 operation journal 的前置条件。
- **可恢复文件写入先于切换入口**：备份、同目录临时文件、平台原子替换、并发指纹复核、写后复读和崩溃对账未通过时，不开放真实配置切换。
- **凭据可见不等于可传播**：供应商凭据允许明文保存、查看和导出，但日志、错误、通知、诊断导出和非必要临时文件必须统一脱敏并通过 canary 扫描。
- **WSL2 与 Shell 各自 fail-closed**：WSL2 临时启动必须保留原运行状态并识别用户并发活动；Bash/Zsh 脚本必须使用分层编码、禁止 `eval`，遇到歧义、权限或写入故障时保留原配置。
- **签名与更新是双重信任链**：Windows/macOS 平台签名或公证与 Tauri updater 内容签名分别验证；更新退出必须等待配置事务安全提交和 WSL2 临时启动恢复。
- **完整 v1 统一放行**：真实 Windows x64/ARM64、macOS Intel/Apple Silicon、WSL2、Bash/Zsh、双语、无障碍、数据库升级、故障恢复、安装和更新全部通过，才允许发布同一个不可变候选。

## Phases

**Phase Numbering:**

- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

- [ ] **Phase 1: 可信本地状态与实现契约** - 让本地状态可持久保存、安全升级，并冻结后续直接配置所依赖的外部事实。
- [ ] **Phase 2: 可恢复配置事务与敏感信息边界** - 建立保留用户配置、可恢复写入和统一凭据脱敏的安全内核。
- [ ] **Phase 3: 供应商目录与供应商验证** - 只让完成完整供应商验证的标准供应商进入供应商目录。
- [ ] **Phase 4: 原生 Codex 环境编排** - 可靠检测、切换和重启原生 Codex 环境，同时保留外部配置。
- [ ] **Phase 5: Windows WSL2 管理** - 为每个 WSL2 环境独立、安全地执行单个或批量供应商切换。
- [ ] **Phase 6: 独立 Linux 切换脚本** - 导出可脱离 GPTEasy 长期使用且安全修改 Linux 用户级配置的 Bash/Zsh function。
- [ ] **Phase 7: 设置窗口、托盘与应用生命周期** - 用锁定的 UI Contract 统一承载全部核心能力和桌面生命周期。
- [ ] **Phase 8: 更新、签名与完整首版放行** - 通过用户控制更新、正式签名工件和全矩阵门禁交付完整 v1。

## Phase Details

### Phase 1: 可信本地状态与实现契约

**Goal**: 用户的供应商、环境选择和设置能够可靠保存在本机，并在版本升级或降级边界下保持可恢复。
**Depends on**: Nothing (first phase)
**Requirements**: STATE-01, STATE-02, STATE-03, STATE-04, STATE-05
**Success Criteria** (what must be TRUE):

  1. 用户关闭并重新打开 GPTEasy 后，供应商目录、验证状态、各受管环境的当前供应商和应用设置保持不变。
  2. 用户不需要产品账户，供应商凭据、当前状态和诊断日志只保存在当前用户本机，且不会默认上传。
  3. 任一正式历史数据库都能通过永久顺序迁移升级到当前版本，升级前生成可打开的完整备份并只保留最近三份。
  4. 迁移失败时原数据库完整回滚且应用不会清空数据继续运行；旧版应用遇到更高版本数据库时拒绝写入，并允许用户通过升级前备份完成降级恢复。

**依赖与风险理由**: 在确定 SQLite schema 和环境状态模型前，必须以目标 Codex 版本、正式宿主应用、代表性 WSL2 环境和签名打包 smoke 验证配置路径、字段、认证/供应商凭据载体、进程身份、停止发行版默认用户发现及当前用户安装可行性。任何未验证事实都作为 blocker，而不是由后续业务代码猜测。
**Plans**: 6/28 plans executed

Plans:
**Wave 1**

- [x] 01-01-PLAN.md — 建立隔离公开 registry 的 npm legitimacy verifier 与污染/泄漏负例
- [x] 01-03-PLAN.md — 固定唯一 Scope/Target/Mode runner CLI、退出码与 consumer tests

**Wave 2** *(blocked on Wave 1 completion)*

- [x] 01-02-PLAN.md — 在首次 npm install 前完成人工官方 package 批准
- [x] 01-04-PLAN.md — 建立 manifest schema、脱敏 validator 与 attested provenance 核心
- [x] 01-05-PLAN.md — 实现固定最低版本、认证和仓库权限的 gh preflight
- [x] 01-07-PLAN.md — 建立只读 plan/source/path/CLI/digest 机器审计

**Wave 3** *(blocked on Wave 2 completion)*

- [ ] 01-06-PLAN.md — 在真实 evidence verification 前批准 gh 环境
- [ ] 01-08-PLAN.md — 锁定可重复构建且无业务范围的 React/TypeScript 壳
- [ ] 01-11-PLAN.md — 实现 Windows CLI/正式宿主 canary parity 与被动 WSL2 probes
- [ ] 01-12-PLAN.md — 建立 macOS 双架构原生 zsh Wave 0 与宿主 contract

**Wave 4** *(blocked on Wave 3 completion)*

- [ ] 01-09-PLAN.md — 创建最小 Tauri/Rust 当前用户应用壳

**Wave 5** *(blocked on Wave 4 completion)*

- [ ] 01-10-PLAN.md — 用真实 AppHandle 验证固定 app_local_data_dir 与跨进程 reopen

**Wave 6** *(blocked on Wave 5 completion)*

- [ ] 01-13-PLAN.md — 建立 Windows package 正控制、账户生命周期与 attested workflow
- [ ] 01-14-PLAN.md — 建立 macOS package 正控制、Wave 0 依赖与 attested workflow

**Wave 7** *(blocked on Wave 6 completion)*

- [ ] 01-15-PLAN.md — 获取 Windows x64/ARM64 独立 attested freeze evidence
- [ ] 01-16-PLAN.md — 获取 macOS Intel/Apple Silicon 独立 attested freeze evidence

**Wave 8** *(blocked on Wave 7 completion)*

- [ ] 01-17-PLAN.md — 执行 Strict freeze 并批准 schema/backup 两个 one-way 合同

**Wave 9** *(blocked on Wave 8 completion)*

- [ ] 01-18-PLAN.md — 以 Tauri command→SQLite→新进程 bootstrap 贯通生产 tracer

**Wave 10** *(blocked on Wave 9 completion)*

- [ ] 01-19-PLAN.md — 扩展完整供应商/验证/环境/设置状态重开

**Wave 11** *(blocked on Wave 10 completion)*

- [ ] 01-20-PLAN.md — 建立 truthful installed state smoke、local-only 与跨进程协调

**Wave 12** *(blocked on Wave 11 completion)*

- [ ] 01-21-PLAN.md — 创建 append-only registry 与确定性 create-once v001 fixture

**Wave 13** *(blocked on Wave 12 completion)*

- [ ] 01-22-PLAN.md — 建立历史迁移矩阵、history lock 与 forbidden-migration lint

**Wave 14** *(blocked on Wave 13 completion)*

- [ ] 01-23-PLAN.md — 实现统一只读 DB contract validator 与 preflight

**Wave 15** *(blocked on Wave 14 completion)*

- [ ] 01-24-PLAN.md — 实现 verified backup、三份 retention、rollback 与并发协调

**Wave 16** *(blocked on Wave 15 completion)*

- [ ] 01-25-PLAN.md — 实现 higher-schema 拒写、quarantine、restore 与 headless recovery

**Wave 17** *(blocked on Wave 16 completion)*

- [ ] 01-26-PLAN.md — 重跑 Windows 双架构最终 full-state/recovery installed evidence
- [ ] 01-27-PLAN.md — 重跑 macOS 双架构最终 full-state/recovery installed evidence

**Wave 18** *(blocked on Wave 17 completion)*

- [ ] 01-28-PLAN.md — 执行只读 PhaseComplete 与 STATE-01..STATE-05 最终批准

### Phase 2: 可恢复配置事务与敏感信息边界

**Goal**: 用户的 Codex 配置在任何正常切换、并发修改或故障中都不会被静默破坏，非必要输出也不会泄露供应商凭据。
**Depends on**: Phase 1
**Requirements**: STATE-06, STATE-07, STATE-08, OPS-01, OPS-02, OPS-03
**Success Criteria** (what must be TRUE):

  1. GPTEasy 修改 Codex 配置时只改变供应商相关字段或明确的 GPTEasy 管理区块，用户其他设置、未知字段和可保留格式保持不变。
  2. 每个受管环境在写入前生成带时间戳的配置备份并只保留最近五份，正式写入使用同目录临时文件和平台安全的原子替换，成功后复读确认实际内容。
  3. 外部并发修改、损坏或重复区块、歧义、权限错误、磁盘错误或中断发生时，应用停止修改、不给出假成功，并向用户呈现明确的恢复或重试状态。
  4. 用户可以查看最近七天的本地脱敏诊断日志并主动生成诊断导出；日志、技术详情、复制内容和导出都不包含供应商凭据、完整请求、模型输出、SQLite 数据库、配置备份或 Linux 切换脚本，也不会默认上传。

**依赖与风险理由**: 这是所有受管环境共享的数据完整性边界。Codex 配置契约必须通过 corpus 验证，写入流程必须具备资源锁、短期计划凭证、operation journal、提交前文件指纹复核、写后复读和启动恢复；凭据脱敏必须采用字段允许清单与 canary 扫描，不能依赖事后字符串替换。
**Plans**: TBD

### Phase 3: 供应商目录与供应商验证

**Goal**: 用户只能保存并使用真正完成 Codex 所需验证闭环的标准供应商。
**Depends on**: Phase 2
**Requirements**: PROV-01, PROV-02, PROV-03, PROV-04, PROV-05, PROV-06, PROV-07, PROV-08, PROV-09, PROV-10, PROV-12, VALD-01, VALD-02, VALD-03, VALD-04, VALD-05, VALD-06, VALD-07, VALD-08
**Success Criteria** (what must be TRUE):

  1. 用户可以创建只包含名称、服务地址、API Key 和默认模型的标准供应商；远程地址强制 HTTPS，只有三个锁定回环地址允许 HTTP，供应商创建后获得不可变供应商 ID。
  2. 服务地址和 API Key 完整后，用户可以获取实际可用模型并从中选择默认模型；完整供应商验证依次完成默认模型确认、Responses API 流式响应和工具调用闭环，任一步失败都会停止后续步骤。
  3. 验证页面逐项显示等待、进行中、成功或失败，并提供非技术说明、HTTP 状态码和脱敏技术详情；全部成功后才记录验证时间并允许保存。
  4. 用户修改关键配置时必须重新验证，失败后原来的已验证配置和环境引用继续可用；只修改名称等非关键字段时可以直接保存。
  5. DayWay 始终作为内置推荐供应商排在设置列表第一项，未配置时不参与切换；推荐模板更新不覆盖已验证配置，仍被受管环境使用的供应商不能删除，而外部 Linux 切换脚本不构成删除阻塞。

**依赖与风险理由**: 供应商验证必须先用 fake provider 固定模型发现、SSE 分片/断流、工具调用、超时、重定向和大小上限，再进行受控真实供应商验收。Authorization 不得跨主机重定向，失败验证不得覆盖已验证配置，网络请求和响应正文不得进入日志。
**Plans**: TBD
**UI hint**: yes

### Phase 4: 原生 Codex 环境编排

**Goal**: 用户可以在保留实际配置和运行中任务边界的前提下，可靠切换原生 Codex 环境使用的认证模式和当前供应商。
**Depends on**: Phase 3
**Requirements**: NATV-01, NATV-02, NATV-03, NATV-04, NATV-05, NATV-06, NATV-07, NATV-08, NATV-09, NATV-10, NATV-11, NATV-12, NATV-13
**Success Criteria** (what must be TRUE):

  1. 应用把当前用户的统一 ChatGPT 桌面应用 Codex 功能和本机原生 Codex CLI 作为同一个原生 Codex 环境检测；只有 CLI、两者都存在或两者都缺失时，都展示正确且不阻断无关功能的状态。
  2. 无法唯一匹配供应商目录的原生 Codex 配置显示为外部配置且不会被自动覆盖；用户可以在默认的代理 API 模式和只能从设置中主动启用的原厂登录模式之间切换。
  3. 首个已验证供应商自动成为原生 Codex 环境的当前供应商，后续已验证供应商只加入供应商目录；没有运行中 Codex 进程时，用户可以从设置窗口或托盘立即切换。
  4. 存在运行中 Codex 进程时，用户在写入前可以选择立即重启、稍后重启或取消；取消不改变配置或 SQLite 状态，稍后重启进入待重启，立即重启只恢复可安全控制的桌面宿主应用，无法安全恢复的 CLI 保持明确待重启。
  5. 已确认保存的供应商配置传播完成写入后，用户只能选择立即重启或稍后重启，界面不会把已经提交的新配置伪装成可取消回旧配置。

**依赖与风险理由**: 进程身份必须基于正式可验证身份而非名称或长期复用 PID；配置已写入、受管环境当前供应商和运行中进程已读取必须分别建模。设置窗口与托盘必须调用同一 plan/apply use case，任何访问拒绝或进程恢复不确定性都保守呈现待重启。
**Plans**: TBD
**UI hint**: yes

### Phase 5: Windows WSL2 管理

**Goal**: Windows 用户可以分别管理每个 WSL2 环境，并在不破坏发行版原运行状态的前提下执行单个、批量和供应商配置传播。
**Depends on**: Phase 4
**Requirements**: PROV-11, WSL-01, WSL-02, WSL-03, WSL-04, WSL-05, WSL-06, WSL-07, WSL-08, WSL-09, WSL-10
**Success Criteria** (what must be TRUE):

  1. Windows 用户在 WSL2 页面看到每个 WSL2 环境的发行版名称、WSL2 默认用户、运行状态、当前供应商状态和 WSL2 待重启状态；被动检测不会启动已停止发行版，也不会管理其他 Linux 用户。
  2. 新检测到的发行版保持未配置 WSL2 环境；用户可以独立切换任一运行中的 WSL2 环境，且原生 Codex 环境和其他 WSL2 环境不受影响。
  3. 用户切换已停止发行版前会看到 WSL2 临时启动提示；操作完成或失败后恢复原停止状态，若检测到用户并发活动或无法确认临时启动所有权则不强制停止，并清楚报告恢复状态。
  4. 用户可以确认受影响发行版后把一个已验证供应商应用到全部 WSL2，结果按发行版分别展示成功、失败或 WSL2 待重启；仍使用旧配置的 Codex 进程不会被主动终止。
  5. 已验证供应商更新后，新配置自动传播到所有当前使用该供应商的原生 Codex 环境和 WSL2 环境，并逐环境报告独立结果。

**依赖与风险理由**: 停止发行版的 WSL2 默认用户发现和 WSL2 临时启动所有权必须由真实环境验证。供应商数据不得进入命令行，发行版与用户参数不得通过字符串拼接进入 Shell；批量切换是多个独立操作的编排，禁止使用全局 `wsl --shutdown` 或伪造跨发行版原子成功。
**Plans**: TBD
**UI hint**: yes

### Phase 6: 独立 Linux 切换脚本

**Goal**: 用户可以导出包含全部已验证供应商的自包含脚本，并在外部 Linux 上长期、安全地切换当前用户的 Codex 配置。
**Depends on**: Phase 5
**Requirements**: LNX-01, LNX-02, LNX-03, LNX-04, LNX-05, LNX-06, LNX-07, LNX-08, LNX-09
**Success Criteria** (what must be TRUE):

  1. 用户可以分别生成 Bash 4+ 或 Zsh 5+、简体中文或英语的 Linux 切换脚本；脚本包含导出时全部已验证供应商及明文供应商凭据，不包含未验证供应商，也不依赖额外运行时或 GPTEasy。
  2. 复制或保存前用户固定看到明文 API Key 风险提示；“复制脚本”为主操作，保存文件使用锁定文件名，并提供手工安装说明而不自动修改 Shell 启动文件。
  3. 用户调用交互式 Linux 供应商选择 function 时可以看到目标 Linux 的当前供应商与可用供应商，也可以直接退出且不修改任何配置。
  4. 用户选择供应商后，function 只修改当前 Linux 用户配置中的 GPTEasy 管理区块，保留其他配置，并使用安全权限、最近五份备份、同目录临时文件、原子替换和写后校验。
  5. GPTEasy 管理区块损坏、重复或存在歧义，或者发生只读目录、磁盘、信号中断或替换失败时，function 停止修改并保留原配置。

**依赖与风险理由**: Bash 与 Zsh 必须使用独立模板，对 TOML 值和 shell literal 分层编码，拒绝不支持的控制字符并禁止 `eval`。脚本需要在真实 Bash 4+/Zsh 5+ 中执行恶意字段、Unicode、权限、磁盘满和 SIGINT 测试，而不只做语法检查。
**Plans**: TBD
**UI hint**: yes

### Phase 7: 设置窗口、托盘与应用生命周期

**Goal**: 用户可以通过一致、清晰、双语且托盘优先的桌面界面完成所有核心任务，并理解应用的驻留和通知行为。
**Depends on**: Phase 6
**Requirements**: UX-01, UX-02, UX-03, UX-04, UX-05, UX-06, UX-07, UX-08, UX-09, UX-10, UX-11, OPS-07, OPS-08, OPS-09
**Success Criteria** (what must be TRUE):

  1. 设置窗口按平台显示供应商、WSL2、Linux 脚本和设置页面，首次启动直接进入供应商页面并展示欢迎横幅；所有空状态都有明确说明和下一步。
  2. 用户通过紧凑供应商卡片和统一完整单页查看、新增、编辑、验证或删除供应商；列表达到统一且已记录的阈值时可搜索，API Key 的明文、隐藏、复制和离页恢复行为符合 UI Contract。
  3. 托盘只列出已验证供应商并只切换原生 Codex 环境，正确显示当前供应商、原厂登录模式、外部配置或待重启状态；关闭设置窗口后应用继续托盘驻留，只有明确“退出 GPTEasy”才结束程序。
  4. 页面内状态、Toast、系统通知和模态对话框各自只承担锁定职责，同一结果不重复通知，通知与错误正文不包含 API Key 或完整服务地址。
  5. 用户可以切换跟随系统、浅色或深色主题以及简体中文或英语；登录启动默认关闭并在首次使用时选择，所有设置会同步到窗口、托盘、原生对话框和通知并反映实际系统状态。

**依赖与风险理由**: React 只能消费 Rust 后端的窄 command 与脱敏投影，不能成为第二权威状态源或持久保存完整 API Key。单实例、唯一设置窗口、托盘、listener、登录启动和通知必须形成可重复 show/hide/close/reopen/quit 的生命周期状态机；本阶段计划必须记录供应商搜索阈值。
**Plans**: TBD
**UI hint**: yes

### Phase 8: 更新、签名与完整首版放行

**Goal**: 用户可以在受支持平台上安全安装和更新 GPTEasy，并且完整 v1 只有在全部平台、恢复和无障碍门禁通过后才发布。
**Depends on**: Phase 7
**Requirements**: OPS-04, OPS-05, OPS-06, REL-01, REL-02, REL-03, REL-04, REL-05, REL-06, REL-07, REL-08
**Success Criteria** (what must be TRUE):

  1. 应用启动后每天最多检查一次更新且不上传供应商数据；发现更新后，下载和安装都由用户确认，退出安装前会等待配置操作到达安全提交点、刷新状态并处理 WSL2 临时启动恢复。
  2. Windows 10 22H2+ x64/ARM64 和 macOS 14+ Intel/Apple Silicon 用户可以按当前用户范围安装、运行和更新正式工件；Linux 不提供 GUI 安装物，只验收独立 Linux 切换脚本。
  3. 所有核心供应商、原生 Codex 环境、WSL2、Linux 脚本和设置操作都可通过键盘完成，并在屏幕阅读器、清晰焦点、WCAG AA、200% 缩放、高对比度和减少动态效果下保持可用。
  4. Windows/macOS 工件分别通过平台代码签名或公证、平台信任验证和 Tauri updater 内容签名；旧正式版本可以安全升级，错误签名、错误架构、中断、失败或用户取消不会损坏已安装应用或用户状态。
  5. 只有 Windows/macOS 全架构、真实 WSL2、Bash/Zsh、双语、无障碍、历史数据库升级、配置故障恢复、安装和更新门禁全部通过，且工件、更新元数据、签名和文档来自同一不可变候选时，才发布完整 v1。

**依赖与风险理由**: 平台签名/公证与 Tauri updater 内容签名是两条独立信任链，必须验证 N-1/N-2 正式安装版、密钥恢复、架构与 URL 匹配、取消和故障恢复。任何未关闭的严重数据完整性、供应商凭据泄露、WSL2 生命周期、Shell 安全或平台发布阻断问题都会阻止 Gate E 放行。
**Plans**: TBD
**UI hint**: yes

## Coverage

| Phase | Requirement Count |
|-------|-------------------|
| 1. 可信本地状态与实现契约 | 5 |
| 2. 可恢复配置事务与敏感信息边界 | 6 |
| 3. 供应商目录与供应商验证 | 19 |
| 4. 原生 Codex 环境编排 | 13 |
| 5. Windows WSL2 管理 | 11 |
| 6. 独立 Linux 切换脚本 | 9 |
| 7. 设置窗口、托盘与应用生命周期 | 14 |
| 8. 更新、签名与完整首版放行 | 11 |
| **Total** | **88** |

每项 v1 要求只映射到一个阶段；无遗漏、无重复。完整逐项映射见 `.planning/REQUIREMENTS.md` 的 Traceability 表。

## Progress

**Execution Order:**
Phases execute in numeric order: 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. 可信本地状态与实现契约 | 6/28 | In Progress|  |
| 2. 可恢复配置事务与敏感信息边界 | 0/TBD | Not started | - |
| 3. 供应商目录与供应商验证 | 0/TBD | Not started | - |
| 4. 原生 Codex 环境编排 | 0/TBD | Not started | - |
| 5. Windows WSL2 管理 | 0/TBD | Not started | - |
| 6. 独立 Linux 切换脚本 | 0/TBD | Not started | - |
| 7. 设置窗口、托盘与应用生命周期 | 0/TBD | Not started | - |
| 8. 更新、签名与完整首版放行 | 0/TBD | Not started | - |
