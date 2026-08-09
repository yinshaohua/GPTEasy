---
spike: 010b
name: linux-switch-functions-zsh
type: comparison
validates: "Given 只有 Zsh 5+ 且无额外运行时的 Linux 环境，when source 导出脚本并交互选择、取消或重复切换供应商，then 只有明确选择后才安全替换管理区块、备份并保留其他配置"
verdict: VALIDATED
related: [006, 009, 010a]
tags: [zsh, linux, shell, managed-block, backup, comparison]
---

# Spike 010b: Zsh 独立供应商切换函数

## What This Validates

**Given** 只有 Zsh 5+ 和常见 GNU/Linux 基础命令、没有 Python、Node.js、jq 或 GPTEasy 可执行文件的 Linux 环境，  
**when** 用户 source 导出的全供应商脚本，并选择、取消、重复切换、恢复备份或遇到损坏管理区块，  
**then** source 本身不修改配置，只有明确选择后才安全替换管理区块，保留区块外内容和文件权限，并只保留最近五份备份。

## Research

### 已检查的资料

- Zsh Documentation：`https://zsh.sourceforge.io/Doc/Release/`
- Zsh 参数和数组：`https://zsh.sourceforge.io/Doc/Release/Parameters.html`
- Zsh builtins：`https://zsh.sourceforge.io/Doc/Release/Shell-Builtin-Commands.html`
- Ubuntu `zsh` / `zsh-common` 5.9 包，仅解压到 Spike `.run/`

### 与 Bash 变体的差异

核心文件协议与 010a 相同，但 Zsh 入口需要：

- 每个状态敏感函数使用 `emulate -L zsh`。
- 开启局部 `nonomatch`，避免没有备份时未匹配 glob 直接中止函数。
- 交互读取使用 `read -r 'choice?提示'`，不能照搬 Bash 的 `read -p`；Zsh 中 `-p` 表示从 coprocess 读取。
- 测试数组采用 Zsh 默认的 1-based 索引。

## How to Run

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\010-b-linux-switch-functions-zsh\run.ps1
```

当前 Ubuntu WSL 未安装系统 Zsh。运行脚本会用 `apt download` 获取 Ubuntu Zsh 5.9 包并只解压到 `.run/zsh-root/`，不会执行系统安装或要求 root。

## What to Expect

Zsh `5.9` 下 `.run/summary.json` 为 12/12，与 Bash 变体覆盖同一组行为：

- source 和取消零写入
- 首次建立区块
- 权限保留
- 后续区块外字节一致
- 特殊字符预转义
- 当前 ID 读取
- 损坏和冲突停止
- 最近五份备份
- 最新备份恢复
- 无额外运行时依赖

## Observability

- 测试工作区位于 WSL `/tmp` ext4 的带空格路径。
- `.run/` 中的 Ubuntu 包和解压结果不进入 Git。
- 目标脚本不读取真实用户 Codex 配置，也不调用网络。

## Investigation Trail

1. **不能把 Bash 文件直接声明为 Zsh 兼容**：大部分核心语法可共享，但交互 `read` 和 glob 未匹配行为不同。
2. **`emulate -L zsh` 隔离调用者选项**：用户可能启用 `KSH_ARRAYS`、`NOMATCH` 等选项，导出函数不能永久改变或依赖其 shell 配置。
3. **`read -p` 语义不同**：Bash 用它显示提示，Zsh 用它读取 coprocess；Zsh 必须使用 `read 'name?prompt'`。
4. **缺少系统 Zsh 不阻止验证**：Ubuntu 包被下载并解压到 Spike 私有 `.run`，实际执行的是原生 Zsh 5.9，不是语法模拟器。
5. **核心文件实现可以共享生成模板**：管理区块、备份、并发检查和恢复逻辑与 Bash 几乎一致，正式代码应由一个 Rust 生成器生成共享核心，再注入 shell 专属的交互和选项包装，避免手工维护两份漂移。
6. **Zsh 同样受 shell 持久性边界限制**：同目录 `mv` 能避免部分文件，但无法等同于 Rust 的文件和父目录同步协议。

## Results

### Verdict: VALIDATED ✓

Zsh 5.9 的 12 个场景全部通过。Zsh 变体能满足与 Bash 相同的长期独立切换契约。

### Head-to-head

| 维度 | Bash 4.4 | Zsh 5.9 |
|---|---|---|
| 12 项行为矩阵 | 12/12 | 12/12 |
| source 零副作用 | 通过 | 通过 |
| 交互提示 | `read -p` | `read 'name?prompt'` |
| 调用者选项隔离 | 函数局部变量即可 | 推荐 `emulate -L zsh` |
| 未匹配 glob | 默认保留字面值 | 默认报错，需局部 `nonomatch` |
| 核心写入协议 | 可共享模板 | 可共享模板 |

### 推荐

正式产品继续导出两个文件，但不要手工维护两套业务逻辑。Rust 生成器应共享 provider/block/backup 模板，只分叉最薄的 shell 交互层。

### 限制

- 只实际执行 Zsh 5.9，没有覆盖所有 Zsh 5.x 小版本。
- 与 Bash 一样依赖常见 GNU/Linux 基础命令。
- 脚本和备份包含全部供应商明文凭据。
