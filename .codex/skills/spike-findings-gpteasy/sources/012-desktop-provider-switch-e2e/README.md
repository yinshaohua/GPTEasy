---
spike: 012
name: desktop-provider-switch-e2e
type: standard
validates: "Given Tauri UI、真实已验证供应商、SQLite 状态、Codex 配置及运行中的桌面和 CLI，when 用户完成验证并选择立即重启、稍后重启或取消，then 验证、迁移、Saga、协调和进程语义的数据交接完整且最终状态可解释"
verdict: VALIDATED
related: [004, 006, 007, 008, 011]
tags: [tauri, provider, sqlite, config, reconciliation, process, integration, e2e]
---

# Spike 012: 桌面供应商切换端到端

## What This Validates

**Given** Tauri UI、真实已验证供应商、SQLite 状态、隔离的 Codex 用户配置，以及当前运行中的桌面 Codex 和本机 CLI，  
**when** 用户选择立即重启、稍后重启或取消，并在配置替换、数据库提交、外部编辑和重启边界发生故障，  
**then** 供应商验证、首次接管、可恢复 Saga、最终有效配置协调和进程决策形成一条完整链路，API Key 不进入数据库或诊断，真实 CLI 不被终止。

## Research

### 已检查的官方资料

- Tauri 2 Rust command 调用：`https://v2.tauri.app/develop/calling-rust/`
- Tauri 2 state management：`https://v2.tauri.app/develop/state-management/`
- `rusqlite` transaction API：`https://docs.rs/rusqlite/latest/rusqlite/struct.Transaction.html`
- OpenAI Responses API：`https://platform.openai.com/docs/api-reference/responses`
- OpenAI function calling：`https://platform.openai.com/docs/guides/function-calling`
- OpenAI Codex 配置基础：`https://developers.openai.com/codex/config-basic`
- 本机 `codex-cli 0.146.0` app-server `config/read`
- Spike 004、006、007、008、011 的已验证实现和限制

### 方案比较

| Approach | Tool/Library | Pros | Cons | Status |
|---|---|---|---|---|
| 各组件独立运行，通过 JSON 文件串联 | 011 validator + 006 writer + 007 Saga | 最大程度复用现有 Spike | 验证结果没有天然绑定到之后使用的 Key；组件间存在 TOCTOU 和敏感文件交接 | 淘汰为正式链路 |
| UI 直接依次调用多个松散 Tauri command | Tauri command | 前端易展示阶段 | 用户可在验证与保存之间修改字段或重复点击，后端难保证同一组合 | 不采用 |
| 单个后端调用持有类型化 `VerifiedProvider` | Rust + Tauri + `rusqlite` | 地址、模型和 Key 在同一进程中绑定；只有验证类型可进入 Saga | 单次真实验证可能持续数秒，需要后台线程和阶段反馈 | **采用** |
| 把桌面重启纳入配置回滚事务 | 进程终止与重新激活 | 表面上像“全成功或全失败” | 重启不可补偿，CLI 无法恢复原终端；会错误回滚已生效配置 | 淘汰 |

**Chosen approach:** Tauri 的一个后端调用加载或接收供应商，在同一 Rust 进程完成完整验证，产生不可由 UI 伪造的 `VerifiedProvider`，随后执行首次接管、Saga、app-server 协调和重启计划。真实进程只扫描；自动化实验不终止当前会话。

### 验证与保存的绑定

仅记录“该供应商验证过”不够。地址、模型或 Key 可能在验证后变化。本 Spike 为精确组合计算：

```text
SHA-256("gpteasy-provider-combination-v1\0" + base_url + "\0" + model + "\0" + api_key)
```

- 只有 `VerifiedProvider` 可以进入切换 Saga。
- 切换前重新计算 fingerprint，与验证证据比较。
- SQLite 只保存 fingerprint，不保存 API Key。
- 任一字段变化都会产生不同 fingerprint，要求重新验证。

## How to Run

### 完整自动化矩阵和真实供应商

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\012-desktop-provider-switch-e2e\run.ps1
```

执行：

1. Rust 单元测试。
2. 15 项确定性端到端矩阵。
3. 若 `.codex/skills/spike-findings-gpteasy/.secrets/provider.json` 存在且被 Git 忽略，在同一 Rust 进程运行真实供应商模型发现、Responses SSE、strict function call 和 nonce 回传。
4. 用真实验证结果写入隔离 Codex 配置和 SQLite。
5. 调用原生 `codex app-server config/read(cwd, includeLayers=true)` 获取有效模型、provider 和来源。
6. 扫描当前真实桌面宿主、bundled Codex 和本机 CLI，并把结果送入“稍后重启”边界。
7. 构建 Tauri release 应用。
8. 扫描 `.run/evidence/`，确认真实 Key 未泄漏。

跳过真实网络：

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\012-desktop-provider-switch-e2e\run.ps1 -SkipLive
```

### 交互式 Tauri UI

```powershell
cd .codex\skills\spike-findings-gpteasy\sources\012-desktop-provider-switch-e2e
npm run tauri dev
```

UI 支持：

- 确定性或真实供应商。
- 立即重启、稍后重启、取消。
- prepared、配置替换、状态提交、外部编辑和重启失败注入。
- 真实进程扫描。
- 验证、Saga、有效配置和协调状态的流水线展示。
- 脱敏报告导出。

所有配置和数据库位于 `.run/ui/`，不会读取或修改真实 `~/.codex/config.toml`。

