---
spike: 004
name: tauri-tray-process-restart
type: standard
validates: "Given Tauri 2 托盘程序运行且桌面 Codex 或 Codex CLI 可能持有旧配置，when 检测进程并切换供应商，then 能呈现立即重启、稍后重启、取消和明确退出语义且不误杀无关进程"
verdict: PARTIAL
related: [001, 003a, 003b, 005]
tags: [tauri, tray, process, restart, windows, macos]
---

# Spike 004: Tauri 托盘、进程检测与重启语义

## What This Validates

**Given** Tauri 2 应用驻留托盘，桌面 Codex 和本机 Codex CLI 可能仍在使用旧配置，  
**when** GPTEasy 检测相关进程并处理供应商切换，  
**then** 能区分桌面主进程、bundled Codex 子进程和 CLI，并给出“立即重启／稍后重启／取消”计划而不误杀无关进程。

## Research

### 版本与资料

截至 **2026 年 8 月 5 日**，本 Spike 使用：

- Tauri Rust crate `2.11.5`
- Tauri CLI `2.11.4`
- `sysinfo` `0.39.6`

资料：

- Tauri 2 系统托盘：`https://v2.tauri.app/learn/system-tray/`
- Tauri `WindowEvent::CloseRequested`：`https://docs.rs/tauri/latest/tauri/enum.WindowEvent.html`
- `sysinfo::Process`：`https://docs.rs/sysinfo/latest/sysinfo/struct.Process.html`

### 方案比较

| 方案 | 优点 | 缺点 | 状态 |
|---|---|---|---|---|
| Tauri 2 `TrayIconBuilder` + Rust commands | 单一跨平台代码；菜单、窗口和退出事件统一 | 应用激活与进程重启仍需平台分支 | **采用** |
| Windows/macOS 分别写原生托盘 | 平台能力最完整 | 重复实现、维护成本高 | 暂不采用 |
| 仅按进程名检测 | 简单 | 无法区分桌面 bundled `codex` 与本机 CLI；Electron helper 也会误判 | 淘汰 |
| `sysinfo` + 路径 + 父子关系 + 命令类型 | 跨平台且证据较完整 | macOS 应用名和路径仍需真实机校准 | **采用** |

## How to Run

### 自动化进程与状态机矩阵

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\004-tauri-tray-process-restart\run-process-tests.ps1
.\.codex\skills\spike-findings-gpteasy\sources\004-tauri-tray-process-restart\run-tauri-smoke.ps1
```

测试使用自己编译的 fixture `ChatGPT.exe` 和 `codex.exe`，只终止 `.run/` 下的 fixture，不会终止真实 Codex。

### 交互式体验

```powershell
cd .codex/skills/spike-findings-gpteasy/sources/004-tauri-tray-process-restart
npm install
npm run tauri dev
```

体验步骤：

1. 页面显示真实桌面主进程、桌面 Codex 子进程和本机 CLI。
2. 点击三种决策查看计划。
3. 关闭窗口，确认应用继续驻留托盘。
4. 从托盘选择“显示设置”恢复窗口。
5. 从托盘选择“明确退出”结束应用。

## What to Expect

自动矩阵应为 8/8：

- fixture 桌面主进程、桌面 Codex 子进程和 CLI 分别正确分类。
- “立即重启”把 CLI 标记为人工重启，不静默终止。
- “稍后重启”允许写配置并进入待重启。
- “取消”不允许写配置。
- fixture 桌面进程树可终止并重新启动。
- fixture CLI 不被自动重新启动。

Tauri smoke test 应显示：

- 应用成功启动。
- 向主窗口发送关闭消息后进程仍然存活。
- 测试最后只强制结束 GPTEasy Spike 自身，不触碰 Codex。

## Observability

- `.run/summary.json`：八项状态机断言。
- `.run/real-processes.json`：真实进程的 PID、PPID、名称、角色、路径和分类理由。
- `.run/tauri-smoke.json`：启动与关闭窗口后的驻留结果。
- 不记录进程完整命令行，避免命令参数中潜在的凭据或敏感输入进入诊断。

## Investigation Trail

1. **真实 Windows 环境**：检测到 `OpenAI.Codex_26.730.8199.0_x64__2p2nqsd0c76g0`，其主进程为 `ChatGPT.exe`，bundled 子进程为 `resources\codex.exe ... app-server`，另有本机 CLI `codex.exe`。
2. **不能只看进程名**：桌面 bundled `codex.exe` 与 CLI 同名，必须使用父 PID 和可执行路径区分。
3. **不能把所有 ChatGPT.exe 当主进程**：Electron renderer、GPU、网络和 crashpad helper 都复用同一可执行文件。最终算法只把没有 `--type=` 的 packaged 进程视为桌面根。
4. **Windows packaged app 重启**：直接执行 WindowsApps 内的二进制可能被访问控制拒绝。当前安装的 AppUserModelID 为 `OpenAI.Codex_2p2nqsd0c76g0!App`，应通过 `explorer.exe shell:AppsFolder\...` 激活。
5. **CLI 无法透明重启**：GPTEasy 无法可靠恢复原终端的 TTY、当前目录、stdin、环境和会话状态。因此“立即重启”只能自动处理桌面应用；CLI 必须提示用户在原终端退出并重新运行。
6. **关闭不等于退出**：Tauri `CloseRequested` 中调用 `prevent_close` 并隐藏窗口；托盘“明确退出”设置退出标记后调用 `app.exit(0)`。自动 smoke 已证明收到关闭消息后进程继续存活。
7. **fixture 安全边界**：执行重启测试前规范化 fixture 根目录，只允许终止可执行路径位于该目录内的测试进程。
8. **macOS 交叉编译边界**：从 Windows 对 Tauri macOS target 执行 `cargo check` 时，`objc2-exception-helper` 需要 Apple/Clang 工具链，无法在当前机器完成；这不是 Rust 业务代码错误，但意味着必须在真实 Mac 或 macOS CI 上验证。

## Results

### Verdict: PARTIAL ⚠️

**Windows 已验证：**

- Tauri 2 托盘应用可编译、启动，并在关闭主窗口后继续驻留。
- 当前真实 Codex 拓扑被准确收敛为 1 个桌面根、1 个 bundled Codex 子进程和 1 个本机 CLI。
- fixture 进程检测和重启矩阵 8/8 通过。
- Windows packaged Codex 应通过 AppUserModelID 重新激活，而不是直接执行 WindowsApps 路径。

**尚未验证：**

- macOS 14+ 上的真实进程名称、bundle 路径、托盘图标、关闭行为和 `open -a Codex` 激活。
- 用户尚未人工确认托盘图标与菜单视觉体验。
- 正式桌面 Codex 的实际终止与重新激活没有执行，以避免中断当前会话。

### 对正式产品的关键调整

- UI 文案不应承诺“自动重启所有 Codex”。桌面应用可自动重启，CLI 应进入“需要在原终端重启”状态。
- 进程检测必须保留路径与父子关系，不能仅按名称匹配。
- “取消”必须发生在配置写入前；“稍后重启”写入后进入待重启；“立即重启”写入后仅自动处理桌面进程树。
