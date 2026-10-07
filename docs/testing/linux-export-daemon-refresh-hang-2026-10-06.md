# Linux 导出切换后等待 daemon 刷新的回归

## 用户现象与边界

切换显示成功和权限警告后不返回；中断后供应商实际已变化，Codex 报
`Server is draining; retry after reconnecting`。用户补充切回原供应商后 Codex 恢复。

代码缺陷已通过模拟阻塞的 Codex 控制命令复现：切换提交并释放锁后，原脚本同步
执行 `codex app-server daemon restart`，没有期限，且丢弃全部输出。
这证明无限等待的脚本缺陷，不证明远端供应商兼容性或真实 daemon draining 的根因。

## 修复契约

- 配置成功和 daemon 控制结果分开报告；刷新失败不回滚配置。
- 使用 Linux GNU coreutils `timeout`，默认 75 秒后 TERM，再等 5 秒后 KILL；该窗口覆盖 Codex managed daemon 默认的优雅 draining 时间，并保留启动余量。
- 控制命令使用独立进程组和空 stdin，不读取终端、不在后台继续无期限等待。
- 未安装 Codex 不刷新；缺少 timeout 时跳过刷新而非退回无期限命令。
- 只输出稳定脱敏阶段及退出码，不输出原始 CLI 错误、凭据或供应商服务地址。
- `command_completed` 只表示控制命令成功退出，不声明 daemon ready 已被验证。

诊断前缀为 `[gpteasy daemon-refresh]`，阶段为 `stage=restart`，结果为
`started`、`command_completed`、`command_failed`、`timeout` 或
`skipped_no_timeout`。诊断输出在目标 Linux 终端提供；不增加持久化日志或秘密副本。

## 自动证据

命令：

```powershell
$env:GPTEASY_REQUIRE_SHELL_MATRIX='1'
cargo test --manifest-path src-tauri/Cargo.toml --test linux_export -- --nocapture
```

2026-10-06 在 Windows 主机经 WSL Ubuntu 的 Bash 5.2.21 / Zsh 5.9 执行：
17 passed，0 failed，0 ignored。

新增回归 `shell_switch_returns_when_daemon_restart_hangs_without_rolling_back`：

- 修复前，阻塞且忽略 TERM 的 fake Codex 使外部截止期限触发，测试失败。
- 修复后，切换按期返回，输出 timeout 诊断，当前供应商与凭据保留、共享锁已释放；正常 draining 超过旧 10 秒但在 75 秒窗口内完成时不会被误判超时。
- 成功、非零退出、无 CLI、无 timeout 分支均有结果断言。
- 控制命令收到 EOF，原始敏感错误输出不进入终端。

其余既有导出测试保持通过，包括 source 零写入、恢复点、权限、并发检查和秘密 canary。
未执行远端服务器、真实 DayWay-DS 或真实 Codex daemon 验收，不能记录为已通过。

## 交付状态

保持未发布；已有导出文件不会自动更新，需用包含修复的构建重新导出并重新 source。
