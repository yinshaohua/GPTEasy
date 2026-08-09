---
spike: 010a
name: linux-switch-functions-bash
type: comparison
validates: "Given 只有 Bash 4+ 且无额外运行时的 Linux 环境，when source 导出脚本并交互选择、取消或重复切换供应商，then 只有明确选择后才安全替换管理区块、备份并保留其他配置"
verdict: VALIDATED
related: [006, 009, 010b]
tags: [bash, linux, shell, managed-block, backup, comparison]
---

# Spike 010a: Bash 独立供应商切换函数

## What This Validates

**Given** 只有 Bash 4+ 和常见 GNU/Linux 基础命令、没有 Python、Node.js、jq 或 GPTEasy 可执行文件的 Linux 环境，  
**when** 用户 source 导出的全供应商脚本，并选择、取消、重复切换、恢复备份或遇到损坏管理区块，  
**then** source 本身不修改配置，只有明确选择后才安全替换管理区块，保留区块外内容和文件权限，并只保留最近五份备份。

## Research

### 已检查的资料

- GNU Bash Manual：`https://www.gnu.org/software/bash/manual/bash.html`
- GNU Coreutils `mktemp`：`https://www.gnu.org/software/coreutils/manual/html_node/mktemp-invocation.html`
- POSIX `mv`：`https://pubs.opengroup.org/onlinepubs/9799919799/utilities/mv.html`
- GNU Bash 4.4 源码：`https://ftp.gnu.org/gnu/bash/bash-4.4.tar.gz`

### 选定实现

- 导出时把全部已验证供应商及其明文凭据预渲染成带引号的 TOML heredoc。
- 公共入口为 `gpteasy_select_provider`、`gpteasy_current_provider` 和 `gpteasy_restore_latest`。
- `GPTEASY_CODEX_HOME` 只作为测试覆盖；正式默认目标为 `$HOME/.codex/config.toml`。
- 使用精确整行标记扫描，不尝试写通用 TOML 解析器。
- 无区块且没有受管键冲突时可以建立区块；发现已有顶层 `model`、`model_provider` 或 GPTEasy provider 键时停止并要求结构化迁移。
- 同目录 `mktemp`、候选文件、原文件 `cksum` 并发检查和 `mv` 完成替换。
- 备份名使用 UTC 纳秒时间戳，按文件名逆序保留最近五份。

## How to Run

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\010-a-linux-switch-functions-bash\run.ps1
```

若本地 `.run/` 中没有 Bash 4.4，脚本会下载 GNU Bash 4.4 源码并只安装到 Spike 的 `.run/bash-4.4-install/`，不会修改 WSL 系统。

## What to Expect

GNU Bash `4.4.0(1)-release` 下 `.run/summary.json` 为 12/12：

- source 零副作用。
- 取消不修改文件。
- 首次选择建立管理区块。
- 保留原文件权限。
- 后续切换区块外字节一致。
- 引号、反斜杠、美元符号和中文凭据预转义正确。
- 能读取当前不可变供应商 ID。
- 损坏标记安全停止。
- 无区块但已有受管键时要求迁移。
- 只保留最近五份备份。
- 最新备份原子恢复且字节一致。
- 脚本不依赖 Python、Node、jq、Perl 或 Ruby。

## Observability

- 测试工作区位于 WSL `/tmp` 的带空格路径，不触碰真实 `$HOME/.codex`。
- 仓库只保存 `.run/summary.json` 运行产物，且 `.run/` 被忽略。
- 测试供应商和 Key 都是明显的假值。

## Investigation Trail

1. **source 必须只定义函数和常量**：首次 hash 检查证明 source 前后配置完全不变。
2. **预渲染比运行时转义可靠**：GPTEasy 导出时已经拥有结构化供应商数据，可以生成 quoted heredoc，目标脚本无需实现 TOML 字符串转义器。
3. **无 TOML 解析器意味着保守接管**：脚本只能安全识别精确管理区块和常见根级冲突。已有外部受管键时停止，不能像 Rust `toml_edit` 一样结构化迁移。
4. **同目录临时文件是必要条件**：只有同文件系统 `mv` 才能利用 rename 替换语义。
5. **并发检查可用 `cksum` 实现**：生成候选和备份后再次比较原文件，发现外部变化就不覆盖。
6. **按 mtime 选备份在挂载文件系统上不稳定**：最初使用 `ls -t` 时，DrvFS 的时间行为导致恢复选错版本。改为按包含 UTC 纳秒时间戳的文件名逆序排序后稳定。
7. **权限测试必须在 Linux 文件系统执行**：DrvFS 默认显示 `0777`，不能用于验证 `chmod --reference`。最终矩阵使用 WSL `/tmp` ext4。
8. **最低版本实际验证**：不是只在当前 Bash 5.2 上推断兼容；Spike 在隔离安装的 Bash 4.4.0 上执行全部 12 个场景。
9. **备份也是明文敏感文件**：导出脚本、当前配置和备份都包含 API Key，必须在导出和复制位置提示风险。

## Results

### Verdict: VALIDATED ✓

Bash 4.4.0 最低目标上的 12 个场景全部通过。独立脚本可以在没有额外运行时的 Linux 环境中长期切换供应商，并保持取消、损坏停止、区块外保留、权限、备份和恢复语义。

### 限制

- 使用 `awk`、`cksum`、`mktemp`、`sort`、`date %N`、`chmod --reference` 等常见 GNU/Linux 命令，不面向 BusyBox-only 或非 GNU 用户空间。
- shell 层无法提供与 Rust `sync_all` 相同的断电持久性保证。
- 无管理区块且已有供应商键时必须停止，首次结构化迁移仍需要 GPTEasy/Rust。
- 脚本包含全部供应商明文凭据，不能当作无敏感信息的普通配置文件。
