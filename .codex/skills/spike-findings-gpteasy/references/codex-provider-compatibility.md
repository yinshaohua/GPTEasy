# Codex 与供应商兼容

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- 直接修改当前用户的 Codex 配置，不在请求链路中运行本地代理。
- 原生 Codex 环境同时覆盖统一 ChatGPT 桌面应用中的 Codex 与本机 Codex CLI。
- 供应商至少包含服务地址、API Key 和默认模型，并使用不可变 ID。
- 保存供应商前必须验证模型发现、Responses API 流式响应和工具调用闭环。
- 地址、凭据或默认模型变化时必须重新验证；失败保留旧配置。
- 远程供应商必须使用 HTTPS；只有 `localhost`、`127.0.0.1` 和 `[::1]` 允许 HTTP。
- 真实凭据不得进入命令行、日志、诊断、运行产物或 Git。

## How to Build It

### 1. 明确受管配置边界

正式产品只管理当前用户默认的 `~/.codex/config.toml`：

- Windows：`%USERPROFILE%\.codex\config.toml`
- macOS：`$HOME/.codex/config.toml`

不要把任意 `CODEX_HOME` 当成可跟随的自定义路径。若检测到运行中进程使用覆盖路径，应展示为首版不受管的外部环境，而不是静默修改另一个目录。

Codex 用户层管理区块至少生成：

```toml
# >>> GPTEasy managed provider >>>
# GPTEasy provider-id: immutable-provider-id
model = "provider-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "Provider"
model_providers.gpteasy.base_url = "https://provider.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.supports_websockets = false
model_providers.gpteasy.experimental_bearer_token = "API Key"
# <<< GPTEasy managed provider <<<
```

不可变 ID 放在注释中，由 GPTEasy 解析，不传给供应商，也不依赖 Codex 对未知 provider 字段的容忍。配置的实际写入必须走 `safe-config-editing.md` 的迁移和原子替换协议。

### 2. 在同一 Rust 进程中执行四阶段验证

API Key 由 Tauri command 以内存参数传给 Rust 验证服务。不要启动带 Key 参数的外部进程，也不要把持久环境变量作为正式凭据通道。

验证顺序固定为：

1. **URL 安全策略**：解析并规范化 base URL；远程只接受 HTTPS。
2. **模型发现**：`GET <base_url>/models`，要求 `data[].id` 精确包含默认模型。
3. **Responses 流与工具调用**：第一轮强制调用 strict function，并验证 SSE 完成。
4. **工具结果回传**：第二轮发送 `function_call_output`，要求最终流式回答包含同一 nonce。

URL 门禁沿用：

```rust
match url.scheme() {
    "https" => Ok(url),
    "http" if is_loopback(&url) => Ok(url),
    "http" => Err(ValidationFailure::new(
        "security_policy",
        "remote provider must use HTTPS; HTTP is allowed only for loopback addresses",
    )),
    other => Err(ValidationFailure::new(
        "security_policy",
        format!("unsupported URL scheme: {other}"),
    )),
}
```

工具闭环必须使用每次验证唯一的 nonce：

```rust
let first_payload = json!({
    "model": model,
    "input": [user_input.clone()],
    "tools": [tool.clone()],
    "tool_choice": {"type": "function", "name": "gpteasy_probe"},
    "parallel_tool_calls": false,
    "stream": true
});

let function_output = json!({
    "type": "function_call_output",
    "call_id": call.call_id,
    "output": json!({"ok": true, "nonce": nonce}).to_string()
});
```

第一轮至少验证：

- HTTP 成功且 `Content-Type` 以 `text/event-stream` 开头。
- 流中出现 `response.completed`。
- `response.output_item.done` 包含名为 `gpteasy_probe` 的 `function_call`。
- 最终 `arguments` 是合法 JSON，且 nonce 精确匹配。

第二轮至少验证：

- 使用同一个 `call_id`。
- 提交原始 `function_call` 和对应 `function_call_output`。
- 流中出现 `response.completed`。
- 最终文本包含同一 nonce。

### 3. 用健壮的 SSE 状态机处理真实事件

真实供应商会发送比最小 mock 更多的事件，并把函数参数拆成多个 delta。解析器应：

1. 容忍并记录未知或附加事件类型。
2. 不假设 `arguments` 只出现在单个 delta 中。
3. 以 `response.output_item.done.arguments` 的完整 JSON 作为函数参数门禁。
4. 同时收集 `response.output_text.delta` 和最终 message content。
5. 只在看到 `response.completed` 后判定流完成。

blocking `reqwest` 没有可直接配置的独立流读取 timeout。已验证做法是让 reader thread 阻塞读取，主线程通过 channel 等待：

```rust
let wait = remaining.min(timeouts.read);
match receiver.recv_timeout(wait) {
    Ok(StreamRead::Line(line)) => consume(line),
    Err(RecvTimeoutError::Timeout) if events.is_empty() => {
        return Err(failure("first_event_timeout"));
    }
    Err(RecvTimeoutError::Timeout) => {
        return Err(failure("stream_idle_timeout"));
    }
    _ => { /* transport or EOF handling */ }
}
```

