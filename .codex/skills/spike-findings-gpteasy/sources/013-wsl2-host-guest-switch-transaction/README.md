---
spike: 013
name: wsl2-host-guest-switch-transaction
type: standard
validates: "Given Windows 上运行中或已停止的真实 WSL2 用户发行版，when GPTEasy 从宿主机向默认用户安全传递供应商并执行切换，then 凭据不进入命令行或日志、配置协议成立且所有路径恢复原生命周期状态"
verdict: VALIDATED
related: [006, 007, 008, 009, 010a, 010b]
tags: [wsl2, windows, linux, credential, config, lifecycle, integration]
---

# Spike 013: WSL2 宿主到客体切换事务

## What This Validates

**Given** Windows 上运行中或已停止的真实 WSL2 用户发行版，  
**when** Windows Rust 后端读取默认用户配置、在内存中完成结构化迁移，并通过 `wsl.exe` stdin 把完整候选文件交给 Linux 客体原子写入器，  
**then** API Key 不进入 Windows/Linux 命令行或证据日志，配置与备份协议成立，并且原来停止的发行版在成功、失败、损坏和并发路径都恢复停止，原来运行的发行版保持运行。

## Research

### 已检查的官方资料

- Microsoft WSL 基础命令：`https://learn.microsoft.com/windows/wsl/basic-commands`
- Microsoft WSL 文件系统：`https://learn.microsoft.com/windows/wsl/filesystems`
- Microsoft WSL 1/2 比较：`https://learn.microsoft.com/windows/wsl/compare-versions`
- Microsoft WSL 导入发行版：`https://learn.microsoft.com/windows/wsl/use-custom-distro`
- Ubuntu Base 24.04.3 发布目录：`https://cdimage.ubuntu.com/ubuntu-base/releases/24.04.3/release/`
- Spike 006 首次接管事务、009 WSL 生命周期、010a/010b Linux 写入协议

### 方案比较

| Approach | Tool/Library | Pros | Cons | Status |
|---|---|---|---|---|
| 把 Key 作为 `wsl.exe` 参数传入 | `wsl.exe --exec helper KEY` | 实现最简单 | Key 出现在 Windows 和 Linux 进程命令行，可被进程检查工具读取 | 淘汰 |
| Windows 直接写 `\\wsl.localhost\发行版\...` | Windows Rust 文件 API | Key 保持在同一进程，能复用 `toml_edit` | 9P/UNC 上的 Linux 权限、rename、同步和默认用户路径语义不够直接；访问也会启动停止发行版 | 不采用 |
| 在 shell 中重新实现完整 TOML 迁移 | Bash/Zsh/POSIX shell | 客体内部完成全部操作 | 通用 TOML、安全字符串转义和首次迁移过重，重复 Spike 006，容易损坏未知配置 | 淘汰 |
| Windows 内存渲染 + stdin 传完整候选 + 客体原子写入 | Rust `toml_edit` + `wsl.exe` stdin + POSIX helper | 结构化迁移复用 006；Key 不进参数；Linux 自己处理权限、备份、rename 和 sync | 需要一个无凭据 helper；宿主与客体各承担一半协议 | **采用** |

**Chosen approach:** Windows Rust 后端负责解析和渲染候选配置；无凭据的固定客体 helper 只从 stdin 读取候选、检查原哈希、备份、同步和替换。所有 `wsl.exe` 参数只包含发行版名、固定 helper 路径、SHA-256 和非敏感测试模式。

### 事务边界

1. 切换前记录发行版是否运行。
2. 只有用户明确切换后才允许读取默认用户配置；这一步会启动已停止发行版。
3. Rust 在 Windows 内存中完成 Spike 006 的首次结构化迁移或后续管理区块替换。
4. 客体 helper 从 stdin 接收完整候选文件。
5. helper 比较旧配置 SHA-256，生成同目录临时文件，备份并保留权限，再次比较哈希后 `mv`。
6. 原状态为 Stopped 时，无论成功还是错误，都调用 `wsl --terminate`；原状态为 Running 时不终止。
7. 写入完成不主动终止 WSL 内 Codex，只由上层标记待人工重启。

## How to Run

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\013-wsl2-host-guest-switch-transaction\run.ps1
```

运行脚本会：

1. 通过 `127.0.0.1:7897` 下载官方 Ubuntu Base 24.04.3 amd64 rootfs。
2. 用官方 `SHA256SUMS` 校验下载内容。
3. 只使用固定名称 `GPTEasy-Spike-013` 导入一次性 WSL2 发行版。
4. 创建默认用户 `gpteasy` 并安装不含凭据的固定 writer。
5. 执行 10 项真实 WSL 矩阵。
6. 注销一次性发行版，并验证运行前后的发行版集合与运行集合完全恢复。
7. 输出 `.run/evidence/summary.json` 与 `.run/evidence/lifecycle.json`。

默认会注销测试发行版。调试时可使用：

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\013-wsl2-host-guest-switch-transaction\run.ps1 -KeepDistro
```

代码只接受精确的 `GPTEasy-Spike-013` 名称；不会对其他发行版执行写入或注销。

## What to Expect

