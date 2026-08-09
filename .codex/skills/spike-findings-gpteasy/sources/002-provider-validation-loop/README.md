---
spike: 002
name: provider-validation-loop
type: standard
validates: "Given 标准供应商的服务地址、API Key 和默认模型，when 执行模型发现、Responses API 流式请求及工具调用闭环，then 能可重复地判定供应商是否满足 Codex 使用要求并提供脱敏诊断"
verdict: PARTIAL
related: [001, 004]
tags: [responses-api, sse, tools, provider, validation]
---

# Spike 002: 供应商验证闭环

## What This Validates

**Given** 标准供应商的服务地址、API Key 和默认模型，  
**when** 执行模型发现、Responses API SSE 流式请求、函数调用及工具结果回传，  
**then** GPTEasy 能以可重复、可诊断的方式判定供应商是否满足 Codex 使用要求。

## Research

### 已检查的资料

- OpenAI Responses API 参考：`https://platform.openai.com/docs/api-reference/responses`
- OpenAI Responses 流式事件参考：`https://platform.openai.com/docs/api-reference/responses-streaming`
- OpenAI Function Calling 指南：`https://platform.openai.com/docs/guides/function-calling`
- OpenAI Codex `rust-v0.146.0` 的 Responses SSE 解析与测试工具：
  - `codex-rs/codex-api/src/sse/responses.rs`
  - `codex-rs/core/tests/common/responses.rs`
  - `codex-rs/core/tests/common/streaming_sse.rs`

### 方案比较

| 方案 | 工具/协议 | 优点 | 缺点 | 状态 |
|---|---|---|---|---|
| 只请求 `/models` | 普通 JSON HTTP | 快、错误简单 | 无法证明 Responses、流式事件或工具调用可用 | 淘汰 |
| 单次 Responses 文本请求 | `/responses` + SSE | 能证明基础 Responses 流可读 | 兼容文本不代表兼容 Codex 工具循环 | 不充分 |
| 两轮工具调用闭环 | `/models` + 两次 `/responses` SSE | 同时证明模型发现、SSE、函数参数、工具输出回传和最终回答 | 验证成本较高；供应商必须较完整地实现 Responses | **采用** |
| 直接启动 Codex 完成验证 | Codex CLI/app-server | 最贴近最终使用 | 难稳定区分失败阶段，日志与权限控制复杂，可能执行真实工具 | 作为后续兼容复核，不作为首层验证器 |

### 选定协议

1. 对远程地址强制 HTTPS；仅 `localhost`、`127.0.0.1` 和 IPv6 回环允许 HTTP。
2. `GET <base_url>/models`，要求响应的 `data[].id` 精确包含默认模型。
3. 第一轮 `POST <base_url>/responses`：
   - `stream = true`
   - 提供 strict function `gpteasy_probe`
   - `parallel_tool_calls = false`
   - 强制选择 `gpteasy_probe`
   - 要求 SSE 以 `response.completed` 正常结束
   - 要求 `response.output_item.done` 中出现合法 function call，并回传正确 nonce
4. 第二轮发送 `function_call` 与 `function_call_output`，要求最终流式文本包含同一 nonce。
5. 所有日志都只保存 URL、状态、事件类型、耗时、模型名和布尔结果，不保存 Key、完整请求、完整模型输出或工具输出。

## How to Run

### 完整场景矩阵

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\002-provider-validation-loop\run.ps1
```

### 验证真实供应商

```powershell
$env:GPTEASY_PROVIDER_KEY = '<API Key>'
cargo run --manifest-path .codex/skills/spike-findings-gpteasy/sources/002-provider-validation-loop/Cargo.toml -- `
  validate 'https://provider.example/v1' 'provider-model-id' `
  '.codex/skills/spike-findings-gpteasy/sources/002-provider-validation-loop/.run/live-provider.jsonl'
