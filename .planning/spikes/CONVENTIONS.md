# Spike Conventions

Patterns and stack choices established across spike sessions. New spikes follow these unless the question requires otherwise.

## Stack

- 桌面交互实验使用 Rust 2021 与 Tauri 2。
- Windows 自动化和场景编排使用 PowerShell 7；核心判断、配置处理和网络验证使用 Rust。
- 应用内部状态和跨资源恢复实验使用 SQLite，并由 Rust 后端独占访问。
- Linux 独立导出物分别面向 Bash 4+ 与 Zsh 5+；核心模板共享，shell 专属层只处理交互和选项差异。
- 每个 Spike 保持独立可运行，不依赖项目尚未建立的正式应用代码。
- 需要外部协议时优先使用本地 deterministic mock；需要进程行为时使用路径受限的 fixture。

## Structure

- 每个实验位于 `.planning/spikes/NNN-name/`，包含独立 `README.md`、运行脚本和源代码。
- 运行产物统一写入 Spike 自己的 `.run/` 并加入 `.gitignore`。
- Rust 构建产物 `target/`、Node `node_modules/` 和锁文件不进入 Spike 提交。
- 自验证实验输出 `.run/summary.json`；长流程按场景写独立 JSON/JSONL 证据。
- Tauri Spike 使用 `web/` 保存无构建器静态前端，`src-tauri/` 保存 Rust 应用和配置。
- 临时真实凭据统一放在 `.planning/spikes/.secrets/`，该目录必须由 `.planning/spikes/.gitignore` 忽略；运行前使用 `git check-ignore` 再验证。

## Patterns

- 不读取、输出或提交真实供应商 Key；测试使用明显的假凭据。
- Key 不放在外部进程命令行。正式 Tauri command 在同一 Rust 进程内以内存参数传递。
- 日志只保留事件类型、耗时、状态、路径和布尔判据，不保存完整请求、模型输出、配置正文或进程完整命令行。
- 远程供应商地址只允许 HTTPS；HTTP 只用于 `localhost`、`127.0.0.1` 和 `[::1]` 回环测试。
- 供应商验证固定覆盖 URL 策略、模型发现、Responses SSE/工具调用和工具结果回传；地址、Key 或默认模型变化后全量重跑。
- 修改配置时先解析和校验，再备份、写同目录临时文件、同步、检查并发变化并原子替换。
- Windows 已有文件替换使用 `ReplaceFileW`；macOS/Unix 使用同文件系统 rename 并同步父目录。
- 首次接管使用结构化 TOML 迁移；管理区块建立后使用 dotted-key 区块替换，标记损坏或重复时停止。
- 首次接管必须在同一事务中移除旧受管键、把无直属值的显式 `model_providers` 父表转为 implicit、建立唯一管理区块并重新解析；不能简单串联结构化写入与区块插入。
- 不可变供应商 ID 写在管理区块注释 `# GPTEasy provider-id:` 中，不依赖 Codex 对未知 provider 字段的容忍。
- 当前用户默认 `~/.codex/config.toml` 是受管写入目标；若发现无法匹配的 provider 或外部配置层，展示实际状态而不自动争夺。
- 最终有效 Codex 状态通过 app-server `config/read(cwd, includeLayers=true)` 读取，只保留 model/provider 和字段来源摘要，不保存可能包含凭据的完整响应。
- SQLite、Codex 配置和进程之间使用可恢复 Saga；持久化 `prepared` 意图和旧/新配置哈希，恢复时按旧哈希回滚、新哈希前滚、未知哈希转外部配置。
- 所有跨平台结论区分“当前平台实测”“目标编译检查”和“待真实机器验证”，不把交叉编译失败误判为业务不可行。
- 进程检测组合名称、可执行路径、父子关系和 Electron `--type=` 参数；不能只按进程名分类。
- 自动重启只应用于可恢复的桌面应用；CLI 的 TTY、cwd、stdin 和会话不可可靠恢复，因此要求人工重启。
- WSL2 检测只使用全部/运行发行版列表和当前用户 Lxss 注册表，不执行发行版内命令；数据库身份使用注册 GUID，显示名称重复或无法解歧时停止管理。
- 已停止 WSL2 环境只在用户明确切换时临时启动，并在成功、失败和配置损坏路径都恢复原停止状态。
- Bash/Zsh 导出脚本 source 时零写入；无管理区块但已有供应商键时停止并要求结构化迁移，后续只替换区块。
- Linux 备份使用 UTC 纳秒时间戳文件名并按文件名逆序裁剪，避免依赖 DrvFS 等挂载文件系统的 mtime 排序。
- blocking `reqwest` 的 SSE 首事件、空闲和 overall 截止时间通过 reader thread 与 channel timeout 区分；429 单独分类为 `rate_limit`。
- updater 检查与下载/安装必须是两个独立用户操作，不能在检查成功后自动安装。

## Tools & Libraries

- `tauri` 2.11.5、`@tauri-apps/cli` 2.11.4：Windows 托盘和 NSIS 构建已验证。
- `tauri-plugin-updater` 2.10.1：Windows updater 签名产物已验证。
- `sysinfo` 0.39.6：Windows 真实进程和 fixture 拓扑已验证。
- `toml_edit` 0.23：注释、未知字段和局部格式保留已验证。
- `windows-sys` 0.61：`ReplaceFileW` 原子替换已验证。
- `reqwest` 0.12：Responses SSE 与工具调用验证器已验证。
- `rusqlite` 0.40（bundled SQLite）：切换 Saga、WAL、`BEGIN IMMEDIATE` 和崩溃恢复矩阵已验证。
- `serde` / `serde_json` 1.x：所有实验统一使用的结构化输入、输出和证据格式。
- `codex-cli` 0.146.0 app-server：`config/read` 的 user/project/sessionFlags 层和字段 origins 已验证。
- GNU Bash 4.4.0、Zsh 5.9：独立 Linux 切换函数的 12 项对照矩阵已验证。
