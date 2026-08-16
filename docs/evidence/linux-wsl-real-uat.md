# Issue #31 GNU/Linux 与 Windows x64 + WSL2 真实 UAT

Issue #31 在同一套 Full 门禁上完成两类真实宿主验收：Windows x64 使用一次性 WSL2 发行版覆盖 Running 与 Stopped 生命周期；独立 Ubuntu GNU/Linux 使用 QEMU 虚拟机，其内核不含 `microsoft` 或 `wsl` 标记。两边均使用真实 shell、文件系统权限和 Codex CLI 0.147.0，不以 fixture 版本输出代替 Codex 配置读取。

两类宿主受验实现提交均为 `b6027659c388235d349c958d970808564d61785b`。独立 Ubuntu 使用该提交的无 `.git` 源码包，源码包 SHA-256 为 `ad793367916cef294aa93d2800e9809de9570689fe343edaf77b932347b07d5b`，并通过 `-SourceCommit` 把报告绑定到同一提交。

## 环境与前置条件

| 项目 | Windows x64 + WSL2 | 独立 GNU/Linux |
| --- | --- | --- |
| 宿主 | Windows 11 x64，10.0.26200 | QEMU 11.1.0 + WHPX，`pc` machine |
| 客体 | 一次性 `GPTEasy-UAT-31`，Ubuntu 24.04.4 LTS | Ubuntu 24.04.4 LTS 官方 cloud image |
| 内核 | 6.6.87.1-microsoft-standard-WSL2 | 6.8.0-137-generic |
| Bash 4.4 | 4.4.0 | 4.4.0 |
| 当前 Bash | 5.2.21 | 5.2.21 |
| Zsh | 5.9 | 5.9 |
| Codex CLI | 0.147.0 | 0.147.0 |
| 验证接口 | `app-server config/read` | `app-server config/read` |
| 工具链 | Windows PowerShell + WSL2 | Node 24.15.0、npm 11.12.1、PowerShell 7.6.4、Rust 1.97.1 |

独立 Linux 使用 Ubuntu Noble 20260814 固定镜像，下载目录为 `https://cloud-images.ubuntu.com/noble/20260814/`，SHA-256 为 `6e40c07ae715f744f84af0bec76415cc1987dd115b4b8de437818561f01a3733`。镜像在启动前完成哈希和 qcow2 完整性检查；验收通过 overlay 写入，不修改基础镜像。

`gh` 只用于读取公开的 #29/#35 PRD。Ubuntu VM 没有持久化宿主 GitHub 配置；执行时通过标准输入临时提供认证，令牌不进入参数、日志、文档或验收证据。

## 可复现命令

Windows x64 在确认发行版可销毁后运行：

```powershell
npm run acceptance:linux-wsl -- `
  -WslDistribution GPTEasy-UAT-31 `
  -Bash44Path /opt/gpteasy-uat/bash-4.4/bin/bash `
  -BashCurrentPath /bin/bash `
  -Zsh59Path /usr/bin/zsh `
  -CodexPath /usr/local/bin/codex `
  -SourceCommit <源码包对应的 40 位 Git commit SHA> `
  -ConfirmDisposableWsl
```

Ubuntu VM 在已认证 `gh`、已安装仓库依赖并设置工具链 `PATH` 后运行：

```bash
pwsh -NoProfile -File scripts/run-linux-wsl-acceptance-gate.ps1 \
  -Mode Full \
  -SourceCommit <源码包对应的 40 位 Git commit SHA> \
  -Bash44Path /opt/gpteasy-uat/bash-4.4/bin/bash \
  -BashCurrentPath /bin/bash \
  -Zsh59Path /usr/bin/zsh \
  -CodexPath /opt/gpteasy-uat/node/bin/codex
