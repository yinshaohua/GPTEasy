# Issue #29 本机实现验证与当前用户安装

本记录绑定主干提交 `766e35f9ea7566e6288050775e333077e545101f`，用于记录 Issue #29 完成后的 Windows x64 开发机复核。测试和门禁均在 2026-08-16 执行，工作树在构建前保持干净。

## 自动化结果

| 检查 | 结果 |
| --- | --- |
| `npm run check` | 通过，TypeScript 与 ESLint 无错误 |
| `npm test` | 通过，2 个文件、51 个测试 |
| `cargo test --manifest-path src-tauri/Cargo.toml --all-targets -- --test-threads=1` | 通过，50 个测试通过，1 个真实环境测试忽略 |
| `npm run acceptance`（Issue #28 并行门禁） | 通过，20/20，凭据 canary 未泄漏 |
| `npm run acceptance:linux-wsl:automated`（Issue #35） | 通过，7/7 矩阵，10 类泄漏表面均通过 |
| Linux 导出目标测试 | 通过，Bash/Zsh 共 12 个测试 |
| WSL2 共享协议 feature 测试 | 通过，非破坏性 guest 用例按平台前置条件忽略 |

自动化 Linux/WSL 门禁在本机报告中如实标记了未执行的独立 GNU/Linux、真实 Running/Stopped guest 和真实 Codex 项；这些项目由 [Issue #31 真实 UAT](linux-wsl-real-uat.md) 的 Windows x64 + WSL2 与独立 Ubuntu 证据覆盖，未把未执行项冒充通过。

## 构建与安装

使用 `npx --no-install tauri build --target x86_64-pc-windows-msvc` 构建当前用户 NSIS 包：

`src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/GPTEasy_0.1.0_x64-setup.exe`

- 安装包大小：3,449,444 bytes
- SHA-256：`48ab400d312f56ddf168754d7242d1ef9af018ddd3ccea6c269806ee74edce07`
- Authenticode：`NotSigned`（开发包，正式发布仍需签名）
- 安装命令：NSIS `/S`
- 安装退出码：`0`
- 安装范围：当前用户
- 安装目录：`C:\Users\yinsh\AppData\Local\GPTEasy`
- 开始菜单快捷方式指向安装目录内的 `gpteasy.exe`

安装后检查确认文件版本与产品版本均为 `0.1.0`，从安装目录启动进程成功并保持运行；随后只结束了本次验证启动的进程。未修改用户 Codex 配置，也未执行任何真实供应商切换。

开发安装包为未签名构建，且 Tauri bundle metadata patch 会使安装后的可执行文件哈希不与构建目录中的 release 文件直接相等；安装验证以安装器退出码、当前用户路径、版本信息、快捷方式目标和实际启动结果为准。
