---
spike: 005
name: desktop-install-update-matrix
type: standard
validates: "Given Windows/macOS x64/ARM64 目标，when 构建、安装和更新 Tauri 2 应用，then 能确认当前用户安装、权限、签名公证、更新包和显式确认更新的可交付方式"
verdict: PARTIAL
related: [004]
tags: [tauri, installer, updater, windows, macos]
---

# Spike 005: 桌面安装与更新矩阵

## What This Validates

**Given** Windows 10 22H2+ 与 macOS 14+ 的 x64/ARM64 目标，  
**when** 构建、安装和更新 Tauri 2 应用，  
**then** 能确认当前用户安装路径、权限、签名/公证约束、更新包格式和显式确认更新流程。

## Research

### 已检查的资料

- Tauri Windows installer：`https://v2.tauri.app/distribute/windows-installer/`
- Tauri updater plugin：`https://v2.tauri.app/plugin/updater/`
- Tauri updater artifacts：`https://v2.tauri.app/distribute/updater/`
- Tauri macOS application bundle：`https://v2.tauri.app/distribute/macos-application-bundle/`
- Tauri macOS code signing：`https://v2.tauri.app/distribute/sign/macos/`
- Tauri macOS notarization：`https://v2.tauri.app/distribute/sign/macos/`
- Tauri GitHub Actions pipeline：`https://v2.tauri.app/distribute/pipelines/github/`

### 交付方式比较

| 平台/格式 | 当前用户安装 | 自动更新 | 结论 |
|---|---|---|---|
| Windows NSIS `currentUser` | **是**，安装到 `%LOCALAPPDATA%` | 支持 `.exe` + `.sig` | **首选** |
| Windows MSI | 主要面向系统级安装 | 可交付但不符合严格当前用户目标 | 不采用 |
| macOS 默认 DMG 的 `/Applications` 拖放 | 通常是机器级位置，可能需要管理员权限 | 安装后可更新 | **不满足“只为当前用户安装”的严格语义** |
| macOS `.app` 安装到 `~/Applications` | **是** | updater 可原地更新 | **推荐目标** |
| macOS universal `.app`/DMG | 同时支持 Intel/Apple Silicon | 包体更大，签名和公证一次完成 | 可选统一包 |
| macOS 分架构 `.app`/DMG | 包体更小 | 发布矩阵和更新清单更复杂 | 可作为补充 |

## How to Run

### 构建 Windows NSIS 与 updater 签名

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\005-desktop-install-update-matrix\run-build.ps1
```

脚本会：

1. 在 `.run/` 生成仅用于 Spike 的 updater 私钥和公钥。
2. 通过 build override 把公钥注入应用配置。
3. 构建 x64 NSIS 当前用户安装包。
4. 生成 updater `.sig` 文件。
5. 不把私钥、密码、构建产物或锁文件提交到 Git。

### 安装、验证并卸载

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\005-desktop-install-update-matrix\verify-current-user-install.ps1
```

测试静默安装唯一标识的 Spike 应用，确认安装位置位于当前用户 `%LOCALAPPDATA%`，随后调用其卸载程序清理。

### 生成 updater manifest

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\005-desktop-install-update-matrix\make-update-manifest.ps1
```

### macOS CI 模板

参考 `macos-build.yml.example`，在真实 macOS runner 上构建：

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `universal-apple-darwin`

## What to Expect

Windows x64 实测产物：

- `GPTEasy Spike 005_0.1.0_x64-setup.exe`
- 大小约 2.9 MB
- `GPTEasy Spike 005_0.1.0_x64-setup.exe.sig`
- 签名文本长度 432
- 安装位置：`C:\Users\<user>\AppData\Local\GPTEasy Spike 005`
- 应用和 `uninstall.exe` 均存在
- 静默卸载后应用文件和开始菜单项均被清理

更新 UI 有两个不同操作：

1. “检查更新”只调用 `check()`，不下载。
2. 用户再次确认“下载并安装”后，才调用 `download_and_install()`。

## Observability

- `.run/build-summary.json`：CLI 版本、安装包路径、大小、签名路径及签名大小。
- `.run/install-summary.json`：安装返回码、当前用户路径、应用与卸载程序存在性。
- `.run/latest.json`：可发布 updater manifest 示例。
- `.run/updater.key`：Spike 私钥，已被 `.gitignore` 排除，不能进入源代码或诊断导出。

## Investigation Trail

1. **Windows 当前用户安装**：配置 `bundle.windows.nsis.installMode = "currentUser"` 后，NSIS 在无管理员提升的情况下安装到 `%LOCALAPPDATA%\GPTEasy Spike 005`。
2. **安装清理**：安装包返回 0，应用与卸载器存在；执行卸载后安装目录和开始菜单项清理完成。
3. **更新签名与平台代码签名是两件事**：Tauri updater 私钥用于验证更新内容；Windows Authenticode 和 Apple Developer ID 用于操作系统信任。正式发布必须同时配置，不能用 updater 签名替代系统代码签名。
4. **显式确认不是 updater 默认保证**：如果应用启动后直接调用 `download_and_install()`，仍可能静默更新。本 Spike 把检查和安装拆成两个 Rust command，并在前端第二次确认。
5. **Windows updater 行为**：采用 `passive` installer UI，但下载/安装入口只有用户确认后才调用；“passive”不等于后台自动触发。
6. **Windows ARM64 边界**：已安装 Rust `aarch64-pc-windows-msvc` 标准库，但 `ring` 构建需要 ARM64 MSVC/Clang 工具链，当前 x64 环境未安装，未完成 ARM64 installer。
7. **macOS 交叉构建边界**：Tauri/Objective-C 依赖需要 Apple 工具链，不能从当前 Windows 主机完成可靠验证；必须使用 macOS runner。
8. **严格当前用户安装与默认 DMG 冲突**：Tauri 默认 DMG 引导拖入 `/Applications`，不能保证只安装到当前用户。若产品要求严格，应把正式安装目标定为 `~/Applications/GPTEasy.app`，通过用户级 bootstrap/安装说明完成首次放置；默认 DMG 只能作为不严格的可选分发方式。
9. **macOS 更新位置推论**：updater 原地替换已安装 app。只要首次安装在 `~/Applications` 且权限归当前用户，后续更新不需要写 `/Applications`；该结论仍需真实 Mac 验证。
10. **最低系统版本**：配置中已设置 macOS `minimumSystemVersion = "14.0"`，与产品支持范围一致。

## Results

### Verdict: PARTIAL ⚠️

**Windows x64 已验证：**

- Tauri 2.11.5 能生成 NSIS 当前用户安装包。
- 安装、开始菜单注册和卸载均在当前用户范围完成。
- Tauri updater 生成独立签名产物，并可生成符合平台映射结构的 `latest.json`。
- UI/命令层能够强制“检查”和“下载并安装”分离，满足显式确认契约。

**尚未验证：**

- Windows ARM64 安装包需要带 ARM64 C++ 工具的构建机。
- Windows Authenticode 证书签名、SmartScreen 信誉及真实 HTTPS updater endpoint。
- macOS Intel、Apple Silicon、universal bundle、Developer ID 签名、公证、`~/Applications` 首次安装和 updater 原地替换。

### Feasibility

Windows x64 发布链路可行。Windows ARM64 与 macOS 不是架构否定，而是构建基础设施和签名凭据缺口。  
macOS 需要产品层面的明确决定：若“当前用户安装”不可妥协，就不能把默认拖入 `/Applications` 的 DMG 作为唯一正式安装路径。
