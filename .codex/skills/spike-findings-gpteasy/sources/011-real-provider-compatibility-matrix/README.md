---
spike: 011
name: real-provider-compatibility-matrix
type: standard
validates: "Given Git 忽略的项目本地私密文件中的真实供应商地址、API Key 和模型，when 在分阶段截止时间、限流和协议差异下运行完整 nonce 工具闭环，then 能形成真实兼容结论、稳定失败分类和脱敏证据"
verdict: VALIDATED
related: [001, 002, 007, 008]
tags: [provider, responses-api, sse, tools, timeout, rate-limit, live]
---

# Spike 011: 真实供应商兼容矩阵

## What This Validates

**Given** Git 忽略的项目本地文件中保存真实供应商服务地址、API Key 和模型 ID，  
**when** 验证器执行 URL 安全策略、模型发现、Responses SSE strict function call、工具结果回传，并在 deterministic mock 中覆盖限流和分阶段超时，  
**then** 能对真实供应商形成可重复的兼容结论，同时保证 Key 不进入命令行、日志、诊断、Git 或运行产物。

## Research

### 已检查的资料

- OpenAI Responses API：`https://platform.openai.com/docs/api-reference/responses`
- OpenAI Responses streaming events：`https://platform.openai.com/docs/api-reference/responses-streaming`
- OpenAI function calling：`https://platform.openai.com/docs/guides/function-calling`
- `reqwest` blocking client：`https://docs.rs/reqwest/0.12`
- Spike 002 的模型发现、SSE 和 nonce 工具闭环实现

### 超时方案比较

| 方案 | 优点 | 问题 | 状态 |
|---|---|---|---|
| 只设置 HTTP overall timeout | 简单 | 无法区分首事件慢、流中途停顿和整体超时 | 淘汰 |
| 使用 blocking `ClientBuilder::read_timeout` | API 设计直观 | 实测 `reqwest` 0.12 和 0.13 blocking builder 都没有该方法 | 不可用 |
| 每个 SSE reader 使用线程，主线程通过 channel `recv_timeout` 等待 | 可独立区分首事件、空闲和整体截止时间；保留 blocking client | 超时后 reader 线程可能等待底层连接结束，但进程退出会回收 | **采用** |
| 全面改为 async reqwest + Tokio | 超时和取消最灵活 | 为单个 Spike 引入较大运行时和重构，正式应用是否 async 尚未决定 | 暂不采用 |

### 分阶段截止时间

deterministic 模式使用短截止时间：

- connect：500 ms
- 每次读取/事件空闲：500 ms
- overall：1200 ms

真实供应商模式：

- connect：10 s
- 首事件/事件空闲：30 s
- overall：120 s

失败分类：

| 分类 | 判据 |
|---|---|
| `response_header_timeout` | 建立请求后在响应头阶段超时 |
| `first_event_timeout` | 收到 SSE 响应头后，在首个事件前超过读取截止 |
| `stream_idle_timeout` | 已收到事件，但后续事件间隔超过读取截止 |
| `overall_timeout` | 持续有事件但整个 Responses 请求超过总截止 |
| `rate_limit` | `/models` 或 `/responses` 返回 429，并只记录脱敏 `Retry-After` |

### 真实凭据文件

默认位置：

```text
.codex/skills/spike-findings-gpteasy/.secrets/provider.json
```

保护规则：

1. `.gitignore` 忽略整个 `.secrets/`。
2. PowerShell 和 Rust 两层都执行 `git check-ignore`。
3. 若项目内文件未被 Git 忽略，验证器拒绝读取。
4. Key 只从 JSON 读入当前 Rust 进程内存，不进入命令行或环境变量。
5. 真实验证结束后递归扫描 `.run/live/`；任一文件包含 Key 原始字节即失败。

## How to Run

先创建：

```json
{
  "base_url": "https://provider.example/v1",
  "api_key": "真实 API Key",
  "model": "真实模型 ID"
}
```

保存到 `.codex/skills/spike-findings-gpteasy/.secrets/provider.json`，然后执行：

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\011-real-provider-compatibility-matrix\run.ps1
```

只运行 deterministic 矩阵：

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\011-real-provider-compatibility-matrix\run.ps1 -SkipLive
```

## What to Expect

