# #69 独立 Linux 离线模型目录实施归档

实施日期：2026-10-03。关联父任务 #67；前置 #68 已交付有效快照与共享生成器。

Bash/Zsh 导出携带每个有效供应商的完整离线目录及冻结绑定。source 只建立定义；明确切换以隔离原生 CLI 的 model/list 返回确认 schema 能力，随后在共享锁内创建恢复点、提交不可变目录、原子切换唯一目录指针并复读。restore 验证旧目录并恢复完整旧配置，可跨导出及供应商集合变化。未使用 Python/Node/jq，也未控制用户后台服务。

目录绑定包含来源、供应商、默认模型、关键组合指纹、字节摘要及生成协议/策略；模型集合由共享生成器序列化为完整载荷并被摘要覆盖。显示名称可维护，关键组合变化则重新验证和导出。受管目录私有且无凭据，API Key 继续使用独立命令式凭据，auth.json 保持原字节。目录不自动清理，以保留当前配置与恢复点引用；配置提交后复核失败保留现场，不盲目回滚。

统一待刷新提示绑定当前 uid/home，脱敏问题日志记录能力、恢复点、目录、配置和复读阶段。补充回归发现并修复 Zsh 特殊变量 path 影响 PATH、失败时锁释放、恢复确认竞态以及系统时钟回拨造成的恢复点排序问题。递增恢复点序号只是锁内排序，不是持久化 Saga 或已应用证据。状态读取继续只读兼容 schema v1；schema v2 的当前状态同时比较目录内容摘要，等价导出来源不会误报更新，目录内容变化会显示待刷新。

## 验证

- 导出与公开 shell 黑盒测试覆盖快照缺失/损坏/绑定失效、完整模型载荷、陈旧关键组合、协议/策略/来源、显示名维护、source 零外部工具调用、无目录能力/Windows 入口、真实 Bash/Zsh、特殊路径和文件身份。
- 故障测试覆盖目录提交前失败、目录已提交而配置替换失败、目录提交后篡改、配置提交后同步失败、恢复确认期间备份变化、受管目录缺失/篡改/symlink/hardlink，以及跨导出恢复与原配置不存在。
- 真实原生 Linux Codex 0.160.0 在 WSL 内的隔离 home 通过 config/read、model/list，验证默认与非默认模型都可见、四档 low/medium/high/xhigh 和默认 high。测试只使用假凭据，无上游请求。
- `npm run check`、`npm test`（113 项前端测试、29 项发布脚本测试）、`npm run build` 已通过。
- `GPTEASY_REQUIRE_SHELL_MATRIX=1 cargo test --manifest-path src-tauri/Cargo.toml --test linux_export` 的 23 项测试已通过，实际执行 WSL Ubuntu 中的 Bash/Zsh；包括两个提交点的进程强制终止、目录权限、时钟回拨、控制字符 CODEX_HOME、恢复复读失败分类、schema v1 只读兼容和目录内容差异。
- `GPTEASY_REQUIRE_SHELL_MATRIX=1 cargo test --manifest-path src-tauri/Cargo.toml` 完整 Rust 测试通过；需要真实用户环境的 opt-in 测试仍保持 ignored，未视为通过。
- `pwsh -NoProfile -File scripts/run-linux-wsl-acceptance-gate.ps1 -Mode Automated -WslDistribution Ubuntu` 通过，凭据泄漏扫描为 false。开发证据位于 `src-tauri/target/acceptance/linux-wsl/3ae115e032f1426089bdae86196048f7/evidence.json`；运行开始于本地提交前，报告记录基线 `35d9606bfd27971e3ec55af8bff48fbc0b3244de`，实际验证本轮工作区，不作为干净提交或正式发布证据。该历史门禁报告仍使用 #35 的 schema v1 coverage 标签；本轮 schema v2 的实际覆盖以 23 项导出测试为准。
- 设置 `GPTEASY_RUN_REAL_CODEX_ACCEPTANCE=1`、指定原生 `GPTEASY_REAL_CODEX` 并运行 `cargo test --manifest-path src-tauri/Cargo.toml --test real_linux_codex -- --ignored --nocapture`：1 项通过，显式检查 config/read 和 model/list。
- Standards / Spec 最终复核：已关闭恢复提交后缺少固定 `restore_readback` 失败阶段、控制字符换行校验不完整、协议常量重复、schema v1 状态读取误依赖 v2 目录字段、只读状态未识别目录快照差异等发现；新增回归测试覆盖这些边界。

本轮真实 shell 使用 WSL Ubuntu 中的 Bash 5.2.21 和 Zsh 5.9；不将其记作独立 GNU/Linux 宿主、真实交互菜单或 Running/Stopped WSL 深度验收。Bash 4.4 单独矩阵尚未运行。WSL 桌面双向 schema v2 共同管理与共享后台服务确认刷新由后续任务负责，#67 保持打开，ADR-0054 仍为 proposed。未打 tag、未发布 GitHub/Gitee Release。

协议取舍见 [ADR-0055](../adr/0055-standalone-linux-offline-model-catalog.md)，用户操作见 [Linux 离线脚本说明](../linux-offline-catalog.md)。
