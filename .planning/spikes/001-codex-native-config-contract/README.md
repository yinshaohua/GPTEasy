---
spike: 001
name: codex-native-config-contract
type: standard
validates: "Given Windows/macOS 当前用户可能同时使用桌面 Codex 与本机 Codex CLI，when 定位、读取并隔离测试原生配置，then 能确认默认路径、共享关系、供应商字段写法、凭据来源和重启生效边界"
verdict: PARTIAL
related: [002, 003a, 003b, 004]
tags: [codex, config, windows, macos, desktop, cli]
---

# Spike 001: Codex 原生配置契约

## What This Validates

**Given** Windows/macOS 当前用户可能同时使用统一 ChatGPT 桌面应用中的 Codex 与本机 Codex CLI，  
**when** 定位、读取并在隔离的 `CODEX_HOME` 中测试原生配置，  
**then** 能确认默认路径、共享关系、供应商字段写法、凭据来源和重启生效边界。

## Research

### 已检查的资料

- OpenAI Codex 配置基础说明：`https://developers.openai.com/codex/config-basic`
- OpenAI Codex 高级配置说明：`https://developers.openai.com/codex/config-advanced`
- OpenAI Codex 配置完整参考：`https://developers.openai.com/codex/config-reference`
- OpenAI Codex 源码仓库的 `rust-v0.146.0` 提交：`e363b08c9175ac1cbe5893615dd2cb9ddf95043b`
  - `codex-rs/utils/home-dir/src/lib.rs`
  - `codex-rs/model-provider-info/src/lib.rs`
  - `codex-rs/config/src/loader/mod.rs`

### 方案比较

| 方案 | 依据 | 优点 | 缺点 | 状态 |
|---|---|---|---|---|
| 只修改 `~/.codex/config.toml` | Codex 源码默认路径与用户层加载顺序 | 最简单，桌面和 CLI 可以自然共享 | 仍需处理 `CODEX_HOME`、项目层配置和重启 | **采用** |
| 依赖操作系统环境变量承载 Key | `env_key` provider 字段 | 避免把 Key 直接写入 TOML | 桌面应用从 Finder/开始菜单启动时不一定继承 GPTEasy 后续写入的进程环境；当前运行进程也不会刷新 | **不作为首版唯一方案** |
| 将 Key 写入 `experimental_bearer_token` | Codex provider 源码支持直接构造 `Authorization: Bearer` | 桌面与 CLI 重启后都能从同一配置读取 | Codex 源码明确不推荐，属于带有 `experimental` 名称的配置 | **可作为标准供应商兼容方案，但需记录风险** |

### 关键契约

1. 未设置 `CODEX_HOME` 时，Codex 默认使用当前用户主目录下的 `.codex`，配置文件为 `config.toml`，认证状态文件为 `auth.json`。
2. `CODEX_HOME` 必须指向已存在的目录；Codex 会规范化该路径，不能把它当作“创建目录并继续”的写入 API。
3. `model_provider` 是顶层当前供应商 ID；供应商定义位于 `[model_providers.<id>]`。
4. 供应商至少可用以下字段：
   - `name`
   - `base_url`
   - `wire_api = "responses"`
   - `env_key = "ENV_VAR_NAME"`，这里写的是**环境变量名**，不是 Key 本身
   - `experimental_bearer_token = "实际 Key"`，这里才是直接存储 Bearer token 的字段
5. `requires_openai_auth = true` 会把认证路由到 OpenAI/ChatGPT 登录及 `auth.json`，不适合 GPTEasy 的标准供应商模式。
6. Codex 的用户层配置只是配置层之一，项目目录的 `config.toml`、父目录 `.codex/config.toml`、Git 仓库 `.codex/config.toml` 和系统/托管层配置也可能参与合并；GPTEasy 必须展示“外部配置”而不能假定单文件等于最终有效配置。

## How to Run

### Windows

在项目根目录执行：

```powershell
.\.planning\spikes\001-codex-native-config-contract\inspect-windows.ps1
.\.planning\spikes\001-codex-native-config-contract\run.ps1
```

`run.ps1` 会：

1. 使用 Rust TCP mock server，不接触真实供应商或真实 Key。
2. 在隔离目录中分别测试 `env_key` 与 `experimental_bearer_token`。
3. 测试缺失 `env_key` 环境变量时是否在发请求前失败。
4. 记录脱敏请求摘要，不写入实际凭据。

### macOS

```bash
chmod +x .planning/spikes/001-codex-native-config-contract/inspect-macos.sh
./.planning/spikes/001-codex-native-config-contract/inspect-macos.sh
```