`summary.json` 应为 10/10：

- 取消不启动已停止发行版。
- 停止发行版成功切换后恢复停止。
- 假 Key 不出现在 Windows `wsl.exe`、Linux helper 或其父进程命令行。
- 后续切换保持管理区块外字节不变。
- 替换前注入失败保持旧配置并恢复停止。
- 管理标记损坏在客体写入前停止。
- 客体并发编辑触发哈希冲突，不被覆盖。
- 最近只保留五份备份，配置权限保持 `0600`。
- 只修改默认用户配置，root 用户配置不变。
- 原来运行的发行版在切换后仍运行。

此外：

- `secret_in_artifacts = false`
- `host_lifecycle_restored = true`
- 测试前后真实 `Ubuntu` 的运行状态不变
- 一次性发行版在结束后不存在

## Observability

- 客体 writer 只输出状态、阶段、备份数量、权限和命令行泄漏布尔值。
- 不输出候选配置、API Key、base URL、模型或备份正文。
- Rust 摘要只记录候选字节数、退出码、原/终生命周期状态和脱敏 writer JSON。
- `.run/evidence/` 会扫描固定假 Key 原始字节；发现即拒绝 `VALIDATED`。
- rootfs cache 和一次性 VHD 不属于诊断证据目录；VHD 在注销发行版时删除，其中的受管配置是测试目标本身，而不是日志副本。
- `lifecycle.json` 保存测试前后的发行版集合、运行集合和 Ubuntu Base SHA-256。

## Investigation Trail

1. **直接传参被排除**：供应商 Key 不能作为 `wsl.exe` 或 shell 参数。最终所有敏感候选字节只经 stdin 进入 guest writer。
2. **UNC 写入不是首选**：Windows 侧直接写 `\\wsl.localhost` 会把 Linux 权限和同步语义交给共享文件协议。本实验改为由客体在 ext4 中完成临时文件、权限、备份和 rename。
3. **结构化迁移仍在 Rust 中完成**：客体 shell 不实现 TOML。首次接管移除旧受管键、保留未知字段和旧 provider，并生成唯一 dotted-key 管理区块。
4. **固定 helper 不含秘密**：helper 可重复安装；每次切换的配置正文通过 stdin 发送，不创建包含全部供应商的永久脚本。
5. **真实停止生命周期已验证**：不是 fixture 状态机。本次导入真实 Ubuntu Base WSL2，逐次从 Stopped 启动、写入并 `--terminate`。
6. **失败路径同样恢复**：替换前注入失败、损坏管理标记和并发修改都以发行版停止、旧配置不被错误覆盖结束。
7. **两侧命令行都做了门禁**：Windows 使用 `sysinfo` 扫描进程参数；Linux writer 检查自身与父进程 `/proc/*/cmdline`，三项均未发现固定假 Key。
8. **并发检测需要在客体最终确认**：仅在 Windows 渲染前比较内容不够。helper 在生成候选和备份后再次比较 SHA-256，真实并发追加触发 `pre_replace` 冲突。
9. **运行状态是操作级 finally**：停止发行版在每个 transaction 返回后恢复；运行中的 keeper 场景证明操作不会把原来运行的发行版终止。
10. **证据扫描边界被修正**：第一轮把正在运行的测试 VHD 也当作“诊断产物”，必然发现目标配置中的假 Key。最终只扫描可导出的 `.run/evidence/`，同时在注销后验证 VHD 和测试发行版已移除。

## Results

### Verdict: VALIDATED ✓

真实 WSL2 一次性 Ubuntu Base 发行版中的 10 个场景全部通过，运行前后的真实发行版集合和运行集合完全恢复。

### 已验证

- Windows Rust 可以在内存中完成 006 的首次接管和后续区块替换，再把候选通过 stdin 交给 WSL。
- API Key 无需进入 Windows 或 Linux 进程参数。
- 停止发行版只在明确切换后启动，并在成功、故障、损坏和并发冲突路径恢复停止。
- 运行中的发行版不会被切换操作终止。
- Linux ext4 内配置保持 `0600`，备份保留最近五份。
- 只修改发行版默认用户的 `~/.codex/config.toml`。
- 并发客体编辑不会被旧候选覆盖。
- 诊断证据和最终摘要不包含假 Key。

### 限制

- 使用一次性 Ubuntu Base 24.04.3 amd64 发行版；尚未覆盖 Windows ARM64/WSL ARM64。
- 没有在用户长期使用的真实 Ubuntu 中修改配置，以避免触碰真实 Codex 环境。
- 没有启动真实 WSL Codex 进程；切换后只验证“不终止发行版”，正式 UI 仍应提示用户人工重启 Codex。
- 009 发现的重复 `DistributionName` 到注册 GUID/命令目标解歧问题仍未解决；无法唯一定位时必须在进入本事务前停止。
- guest writer 依赖 Ubuntu/GNU 用户空间的 `sha256sum`、`awk`、`find`、`sort`、`date %N`、`stat`、`sync` 和 `mv`。
- shell `sync -f` 与 `mv` 提供实际 Linux 文件系统语义，但本实验不是断电一致性认证。
