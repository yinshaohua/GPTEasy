# macOS 真实宿主契约

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- 首版支持 macOS 14 或更高版本的 Intel 与 Apple Silicon。
- 严格当前用户安装必须以 `~/Applications/GPTEasy.app` 为目标。
- 默认指向 `/Applications` 的 DMG 不能作为唯一正式安装路径。
- 应用关闭窗口后继续托盘驻留，只有托盘中的明确退出操作才结束进程。
- 相关 Codex 进程不得只按名称分类，也不得由探针静默终止。
- 更新必须由用户确认，不进行静默安装。
- 系统代码签名、公证、Gatekeeper 和 Tauri updater 内容签名必须分别验证。
- Windows 交叉构建或 fixture 不能替代真实 macOS 宿主证据。

## How to Build It

### 1. 明确三层证据等级

macOS 能力必须按证据来源标记：

| 等级 | 能证明 | 不能证明 |
|------|--------|----------|
| 非 macOS 开发主机 | Rust/Tauri 代码可编译、纯逻辑矩阵成立 | LaunchServices、APFS、WindowServer、真实进程或签名 |
| macOS CI 原生构建 | Intel/ARM64 原生 bundle、metadata、基础签名检查 | 托盘视觉、真实 Codex 拓扑、用户交互和两版本 updater |
| 真实用户 Mac | `~/Applications`、托盘关闭语义、LaunchServices、真实进程、签名、公证和更新体验 | 仍需按架构和发布凭据覆盖完整矩阵 |

证据 JSON 必须包含 `evidence_level`。非 macOS 运行固定标记为 `non_macos_development_host`，不得授予 native verdict。

### 2. 构建 `.app` 并显式安装到用户目录

Tauri 配置至少包含：

```json
{
  "bundle": {
    "active": true,
    "targets": ["app"],
    "macOS": {
      "minimumSystemVersion": "14.0"
    }
  }
}
```

真实主机流程：

```zsh
npm run tauri build -- --bundles app
mkdir -p "$HOME/Applications"
/usr/bin/ditto "$BUILT_APP" "$HOME/Applications/GPTEasy.app"
```

安装验证读取：

- 当前执行路径是否位于 `$HOME/Applications/.../.app/Contents/MacOS/`
- `CFBundleIdentifier`
- `LSMinimumSystemVersion`
- 主机 macOS 版本和 `uname -m`
- `~/Applications` 是否可写

路径分类保持显式：

```text
current_user
system_applications
other_app_bundle
unbundled
unknown
```

`/Applications` 下可运行不代表满足严格当前用户安装要求。

### 3. 同时发现 Codex 与 ChatGPT 应用品牌

应用 bundle 候选至少覆盖：

```text
/Applications/Codex.app
/Applications/ChatGPT.app
~/Applications/Codex.app
~/Applications/ChatGPT.app
```

记录路径、bundle ID 和位置类别，不读取应用私有配置。

进程分类分两遍：

1. 找出 `.app/Contents/MacOS/` 下、不是 `Contents/Resources/codex` 且没有 Electron `--type=` 参数的桌面根。
2. 根据父 PID、resource 路径和进程名分类 bundled Codex 与独立 CLI。

稳定角色：

- `desktop_root`
- `desktop_codex_child`
- `cli`

桌面重新激活只生成 LaunchServices 候选：

```text
open -a Codex
open -a ChatGPT
```

探针不得自动终止真实 Codex。正式“立即重启”只能在用户确认后，对已识别桌面根进程树执行，并用 LaunchServices 重新激活。

### 4. 实现托盘驻留和明确退出

窗口关闭：

```rust
if let WindowEvent::CloseRequested { api, .. } = event {
    if !EXPLICIT_EXIT.load(Ordering::SeqCst) {
        api.prevent_close();
        let _ = window.hide();
    }
}
```

托盘至少提供：

- 显示窗口
- 导出脱敏证据
- 明确退出

明确退出先设置原子标志，再调用 `app.exit(0)`。同时拦截非明确的 `RunEvent::ExitRequested`，避免关闭最后一个窗口或平台事件意外结束托盘进程。

