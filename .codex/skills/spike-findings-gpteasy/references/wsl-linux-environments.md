# WSL2 与 Linux 导出物

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- WSL2 检测不得启动或进入发行版。
- 用户明确切换已停止发行版时才临时启动，并在处理结束后恢复原停止状态。
- WSL2 首版只管理发行版默认用户的 Codex 配置，不主动终止其中运行的 Codex。
- Linux 导出物分别支持 Bash 4+ 与 Zsh 5+。
- 导出脚本不依赖 Python、Node.js、jq、第三方解析器或 GPTEasy 可执行文件。
- 脚本被 source 时不得修改配置；只有用户调用交互式 function 并明确选择供应商后才写入。
- 管理区块、备份、并发停止和外部配置保护必须与桌面应用保持同一协议。

## How to Build It

### 1. 用无副作用的 Windows 探针发现 WSL2

检测阶段只调用：

```text
wsl.exe --version
wsl.exe --list --quiet
wsl.exe --list --running --quiet
```

再只读：

```text
HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss
```

机器状态来自集合差：

- `all_names`：全部发行版。
- `running_names`：运行中发行版。
- `all_names - running_names`：已停止发行版。

不要解析 `wsl --list --verbose` 的 `Running` / `Stopped` 文本，因为标题和状态会本地化。

当前 `wsl.exe` 输出按 UTF-16 读取：

```powershell
$info.StandardOutputEncoding = [System.Text.Encoding]::Unicode
$info.StandardErrorEncoding = [System.Text.Encoding]::Unicode
```

检测前后再次读取运行集合并比较，作为“探针没有启动发行版”的可执行门禁。

### 2. 数据库身份使用注册 GUID

Lxss 注册表可提供：

- 注册 ID / GUID
- `DistributionName`
- `DefaultUid`
- WSL version
- BasePath 是否存在

显示名称可能重复，而 `wsl.exe -d NAME` 未必能唯一对应注册记录。正式数据模型：

```text
environment_id = Lxss registration GUID
display_name   = DistributionName
command_name   = 只有可唯一解歧时才允许用于 wsl.exe -d
```

若名称与注册记录无法唯一对应，进入 `needs_attention`，不启动、不写入。

过滤基础设施发行版，例如 `docker-desktop`。不要仅靠名称字符串作为永远可靠的规则；保留可扩展的基础设施判定和用户显式排除。

### 3. 把单个发行版切换实现为生命周期 Saga

状态机：

| 原状态 | 用户动作 | 行为 | 结束状态 |
|--------|----------|------|----------|
| Running | 取消 | 不写配置 | Running |
| Stopped | 取消 | 不启动、不写配置 | Stopped |
| Running | 应用切换 | 修改默认用户配置 | Running |
| Stopped | 应用切换 | 临时启动、修改、终止 | Stopped |
| Stopped | 写入失败或区块损坏 | 临时启动、停止修改、终止 | Stopped |

关键结构：

```rust
let originally_stopped = distribution.state == State::Stopped;
if originally_stopped {
    start_distribution();
}

let result = apply_provider_to_default_user();

if originally_stopped {
    terminate_distribution();
}

result?;
```

正式实现必须使用 `finally`/RAII 语义，确保临时启动后的所有成功和失败路径都恢复原状态。不能只在正常返回前调用 terminate。

运行中发行版：

- 不为切换而终止整个发行版。
- 只修改默认用户的 `~/.codex/config.toml`。
- 若检测到 WSL 内 Codex 仍运行，记录 `pending_restart`，提示用户人工重启。

批量切换是逐环境独立 Saga；一个发行版失败或临时启动，不得改变其他发行版的原状态。

### 4. 默认用户名只在允许启动后解析

检测阶段不能运行：

```text
wsl.exe -d NAME -- id -un
```

因为这会启动已停止发行版。检测阶段只读取注册表 `DefaultUid`。当用户明确切换、且发行版已经允许启动后，才在发行版内把 UID 解析为用户名并定位：

```text
$HOME/.codex/config.toml
```

首版只处理默认用户，不扫描或修改其他 Linux 用户。

### 5. 由 Rust 生成两个独立导出文件

正式产品继续导出：

```text
gpteasy-switch.bash
gpteasy-switch.zsh
```

一个 Rust 生成器共享：

- 供应商目录和不可变 ID
- 预转义的 TOML 管理区块
- 标记扫描
- 冲突检测
- 备份命名与裁剪
- 并发 fingerprint
- 恢复流程

只分叉薄的 shell 层：

| 行为 | Bash 4+ | Zsh 5+ |
|------|---------|--------|
| 交互读取 | `read -r -p '提示' choice` | `read -r 'choice?提示'` |
| 调用者选项隔离 | 局部变量和保守语法 | 每个状态敏感函数 `emulate -L zsh` |
| 未匹配 glob | 默认保留字面值 | 局部启用 `nonomatch` |
| 数组索引 | 0-based | 默认 1-based |

