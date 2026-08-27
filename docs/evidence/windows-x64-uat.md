# Windows x64 候选、维护者验收与可选真实 UAT

Windows 正式发布采用两层必需门禁和一层可选深度验收：

1. `candidate:windows` 在干净的 `main` 上执行类型检查、完整测试、Issue #28 综合验收门禁、发布树检查、领域/UI 合同一致性检查和 Tauri x64 NSIS 构建。
2. `release:check -Mode Release -ConfirmMaintainerAcceptance` 复核当前提交、候选 manifest、安装包哈希、自动门禁、发布树、当前领域/UI 合同和 Authenticode 状态，并要求维护者明确确认人工测试结果和发布授权。
3. `uat:windows` 在一次性 Windows x64 当前用户账户中记录真实供应商、Codex CLI、打包应用单实例与安装生命周期的脱敏证据；它是按风险或维护者要求执行的可选深度验收，不是每次正式发布的默认前置条件。

候选、发布检查和可选 UAT 共享 `scripts/windows-release-contract.json`：Issue 身份、窗口尺寸、当前合同文档和 UAT check ID 只在该结构化合同中定义一次。合同同时登记 #39 的 `session_*` 检查，覆盖真实 App Server 方法/筛选、协议降级、外部消费者 mutation 门禁、无闪窗生命周期、Job Object 退出回收和精确所有权恢复。当前领域与 UI 文档使用稳定标记声明桌面控制只允许可信启动、用户确认后的可信桌面进程树重启和 CLI 生命周期隔离；删除标记或加入静默终止、CLI 控制等越界声明会使发布合同门禁失败。

真实 Codex App Server 合同测试位于 `src-tauri/tests/real_session_contract.rs`，默认 `#[ignore]`，不会把开发机的 Codex 登录状态变成普通 CI 前置条件。一次性 UAT 环境可设置 `GPTEASY_RUN_REAL_CODEX_SESSION_CONTRACT=1` 后用 `cargo test --manifest-path src-tauri/Cargo.toml --test real_session_contract -- --ignored` 运行；归档/取消归档还需要显式设置 `GPTEASY_RUN_REAL_CODEX_SESSION_MUTATIONS=1` 和目标 `GPTEASY_REAL_CODEX_SESSION_ID`，永久删除另需 `GPTEASY_ALLOW_REAL_CODEX_DELETE=1`。

## 构建候选

在提交完成且工作树干净后运行：

```powershell
npm run candidate:windows
```

构建清单写入 `src-tauri/target/release-candidate/manifest.json`，安装包位于对应 target 的 `release/bundle/nsis/`。两者都在 Git 忽略目录中。清单只记录相对路径、SHA-256、大小、提交和签名状态。

## 可选 UAT 前置条件

- 使用 Windows 10 22H2（build 19045）或更高版本的 x64 一次性当前用户账户，不使用日常开发账户。
- Codex CLI 0.147.0 或更高版本。为验收 #49 的桌面操作，需要安装当前用户可发现、发布者身份可验证的 OpenAI ChatGPT/Codex AppX 桌面版。
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

## 执行可选 UAT

功能验收包运行：

```powershell
npm run uat:windows -- --InstallerPath <setup.exe> -CandidateManifestPath <manifest.json> -ConfirmDisposableEnvironment
```

脚本依次完成以下检查，只有操作员实际观察到行为后才能输入精确的 `PASS`：

- 安装后应用可启动，真实供应商完成模型发现、Responses API 流式 strict 工具调用和 nonce 回传，并经明确保存与切换生效。
- 首次启动后记录同路径进程 PID；最小化设置窗口并再次启动同一安装，确认原窗口显示、取消最小化并聚焦，原 PID 保持唯一且托盘仍只有一个入口。
- 设置页和托盘选择非当前供应商时复用同一个“取消 / 切换”确认，确认不显示 Codex 工件、字段或重启选项；切换成功后当前供应商立即更新，制造安全失败后界面重新读取环境实际状态。
- 供应商切换本身仍只产生被动待重启，不自动控制消费者；旧消费者从原入口自然退出后状态自动清除。顶部桌面状态需分别验收可信启动、用户确认后的可信桌面进程树重启和失败不假报成功，并确认独立 Codex CLI 的 PID、终端和任务不受影响。新 Codex CLI 完成真实请求，证明读取目标配置和凭据载体。
- 恢复上次配置、有效外部配置接管、管理冲突阻断、OpenAI 登录模式和关闭窗口后的托盘驻留均由人工确认；退出前重新应用秘密文件中的目标供应商。
- 在 `1120 × 620` 默认尺寸和 `680 × 520` 最小尺寸分别确认底部操作可达，文字、标签和按钮不重叠，默认宽度下行操作不被旧断点强制换行。
- GPTEasy 从托盘明确退出后，脚本等待最多 2 秒并确认同一可执行路径的进程数归零；不同安装路径或开发构建不会被计入。随后在内存中核对最终 `config.toml` 包含秘密文件中的同一地址和模型但不含 Key，并核对 `auth.json` 的 `OPENAI_API_KEY` 与同一 Key 精确匹配。脚本不输出或复制任何工件正文。
- 脚本自动复核当前用户安装路径、开始菜单项、覆盖安装、覆盖后启动、静默卸载，以及卸载前后 `state.sqlite3` 的存在性和 SHA-256 不变。

脚本本身不会终止 GPTEasy 或 Codex；桌面正常退出和受控进程树结束只由操作员在 GPTEasy 界面中明确确认触发。覆盖安装和卸载前，操作员必须使用托盘中的“退出”结束 GPTEasy。成功证据写入 `src-tauri/target/uat/<UTC 时间>/evidence.json`，不包含用户名、绝对路径、服务地址、模型或 API Key。

## 发布检查

提供交互式 UAT 证据时，可以单独复核验收包：

```powershell
npm run release:check -- -Mode Acceptance -EvidencePath <evidence.json> -InstallerPath <setup.exe> -CandidateManifestPath <manifest.json>
```

正式对外发布默认由维护者明确确认人工测试，不要求 `EvidencePath`：

```powershell
npm run release:check -- -Mode Release -InstallerPath <setup.exe> -CandidateManifestPath <manifest.json> -ConfirmMaintainerAcceptance
```

发布检查绑定候选 manifest、当前提交和安装包哈希，并重新运行发布树及领域/UI 合同一致性检查，确认 ADR-0041、领域词汇和当前 UI 合同只允许可信桌面启动、用户确认后的可信桌面进程树重启和 CLI 生命周期隔离。它重新读取安装包 Authenticode 状态，只接受 `Valid` 或 `NotSigned`，并要求它与候选 manifest 完全一致；无效、未知或被破坏的签名仍会失败。若提供 `EvidencePath`，还会绑定并复核同一候选的交互式 UAT JSON，且默认拒绝测试生成的 synthetic evidence。

当前开发机不满足一次性账户和真实凭据前置条件时，不得生成或声称真实 UAT 通过；维护者仍可在完整候选门禁通过且已经完成与本次风险相称的人工测试后明确授权正式发布。
