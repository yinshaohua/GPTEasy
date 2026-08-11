# 安装与更新

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- 首版支持 Windows 10 22H2+ x64/ARM64，以及 macOS 14+ Intel/Apple Silicon。
- 只提供当前用户安装和更新，不提供整机所有用户安装。
- 更新必须由用户确认，不进行静默下载和安装。
- macOS 严格当前用户安装目标为 `~/Applications/GPTEasy.app`；默认指向 `/Applications` 的 DMG 不能是唯一正式安装路径。

## How to Build It

### 1. Windows 使用 NSIS `currentUser`

Tauri bundle 配置：

```json
{
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "createUpdaterArtifacts": true,
    "windows": {
      "nsis": {
        "installMode": "currentUser"
      }
    }
  }
}
```

该模式已验证可在无管理员提升时安装到 `%LOCALAPPDATA%`，并创建当前用户开始菜单项。发布验证至少检查：

- 安装器返回码
- 安装路径位于规范化后的 `%LOCALAPPDATA%`
- 主程序和 `uninstall.exe` 存在
- 开始菜单项属于当前用户
- 静默卸载测试能清理应用目录和开始菜单项

安装和卸载完成后的开始菜单证据应直接检查当前用户 `%APPDATA%\Microsoft\Windows\Start Menu\Programs` 下的预期 `.lnk` 文件。不要用 `Get-StartApps` 的即时结果作为同步门禁；它依赖 Windows Shell 的异步缓存，安装器已经创建快捷方式时仍可能短暂返回旧列表。

不要用 MSI 作为严格当前用户安装的主交付格式。

### 2. 把更新检查和下载/安装拆成两个命令

`check_update` 只检查并把 `Update` 暂存在 Rust 状态：

```rust
let update = app.updater()?.check().await?;
*pending.0.lock()? = update;
```

`install_update` 只能消费已经检查并由 UI 再次确认的 pending update：

```rust
let update = pending
    .0
    .lock()
    .map_err(|error| error.to_string())?
    .take()
    .ok_or_else(|| "没有已经确认的待安装更新".to_string())?;

update.download_and_install(|_, _| {}, || {}).await?;
```

UI 流程固定为：

1. 用户或每日一次的计划任务触发“检查更新”。
2. 只展示版本、发布日期和说明，不下载。
3. 用户明确点击“下载并安装”。
4. Rust 消费 pending update 并执行安装。

不要在 `check()` 返回新版本后自动调用 `download_and_install()`。Windows updater 的 `passive` 安装界面不等于允许后台自动触发。

### 3. 同时配置两类签名

必须区分：

- **Tauri updater 签名**：私钥签署更新产物，应用内公钥验证下载内容。
- **操作系统代码签名**：Windows Authenticode 或 Apple Developer ID，建立系统信任并减少安全警告。

updater 签名不能替代系统代码签名。私钥和密码只能放在 CI secret 中；Spike 的临时 key 生成脚本只用于实验，不能复用到正式发布。

生成更新清单时，每个平台条目包含 URL 和对应 `.sig` 内容，并确保 URL 使用真实 HTTPS 发布端点。

### 4. 用原生 runner 构建发布矩阵

建议 CI 矩阵：

| 平台 | 目标 | Runner 要求 |
|------|------|-------------|
| Windows | `x86_64-pc-windows-msvc` | x64 MSVC 工具链 |
| Windows | `aarch64-pc-windows-msvc` | ARM64 C++/MSVC 或兼容 Clang 工具链 |
| macOS | `x86_64-apple-darwin` | macOS runner、Intel target |
| macOS | `aarch64-apple-darwin` | macOS runner、Apple Silicon target |
| macOS | `universal-apple-darwin` | 合并两架构、签名和公证 |

每个发布候选必须验证：

- 应用最低系统版本
- bundle 架构
- 系统代码签名
- macOS 公证与 stapling
- updater 产物及签名
- 更新清单平台/架构映射
- 当前用户安装和原地更新路径

### 5. macOS 首次安装必须显式落到用户目录

若“严格当前用户安装”不可妥协，正式流程应把应用放到：

```text
~/Applications/GPTEasy.app
```

可以使用签名、公证后的 `.app`/归档配合用户级 bootstrap 或明确安装引导。默认让用户拖到 `/Applications` 的 DMG 只能作为非严格的可选分发方式，不能是唯一正式路径。

首次安装后，updater 应原地替换 `~/Applications` 中当前用户拥有的 app。该链路需要在真实 Intel 和 Apple Silicon Mac 上验证权限、签名、公证和更新后重启。

## What to Avoid

- **不要把 MSI 作为当前用户安装主方案。**
- **不要把默认 `/Applications` DMG 当成严格用户级安装。**
- **不要在检查到更新后立即自动下载或安装。**
- **不要把 updater 签名误认为 Windows/macOS 代码签名。**
- **不要把 Spike 临时私钥提交、复用或放进诊断。**
- **不要只在 Windows x64 构建成功后宣称全平台可交付。**
- **不要从 Windows 交叉构建失败推断 macOS 业务代码不可行。**

## Constraints

- Windows x64 NSIS 当前用户安装、卸载、updater 产物和 `.sig` 已实测。
- Windows ARM64 目前缺少 ARM64 原生 C++ 构建工具，尚未生成安装包。
- Windows Authenticode、SmartScreen 信誉和真实 HTTPS updater endpoint 尚未验证。
- macOS Intel、Apple Silicon、universal bundle、Developer ID 签名、公证、`~/Applications` 首次安装和 updater 原地替换都必须在真实 macOS runner/机器验证。
- universal 包发布简单但体积更大；分架构包更小，但更新清单和发布矩阵更复杂。

## Origin

Synthesized from spikes: 005
Source files available in: `sources/005-desktop-install-update-matrix/`