```

## 脱敏证据

证据保存在 Git 忽略的 `src-tauri/target/acceptance/linux-wsl/`，便于本机复核而不把环境路径和执行日志提交到 Git：

| 宿主 | `evidence.json` | SHA-256 | 结果 |
| --- | --- | --- | --- |
| Windows x64 + WSL2 | `1486b9607b0b4d4fb72f6b14f9bce018/evidence.json` | `0dfad2e0b2e9f6ca079d3a304d108b51a591768e1eec2e71f2e9f2ce4a02abb9` | Full 通过 |
| 独立 Ubuntu GNU/Linux | `ecfa23a3d17c4f8dbeae9b92b4883e6a/evidence.json` | `c03fd39e69ed9b482f755eba401a0a47273ab3f1a61e3da501deec86b72cf9da` | Full 通过 |

每份通过证据均记录七组自动化矩阵、三种 shell 的实际版本、真实 Codex 版本与验证接口、真实环境门禁、平台前置条件、PRD 检查及十个泄漏扫描面。日志只在全部 canary 扫描通过后落盘；报告中的 `leaked` 为 `false`，扫描面为进程参数、标准输出、标准错误、前端 DOM、通知、错误详情、测试日志、应用后端日志、截图辅助和最终报告。Windows 报告的应用日志面包含 WSL 协调、删除与凭据清理及 Running/Stopped guest 四个后端步骤；Linux 报告包含前两项后端步骤。

Windows 证据同时记录 `wsl2-running-guest` 与 `wsl2-stopped-guest` 通过。Running harness 从同一桌面目录导出静态快照，在真实删除审计后删除其中一个供应商，再由 shell 应用旧快照；桌面识别 `ProviderMissing` 且没有重建目录记录。Stopped 门禁在 harness 前后都从 `wsl.exe --list --running --quiet` 确认实际停止；普通探测期间发行版保持 Stopped，显式操作后只等待自然停止，测试和调用记录均不含发行版级终止。

Ubuntu 证据记录 `independent-gnu-linux` 通过，且 runner 在进入矩阵前读取 `/proc/sys/kernel/osrelease` 拒绝 WSL 内核。真实 Codex 黑盒在隔离 `CODEX_HOME` 中应用导出快照，再从 `config/read` 响应核对模型、供应商 ID、名称、服务地址与 Responses wire API；前后两次核对 `auth.json` SHA-256 不变。

## Issue #31 验收映射

| #31 验收项 | 自动化或真实证据 |
| --- | --- |
| 独立 Ubuntu 使用真实 shell、权限和 Codex | Ubuntu 三项 shell matrix、`independent-gnu-linux`、`real-codex-config-read` |
| Windows x64 + WSL2 Running/Stopped | Windows `wsl2-running-guest`、`wsl2-stopped-guest` 及实际 Stopped 前后检查 |
| 桌面与 shell 双向切换、实际状态识别 | `wsl-shared-protocol` 与 Running guest harness |
| 共享锁及各持久化 Saga 阶段恢复 | Running guest harness 覆盖 registered、locked、prepared、artifacts_replaced、state_committed |
| 旧格式、未知 schema、旧快照、删除 ID | shell matrix、`wsl-shared-protocol`、`provider-deletion-and-credential-cleanup` 及 Running guest 的真实目录删除后旧快照应用 |
| Stopped 无副作用探测及自然停止 | Stopped guest harness 与 runner 实际状态检查 |
| 删除核验和跨来源凭据引用清理 | `provider-deletion-and-credential-cleanup` 与 Stopped guest harness |
| `auth.json` 逐字节不变 | 三种真实 shell、真实 Codex、Running/Stopped guest harness |
| API Key 不进入公开表面 | 两份证据的十个 canary 扫描面，`leaked=false` |
| 环境、版本、矩阵、脱敏和复现步骤 | 本文及两份 `evidence.json` |
| 未验证范围如实记录 | 本文“未验证范围” |
| #29 用户可观察项可追溯 | 下表 |

## Issue #29 可追溯关系

| #29 用户故事 | 自动化验证 | 真实环境验证 |
| --- | --- | --- |
| 1-15 导出选择、快照、敏感提示、覆盖与用法 | `linux-export-generator`、`react-linux-wsl-workflows` | 三种导出物由真实 shell 加载和执行 |
| 16 source 零写入 | `linux-shell-public-behavior` | Bash 4.4、当前 Bash、Zsh 5.9 同矩阵 |
| 17-27 菜单、状态、命令、恢复与解锁 | `linux-shell-public-behavior` | 三种真实 shell 的公开入口 |
| 28-39 Codex、凭据、权限、原子写入和恢复点 | shell matrix、`real-codex-config-read` | 两类宿主的真实权限与 Codex；`auth.json` 字节不变 |
| 40-45 双向状态、共享锁与崩溃恢复 | `wsl-shared-protocol`、SQLite Saga | Running guest 与五个持久化阶段中断恢复 |
| 46-51 Stopped 探测、授权启动、自然停止和旧格式 | `wsl-shared-protocol` | 实际 Stopped 前后检查与 Stopped guest harness |
| 52-55 删除核验、旧快照、删除 ID和引用清理 | `provider-deletion-and-credential-cleanup` | Running/Stopped guest 的实际配置和凭据工件 |
| 56-58 外部行为门禁、秘密扫描和完整交付 | 单命令 Full gate、九面 canary 扫描 | Windows x64 + WSL2 与独立 Ubuntu 证据并列通过 |

## 未验证范围

本轮没有验证 Windows ARM64、除 `GPTEasy-UAT-31` 外的其它真实 WSL2 发行版、Ubuntu 以外的 GNU/Linux 发行版、Bash 4.4/5.2 与 Zsh 5.9 之外的 shell 版本，也不把这些范围声明为已覆盖。它们是后续扩展矩阵，不阻塞 #31。
