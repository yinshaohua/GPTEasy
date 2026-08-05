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
- 供应商至少包含服务地址、API Key 和默认模型。
- 保存供应商前必须验证模型发现、Responses API 流式响应和工具调用闭环。
- 远程供应商必须使用 HTTPS；只有 `localhost`、`127.0.0.1` 和 `[::1]` 允许 HTTP。

## How to Build It

### 1. 明确受管配置边界

正式产品只管理当前用户默认的 `~/.codex/config.toml`：

- Windows：`%USERPROFILE%\.codex\config.toml`
- macOS：`$HOME/.codex/config.toml`

不要把任意 `CODEX_HOME` 当成可跟随的自定义路径。若检测到运行中进程使用覆盖路径，应把它展示为不受首版管理的外部环境，而不是静默修改另一个目录。

Codex 用户层配置至少需要生成以下逻辑字段：

```toml
model = "provider-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "Provider"
model_providers.gpteasy.base_url = "https://provider.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.supports_websockets = false
model_providers.gpteasy.experimental_bearer_token = "API Key"
```

当前实验已证明 `model_provider`、`model`、`base_url`、`wire_api = "responses"` 和 Bearer token 可以被 Codex CLI 读取，并产生流式 Responses 请求和工具定义。配置的实际写入必须走 `safe-config-editing.md` 的迁移和原子替换协议。

### 2. 区分用户文件与最终有效配置

用户层 `config.toml` 只是 Codex 配置层之一。项目目录、仓库 `.codex/config.toml`、父目录和托管层都可能覆盖或补充它。读取状态时：

1. 解析 GPTEasy 管理区块，尝试匹配不可变供应商 ID。
2. 若用户层当前 provider 无法匹配已保存供应商，展示为“外部配置”。
3. 不因外部配置存在而持续争夺或自动覆盖文件。
4. 诊断中明确区分“用户层写入成功”和“最终有效配置可能被更高优先级层覆盖”。

### 3. 在同一 Rust 进程中运行四阶段供应商验证

API Key 由 Tauri command 以内存参数传入 Rust，不启动带 Key 参数的外部验证进程，也不依赖持久环境变量。

验证顺序固定为：

1. **URL 安全策略**：解析并规范化 base URL；远程只接受 HTTPS。
2. **模型发现**：`GET <base_url>/models`，要求 `data[].id` 精确包含默认模型。
3. **Responses 流与工具调用**：第一轮强制调用 strict function，并验证 SSE 正常完成。
4. **工具结果回传**：第二轮发送 `function_call_output`，要求最终流式回答包含同一 nonce。

核心 URL 策略可沿用已验证模式：

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

工具闭环必须使用每次验证唯一的 nonce，避免供应商返回固定文本造成假阳性：

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

- HTTP 状态成功且 `Content-Type` 为 `text/event-stream`
- 流中出现 `response.completed`
- `response.output_item.done` 包含 `function_call`
- 函数名为 `gpteasy_probe`
- `arguments` 是合法 JSON，且 nonce 精确匹配

第二轮至少验证：

- 使用同一个 `call_id`
- 提交原始 `function_call` 和对应 `function_call_output`
- 流中出现 `response.completed`
- 最终文本包含同一 nonce

### 4. 将失败分类直接映射到 UI

保留稳定的机器可读分类：

| 分类 | 用户含义 |
|------|----------|
| `security_policy` | 地址协议不安全或 URL 非法 |
| `transport` | DNS、连接、TLS 或超时失败 |
| `authentication` | 401/403，凭据不可用 |
| `model_discovery` | 模型列表不可读或没有默认模型 |
| `streaming` | 非 SSE、首事件超时、流中断或缺少完成事件 |
| `responses_protocol` | Responses 状态或事件结构不兼容 |
| `tool_call` | 没有函数调用、函数名错误或参数非法 |
| `tool_result` | 工具结果没有完成回传闭环 |

只有全部阶段成功，才把新配置标记为已验证并进入保存事务。服务地址、API Key 或默认模型任一变化，都必须使旧验证失效并重新运行全部阶段；验证失败时保留原已验证配置。

### 5. 记录脱敏且可操作的诊断

每个阶段记录时间戳、阶段名、耗时、状态码、事件类型、模型 ID 和布尔判据。不要记录：

- API Key 或完整 Authorization
- 完整请求正文
- 完整模型输出
- 工具输出正文
- 可能含敏感输入的完整命令行

建议分别设置连接、首事件、流空闲和整体截止时间，并把触发的截止类型写入分类。

## What to Avoid

- **不要只调用 `/models`。** 这不能证明 Responses 流或工具调用与 Codex 兼容。
- **不要只做一次文本响应。** 文本流成功仍可能在 function call 或工具结果回传时失败。
- **不要把 `env_key` 作为桌面和 CLI 的唯一凭据方案。** 已运行或从 Finder/开始菜单启动的桌面应用不会获得 GPTEasy 后续设置的进程环境。
- **不要把 Key 放进命令行参数。** 正式验证器应在 Tauri/Rust 进程内执行。
- **不要把收到任意 SSE 数据当成功。** 必须看到 `response.completed`。
- **不要接受远程 HTTP，也不要提供绕过开关。**
- **不要假定修改用户层文件就等于最终有效配置。** 必须保留“外部配置/覆盖层”的产品状态。

## Constraints

- `experimental_bearer_token` 的字段名和上游实现都带实验性质；当前可用，但正式代码应集中封装 Codex provider 渲染并准备兼容迁移。
- 明文凭据是项目已接受的产品决策，但主配置和备份必须使用当前用户访问控制；日志仍然必须脱敏。
- Windows 已验证桌面 Codex 与本机 CLI 默认共享当前用户的 `~/.codex`。macOS 默认路径、进程拓扑和共享关系尚未在真实机器验证。
- 内置 mock 的八类协议场景已验证，但尚未对真实第三方供应商完成兼容认证。不能把 DayWay 或其他服务标记为“已验证供应商”，直到真实闭环通过。
- 当前实验只有连接超时和整体超时；正式实现还需要首事件、流空闲、限流和重试策略。

## Origin

Synthesized from spikes: 001, 002
Source files available in: `sources/001-codex-native-config-contract/`, `sources/002-provider-validation-loop/`