不要尝试维护一份“同时兼容 Bash 和 Zsh”的交互脚本；共享生成模板，但输出两个明确目标。

### 6. source 只能定义函数和常量

导出文件加载时只允许定义：

- `gpteasy_select_provider`
- `gpteasy_current_provider`
- `gpteasy_restore_latest`
- 以 `gpteasy__` 开头的内部函数和常量

source 阶段不得：

- 创建 `~/.codex`
- 读取或写入配置
- 创建备份
- 发起网络请求
- 自动选择供应商

测试应在 source 前后对配置执行 `cksum`，作为零副作用门禁。

### 7. 预渲染 TOML，不在 shell 中实现转义器

GPTEasy 导出时已经拥有结构化供应商数据。Rust 生成器应为每个供应商输出带单引号 heredoc delimiter 的完整 TOML：

```bash
cat <<'GPTEASY_BLOCK'
# >>> GPTEasy managed provider >>>
# GPTEasy provider-id: provider-id
model = "model"
model_provider = "gpteasy"
model_providers.gpteasy.base_url = "https://provider.example/v1"
model_providers.gpteasy.experimental_bearer_token = "pre-escaped-key"
# <<< GPTEasy managed provider <<<
GPTEASY_BLOCK
```

这样 `$`、反斜杠、引号和 Unicode 都在生成时完成 TOML 转义，目标 shell 不需要实现字符串解析器。

### 8. shell 写入协议保持保守

Linux 独立脚本没有 TOML 解析器，因此：

1. 精确扫描管理区块开始/结束整行。
2. 无区块且存在顶层 `model`、`model_provider` 或 `model_providers.gpteasy` 时停止，提示先由桌面 GPTEasy 完成结构化迁移。
3. 区块恰好一对时，只替换区块。
4. 同目录 `mktemp` 创建候选文件。
5. 使用原配置 `cksum` 作为并发 fingerprint。
6. 创建带 UTC 纳秒时间戳的备份。
7. 保留原文件权限。
8. 替换前再次比较 fingerprint。
9. 使用同文件系统 `mv` 替换。

备份按文件名逆序排序保留五份：

```text
config-YYYYMMDDTHHMMSSNNNNNNNNN-PID-RANDOM.toml
```

不要依赖 mtime；DrvFS 等挂载文件系统的时间排序可能不稳定。

### 9. 明确提示导出物的凭据风险

Bash/Zsh 导出文件、当前配置和备份都包含全部供应商明文凭据。产品必须：

- 导出前明确告知敏感性。
- 默认设置当前用户读取权限。
- 不把导出内容写入日志或诊断。
- 建议用户只复制到受信任 Linux 账户。
- 提供删除和重新导出的简单路径。

## What to Avoid

- **不要在检测阶段进入发行版。** `id -un`、`echo $HOME` 等命令都会启动已停止环境。
- **不要解析本地化的 `wsl --list --verbose` 状态列。**
- **不要用显示名称作为数据库主键。**
- **不要管理 `docker-desktop` 等基础设施发行版。**
- **不要在失败路径忘记恢复原停止状态。**
- **不要为重启 Codex 而终止整个 WSL 发行版。**
- **不要扫描或修改非默认用户。**
- **不要让一个批量操作共享无法隔离的临时启动状态。**
- **不要在 source 时写配置或自动选供应商。**
- **不要在 shell 中实现通用 TOML 解析或运行时 TOML 转义。**
- **不要在无管理区块但已有供应商键时直接接管。**
- **不要依赖 Python、Node、jq、Perl 或 Ruby。**
- **不要按 mtime 选择最新备份。**
- **不要把 shell `mv` 描述成与 Rust `sync_all` 相同的断电持久性保证。**

## Constraints

- 009 已在真实 Windows 上验证只读探针前后运行集合不变，并以 fixture 验证生命周期、失败、批量、默认用户和备份场景。
- 真实已停止用户发行版的启动耗时、默认用户名解析、配置写入、终止时机和 WSL 内进程探针仍未执行，因此 009 为 PARTIAL。
- 注册表重复显示名称到 `wsl.exe -d NAME` 的可靠解歧尚未解决；无法唯一对应时必须停止。
- 010a 在 GNU Bash 4.4.0 上通过 12/12；010b 在 Zsh 5.9 上通过同一 12/12 矩阵。
- 两个脚本依赖常见 GNU/Linux 命令：`awk`、`cksum`、`mktemp`、`sort`、`date %N`、`cp`、`chmod --reference` 和 `mv`。
- 不面向 BusyBox-only、非 GNU 用户空间或没有纳秒 `date` 的最小系统。
- shell 层无法提供 Rust 文件和父目录 `sync_all` 的完整持久性边界。
- 导出文件和备份包含明文 Key，必须按敏感文件处理。

## Origin

Synthesized from spikes: 009, 010a, 010b
Source files available in: `sources/009-wsl2-environment-lifecycle/`, `sources/010-a-linux-switch-functions-bash/`, `sources/010-b-linux-switch-functions-zsh/`
