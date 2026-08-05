---
spike: 008
name: external-config-reconciliation
type: standard
validates: "Given 用户层配置被外部修改、存在覆盖层或供应商身份匹配歧义，when GPTEasy 启动或重新扫描，then 能识别受管供应商、展示外部配置和层级差异且不自动覆盖"
verdict: VALIDATED
related: [001, 006, 007]
tags: [codex, config-layer, provider-id, reconciliation, external-config, integration]
---

# Spike 008: 外部配置与供应商身份协调

## What This Validates

**Given** 当前用户 Codex 配置可能被外部工具修改，项目或会话层可能覆盖用户层，而且没有供应商 ID 的旧配置可能唯一、歧义或完全无法匹配，  
**when** GPTEasy 启动或重新扫描用户文件并调用 Codex app-server `config/read` 获取最终有效配置及来源，  
**then** 能稳定区分受管当前、受管漂移、受管但被覆盖、旧配置唯一匹配、外部歧义、外部未匹配和需要人工处理，并且不自动覆盖任何外部状态。

## Research

### 已检查的资料

- Codex 配置基础：`https://developers.openai.com/codex/config-basic`
- Codex 高级配置与配置层：`https://developers.openai.com/codex/config-advanced`
- Codex 配置参考：`https://developers.openai.com/codex/config-reference`
- OpenAI Codex app-server README：`https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md`
- 本机 `codex-cli 0.146.0` 生成的 `ConfigReadParams` 和 `ConfigReadResponse` JSON Schema

### 方案比较

| 方案 | 优点 | 缺点 | 状态 |
|---|---|---|---|
| 只读取 `~/.codex/config.toml` | 无需启动 Codex 子进程 | 无法知道项目、会话、系统和托管层的最终结果 | 不充分 |
| 自己重新实现 Codex 配置层合并 | 不依赖 app-server | 容易随 Codex 版本漂移，尤其是字段限制和来源规则 | 淘汰 |
| 调用 app-server `config/read(cwd, includeLayers=true)` | 返回最终 config、layers 和逐字段 origins，与实际 Codex 同源 | 响应可能包含完整配置和凭据，必须只在内存中脱敏提取 | **采用** |
| 用地址、名称或模型作为供应商身份 | 无需额外元数据 | 地址、名称、Key 和模型都可变或重复 | 淘汰 |
| 在管理区块注释中保存不可变供应商 ID | Codex 忽略注释，不依赖未知字段兼容性；区块替换可稳定维护 | 必须由 GPTEasy 自己解析并检查重复 | **采用** |

### app-server 调用

实验启动隔离的 `codex app-server`，完成：

1. `initialize`
2. `initialized`
3. `config/read`，参数包含目标 `cwd` 与 `includeLayers = true`

只提取以下脱敏字段：

- `config.model`
- `config.model_provider`
- `origins.model.name.type`
- `origins.model_provider.name.type`
- `layers[].name.type`

不能保存原始 `config/read` 响应，因为有效 config 可能包含 `experimental_bearer_token`。

### 供应商 ID 格式

管理区块使用注释：

```toml
# >>> GPTEasy managed provider >>>
# GPTEasy provider-id: 8a5d...immutable-id
model = "provider-model"
model_provider = "gpteasy"
model_providers.gpteasy.base_url = "https://provider.example/v1"
# <<< GPTEasy managed provider <<<
```

该 ID 只用于匹配 GPTEasy 供应商目录，不传给上游服务，也不参与 Codex provider schema。

## How to Run

```powershell
.\.planning\spikes\008-external-config-reconciliation\run.ps1
```

`run.ps1` 会从当前 npm Codex 安装中定位原生 `codex.exe`，所有 app-server 场景使用隔离 `CODEX_HOME` 和假凭据。

## What to Expect

`.run/summary.json` 应显示 10/10：