默认策略可从已验证值起步：

| Deadline | Live default |
|----------|--------------|
| Connect | 10 s |
| First event / stream idle | 30 s |
| Overall per Responses request | 120 s |

UI 可以允许未来按策略调整，但不能把三个截止时间合并成一个模糊的“网络超时”。

### 4. 保留稳定的失败分类

| 分类 | 判据或用户含义 |
|------|----------------|
| `security_policy` | URL 非法、远程 HTTP 或不支持的 scheme |
| `transport` | DNS、连接、TLS 或普通 I/O 失败 |
| `response_header_timeout` | Responses 请求在响应头阶段超时 |
| `first_event_timeout` | 收到 SSE 响应头后，首事件超过截止时间 |
| `stream_idle_timeout` | 已收到事件，但事件间空闲超过截止时间 |
| `overall_timeout` | 持续有事件，但单次 Responses 请求超过总截止 |
| `authentication` | `/models` 或 `/responses` 返回 401/403 |
| `rate_limit` | 返回 429；只记录脱敏 `Retry-After` |
| `model_discovery` | 模型列表不可读或没有默认模型 |
| `streaming` | 非 SSE、流中断或缺少完成事件 |
| `responses_protocol` | 状态或事件 JSON 结构不兼容 |
| `tool_call` | 没有函数调用、函数名错误或参数非法 |
| `tool_result` | 工具结果没有完成回传闭环 |

只有所有阶段成功，才把地址、Key、模型这一组合标记为已验证。任一字段变化后，新组合先验证，成功后再替换旧记录。

### 5. 区分用户文件与最终有效配置

用户层 `config.toml` 只是 Codex 配置层之一。项目目录、仓库 `.codex/config.toml`、会话参数和托管层都可能覆盖它。

状态读取走 `switch-consistency-reconciliation.md` 的协调协议：

1. 解析用户管理区块及不可变 ID。
2. 调用 Codex app-server `config/read(cwd, includeLayers=true)`。
3. 只提取 effective model/provider 和字段来源摘要。
4. 用户层正确但被高优先级层覆盖时，展示覆盖来源，不自动改写文件争夺优先级。

### 6. 让真实凭据和诊断可执行地隔离

正式 Tauri 应把 Key 保持在当前 Rust 进程内存中。Spike 或诊断工具需要项目本地秘密文件时：

1. 文件放在 Git 忽略目录。
2. 读取前执行 `git check-ignore --quiet -- <path>`。
3. 不把 Key 放进参数或日志。
4. 日志只记录阶段、耗时、状态、事件类型、正文长度和布尔判据。
5. 运行后递归扫描产物目录，若发现 Key 原始字节则判定失败。

## What to Avoid

- **不要只调用 `/models`。** 这不能证明 Responses 流或工具调用与 Codex 兼容。
- **不要只做一次文本响应。** 文本流成功仍可能在 function call 或工具结果回传时失败。
- **不要把 `env_key` 作为桌面和 CLI 的唯一凭据方案。** 已运行或从 Finder/开始菜单启动的应用不会获得 GPTEasy 后续设置的进程环境。
- **不要把 Key 放进命令行参数、错误正文摘录或完整请求/响应日志。**
- **不要把收到任意 SSE 数据当成功。** 必须看到 `response.completed`。
- **不要假定函数参数只在一个 delta 事件中出现。**
- **不要把所有超时归类为 transport。** 首事件、空闲、整体和响应头超时需要不同恢复提示。
- **不要把 429 归到认证失败。**
- **不要接受远程 HTTP，也不要提供绕过开关。**
- **不要假定修改用户层文件就等于最终有效配置。**

## Constraints

- `experimental_bearer_token` 的字段名和上游实现带实验性质；正式代码应集中封装 provider 渲染并准备兼容迁移。
- 明文凭据是项目已接受的产品决策，但主配置、备份和导出脚本都必须使用当前用户访问控制；日志仍必须脱敏。
- Windows 已验证桌面 Codex 与本机 CLI 默认共享当前用户的 `~/.codex`。macOS 默认路径、进程拓扑和共享关系尚未在真实机器验证。
- deterministic 矩阵已覆盖成功、认证、模型缺失、非 SSE、缺工具、截断、坏参数、429、三类流超时和远程 HTTP。
- 2026-08-05 验证的一个真实地址、Key、模型组合完成了模型发现、strict function call 和 nonce 回传；这不代表同一供应商的其他模型或未来状态持续兼容。
- 没有执行并发、吞吐、成本、长时间稳定性或真实 429 压力测试。

## Origin

Synthesized from spikes: 001, 002, 011
Source files available in: `sources/001-codex-native-config-contract/`, `sources/002-provider-validation-loop/`, `sources/011-real-provider-compatibility-matrix/`
