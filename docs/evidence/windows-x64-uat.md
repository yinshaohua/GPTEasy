# Issue #22 Windows x64 真实 UAT 与安装交付

Issue #22 采用三层门禁，不能用自动化 fixture 替代真实 UAT，也不能用未签名验收包冒充正式发布包：

1. `candidate:windows` 在干净的 `main` 上执行类型检查、完整测试、Issue #22 综合验收门禁、发布树检查和 Tauri x64 NSIS 构建。
2. `uat:windows` 只允许在一次性 Windows x64 当前用户账户中运行，记录真实供应商、Codex CLI、桌面 Codex 和安装生命周期的脱敏证据。
3. `release:check` 复核证据、提交、安装包哈希、发布树和签名。`Acceptance` 允许未签名，`Release` 必须具有有效 Authenticode 签名。

## 构建候选

在提交完成且工作树干净后运行：

```powershell
npm run candidate:windows
```

构建清单写入 `src-tauri/target/release-candidate/manifest.json`，安装包位于对应 target 的 `release/bundle/nsis/`。两者都在 Git 忽略目录中。清单只记录相对路径、SHA-256、大小、提交和签名状态。

正式对外候选使用：

```powershell
npm run candidate:windows -- --RequireAuthenticode
```

## UAT 前置条件

- 使用 Windows 10 22H2（build 19045）或更高版本的 x64 一次性当前用户账户，不使用日常开发账户。
- Codex CLI 0.147.0 或更高版本，以及 OpenAI.Codex 26.803.5235.0 或更高版本 x64 桌面包均已安装。
- GPTEasy 尚未安装，`%LOCALAPPDATA%\com.gpteasy.desktop` 不存在，当前用户 `~/.codex/config.toml` 不存在。
- 工作树位于干净的 `main`，安装包由同一提交构建。
- 真实供应商凭据只保存在被 Git 忽略的 `.codex/skills/spike-findings-gpteasy/.secrets/provider.json`：

```json
{
  "base_url": "https://provider.example/v1",
  "api_key": "真实 API Key",
  "model": "真实模型 ID"
}
```

不要把真实值放入命令参数、环境变量、控制台记录、截图、Issue 或 Git。UAT 脚本只读取该文件以验证保护状态、计算不可逆组合指纹并扫描最终 JSON；供应商字段由操作员在应用中手工输入。

## 执行 UAT

功能验收包运行：

```powershell
npm run uat:windows -- --InstallerPath <setup.exe> -CandidateManifestPath <manifest.json> -ConfirmDisposableEnvironment
```

正式发布包追加 `-RequireAuthenticode`。脚本依次完成以下检查，只有操作员实际观察到行为后才能输入精确的 `PASS`：

- 安装后应用可启动，真实供应商完成模型发现、Responses API 流式 strict 工具调用和 nonce 回传，并经明确保存与切换生效。
- 首次启动后记录同路径进程 PID；最小化设置窗口并再次启动同一安装，确认原窗口显示、取消最小化并聚焦，原 PID 保持唯一且托盘仍只有一个入口。
- 旧 Codex 消费者存在时进入被动待重启且不被 GPTEasy 控制；用户在原入口关闭并重新运行后，新 Codex CLI 和新桌面 Codex 分别完成真实请求，证明读取目标配置和凭据载体。
- 恢复上次配置、有效外部配置接管、管理冲突阻断、OpenAI 登录模式和关闭窗口后的托盘驻留均由人工确认；退出前重新应用秘密文件中的目标供应商。
- GPTEasy 从托盘明确退出后，脚本等待最多 10 秒并确认同一可执行路径的进程数归零；随后在内存中核对最终 `config.toml` 包含秘密文件中的同一地址和模型但不含 Key，并核对 `auth.json` 的 `OPENAI_API_KEY` 与同一 Key 精确匹配。脚本不输出或复制任何工件正文。
- 脚本自动复核当前用户安装路径、开始菜单项、覆盖安装、覆盖后启动、静默卸载，以及卸载前后 `state.sqlite3` 的存在性和 SHA-256 不变。

脚本不会终止 GPTEasy 或 Codex。覆盖安装和卸载前，操作员必须使用托盘中的“退出”结束 GPTEasy。成功证据写入 `src-tauri/target/uat/<UTC 时间>/evidence.json`，不包含用户名、绝对路径、服务地址、模型或 API Key。

## 复核证据

验收包：

```powershell
npm run release:check -- -Mode Acceptance -EvidencePath <evidence.json> -InstallerPath <setup.exe> -CandidateManifestPath <manifest.json>
```

正式对外发布：

```powershell
npm run release:check -- -Mode Release -EvidencePath <evidence.json> -InstallerPath <setup.exe> -CandidateManifestPath <manifest.json>
```

复核会同时绑定候选 manifest、当前提交、UAT JSON 和安装包哈希，并默认拒绝测试生成的 synthetic evidence。正式发布模式会重新读取安装包 Authenticode 状态，只有 `Valid` 才通过。UAT JSON 中的历史签名字段、Tauri 更新签名或未签名功能验收结论都不能绕过该检查。

当前开发机不满足一次性账户、真实凭据和签名证书前置条件时，只能生成并校验未签名安装包，不能生成真实 UAT 通过证据，也不能关闭 Issue #22。
