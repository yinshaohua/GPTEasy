# GPTEasy

[![Latest Release](https://img.shields.io/github/v/release/yinshaohua/GPTEasy?display_name=tag&label=release)](https://github.com/yinshaohua/GPTEasy/releases/latest)
![Windows x64](https://img.shields.io/badge/platform-Windows%20x64-0078D4)
[![MIT License](https://img.shields.io/badge/license-MIT-2ea44f)](LICENSE)

GPTEasy 是一款面向 Windows 用户的 Codex 配置与会话管理工具。主要特点：简单好用，轻巧干净。既让一般用户上手容易，又让 WSL2 甚至 Linux 上工作者都能使用。

## 下载与安装

国内用户可从 [Gitee Releases](https://gitee.com/ericshaohua/gpteasy-releases/releases) 下载最新的 Windows x64 当前用户安装包；Gitee 附件使用 `.exe.bin` 后缀，手工下载后须删除末尾 `.bin` 再运行。[GitHub Releases](https://github.com/yinshaohua/GPTEasy/releases/latest) 继续提供标准 `.exe` 文件名、源码、发布说明和备用下载。两个平台分发同一次正式构建的安装包，安装不需要管理员权限。

`v1.1.0` 是首个内置应用更新信任根的版本；`v1.4.0` 将国内更新源切换到 Gitee。仍在使用 `v1.3.0` 或更早版本的用户须先手工安装 `v1.4.0` 或更高版本，后续正式更新才会由应用自动检查和下载；签名验证通过后仍需用户明确选择“重启并更新”。

系统要求：

- Windows 10 22H2（build 19045）或更高版本，x64
- Codex CLI `0.147.0` 或更高版本

下载后，请同时获取同一 Release 中的 `SHA256SUMS.txt`，并在 PowerShell 中校验安装包：

```powershell
Get-FileHash .\GPTEasy_*_x64-setup.exe -Algorithm SHA256
```

安装包目前没有 Authenticode 代码签名，Windows Defender SmartScreen 可能显示“Windows 已保护你的电脑”。请确认安装包来自本仓库的 Release，并且 SHA-256 与 `SHA256SUMS.txt` 一致；确认后可通过“更多信息”选择“仍要运行”。

## 推荐供应商

<table>
  <tr>
    <td width="180" align="center" valign="middle">
      <a href="https://dayway.site/">
        <img src="https://raw.githubusercontent.com/yinshaohua/GPTEasy/main/images/dayway-512x512.png" alt="DayWay" width="150">
      </a>
    </td>
    <td valign="middle">
      <strong><a href="https://dayway.site/">DayWay：GPTEasy 唯一赞助商</a></strong><br>
      提供 GPT 模型，便宜，稳定，有独立生图台，能开国内发票。
    </td>
  </tr>
</table>

## 主要功能

- **供应商管理**：新增、编辑、排序和删除 OpenAI 兼容 API 供应商，发现可用模型，并在保存前实际验证连接。
- **Codex 环境切换**：切换当前 Windows 用户使用的供应商，支持 OpenAI 登录模式、外部配置接管和最近一次配置恢复。
- **WSL2 与 Linux**：为选定的 WSL2 发行版应用供应商，或导出可用于 Bash、Zsh 的 Linux 脚本。
- **会话管理**：搜索和筛选 Codex 会话，查看详情，导出 Markdown，归档、取消归档或永久删除会话。
- **本地运行**：不需要产品账户，不使用云存储、遥测或供应商同步；应用状态保存在当前用户的本机。
- **托盘驻留**：关闭设置窗口后继续在系统托盘运行，需要通过托盘菜单明确退出。

会话管理只通过 Codex 官方 App Server 访问用户交互会话，不用于在 GPTEasy 内继续对话。

## 界面预览

![GPTEasy 主界面和供应商管理界面](https://raw.githubusercontent.com/yinshaohua/GPTEasy/main/images/gpteasy-main.png)

![GPTEasy WSL2 配置界面](https://raw.githubusercontent.com/yinshaohua/GPTEasy/main/images/gpteasy-wsl2.png)

![GPTEasy 生成 Linux 配置脚本界面](https://raw.githubusercontent.com/yinshaohua/GPTEasy/main/images/gpteasy-linux.png)

## 基本用法

1. 在“供应商管理”中添加服务地址和 API Key，发现模型并完成验证。
2. 保存供应商后将其设为“当前使用”，或者切换到“OpenAI 登录模式”。
3. 如果 Codex 正在运行，请自行重启相关桌面客户端或 CLI，使新配置生效。

GPTEasy 只负责配置和管理，不会主动启动、关闭或重启用户的 Codex 进程。

## 本地开发

项目使用 Tauri 2、Rust、React 和 TypeScript。开发环境需要 Node.js、npm、Rust `1.85` 或更高版本，以及 Tauri 在 Windows 上所需的系统依赖。

```powershell
npm ci
npm run tauri dev
```

提交改动前运行：

```powershell
npm run check
npm test
npm run test:layout
cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=1
```

构建 Windows x64 候选安装包：

```powershell
npm run candidate:windows
```

应用更新的 Gitee 分发信任根、带密码 updater 密钥、冒烟工作流与版本准备流程参见 [`docs/release/gitee-distribution.md`](docs/release/gitee-distribution.md)。首次设置在 Git Bash 或 WSL2 中运行 `bash scripts/setup-gitee-distribution.sh`；私钥和密码不得进入仓库、日志或发布附件。

候选构建要求在干净的 `main` 工作树中运行。

## 参与贡献

欢迎通过 [GitHub Issues](https://github.com/yinshaohua/GPTEasy/issues) 报告问题、提出建议，也欢迎提交 Pull Request。

重大改动请先创建 Issue 讨论。提交 PR 前，请运行上面的检查和测试，并在说明中写清改动目的、用户可见行为和验证结果。

## 项目文档

- [领域上下文](CONTEXT.md)
- [架构决策记录](docs/adr/README.md)
- [实现与外部合同证据](docs/evidence/README.md)

## 许可证

本项目采用 [MIT License](LICENSE)。