Remove-Item Env:GPTEASY_PROVIDER_KEY
```

Key 通过进程环境传入，不放在命令行参数、输出或诊断日志中。

## What to Expect

内置矩阵共八个场景：

| 场景 | 预期分类 |
|---|---|
| 完整模型发现、SSE 和工具闭环 | `validated` |
| `/models` 返回 401 | `authentication` |
| 默认模型不存在 | `model_discovery` |
| Responses 返回普通 JSON 而非 SSE | `streaming` |
| 流结束但没有函数调用 | `tool_call` |
| SSE 在 `response.completed` 前断开 | `streaming` |
| 函数参数不是合法 JSON | `tool_call` |
| 远程 HTTP 地址 | `security_policy` |

运行成功时 `.run/summary.json` 应显示 `passed = 8`、`total = 8`。

## Observability

每个场景产生：

- `client.jsonl`：验证阶段、持续时间、事件类型、成功/失败。
- `server.jsonl`：请求方法、路径、是否有 Authorization、模型、是否流式、是否包含 `function_call_output`。
- `result.json`：最终分类与阶段摘要。

Happy path 的实测事件：

- 第一轮：`response.created` → `response.output_item.added` → `response.function_call_arguments.delta` → `response.function_call_arguments.done` → `response.output_item.done` → `response.completed`
- 第二轮：`response.created` → `response.output_item.added` → `response.output_text.delta` → `response.output_item.done` → `response.completed`

## Investigation Trail

1. **从“连通性测试”改为协议闭环**：仅 `/models` 或一次文本响应无法证明 Codex 可工作，因此加入两轮 function call。
2. **使用 nonce 防止假阳性**：验证器生成唯一 nonce，第一轮模型必须原样传给工具，第二轮最终回答必须包含工具返回的同一 nonce。
3. **要求显式完成事件**：仅收到若干 SSE 数据不算成功；连接必须出现 `response.completed`。截断流被稳定分类为 `streaming`。
4. **严格解析函数参数**：`arguments` 必须为合法 JSON，函数名和 nonce 必须匹配，避免把任意 tool-like 文本误判为成功。
5. **区分认证与协议失败**：401/403 归为 `authentication`；模型不存在、非 SSE、缺失工具调用和流截断分别独立分类，便于 UI 给出可操作提示。
6. **安全地址策略前置**：远程 HTTP 在网络请求前直接拒绝；回环 HTTP 保留给本地测试服务。
7. **命令行泄密边界**：Key 只从 `GPTEASY_PROVIDER_KEY` 进程环境读取，不出现在命令行；未来 Tauri 命令应把 Key 作为内存参数传入 Rust command，而不是启动外部验证进程。
8. **真实供应商边界**：本次没有可用的第三方 API Key，因此只验证了协议实现和错误矩阵，没有给 DayWay 或其他具体供应商授予“已验证供应商”状态。

## Results

### Verdict: PARTIAL ⚠️

**已验证：**

- Rust 可以实现完整的标准供应商验证器，不需要启动或控制 Codex 进程。
- 模型发现、Responses SSE、函数调用、工具输出回传和最终回答可以形成确定性闭环。
- 八类成功/失败场景全部得到预期分类。
- 诊断日志可以在不记录 Key、完整请求或完整模型输出的前提下保留足够证据。
- 远程 HTTPS 与回环 HTTP 规则可以在请求前强制执行。

**尚未验证：**

- 尚未对真实 OpenAI 或第三方供应商执行完整闭环。
- 不同兼容供应商对 `tool_choice`、strict schema、SSE 事件完整度和 `/models` 的实现差异仍需真实数据。
- 超时、限流和供应商重试策略目前只由 HTTP 客户端总超时保护，正式实现应增加连接、首事件、流空闲和整体截止时间的独立配置。

**对正式构建的建议：**

- 只有完整四阶段都成功，才允许保存或替换供应商。
- 供应商地址、Key 或默认模型变化后必须重新运行全部阶段。
- UI 应展示失败分类和阶段耗时，但不展示原始响应正文。
- 正式版本应在相同 Rust 进程中执行验证，避免通过命令行或持久环境变量传递 Key。
