# Codex 0.159.3 思考档位选择器证据

- 调查日期：2026-10-02
- 本机版本：`codex-cli 0.159.3`，实测 `codex --version`
- 官方源码标签：`rust-v0.159.3`
- 旧版本证据：[Codex 0.157.1 目录契约](codex-model-catalog-contract-2026-09-27.md)
- 决策：[ADR-0053](../adr/0053-common-reasoning-selector-defaults.md)

## 根因

`ModelInfo.supported_reasoning_levels` 是无 serde默认值的必填数组。省略它不会继承 Codex 内置档位，而是让自定义目录反序列化失败。`ModelPreset::from` 将 null默认档位转换为 `ReasoningEffort::None`，直接复制 supported列表。TUI模型选择器对空列表直接选默认 effort；reasoning popup对空列表仅补一个默认值，单选项时直接应用，不显示多档选择。由此，空列表加 null默认值不仅移除了深度选择，还可能在用户切换模型时把 `none` 写回配置。

官方固定版本源码：

- [ModelInfo与 ModelPreset转换](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/protocol/src/openai_models.rs)，`ModelInfo` 的 reasoning字段及 `impl From<ModelInfo> for ModelPreset`。
- [TUI模型/思考选择](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/tui/src/chatwidget/model_popups.rs)，`direct_effort`、`open_reasoning_popup`、`apply_model_and_effort`。
- [未知模型 fallback](https://github.com/openai/codex/blob/rust-v0.159.3/codex-rs/models-manager/src/model_info.rs)，`fallback_model_info` 的 null/空列表仅表示保守 fallback，不是“无限制”。

## 隔离真实 App Server实验

每组实验使用新的临时 CODEX_HOME、完整模型条目和假凭据；仅初始化并调用 `model/list`，不发送供应商模型请求、不修改本机默认配置。Windows子进程使用 `CREATE_NO_WINDOW`。

| 目录列表 | 目录默认 | 根级 effort | 实测结果 |
| --- | --- | --- | --- |
| `[]` | `null` | 省略 | 模型返回默认 `none`，可选列表 `[]` |
| 删除字段 | `null` | 省略 | `failed to parse model_catalog_json ... missing field supported_reasoning_levels`；App Server回退默认配置，发现模型不再来自该目录 |
| `low/medium/high/xhigh` | `high` | `none` | 返回四档，目录默认为 `high` |
| `low/medium/high/xhigh` | `high` | 省略 | 返回四档，目录默认为 `high` |

只读检查本机默认 config确认根级 effort为 `none`，目录文件名为 `gpteasy-model-catalog.json`。这说明 `none` 确实存在，但对照实验否定了“只要根级是 none就一定没有选择器档位”的假设。当前值的历史来源无法仅从配置文件断言。

## 回归验证

最初运行以下目录测试，断言三类发现模型无需逐模型配置即有四档，旧实现得到空数组而失败；修改共享模板后通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib discovered_models_offer_common_reasoning_choices_without_per_model_configuration
```

真实消费者回归使用 GPTEasy实际 `render` 的输出，分别对省略根级值和显式 none调用 App Server模型列表，断言三个不同模型 ID均返回四档和 high默认值：

```powershell
$env:GPTEASY_CODEX_EXECUTABLE = '<本机原生 codex 可执行文件的绝对路径>'
cargo test --manifest-path src-tauri/Cargo.toml --lib installed_codex_lists_common_reasoning_choices_with_and_without_root_none -- --ignored
```

实测 1项通过，运行约 0.6秒。测试位于 `src-tauri/src/provider/model_catalog_contract_tests.rs`；常规测试默认忽略，显式调用必须提供可执行文件路径，不能以缺少变量时静默返回来伪装通过。

环境集成测试同时断言所有发现模型使用共享目录默认值、只读检查不改根级 none、明确重新应用供应商清理旧根级 none并重建目录。旧图片模板识别和诊断脱敏测试防止本次模板升级破坏已有兼容路径。

## 2026-10-02 检查结果（历史记录）

- 库测试：139项通过，2项默认忽略（真实 Codex合约测试已单独显式运行；真实 WSL注册表探测未运行）。
- `environment_workflow`：68项通过；`diagnostic_report`：15项通过；`linux_export`：14项通过。
- 真实 Codex目录合约：1项通过。
- `cargo fmt -- --check`、`git diff --check`通过。
- 常规 `cargo clippy --all-targets`成功；严格 `-D warnings`因既有告警失败。常规 Clippy JSON诊断共19处唯一告警位置，其原始代码片段均与 `HEAD`匹配，本次新增代码未引入告警。

## 验证边界

通用四档是 GPTEasy允许客户端尝试的兼容选项，不能证明真实 OpenAI、DeepSeek或兼容供应商的每个模型都接受每档。未实施逐模型能力探测、请求转换、桌面选择器交互 UAT或 WSL/Linux新目录支持。现有会话可能保留显式 effort，需由用户在 Codex中选择；本次测试覆盖新进程读取目录的行为。


## 2026-10-03 / #68 原生消费者复验

本轮使用安装的 **`codex-cli 0.160.0`**，先实跑 `codex.exe --version`，再显式运行上述 ignored 合约测试：**1 项通过**（约 0.9 秒）。仍使用隔离临时 CODEX_HOME、回环不可用端点和假凭据，仅执行 App Server 初始化与 `model/list`；未修改用户默认配置，未向供应商发送模型请求。

GPTEasy 实际生成的三个模型条目在根级 effort 省略和显式 `none` 两组条件下，均返回 `low/medium/high/xhigh` 和默认 `high`。这项证据确认了消费者读取和选择器数据契约，不是桌面交互 UAT，也不认证模型实际支持四档。

#68 进一步明确了权威边界：供应商配置及完整验证快照保存在 **GPTEasy SQLite**；Codex 的 `config.toml`、凭据和 `model_catalog_json` 指向的 JSON 目录是派生工件。保留派生目录的备份与恢复，但不使用它回填失效快照；未删除或迁移 Codex 自身 SQLite。完整自动回归与归档见 [#68 实施归档](../archive/issue-68-effective-model-snapshot.md)。
