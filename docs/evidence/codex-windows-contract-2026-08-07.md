# Codex Windows 本地合同复核

## 复核范围

- 日期：2026-08-07
- 目标：Windows 10 22H2 或更高版本，x64，当前用户默认 Codex 环境
- 本机 CLI：`codex-cli 0.146.1`
- 对应公开发布：[`rust-v0.146.1`](https://github.com/openai/codex/releases/tag/rust-v0.146.1)，发布时间 2026-08-05
- 本机 Windows 应用：`OpenAI.Codex 26.803.5235.0`，x64，AppX 状态正常
- 凭据：未读取本机 `auth.json` 内容，未记录 `codex login status` 的输出，未使用真实 API Key 或 OpenAI 令牌

## 已确认合同

### 默认路径与共同读取行为

| 项目 | 当前合同 | 依据 |
|------|----------|------|
| 当前用户 Codex 根目录 | `%USERPROFILE%\.codex` | [Windows app: Share config, auth, and sessions with WSL](https://learn.chatgpt.com/docs/windows/windows-app#share-config-auth-and-sessions-with-wsl) 明确 Windows 应用与原生 Windows Codex 使用同一目录。 |
| 用户配置 | `%USERPROFILE%\.codex\config.toml` | [Configuration Reference](https://learn.chatgpt.com/docs/config-file/config-reference#configtoml) 明确用户级配置位置。 |
| 本地状态根变量 | `CODEX_HOME`，默认 `~/.codex` | [Environment variables: Core locations](https://learn.chatgpt.com/docs/config-file/config-advanced#config-and-state-locations)；v0.1 只管理 Windows 默认目录，不支持自定义路径。 |
| 桌面应用与 CLI | 读取同一用户级本地配置层 | [Use ChatGPT Work and Codex with Amazon Bedrock](https://learn.chatgpt.com/docs/enterprise/bedrock) 明确 Windows 桌面应用、CLI、IDE 与 SDK 读取同一配置层；Windows app 文档明确同一 Codex home。 |

因此，GPTEasy 管理“当前用户 Codex 环境”，而不是分别管理桌面应用和 CLI。启动协调只读取 `config.toml` 的存在性、TOML 有效性与内容指纹，不创建缺失文件。

### 供应商配置字段

当前 `0.146.1` 接受以下用户级配置合同：

```toml
model = "<default-model>"
model_provider = "<immutable-provider-id>"

[model_providers.<immutable-provider-id>]
name = "<display-name>"
base_url = "https://provider.example/v1"
wire_api = "responses"
requires_openai_auth = true
```

- `model` 和 `model_provider` 位于用户级根配置。
- `model_provider` 指向 `[model_providers.<id>]`。
- `wire_api` 当前只支持 `responses`；旧的 `chat` 值已被移除。
- `requires_openai_auth = true` 使用 Codex 的 OpenAI 登录/API Key 凭据载体，并忽略 `env_key`。
- GPTEasy 不采用 `env_key`、命令认证或 `experimental_bearer_token`，因为 v0.1 不创建环境变量、不运行凭据辅助进程，也不把 Key 写入 `config.toml`。

依据：[Custom model providers](https://learn.chatgpt.com/docs/config-file/config-advanced#custom-model-providers) 与 [`ModelProviderInfo` v0.146.1 源码](https://github.com/openai/codex/blob/rust-v0.146.1/codex-rs/model-provider-info/src/lib.rs)。

### 用户级凭据载体

Codex `0.146.1` 的文件载体是 `%USERPROFILE%\.codex\auth.json`。公开结构包含：

- `auth_mode`
- `OPENAI_API_KEY`
- `tokens`
- `last_refresh`
- 其它当前 Codex 身份字段

`cli_auth_credentials_store` 支持 `file`、`keyring`、`auto` 和内部的临时模式；发布源码的默认值为 `file`。`file` 使用 `CODEX_HOME/auth.json`，`keyring` 使用系统凭据库，`auto` 优先系统凭据库后回退文件。

依据：[Authentication: Credential storage](https://learn.chatgpt.com/docs/auth#credential-storage)、[`AuthDotJson` v0.146.1 源码](https://github.com/openai/codex/blob/rust-v0.146.1/codex-rs/login/src/auth/storage.rs) 与 [`AuthCredentialsStoreMode` v0.146.1 源码](https://github.com/openai/codex/blob/rust-v0.146.1/codex-rs/config/src/types.rs)。

实现约束：

- 后续写入文件载体时只能结构化保留非 GPTEasy 字段，不能重建整个 JSON。
- OpenAI 登录模式不得读取、保存或删除 `tokens` 等登录令牌。
- 若用户显式选择 `keyring` 或 `auto` 且实际凭据不在文件载体，后续切换实现必须停止并给出可解释状态，不能假定写入 `auth.json` 会生效。
- API Key、令牌、完整认证命令输出不得进入日志、普通错误、通知、测试输出或本文档。

### OpenAI 登录检测

官方 CLI 合同是 `codex login status`：存在凭据时退出码为 `0`。Windows 的 npm 全局安装以 `codex.cmd` 提供可执行入口，因此 GPTEasy 通过固定参数的 `cmd.exe /D /S /C "codex login status"` 解析该 shim；命令不包含用户输入。运行时将 stdin、stdout 和 stderr 全部连接到空设备，只读取退出码；这样兼容文件与系统凭据库，也避免命令可能输出的认证摘要进入 UI 或日志。

依据：[CLI command reference: codex login](https://learn.chatgpt.com/docs/developer-commands?surface=cli#cli-codex-login)。

## 验证结论

- 当前 Windows 应用与原生 CLI 的默认根目录和配置层一致，可以作为一个当前用户 Codex 环境协调。
- `config.toml`、供应商字段、Responses-only 协议和默认文件凭据载体在 `0.146.1` 中仍成立。
- 启动阶段可以安全只读报告配置与登录状态；供应商凭据写入必须在后续阶段继续处理文件/Keyring 分支，不能越过该兼容门禁。
- 本阶段没有启动真实模型请求，也没有用模拟结果代替后续真实供应商和桌面 Codex UAT。
