---
spike: 009
name: wsl2-environment-lifecycle
type: standard
validates: "Given 多个运行中或已停止的 WSL2 发行版及其默认用户，when 检测、单独切换或批量切换供应商，then 检测不启动发行版、显式切换才临时启动、只修改默认用户并恢复原停止状态"
verdict: PARTIAL
related: [006, 007, 008, 010a, 010b]
tags: [wsl2, windows, process, config, lifecycle, backup]
---

# Spike 009: WSL2 环境生命周期

## What This Validates

**Given** Windows 当前用户可能有多个运行中、已停止或基础设施用途的 WSL2 发行版，  
**when** GPTEasy 检测环境、单独切换或批量切换供应商，  
**then** 检测阶段不启动或进入任何发行版，显式切换已停止发行版时才临时启动，只修改默认用户配置，并在成功、失败或管理区块损坏后恢复原停止状态。

## Research

### 已检查的资料

- WSL 基本命令：`https://learn.microsoft.com/windows/wsl/basic-commands`
- WSL 文件系统与命令执行：`https://learn.microsoft.com/windows/wsl/filesystems`
- WSL 高级设置与默认用户：`https://learn.microsoft.com/windows/wsl/wsl-config`
- WSL 1/2 架构差异：`https://learn.microsoft.com/windows/wsl/compare-versions`

### 检测方案比较

| 方案 | 是否启动发行版 | 能获得的信息 | 状态 |
|---|---:|---|---|
| 对每个发行版执行 `wsl -d NAME -- id -un` | **会启动** | 默认执行用户、Linux 路径 | 检测阶段淘汰 |
| `wsl --list --verbose` | 不启动 | 名称、状态、版本，但状态文本本地化 | 不作为机器解析主接口 |
| `wsl --list --quiet` + `--list --running --quiet` | 不启动 | 全部名称与运行中名称，可用集合差得到停止状态 | **采用** |
| 当前用户 Lxss 注册表 | 不启动 | 注册 ID、显示名称、DefaultUid、WSL 版本、BasePath 是否存在 | **采用，只读** |

正式检测使用：

1. `wsl.exe --list --quiet`
2. `wsl.exe --list --running --quiet`
3. 只读 `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`

`wsl.exe` 当前输出为 UTF-16，本 Spike 使用 `ProcessStartInfo.StandardOutputEncoding = Unicode`，不解析本地化的 `Running`/`Stopped` 文本。

### 生命周期状态机

| 原状态 | 用户动作 | 行为 | 结束状态 |
|---|---|---|---|
| Running | 取消 | 不写配置 | Running |
| Stopped | 取消 | 不启动、不写配置 | Stopped |
| Running | 应用切换 | 修改默认用户配置 | Running |
| Stopped | 应用切换 | 临时启动、修改、终止 | Stopped |
| Stopped | 写入失败或区块损坏 | 临时启动、停止修改、终止 | Stopped |

WSL 内已有 Codex 进程时，只进入待重启，不调用 kill 或 terminate 处理该 Linux 进程。

## How to Run

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\009-wsl2-environment-lifecycle\run.ps1
```

该脚本先执行真实 Windows 只读探针，再运行路径受限 fixture。不会进入、启动或终止真实发行版。

## What to Expect

`.run/summary.json` 应显示 11/11：

- 真实探针前后运行集合一致，且没有执行发行版内命令。
- fixture 检测无 start/terminate 副作用，并排除 `docker-desktop`。
- 运行中发行版只修改默认用户配置。
- 已停止发行版成功切换后恢复停止。
- 写入失败和管理区块损坏时仍恢复停止。
- Codex 运行中只进入待重启，不产生 kill 动作。
- 取消不会启动或写入。
- 批量切换分别恢复每个发行版的原状态。
- 每个环境保留最近五份备份并支持恢复。

## Observability

- `.run/windows-evidence.json`：WSL 版本、发行版名称、切换前后运行集合，以及脱敏的 Lxss 注册信息。
- 不记录发行版 BasePath，只记录其是否存在。
- fixture 的动作日志仅包含 `start`、`write` 和 `terminate` 及发行版/默认用户名。
- 不读取或输出真实 WSL 用户的 Codex 配置。

## Investigation Trail

1. **真实只读探针**：2026 年 8 月 5 日当前机器为 WSL 2.5.7.0，检测到运行中的 `Ubuntu` 和停止的 `docker-desktop`。探针前后运行集合均为 `Ubuntu`。
2. **不能解析本地化状态列**：`wsl --list --verbose` 的标题和状态会本地化。使用全部名称与运行名称的集合差更稳定。
3. **默认用户不能在检测阶段通过 Linux 命令解析**：执行 `id -un` 会启动已停止发行版。检测阶段只能读取注册表 `DefaultUid`；用户名应在用户明确切换、发行版已经允许启动后解析。
4. **显示名称不是可靠数据库身份**：真实 Lxss 注册表中出现了多个不同注册 ID 使用同一 `Ubuntu` 显示名称，而 `wsl --list` 只展示一个。正式模型必须保存注册 GUID，并在名称到命令目标无法唯一对应时进入需要人工处理，不能仅以显示名称作主键。
5. **基础设施发行版要过滤**：`docker-desktop` 不应出现在可管理的 WSL2 环境列表中。
6. **恢复必须放在 finally 语义中**：fixture 在写入失败和管理区块损坏时都执行了终止动作，证明“恢复原停止状态”不能只放在成功路径。
7. **默认用户边界**：每个 fixture 同时有默认用户和其他用户；切换只改变默认用户 `.codex/config.toml`。
8. **待重启不等于切换失败**：运行中的 WSL Codex 继续使用旧配置，但文件和当前供应商状态已经更新；产品只提示人工重启。
9. **批量操作是逐环境 Saga**：一个发行版的临时启动和恢复不应改变其他发行版的原状态。
10. **配置协议复用 006**：fixture 使用首次结构化接管、管理区块、原子替换和五份备份规则；实际 WSL 内写入方式将在 010 的 Bash/Zsh function 中验证。

## Results

### Verdict: PARTIAL ⚠️

真实 WSL 检测边界已验证：只读命令不会启动发行版，当前机器运行集合保持不变。fixture 的 10 个生命周期、失败、批量、默认用户和备份场景全部通过。

### 已验证

- 发行版检测不需要进入或启动 Linux 环境。
- 可通过全部/运行集合差稳定判断 Running 与 Stopped，而不依赖本地化状态文本。
- 临时启动必须在所有成功和失败路径恢复原停止状态。
- 默认用户、其他用户、基础设施发行版和运行中 Codex 可以清晰分开。
- 批量切换可以建模为逐环境独立操作。

### 尚未验证

- 为避免中断真实 Ubuntu 和 Docker，本次没有在真实发行版内解析默认用户名、写配置或执行 `wsl --terminate`。
- 真实已停止用户发行版的临时启动耗时、失败码和终止时机。
- WSL 内运行中 Codex 的真实进程探针。
- 注册表重复显示名称如何与 `wsl.exe -d NAME` 的实际目标一一对应；在没有可靠解歧前必须安全停止。
