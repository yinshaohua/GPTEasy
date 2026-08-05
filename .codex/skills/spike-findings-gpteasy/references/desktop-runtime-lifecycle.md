# 桌面运行生命周期

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- 使用 Tauri 2 实现托盘桌面应用。
- 检测到相关 Codex 进程时不得静默强制终止。
- 用户主动切换前必须可选择立即重启、稍后重启或取消；取消发生在配置写入前。
- “立即重启”只自动处理桌面宿主进程树；本机 Codex CLI 必须由用户在原终端退出并重新运行。
- 关闭设置窗口后继续托盘驻留，只有托盘中的明确退出操作才结束程序。

## How to Build It

### 1. 用路径、父子关系和命令类型联合分类

使用 `sysinfo` 获取 PID、PPID、进程名、可执行路径和必要的命令类型摘要。分类分两遍：

1. 先定位桌面根进程。
2. 再根据父 PID 和 bundle/package 路径分类 bundled Codex 子进程与独立 CLI。

桌面根进程需要同时满足：

- 名称像 `ChatGPT.exe`、`ChatGPT`、`Codex` 或 `Codex.exe`
- Windows 路径属于 `WindowsApps` 中的 `OpenAI.Codex_*`，或 macOS 路径属于 `.app/Contents/MacOS/`
- 不是 `resources/codex` bundled 二进制
- 命令行参数中没有 Electron helper 的 `--type=`

已验证的核心模式：

```rust
let electron_helper = command
    .iter()
    .skip(1)
    .any(|argument| argument.starts_with("--type="));

desktop_name
    && !bundled_resource
    && !electron_helper
    && (packaged_windows || mac_bundle)
```

角色保持稳定：

| 角色 | 含义 |
|------|------|
| `desktop_root` | 可整体退出并通过应用激活机制重启的桌面主进程 |
| `desktop_codex_child` | 由桌面宿主管理的 bundled Codex/app-server |
| `cli` | 不属于已识别桌面宿主的本机 Codex CLI |
| `legacy_or_other_host` | 旧版或无法可靠归属的相似进程，只提示，不自动处理 |

### 2. 把用户选择编译成写入/重启计划

不要让 UI 直接杀进程。Rust 先生成可审查的 `RestartPlan`：

```rust
pub struct RestartPlan {
    pub decision: String,
    pub write_configuration: bool,
    pub pending_restart: bool,
    pub actions: Vec<RestartAction>,
    pub warnings: Vec<String>,
}
```

状态规则：

| 决策 | 写配置 | 桌面进程 | CLI | 最终状态 |
|------|--------|----------|-----|----------|
| `cancel` | 否 | 不处理 | 不处理 | 保持原状态 |
| `later` | 是 | 保持运行 | 保持运行 | 有相关进程时进入待重启 |
| `immediate` | 是 | 终止桌面树后重新激活 | 不终止，提示人工重启 | CLI 存在时继续待重启 |

推荐执行顺序：

1. 扫描进程并生成计划。
2. 用户确认计划。
3. 若取消，立即返回，不进入写配置事务。
4. 验证并原子写入配置。
5. 仅当写入成功后执行桌面重启计划。
6. CLI 或重启失败的桌面进程进入待重启状态。

对于“已确认保存的供应商配置传播”，取消不再适用；写入后只让用户选择立即或稍后重启。

### 3. 桌面应用按平台激活，不直接执行受保护路径

- Windows packaged app：通过发现到的 AppUserModelID 使用 `explorer.exe shell:AppsFolder\<AUMID>` 激活。
- macOS：使用 bundle ID 或 `open -a Codex` / `open -a ChatGPT`。
- 普通未打包桌面应用：只有在路径和所有权可信时才考虑执行原入口。

Spike 中的 `OpenAI.Codex_2p2nqsd0c76g0!App` 是当前机器证据，不应永久硬编码。正式实现应从已安装包或开始菜单注册中发现 AUMID，并对无法发现的情况降级为人工重启。

自动终止必须按已验证的桌面根 PID 限定进程树。fixture 测试还应像 Spike 一样规范化测试根路径，只允许终止路径位于测试沙盒内的进程。

### 4. 实现“关闭隐藏、明确退出”

Tauri 托盘可沿用：

```rust
if let WindowEvent::CloseRequested { api, .. } = event {
    if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
        api.prevent_close();
        let _ = window.hide();
    }
}
```

托盘至少提供：

- 显示设置
- 重新扫描 Codex 进程
- 明确退出

“明确退出”先设置进程内退出标志，再调用 `app.exit(0)`；同时拦截非明确的 `ExitRequested`，避免关闭最后一个窗口时意外退出。

### 5. 保持诊断最小化

进程诊断记录 PID、PPID、名称、角色、可执行路径和分类理由即可。不要保存完整命令行；若必须判断 `--type=`，只记录布尔结果或已知安全的参数类别。

## What to Avoid

- **不要只按进程名检测。** 桌面 bundled `codex.exe` 与本机 CLI 同名。
- **不要把所有 `ChatGPT.exe` 当桌面根。** Electron renderer、GPU、网络和 crashpad helper 复用同一可执行文件。
- **不要单独重启 bundled Codex 子进程。** 它由桌面主进程拥有。
- **不要直接执行 WindowsApps 内的二进制。** 可能被 ACL 拒绝，应通过应用激活。
- **不要自动终止或重建 CLI。** 无法可靠恢复 TTY、cwd、stdin、环境和会话。
- **不要在配置写入后再向用户提供“取消”。**
- **不要把窗口关闭等同于退出。**
- **不要把 Spike 的固定 AUMID 当成跨版本常量。**

## Constraints

- Windows 已验证真实进程拓扑、8/8 fixture 状态机和 Tauri 关闭后驻留 smoke。
- 为避免中断当前工作，Spike 没有终止并重新激活真实桌面 Codex；真实执行仍需安全的人工验收。
- macOS 14+ 的进程名、bundle 路径、托盘行为和激活方式尚未在真实 Mac 验证。
- 从 Windows 交叉检查 Tauri macOS target 会受 Apple/Clang 工具链限制，不能作为业务代码失败的证据。
- 托盘图标和菜单视觉体验尚未由用户人工确认。

## Origin

Synthesized from spikes: 001, 004
Source files available in: `sources/001-codex-native-config-contract/`, `sources/004-tauri-tray-process-restart/`