### Deterministic

12/12：

- 完整成功闭环
- 认证失败
- 默认模型缺失
- 非 SSE
- 缺少工具调用
- 截断流
- 非法工具参数
- 429 与 `Retry-After`
- 首事件超时
- 流空闲超时
- 整体超时
- 远程 HTTP 安全策略

### Live

2026 年 8 月 5 日的真实执行结果：

| 阶段 | 结果 | 耗时 |
|---|---|---:|
| URL 安全策略 | 通过 | 0 ms |
| 模型发现 | 通过，HTTP 200 | 674 ms |
| SSE 与工具调用 | 通过，19 个事件 | 2872 ms |
| 工具结果回传 | 通过，17 个事件 | 2145 ms |

第一轮首事件约 2549 ms，第二轮首事件约 1901 ms，均在 30 秒截止时间内。最终回答包含验证 nonce。

## Observability

- 每个 deterministic 场景有 `client.jsonl`、`server.jsonl` 和 `result.json`。
- 真实供应商只有 `.run/live/client.jsonl` 和脱敏 `result.json`。
- 记录阶段、耗时、HTTP 状态、事件类型、首事件耗时、事件数量和布尔判据。
- 不记录 API Key、Authorization、完整请求、完整响应正文、模型输出或工具输出。
- 模型发现错误只记录 body 长度，不记录响应摘录。
- `.run/live/` 的 API Key 原始字节扫描通过。

## Investigation Trail

1. **Mock 通过不等于真实兼容**：Spike 002 已证明协议实现，但没有真实 Key。本次真实供应商完整通过模型发现和两轮工具闭环。
2. **真实 SSE 事件比最小 mock 丰富**：第一轮出现 `response.in_progress` 和多次 `response.function_call_arguments.delta`；第二轮还出现 `response.content_part.added/done` 与 `response.output_text.done`。解析器必须容忍和记录未知/附加事件，只对关键完成和输出项做门禁。
3. **函数参数可能被拆成很多 delta**：真实第一轮产生多段 arguments delta，但最终 `response.output_item.done.arguments` 是完整 JSON。验证不能假设参数只在一个事件中出现。
4. **首事件可能以秒计**：真实两轮首事件分别约 2.5 秒和 1.9 秒。过短的统一读取超时会误杀健康供应商，因此正式默认应明显高于本次观测并允许配置策略。
5. **blocking reqwest 没有独立 read timeout builder**：实测 0.12 与 0.13 blocking API 后，改用 reader thread + channel 等待，成功区分首事件、空闲和 overall。
6. **429 是独立产品状态**：限流不能归到认证或一般协议错误；应保留脱敏 `Retry-After` 供 UI 告知用户何时重试。
7. **日志不能保存 body excerpt**：兼容供应商的错误正文可能回显输入或服务细节。本 Spike 移除了 Spike 002 的 200 字符摘录，只记录长度。
8. **项目本地秘密文件更适合临时 Spike**：用户选择把凭据放到可发现、易删除的 `.codex/skills/spike-findings-gpteasy/.secrets/`，但运行前必须验证 Git ignore，而不能只依赖口头约定。
9. **泄漏扫描是可执行门禁**：真实验证后扫描所有 live 产物，确认 Key 未出现；这比人工查看日志更可靠。

## Results

### Verdict: VALIDATED ✓

deterministic 12/12 场景全部通过，真实供应商的四个阶段也全部通过，最终分类为 `validated`，凭据泄漏扫描通过。

### 真实兼容结论

- 真实供应商可以发现指定默认模型。
- 支持 Responses API SSE。
- 支持强制 strict function call。
- 支持 function call arguments delta 与最终完整参数。
- 支持 `function_call_output` 回传。
- 支持最终流式文本并正确返回 nonce。

因此该地址、Key 和模型的当前组合可以标记为**已验证供应商**。地址、Key 或模型任一变化后必须重新运行完整验证。

### 限制

- 只验证了用户提供的一个真实供应商组合，不代表同一供应商的其他模型或未来状态持续可用。
- 没有主动制造真实 429，限流分类由 deterministic mock 验证。
- 没有执行长时间稳定性、并发请求、吞吐或成本测试。
- 验证时间只是最近一次成功证据，不是持续健康监控。
