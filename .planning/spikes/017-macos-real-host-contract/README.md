---
spike: 017
name: macos-real-host-contract
type: standard
validates: "Given macOS 14+ 的 Intel 或 Apple Silicon 真实环境，when 执行配置探针、Codex 进程识别、托盘关闭、应用激活、当前用户安装和 updater 原地替换，then Windows 上的跨平台推断得到真实验证或明确否定"
verdict: PARTIAL
related: [001, 003a, 004, 005, 006, 008]
tags: [macos, tauri, codex, process, install, updater, integration, live]
---

# Spike 017: macOS 真实宿主契约

## What This Validates

**Given** macOS 14+ 的 Intel 或 Apple Silicon 真实环境，  
**when** 构建并放置 Tauri 应用到 `~/Applications`，探测 Codex/ChatGPT 应用与进程，验证关闭隐藏、托盘恢复、LaunchServices 激活、签名门禁和两版本更新 canary，  
**then** 可以把先前仅在 Windows 上成立的 macOS 推断升级为真实宿主证据，或明确指出不成立的契约。

## Research

### 已检查的官方资料

- Tauri 2 macOS 签名与公证：`https://v2.tauri.app/distribute/sign/macos/`
- Tauri 2 updater：`https://v2.tauri.app/plugin/updater/`
- Tauri macOS application bundle：`https://v2.tauri.app/distribute/macos-application-bundle/`
- Apple `codesign` manual：`https://developer.apple.com/library/archive/technotes/tn2206/_index.html`
- Apple 公证工作流：`https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution`
- GitHub-hosted runner 规格与 macOS runner 标签：`https://docs.github.com/en/actions/reference/runners/github-hosted-runners`
- OpenAI Codex 基础配置：`https://developers.openai.com/codex/config-basic`
- OpenAI Codex macOS 应用说明：`https://developers.openai.com/codex/app`

### 方案比较

| Approach | Tool/Library | Pros | Cons | Status |
|---|---|---|---|---|
| 只在 Windows 交叉编译 macOS target | Rust target / Tauri | 能尽早发现部分 `cfg` 和依赖问题 | 缺少 Apple SDK、LaunchServices、APFS、WindowServer，不能验证运行语义 | 淘汰为最终证据 |
| GitHub-hosted macOS runner | `macos-15`、`macos-15-intel` | 可原生构建 ARM64/Intel、检查 bundle metadata 和签名 | 无真实用户桌面 Codex、GUI 人工体验和发布签名凭据 | 作为 CI 证据 |
| 真实用户 Mac 运行交互式 harness | Tauri 2 + 本 Spike UI | 能验证托盘、关闭隐藏、应用发现、`~/Applications` 和实际进程拓扑 | 需要可用 Mac，真实更新还需要签名、公证和两版本产物 | **最终方案** |
| 远程 macOS 虚拟化 | 第三方云 Mac | 可自动化真实 Darwin | 成本、凭据和 GUI 控制复杂，当前项目未配置 | 暂不采用 |

**Chosen approach:** 同一 harness 同时支持本地 Windows 契约测试、GitHub-hosted macOS 原生构建模板和真实用户 Mac 交互验证。结论严格区分三种证据等级，不把 CI 构建冒充真实桌面验证。

### 关键研究结论

1. Tauri 的 macOS updater 分发物是签名的应用更新归档，不是默认 DMG；系统代码签名/公证与 Tauri updater 内容签名是两个独立门禁。
2. `~/Applications/GPTEasy.app` 是严格当前用户安装目标；默认引导拖入 `/Applications` 的分发体验不能单独证明该要求。
3. GitHub-hosted macOS runner 可以验证原生 bundle、目标架构和 metadata，但不包含用户正在使用的 Codex 桌面进程，也不能替代托盘视觉确认。
4. 正式证据至少需要：原生主机版本/架构、bundle 路径与 ID、当前用户目录可写、相关进程拓扑、关闭隐藏事件、LaunchServices 激活、签名与 Gatekeeper 结果、更新前后 canary。

## How to Run

### 当前 Windows 主机：契约和构建门禁

```powershell
.\.planning\spikes\017-macos-real-host-contract\run.ps1
```

执行：

- Rust install-scope 与激活方式矩阵。
- Tauri release 构建，不生成 Windows installer。
- 输出 `.run/summary.json`，明确标记所有 macOS 真实项为 `not_run`。

### 真实 macOS 14+

```zsh
chmod +x .planning/spikes/017-macos-real-host-contract/run-macos.sh
./.planning/spikes/017-macos-real-host-contract/run-macos.sh
```

脚本会：