该脚本只生成路径、架构、CLI 版本和进程命令行摘要，不修改配置。

## What to Expect

Windows 隔离测试应得到：

- `env_key_exit_code = 0`
- `experimental_bearer_token_exit_code = 0`
- `missing_env_exit_code = 1`
- mock server 收到两次 `POST /v1/responses`
- 两次请求均为 `stream = true`，携带 Bearer Authorization，并携带 Codex 工具定义
- 服务器日志只包含 Key 的长度和固定前缀指纹，不包含完整 Key

Windows 真实环境探针应识别：

- 当前用户 Codex 目录为 `C:\Users\<user>\.codex`
- `config.toml` 与 `auth.json` 的默认位置
- 已安装的 `OpenAI.Codex` AppX 包（若存在）
- 桌面应用的 `ChatGPT.exe` 进程与其子进程中的 bundled `codex.exe app-server`
- 本机 CLI 的独立 `codex.exe`

## Observability

mock server 产生 `.run/summary.json` 和 `.run/server.jsonl`：

- 每条请求有时间戳、方法、路径、模型、是否流式、工具数量。
- Authorization 只保留长度和前缀指纹。
- `inspect-windows.ps1` 生成 `.run/windows-evidence.json`，不读取或输出配置文件正文、`auth.json` 内容或完整命令行中的 Key。

## Investigation Trail

1. **默认路径探针**：Rust probe 在 Windows 上解析出 `C:\Users\yinsh\.codex`，与当前 Codex CLI 实际配置目录一致。
2. **桌面进程检查**：当前机器存在 `OpenAI.Codex` AppX `26.730.8199.0`。桌面应用启动 bundled `codex.exe ... app-server`，本机 CLI 另有独立 `codex.exe`。两者命令行都没有 `CODEX_HOME` 覆盖，因此在默认启动条件下都落入当前用户的 `~/.codex`。
3. **`env_key` 测试**：将 `env_key = "GPTEASY_SPIKE_KEY"` 写入 provider 表，并只在 CLI 进程中设置该环境变量；Codex 成功向 `/v1/responses` 发送 Bearer 请求。
4. **直接 Bearer token 测试**：移除环境变量，改用 `experimental_bearer_token = "spike-secret-value"`；Codex 同样成功发送 Bearer 请求，证明桌面应用不依赖 GPTEasy 进程向其注入后续环境变量。
5. **缺失环境变量测试**：`env_key` 未解析到值时，Codex 在请求发送前返回 `Missing environment variable: GPTEASY_SPIKE_KEY`，mock server 没有收到第三次请求。
6. **流式响应修正**：第一版 mock 仅发送 `response.output_text.delta`，因未发送对应 active output item 而出现内部日志；改为发送规范的 `response.output_item.done` message 后，CLI 输出闭环干净。验证器和未来供应商验证应完整跟踪 SSE item 生命周期。
7. **macOS 边界**：已提供真实机器探针，但本次执行环境为 Windows，未把 macOS 路径、桌面进程和安装包声明为已验证。

## Results

### Verdict: PARTIAL ⚠️

**已验证：**

- Tauri/Rust 方案可以直接定位当前用户的 Codex 配置目录。
- Windows 桌面 Codex 与本机 CLI 在默认启动条件下都使用同一个 `~/.codex` 配置根；桌面进程还会启动 bundled Codex app-server。
- `model_provider`、`model`、`base_url` 和 `wire_api = "responses"` 的写法可由当前 Codex CLI 读取。
- `env_key` 和 `experimental_bearer_token` 都能产生实际 Bearer Authorization。
- Responses 流式请求和 Codex 工具定义会从该 provider 配置正常发出。

**尚未验证：**

- macOS 真实默认路径及桌面 Codex/CLI 进程共享关系。
- 运行中的桌面 Codex 或 CLI 是否会重新读取已修改配置；从进程模型看必须以重启作为正式生效边界，Spike 004 将验证检测与用户交互。
- 配置层合并后最终生效 provider 与 GPTEasy 单文件修改之间的所有冲突场景。

**对后续 Spike 的影响：**

- Spike 002 应使用 `wire_api = "responses"` 并在供应商验证中捕获模型、流式和工具字段；不能只做 `/models` 或一次非流式请求。
- Spike 003 必须保护 `config.toml` 之外的配置层不被误判，并明确 GPTEasy 只修改用户层。
- 首版标准供应商若要同时支持桌面和 CLI，不能只写 `env_key` 后期待桌面进程获得新环境变量；建议验证成功后写入直接 Bearer token，但要在产品文档和诊断中标记其安全风险。
- Spike 004 的“待重启”状态是必要的，不应把切换后立即视为已生效。
