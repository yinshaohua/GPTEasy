# Issue #35 Linux 与 WSL2 自动验收门禁

正式门禁默认运行 Full 模式，并要求 `gh` 能读取当前 #29/#35 PRD。Windows 上需在同一条命令中提供三个 shell 路径并确认一次性 WSL2 发行版：

```powershell
npm run acceptance:linux-wsl -- `
  -WslDistribution <一次性发行版> `
  -Bash44Path <GNU Bash 4.4 路径> `
  -BashCurrentPath <当前 GNU Bash 路径> `
  -Zsh59Path <Zsh 5.9 路径> `
  -CodexPath <Codex CLI 0.147.0 或更高版本路径> `
  -ConfirmDisposableWsl
```

它生成两个随机 API Key canary，在内存中捕获并扫描进程参数、标准输出/错误、React DOM、通知、错误详情、测试日志、截图辅助和最终报告。只有全部扫描面不含 canary 后，才把日志与 `evidence.json` 写入 `src-tauri/target/acceptance/linux-wsl/<session>/`；检测到泄漏时返回非零状态，且不持久化本轮日志或证据。

矩阵运行 Linux 导出生成器、三个 shell 的同一套公开黑盒行为、WSL2 共享协议与生命周期、SQLite schema/迁移、供应商删除与凭据清理、React Linux/WSL2 用户流程，以及领域/ADR/界面/GitHub PRD 合同检查。公开 shell 黑盒覆盖直接执行、source 零写入、全部子命令、切换、恢复、锁、权限、Codex 版本、symlink、hardlink、并发修改和 `auth.json` 逐字节不变。

只运行可重复自动化与当前 Bash、并把未执行的真实环境门禁写入报告时，使用开发子集：

```powershell
npm run acceptance:linux-wsl:automated
```

在原生 GNU/Linux 上以 `-Mode Full` 运行同一脚本时，三个 shell 目标计入独立 GNU/Linux 门禁；Windows 上计入 WSL2 shell 矩阵。Full 模式还会在隔离的 `CODEX_HOME` 中调用所选真实 Codex 的 `app-server config/read`，验证生成配置实际被目标版本接受。原生 Linux 门禁拒绝 WSL 内核，不能用 WSL2 冒充独立 GNU/Linux。报告始终列出平台前置条件、各 shell 版本、真实 Codex 版本、Running/Stopped WSL2 结果和未执行的真实环境门禁。

Issue #31 的真实环境、执行矩阵、脱敏证据路径和 #29 可追溯关系记录在 [真实 UAT 证据](linux-wsl-real-uat.md)。

Issue #28 的原命令和语义保持不变：

```powershell
npm run acceptance
```

在 Windows 上可用以下命令依次运行 #28 与 #35，并输出并列汇总：

```powershell
npm run acceptance:all -- `
  -WslDistribution <一次性发行版> `
  -Bash44Path <GNU Bash 4.4 路径> `
  -BashCurrentPath <当前 GNU Bash 路径> `
  -Zsh59Path <Zsh 5.9 路径> `
  -CodexPath <Codex CLI 0.147.0 或更高版本路径> `
  -ConfirmDisposableWsl
```

开发机只需并列运行 #28 与 #35 自动化子集时，使用 `npm run acceptance:all -- -LinuxWslMode Automated`；汇总会明确保留未执行的真实环境项。