1. 原生构建 `.app`。
2. 用 `ditto` 放置到 `~/Applications/GPTEasy Spike 017.app`。
3. 读取 bundle ID 和最低系统版本。
4. 执行 `codesign --verify` 与 Gatekeeper assessment。
5. 通过 `open` 启动应用。
6. 写入 `.run/macos-host-summary.json`，然后要求在 UI 中完成四项生命周期检查。

### macOS CI

将 `macos-ci.yml.example` 复制到 `.github/workflows/` 后，可分别在 `macos-15` ARM64 与 `macos-15-intel` runner 构建原生 app。该模板只形成 CI 构建证据，不授予真实桌面验证结论。

## What to Expect

当前 Windows 执行应得到：

- Rust 2 项测试全部通过。
- 五种安装路径分类全部符合预期。
- Tauri release 构建成功。
- `run-macos.sh` 通过 Zsh 语法检查。
- `.run/summary.json` verdict 为 `partial`。

真实 Mac 完整验证还应观察：

- app 位于当前用户的 `~/Applications`。
- `LSMinimumSystemVersion` 为 `14.0`。
- 菜单栏托盘图标可见。
- 关闭窗口后进程仍运行，托盘“显示”可恢复窗口。
- 只有托盘“明确退出”才结束应用。
- 能识别真实 `Codex.app` 或 `ChatGPT.app` bundle，以及桌面宿主、bundled Codex 和独立 CLI。
- 已签名的旧版到新版更新后，应用数据目录中的 canary 保持不变。

## Observability

- 内存事件日志使用 RFC 3339 UTC 时间戳和 `lifecycle`、`tray`、`snapshot`、`canary`、`export` 分类。
- UI 的“导出证据”写入 app data 目录下的 `macos-contract-evidence.json`。
- 快照只保存路径、bundle ID、PID/PPID、进程角色和数量，不保存配置正文、完整命令行或凭据。
- `.run/summary.json` 明确记录当前证据等级，防止非 macOS 结果被误标为 native。

## Investigation Trail

1. **先确认真实宿主资源**：当前环境为 Windows x64，没有配置可发现的 macOS SSH 主机，也没有可直接调度的项目 workflow，因此不能在本次会话取得真实 Mac 证据。
2. **没有用交叉编译填补空白**：先前 Spike 已证明 Apple/Objective-C 依赖需要 Apple 工具链。本次继续把 Windows 结果限制为契约测试和本机 Tauri 构建。
3. **将安装范围变成可执行判据**：矩阵区分 `~/Applications`、`/Applications`、其他 app bundle、未打包二进制和未知路径，5/5 通过。
4. **应用发现不假定单一品牌名**：探针同时查找 `Codex.app` 与 `ChatGPT.app`，进程分类使用 `.app/Contents/MacOS`、bundled resource 路径、父子关系和 Electron `--type=` 边界。
5. **探针不自动中断真实 Codex**：只生成 `open -a Codex` 或 `open -a ChatGPT` 候选，不执行真实终止/重启。
6. **持久状态独立于 app bundle**：UI canary 写入 Tauri app data 目录，供两版本更新前后比较；这避免把“应用能替换”误当成“状态一定保留”。
7. **签名证据分层**：`codesign --verify`、Gatekeeper、Apple 公证和 Tauri updater 签名分别记录，不能互相替代。
8. **CI 不能确认 GUI**：提供 ARM64/Intel runner 模板，但托盘可见性、关闭体验、真实 Codex 拓扑和用户级 updater 仍必须在真实用户 Mac 上完成。

## Results

### Verdict: PARTIAL ⚠️

### 已验证

- 跨平台 Rust/Tauri harness 能在 Windows 完整编译，2 项单元测试通过。
- `~/Applications` 与其他安装位置的五项分类矩阵 5/5 通过。
- macOS runner 脚本通过 Zsh 语法检查。
- harness 可以在真实 Mac 上输出当前用户安装、bundle、Codex 应用/进程、生命周期事件和更新 canary 的脱敏证据。
- GitHub Actions 模板分别覆盖 Apple Silicon 与 Intel 原生 runner。

### 尚未验证

- 没有真实 macOS 14+ 主机运行结果。
- 没有真实 `Codex.app`/`ChatGPT.app` bundle ID、进程名称、父子拓扑和 `open -a` 激活证据。
- 没有真实菜单栏托盘、关闭隐藏和明确退出体验。
- 没有 Developer ID 签名、公证、Gatekeeper accepted 结果。
- 没有从已签名旧版本到新版本的 Tauri updater 原地替换和 canary 保留证据。

### 影响

macOS 仍是首版跨平台承诺的最高风险项。正式规划不能把 001、004、005、006 的 Windows 或目标编译结果升级为 macOS 已验证；需要在可用真实 Mac 或具备签名凭据的发布 runner 上重新运行本 Spike。