| 场景 | 预期状态 |
|---|---|
| 用户层 ID、字段与有效来源完全匹配 | `managed_current` |
| 项目层覆盖 model | `managed_overridden` |
| `-c` 会话参数覆盖 model | `managed_overridden` |
| 已知 ID 但地址或模型漂移 | `managed_drifted` |
| 管理区块 ID 已不在供应商目录 | `external_unknown_id` |
| 无 ID 且地址+模型唯一匹配 | `legacy_unique_match` |
| 无 ID 且多个供应商匹配 | `external_ambiguous` |
| 无 ID 且没有匹配 | `external_unmatched` |
| 管理标记损坏 | `needs_attention` |
| 管理区块包含重复 ID 注释 | `needs_attention` |

## Observability

- `.run/summary.json` 记录 Codex 版本、状态、供应商 ID、有效 model/provider 和字段来源类型。
- 不记录 app-server 原始响应、provider 表、API Key 或配置正文。
- app-server stdout 由内存线程解析，只保留目标 JSON-RPC 响应中的白名单字段。
- app-server stderr 丢弃，避免未来版本把完整配置错误写入实验日志。

## Investigation Trail

1. **app-server 已提供正式读取面**：当前 `ConfigReadParams` 明确支持 `cwd` 和 `includeLayers`，返回 effective config、layers 与逐字段 origins，因此不需要重新实现 Codex 合并算法。
2. **项目层可以产生部分覆盖**：真实 `codex-cli 0.146.0` 中，项目 `.codex/config.toml` 的 `model` 覆盖用户层，但实验中的 `model_provider` 仍来自用户层。最终状态可能是“项目模型 + 用户 provider”，不能只比较单个 current provider 字段。
3. **会话参数是独立来源**：`-c model="session-model"` 的 origin 为 `sessionFlags`，应与项目覆盖一样展示，而不是误判用户配置被修改。
4. **用户文件与有效配置必须分别建模**：用户管理区块可以完全正确，但有效配置仍被高优先级层覆盖；状态应为 `managed_overridden`，不能改写用户文件试图争夺。
5. **已知 ID 优先于模糊匹配**：ID 存在但地址或模型不同表示已验证配置发生漂移，需要重新验证；不能退回按名称或地址匹配并静默接受。
6. **未知 ID 不复用旧身份**：管理区块中的 ID 若已不在目录，状态为 `external_unknown_id`。即使地址和模型碰巧匹配，也不能把已删除或其他设备的身份绑定到当前记录。
7. **无 ID 只允许保守迁移候选**：地址与模型恰好唯一匹配时标记 `legacy_unique_match`，供用户确认迁移；多个匹配或无匹配都保持外部配置。
8. **损坏元数据与损坏区块同级处理**：重复 provider-id 注释、标记缺失或倒置全部进入 `needs_attention`，不自动修复。
9. **原始响应含敏感风险**：`config/read` 返回的是完整 effective config。诊断层只能保留字段来源和非敏感摘要。
10. **007 的 fixture ID 字段不是正式格式**：Spike 007 在纯 TOML fixture 中使用了 provider `id` 键；本 Spike 收敛后的正式方案是管理区块注释，避免依赖 Codex 对未知 provider 字段的长期容忍。

## Results

### Verdict: VALIDATED ✓

本机 `codex-cli 0.146.0` 的三个真实 app-server 层级场景和七个身份协调场景全部通过。

### 已验证

- app-server `config/read` 可以作为最终有效配置和来源的权威读取面。
- 用户层、项目层和会话参数的字段来源能够被区分。
- 不可变 ID、字段漂移和有效层覆盖可以形成互不混淆的产品状态。
- 无 ID 旧配置只在地址与模型唯一匹配时产生保守迁移候选。
- 损坏、歧义和外部配置均不会触发自动覆盖。

### 限制

- 当前真实测试只在 Windows 与 `codex-cli 0.146.0` 执行；未来 Codex 升级需要协议 schema 与行为回归。
- 没有可用的 MDM、enterprise managed 或真实系统 managed config，因此只验证了这些层会出现在协议模型中，没有执行真实覆盖。
- `legacy_unique_match` 只是迁移候选，不等于供应商已经重新验证。
- app-server 是独立子进程；正式应用需要超时、异常退出和版本不兼容降级策略。