真实 Mac 验收必须人工确认：

- 菜单栏图标可见。
- 关闭窗口后进程仍在。
- 托盘“显示”恢复并聚焦窗口。
- 只有“明确退出”结束进程。

### 5. 把四类签名与更新证据分开

必须分别记录：

1. `codesign --verify --deep --strict`
2. Gatekeeper `spctl --assess --type execute`
3. Apple Developer ID 公证与 stapling
4. Tauri updater 更新归档签名

前两项通过不代表已公证，系统签名也不能替代 updater 内容签名。

两版本更新测试：

1. 安装已签名旧版本到 `~/Applications/GPTEasy.app`。
2. 在 Tauri app data 目录写入 canary，而不是写进 app bundle。
3. 用户确认更新。
4. updater 原地替换用户目录中的 app。
5. 新版本启动后读取同一 canary。
6. 再次检查 bundle 路径、版本、代码签名、Gatekeeper 和进程生命周期。

canary 应放在：

```rust
app.path().app_data_dir()?.join("update-canary.txt")
```

这样才能区分“应用 bundle 已替换”和“用户状态仍被保留”。

### 6. 用原生 Intel 与 Apple Silicon runner 建立 CI 门禁

原生构建矩阵至少覆盖：

| Runner | Rust target |
|--------|-------------|
| Apple Silicon macOS runner | `aarch64-apple-darwin` |
| Intel macOS runner | `x86_64-apple-darwin` |

每个 job：

1. 原生构建 `.app`。
2. 定位 bundle。
3. 读取 bundle ID 与最低系统版本。
4. 执行 `codesign --verify`。
5. 保存架构和 metadata 证据。

CI 构建通过后，真实宿主检查仍是发布门禁，不能自动升级为托盘、LaunchServices 或 updater 已验证。

### 7. 导出最小化真实宿主证据

证据可包含：

- 生成时间、macOS 版本、架构
- bundle 路径、bundle ID、最低系统版本、安装范围
- `codesign`、Gatekeeper、公证和 updater 签名状态
- Codex/ChatGPT bundle 的路径和 ID
- PID、PPID、进程角色和数量
- 托盘和生命周期事件类别
- updater 前后版本与 canary 是否保留

不要包含：

- Codex 配置正文
- API Key
- 完整进程命令行
- 完整 app-server 响应
- 签名私钥或公证凭据

## What to Avoid

- **不要把 Windows 编译成功写成 macOS 已验证。**
- **不要把 GitHub-hosted runner 构建成功写成真实桌面体验已验证。**
- **不要只寻找 `Codex.app`。** 统一宿主可能以 `ChatGPT.app` 存在。
- **不要只按进程名识别桌面根和 CLI。**
- **不要由探针自动终止真实 Codex。**
- **不要把 `/Applications` DMG 当成唯一安装路径。**
- **不要把 `codesign --verify` 当成公证或 Gatekeeper accepted。**
- **不要把系统代码签名当成 Tauri updater 内容签名。**
- **不要把更新 canary 写进 app bundle。**
- **不要在没有两版本、发布签名和真实主机时宣称 updater 原地替换成立。**

## Constraints

- Spike 017 当前只完成 Windows 契约矩阵、Tauri harness 构建、Zsh 脚本语法检查和 Intel/Apple Silicon CI 模板，因此 verdict 为 PARTIAL。
- 截至 2026-08-05，没有真实 macOS 14+ 主机运行结果。
- 真实 `Codex.app`/`ChatGPT.app` bundle ID、进程拓扑、托盘视觉、关闭隐藏、LaunchServices 激活仍未验证。
- Developer ID 签名、公证、Gatekeeper accepted 和已签名两版本 updater 原地替换仍未验证。
- macOS 是首版跨平台承诺中的最高风险项；正式规划必须保留真实 Mac 或具备发布凭据 runner 的验收任务。

## Origin

Synthesized from spikes: 017
Source files available in: `sources/017-macos-real-host-contract/`