## What to Expect

`.run/evidence/combined-summary.json`：

- `deterministic_passed = 15`
- `deterministic_total = 15`
- `live_executed = true`
- `live_validated = true`
- `tauri_release_build = "passed"`
- `evidence_secret_leak = false`

2026 年 8 月 5 日的真实供应商执行：

| 阶段 | 结果 | 耗时 |
|---|---|---:|
| 模型发现 | HTTP 200，找到默认模型 | 332 ms |
| SSE 与 strict function call | 21 个事件，出现完成事件 | 3132 ms |
| 工具结果回传 | 25 个事件，最终文本包含 nonce | 2627 ms |

真实 Windows 进程扫描：

- 桌面宿主：1
- bundled Codex：1
- 本机 CLI：2
- 流程结果：`pending_restart`
- 真实进程终止数：0

## Observability

- `events.jsonl` 使用 RFC 3339 UTC 时间和阶段分类。
- 只记录 operation ID、provider ID、fingerprint、配置哈希、阶段、进程布尔值和错误分类。
- 不记录 API Key、完整配置、完整请求/响应、模型输出或完整进程命令行。
- SQLite provider 表保存地址、模型、不可变 ID 和组合 fingerprint，不保存 Key。
- `config/read` 原始响应只在内存处理；证据仅保存 model/provider 和字段来源摘要。
- `.run/workspace/` 是包含受管配置的隔离运行环境；`.run/evidence/` 是允许导出的脱敏证据，两者边界明确。

## Investigation Trail

1. **组件通过不等于接缝通过**：011、006、007、008 和 004 各自成立，但最初没有证明“验证成功的同一 Key”就是最终写入配置的 Key。
2. **引入组合 fingerprint**：验证结果绑定地址、模型与 Key。测试修改 Key 后 fingerprint 必然变化，旧验证证据不能继续保存。
3. **数据库不需要保存 Key**：SQLite 只保存 fingerprint；明文 Key 只存在于验证期间内存和隔离 Codex 配置/备份。
4. **首次接管不是预置区块假设**：端到端场景从已有顶层 `model`、`model_provider`、旧 provider、未知字段和项目 trust 配置开始，迁移后旧 provider 与未知字段仍保留。
5. **故障恢复保持三分支**：旧哈希回滚、新哈希前滚、第三种哈希进入 `needs_attention`。prepared、配置替换和状态提交后的崩溃均得到预期收敛。
6. **重启不回滚配置**：桌面重启失败只进入 `pending_restart`；CLI 始终要求在原终端人工重启。
7. **真实进程参与但不被中断**：扫描识别 1 个桌面宿主、1 个 bundled Codex 和 2 个 CLI，并把存在性送入真实供应商链路；实验没有终止任何真实进程。
8. **app-server 是最终有效状态门禁**：成功切换后 effective model/provider 均来自 user 层；项目层只覆盖 model 时，状态为 `managed_overridden`，用户配置不被反复改写。
9. **Windows canonicalize 出现接缝问题**：`std::fs::canonicalize` 返回 `\\?\` 路径，导致 Codex 项目信任键与 cwd 不匹配，项目层看似存在却不生效。改用 `dunce::canonicalize` 后，project model 正确显示为 project origin。
10. **npm wrapper 不是 app-server 的稳定启动面**：Rust 直接启动 `codex.cmd` 时 JSON-RPC stdio提前关闭。最终定位 npm 包中的原生 `vendor/.../codex.exe`，并使用隐藏窗口运行 app-server。
11. **证据边界必须排除目标配置**：隔离 workspace 中的配置按产品要求包含 Key；泄漏门禁只扫描可导出的 evidence、SQLite 和事件日志。
12. **UI 保持体验但自动化自验证**：Tauri release 构建成功并通过隐藏启动 smoke；核心 verdict 不依赖人工点击。

## Results

### Verdict: VALIDATED ✓

15/15 确定性场景全部通过，真实供应商验证和隔离切换成功，原生 app-server 确认有效状态，真实进程扫描进入重启边界，Tauri release 构建和启动 smoke 通过。

### 已验证

- 未验证供应商无法创建 Saga 或写配置。
- 真实供应商的地址、Key 和模型在同一 Rust 进程完成完整验证并绑定 fingerprint。
- 首次结构化接管、原子替换、备份和 SQLite Saga 能组成同一流程。
- prepared、写入前失败、配置替换后崩溃、状态提交后崩溃和外部编辑均能收敛。
- 真实有效配置可由 app-server 确认，项目层覆盖可见但不会触发自动争夺。
- 取消发生在任何操作或配置写入之前。
- 桌面重启失败不回滚新配置，CLI 不被静默终止。
- API Key 不进入 SQLite、事件日志或脱敏 evidence。
- Tauri UI 能构建、启动并展示完整流水线。

### 限制

- 为避免中断当前会话，没有实际终止和重新激活真实桌面 Codex；该 OS 动作仍沿用 004 的安全边界。
- UI 的“立即重启”在 Spike 中只生成计划和 Saga 结果，不执行真实进程终止。
- 真实供应商只验证一个地址、Key 和模型组合，不代表持续健康。
- `codex-cli 0.146.0` 的 app-server schema 未来升级需要回归。
- 没有覆盖多 GPTEasy 实例同时写入；该资源争用仍属于 frontier Spike 014。
- 没有覆盖系统关机、磁盘损坏、SQLite 文件丢失或真实断电。
