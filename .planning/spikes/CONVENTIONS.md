# Spike Conventions

Patterns and stack choices established across spike sessions. New spikes follow these unless the question requires otherwise.

## Stack

- 桌面交互实验使用 Rust 2021 与 Tauri 2。
- Windows 自动化和场景编排使用 PowerShell 7；核心判断、配置处理和网络验证使用 Rust。
- 每个 Spike 保持独立可运行，不依赖项目尚未建立的正式应用代码。
- 需要外部协议时优先使用本地 deterministic mock；需要进程行为时使用路径受限的 fixture。

## Structure

- 每个实验位于 `.planning/spikes/NNN-name/`，包含独立 `README.md`、运行脚本和源代码。
- 运行产物统一写入 Spike 自己的 `.run/` 并加入 `.gitignore`。
- Rust 构建产物 `target/`、Node `node_modules/` 和锁文件不进入 Spike 提交。
- 自验证实验输出 `.run/summary.json`；长流程按场景写独立 JSON/JSONL 证据。
- Tauri Spike 使用 `web/` 保存无构建器静态前端，`src-tauri/` 保存 Rust 应用和配置。

## Patterns

- 不读取、输出或提交真实供应商 Key；测试使用明显的假凭据。
- Key 不放在外部进程命令行。正式 Tauri command 在同一 Rust 进程内以内存参数传递。
- 日志只保留事件类型、耗时、状态、路径和布尔判据，不保存完整请求、模型输出、配置正文或进程完整命令行。
- 修改配置时先解析和校验，再备份、写同目录临时文件、同步、检查并发变化并原子替换。
- Windows 已有文件替换使用 `ReplaceFileW`；macOS/Unix 使用同文件系统 rename 并同步父目录。
- 首次接管使用结构化 TOML 迁移；管理区块建立后使用 dotted-key 区块替换，标记损坏或重复时停止。
- 所有跨平台结论区分“当前平台实测”“目标编译检查”和“待真实机器验证”，不把交叉编译失败误判为业务不可行。
- 进程检测组合名称、可执行路径、父子关系和 Electron `--type=` 参数；不能只按进程名分类。
- 自动重启只应用于可恢复的桌面应用；CLI 的 TTY、cwd、stdin 和会话不可可靠恢复，因此要求人工重启。
- updater 检查与下载/安装必须是两个独立用户操作，不能在检查成功后自动安装。

## Tools & Libraries

- `tauri` 2.11.5、`@tauri-apps/cli` 2.11.4：Windows 托盘和 NSIS 构建已验证。
- `tauri-plugin-updater` 2.10.1：Windows updater 签名产物已验证。
- `sysinfo` 0.39.6：Windows 真实进程和 fixture 拓扑已验证。
- `toml_edit` 0.23：注释、未知字段和局部格式保留已验证。
- `windows-sys` 0.61：`ReplaceFileW` 原子替换已验证。
- `reqwest` 0.12：Responses SSE 与工具调用验证器已验证。
- `serde` / `serde_json` 1.x：所有实验统一使用的结构化输入、输出和证据格式。
