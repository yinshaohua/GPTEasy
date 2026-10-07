# WSL2 供应商切换后的 Codex daemon 刷新方案

- 方案日期：2026-10-06
- 实施状态：已实现并完成本机 Ubuntu WSL2 真实切换验证；未发布
- 适用范围：Windows 宿主中的“选择 WSL2 供应商”操作
- 目标入口：WSL2 供应商选择对话框中的“应用到 WSL2”
- 关联方案：[刷新模型目录方案](model-catalog-refresh-2026-10-06.md)

## 1. 背景

Linux 导出脚本在供应商配置成功写入后，会尽力执行：

```text
codex app-server daemon restart
```

Windows 当前用户 Codex 环境的供应商切换也已经有独立的 managed daemon 刷新方案。
但“选择 WSL2 供应商”使用的是另一条调用链：Windows Rust 后端探测 WSL2 发行版，
必要时临时启动发行版，把配置和凭据工件通过 guest writer 写入发行版默认用户的
`~/.codex`，提交 WSL2 状态后再恢复发行版生命周期。

当前 WSL2 切换成功后只返回配置和生命周期结果，不会刷新发行版内已经运行的 Codex
app-server daemon。因此，daemon 可能继续使用切换前加载的模型目录，导致 WSL2 中的
Codex 模型选择器仍显示旧供应商模型，直到用户自行执行 daemon restart。

本次实现同时修复了 WSL guest writer 的目录工件协议：V2 bundle 的目录长度和正文此前
未被 guest writer 读取，导致启用 `set -u` 时在写入阶段退出并报告“配置写入失败”。
修复后配置与模型目录采用并发校验、原子替换和双工件回滚，`auth.json` 仍保持不变。

本方案只解决 WSL2 供应商选择后的 daemon 刷新，不重新设计 Windows 当前用户环境的
daemon 刷新，也不改变桌面版进程树重启、Codex CLI 用户进程控制或 WSL2 生命周期规则。

## 2. 产品决策

### 2.1 刷新是切换成功后的独立后置操作

WSL2 供应商切换的主事务和 daemon 刷新必须分离：

```text
读取 WSL2 状态
→ 获取发行版内共享锁
→ 准备并写入配置、凭据及恢复点
→ 提交 WSL2 当前供应商状态
→ 释放配置锁
→ 尝试刷新发行版内 managed daemon
→ 返回供应商切换结果和 daemon 刷新子结果
→ 按原始生命周期处理发行版
```

daemon 刷新失败不得：

- 回滚已经提交的供应商配置；
- 回滚 SQLite 中的 WSL2 当前供应商状态；
- 把“选择 WSL2 供应商”报告为配置切换失败；
- 阻止原本 Stopped 的发行版继续进入自然停止恢复流程；
- 通过 `wsl --terminate` 或其他方式强制结束发行版。

配置写入失败、状态提交失败、并发冲突、锁释放失败和生命周期恢复失败仍属于主
切换失败，继续使用现有 WSL2 错误和恢复语义；只有配置和状态已经成功提交之后发生
的 daemon 刷新问题，才归入独立后置结果。

### 2.2 只刷新目标发行版内的 daemon

WSL2 daemon 刷新必须在目标发行版内执行，不能调用 Windows 侧
`ModelCatalogRefresher`。Windows 侧刷新器管理的是当前 Windows 用户的 `CODEX_HOME`
和 Windows 进程；它无法证明目标 WSL2 发行版内的 socket、进程、版本和配置归属。

WSL2 刷新器的控制对象是：

- 目标 WSL2 发行版；
- 该发行版默认用户的 `CODEX_HOME`；
- 该用户对应的 Codex managed app-server daemon。

不得通过 Windows 进程扫描、进程名匹配、`wsl --terminate` 或批量 kill 来实现刷新。

### 2.3 不为了刷新而启动 daemon

daemon 未运行时返回 `not_running`，表示“没有需要刷新的运行中服务”，而不是失败。
供应商配置和模型目录文件已经落盘，未来 daemon 启动时会读取新内容。

WSL2 供应商切换可以因为配置操作而临时启动一个原本 Stopped 的发行版，但不得因为
daemon 刷新而额外启动 daemon。特别是：

- 原本 Running 的发行版：允许刷新已经运行的 daemon；
- 原本 Stopped、为切换临时启动且配置提交后仍处于 guest 操作阶段：只在确认
  daemon 已经运行时刷新；不得执行启动命令；
- 原本 Stopped、配置完成后已进入自然停止观察：不再重新启动发行版或 daemon，
  结果为 `not_running` 或 `unavailable`，并继续恢复生命周期。

实现时应优先在释放共享配置锁后刷新，避免 daemon 在配置写入尚未完成时重新读取
文件。对于原本 Stopped 的发行版，推荐在 guest writer 完成后、自然停止观察之前
完成一次受限刷新尝试；如果 guest 已经自然退出，则返回 `not_running`，不得重新
拉起它。

### 2.4 只控制 managed daemon，不控制用户 Codex

本方案中的 daemon 是 Codex 官方 managed app-server daemon，不包括：

- WSL2 用户在终端中运行的 Codex CLI 进程；
- 用户启动的交互式 `codex app-server --stdio`；
- GPTEasy 会话管理独占启动的 app-server 子进程；
- Windows 侧 ChatGPT/Codex 桌面版进程；
- WSL2 发行版中的其他服务或用户进程。

刷新命令必须只通过 Codex 官方 daemon 控制入口或等价的、能够验证身份的入口执行。
不得为了刷新模型目录而终止 Codex CLI 或正在运行的用户任务。

## 3. 返回契约

### 3.1 WSL2 切换结果增加可选子结果

`apply_wsl_provider` 的主返回结果继续表达 WSL2 配置和生命周期结果，并增加可选的
daemon 刷新结果：

```text
WslApplyResult {
    environment: WslEnvironmentSummary,
    pending_restart: boolean,
    lifecycle_outcome: WslLifecycleOutcome,
    daemon_refresh: Option<WslDaemonRefreshResult>
}
```

`daemon_refresh` 只在主切换已经完成到可以报告应用结果时附加。主事务失败时，
不得伪造一个“刷新失败”子结果来替代真正的 WSL2 错误。

### 3.2 刷新结果

建议契约如下：

```text
WslDaemonRefreshResult {
    operation_id: string,
    status: refreshed | not_running | failed,
    daemon: managed | unavailable | unsafe | unknown,
    before: optional {
        pid: optional integer,
        version: optional string,
        cli_version: optional string
    },
    after: optional {
        pid: optional integer,
        version: optional string,
        cli_version: optional string
    },
    message_id: string
}
```

字段约束：

- `operation_id` 只用于日志关联和诊断，不作为 WSL2 环境身份；
- `status = refreshed` 只有在 restart 后重新确认 daemon ready 时才允许返回；
- `status = not_running` 表示执行前确认没有运行中的目标 daemon；
- `status = failed` 表示曾确认需要控制但无法安全完成或确认恢复；
- `daemon = unsafe` 表示无法证明 daemon、控制入口或 `CODEX_HOME` 属于目标默认
  用户，必须 fail closed；
- `before` 和 `after` 只返回脱敏身份摘要，不返回 socket 路径、命令行、配置正文或
  原始协议响应；
- API Key、服务地址、模型名称和完整供应商配置不得进入结果对象。

### 3.3 与现有字段的关系

`pending_restart` 继续表示 WSL2 中配置已经变化、运行中的 Codex 消费者可能仍使用
旧配置的状态。daemon 刷新成功后，可以清除或重新计算该状态，但不能简单把
`pending_restart` 当成 daemon 刷新结果：

- daemon refresh 关注 managed app-server daemon 的模型目录；
- `pending_restart` 还可能覆盖 CLI 或其他不能由 GPTEasy 自动重启的消费者；
- daemon 刷新成功不代表 CLI 已重启；
- `pending_restart = false` 也不代表 daemon 一定刷新成功。

第一版建议保持现有 `pending_restart` 语义不变，由 UI 同时展示两个事实：配置已应用、
daemon 刷新结果是什么。若后续需要重新定义 pending 状态，必须另行补充 ADR，不能在
本功能中隐式改变。

## 4. 后端分层设计

### 4.1 WslRuntime 增加目标发行版控制能力

后续实现应把 guest daemon 操作放进 `WslRuntime` 抽象，而不是在 `commands.rs` 中
直接拼接 `wsl.exe` 命令。这样可以：

- 让 `SystemWslRuntime` 负责 Windows 到 guest 的安全进程边界；
- 让 `FakeRuntime` 记录 guest daemon 的调用顺序和结果；
- 让 WSL 应用服务决定何时调用刷新，以及如何隔离主事务和后置结果；
- 避免把 WSL2 具体执行细节泄漏到 Tauri command 层。

建议抽象表达以下能力，而不是暴露通用 shell：

```text
inspect_managed_daemon(environment) -> daemon identity/status
restart_managed_daemon(environment) -> command result
```

如果现有 WSL runtime 不适合直接返回 Windows 侧 `ModelCatalogRefreshResult`，可以
新增 WSL 专用的控制器类型，但它仍应由 runtime 提供经过发行版隔离的执行入口。
不要把完整任意命令、任意发行版名称或任意 guest 参数暴露给前端。

### 4.2 Guest 命令边界

通过 `wsl.exe` 传递的参数只能包含：

- 已从探测快照中唯一解析的发行版 command name；
- 固定的 guest helper 或固定的 `codex app-server daemon` 子命令；
- 非敏感的执行模式或超时控制。

不得通过命令行传递：

- API Key；
- 服务地址；
- 默认模型；
- 完整 `config.toml`；
- 凭据文件内容；
- 任意用户输入形成的 shell 代码。

刷新命令不需要读取配置正文。需要时只在 guest 内由官方 daemon 控制入口读取目标
用户的 `CODEX_HOME`，并通过环境变量或固定工作目录明确作用域。执行前必须确认：

1. 目标发行版仍是原来探测到的注册身份；
2. 目标发行版仍处于允许操作的 Running 状态；
3. 默认用户和 `CODEX_HOME` 与当前 WSL2 环境一致；
4. daemon 控制入口属于该用户，并非其他 Codex 或 GPTEasy app-server；
5. daemon 在执行前确实处于运行状态。

命令返回成功不足以证明刷新完成，必须在 bounded timeout 内重新探测 ready。超时、
协议不兼容、身份无法确认或重启后无法重新发现 daemon，均返回 `failed`。

### 4.3 调用顺序和生命周期

对于原本 Running 的发行版，推荐顺序：

```text
获取共享锁
→ 读取、准备并写入配置
→ 提交 WSL2 当前供应商状态
→ 释放共享锁
→ 检查目标 daemon
→ daemon 运行时执行 restart
→ 等待并确认 daemon ready
→ 返回 WslApplyResult
```

对于原本 Stopped 的发行版，推荐顺序：

```text
用户明确确认切换
→ 临时启动发行版
→ 写入配置并提交 WSL2 状态
→ 释放共享锁
→ 仅在发行版仍运行且 daemon 已运行时尝试 refresh
→ 退出 GPTEasy guest 进程
→ 等待发行版自然停止
→ 返回配置结果、daemon 子结果和生命周期结果
```

任何情况下都必须使用现有 RAII/finally 等价语义保证：

- guest writer 或 daemon 刷新失败不会跳过锁释放；
- 原本 Stopped 的发行版不会因 daemon 刷新错误而跳过自然停止观察；
- 不通过强制 terminate 恢复状态；
- 发行版在自然停止等待超时后，保留实际 Running 状态并如实返回。

如果 daemon 刷新期间发行版自然退出，刷新结果为 `not_running` 或 `failed`，具体取决
于是否已经确认 daemon 在 restart 前运行；不能重新启动发行版来完成刷新。

### 4.4 Tauri command 层

`apply_wsl_provider` 仍然是“选择 WSL2 供应商”的单一后端入口。前端不应先调用
“写配置”，再调用“重启 daemon”。command 层只负责：

- 调用 WSL 应用服务；
- 记录主切换和刷新后置操作的脱敏日志；
- 将组合结果序列化给前端；
- 保持主错误和刷新子结果的边界。

刷新后置操作建议在同一个 `spawn_blocking` WSL 工作单元中执行，以便复用已验证的
发行版身份和生命周期上下文；如果实现拆成异步后置任务，必须保持发行版生命周期
不会在任务运行期间提前恢复或被错误重新启动。

`reclaim_wsl_provider` 是否附带相同刷新逻辑属于实现范围决策：第一版至少覆盖
“选择 WSL2 供应商”调用的 `apply_wsl_provider`；如果接管流程同样会提交新的 Codex
配置，则推荐复用同一后置刷新服务，但必须单独补充 reclaim 的生命周期测试。

## 5. 前端交互设计

### 5.1 选择 WSL2 供应商成功反馈

“应用到 WSL2”成功后，UI 必须明确区分三件事：

1. 供应商是否已应用；
2. daemon 是否刷新；
3. 发行版生命周期是否自然恢复。

建议反馈文案：

- `已将“供应商”应用到 WSL2 发行版“Ubuntu”，Codex 服务已刷新。`
- `已将“供应商”应用到 WSL2 发行版“Ubuntu”，当前没有运行中的 Codex 服务，无需刷新。`
- `已将“供应商”应用到 WSL2 发行版“Ubuntu”，但 Codex 服务刷新失败；配置已生效，请在 WSL2 中重试。`

如果发行版原本 Stopped 但自然停止等待超时，继续显示现有生命周期提示，并追加
daemon 子结果。不得因为刷新失败把整条反馈改成“供应商应用失败”。

### 5.2 不增加 Windows 桌面重启按钮语义

WSL2 选择对话框不应复用 Windows 桌面“重启 Codex”按钮，也不应显示会结束 Windows
桌面进程的操作。WSL2 的 daemon 刷新是切换后的自动后置尝试；失败后提示用户在
目标 WSL2 终端中手动重试即可。

首版不要求增加单独的“刷新 WSL2 daemon”按钮。若未来需要补偿入口，应以选定发行版
为上下文新增独立 command，并保持与供应商切换后置刷新相同的身份验证和错误隔离。

## 6. 日志和诊断证据

新增日志必须帮助区分“配置已提交但 daemon 未运行”“daemon restart 失败”和“daemon
已重启但 ready 确认超时”，同时不得泄漏敏感配置。

建议阶段：

- `wsl.daemon_refresh.start`
- `wsl.daemon_refresh.inspect`
- `wsl.daemon_refresh.not_running`
- `wsl.daemon_refresh.restart_requested`
- `wsl.daemon_refresh.ready`
- `wsl.daemon_refresh.failed`
- `wsl.daemon_refresh.follow_up`

每条日志至少包含：

- `operation_id`；
- WSL2 `environment_id` 的稳定脱敏摘要或允许的关联 ID；
- 触发来源：`wsl_provider_switch` 或 `wsl_provider_reclaim`；
- 当前阶段和结果分类；
- daemon/CLI 版本摘要；
- `originally_running`；
- 是否继续执行生命周期恢复；
- 脱敏错误类别和稳定 `message_id`。

禁止记录：

- API Key；
- 服务地址；
- 默认模型；
- 完整命令行；
- socket 路径和原始 socket 内容；
- 完整 `config.toml`、`auth.json` 或 guest writer bundle；
- WSL 用户的工作目录和任务输入。

至少补充以下诊断证据回归：

| 场景 | 必须能区分的阶段和结果 |
| --- | --- |
| 配置写入失败 | 主切换失败，不产生“daemon 已尝试”日志 |
| 配置已提交、daemon 未运行 | 主切换成功，`not_running`，生命周期继续 |
| daemon 身份不安全 | 主切换成功，`failed/unsafe`，不执行 restart |
| restart 命令失败 | 主切换成功，`failed/command_failed`，生命周期继续 |
| restart 后 ready 超时 | 主切换成功，`failed/timeout`，不回滚配置 |
| daemon 刷新成功 | 主切换成功，`refreshed`，版本/身份摘要不含敏感信息 |
| 原本 Stopped 的发行版 | daemon 结果不改变自然停止观察和最终生命周期状态 |

## 7. 测试设计

### 7.1 Rust 单元测试

围绕 `WslRuntime` fake 和 WSL 应用服务增加以下测试：

- 配置提交失败时不调用 daemon inspect/restart；
- 配置提交成功、daemon 未运行时返回 `not_running`；
- daemon 运行且 restart 和 ready 检查成功时返回 `refreshed`；
- daemon restart 失败时主结果仍为成功，刷新结果为 `failed`；
- ready 检查超时不回滚已写入配置；
- daemon 身份不安全时 fail closed，不执行 restart；
- daemon 刷新失败后共享锁仍释放；
- daemon 刷新失败后原本 Stopped 的发行版仍进入自然停止观察；
- 自然停止等待超时仍保留实际 Running 状态；
- 不会调用发行版级强制终止；
- guest 命令参数不包含 API Key、服务地址、模型或配置正文；
- 同一 WSL2 操作锁下不会并发执行两个 daemon restart。

### 7.2 Rust 集成/guest harness 测试

在已有 WSL guest harness 中增加：

- Running WSL2 切换成功后 guest 记录到固定 daemon restart 命令；
- fake daemon 返回非零退出码时仍能读到已提交的新供应商配置；
- daemon restart 调用记录不包含凭据和完整配置；
- Stopped WSL2 切换不会因为 daemon 刷新重新启动已自然停止的发行版；
- guest daemon 在刷新前自然退出时不发生二次启动；
- WSL2 配置锁、恢复点、凭据清理和 daemon 后置操作的顺序符合协议；
- daemon 刷新结果与 `lifecycle_outcome` 独立返回。

真实 WSL2 验收仍需覆盖 Running 和 Stopped 两种状态。未执行真实 guest 或真实 Codex
daemon 的测试，不得写入“已通过”证据。

### 7.3 前端测试

在“选择 WSL2 供应商”已有测试中增加结果矩阵：

| 主切换 | daemon 刷新 | 预期 UI |
| --- | --- | --- |
| 成功 | `refreshed` | 供应商已应用，Codex 服务已刷新 |
| 成功 | `not_running` | 供应商已应用，无需刷新 |
| 成功 | `failed` | 供应商已应用，但服务刷新失败 |
| 失败 | 无子结果 | 显示原有 WSL2 错误，不显示伪造刷新结果 |

同时验证：

- `apply_wsl_provider` 调用参数不含 API Key、服务地址或完整配置；
- daemon 刷新失败时对话框仍关闭并更新 WSL2 当前供应商；
- 生命周期提示与 daemon 刷新提示不互相覆盖；
- WSL2 UI 不出现 Windows 桌面重启动作或 CLI 自动重启暗示。

### 7.4 Linux 导出脚本回归

Linux Bash/Zsh 脚本已有的 `codex app-server daemon restart` 行为继续保留，不能因为
新增 WSL2 后端实现而改变。需要确保：

- source 脚本仍然零写入；
- 明确选择供应商后才执行 daemon restart；
- restart 失败仍不回滚脚本已经成功完成的配置切换；
- WSL2 桌面应用和独立脚本共享协议，但不会互相重复执行对方的状态机。

## 8. 实现顺序和边界

后续会话建议按以下顺序实现：

1. 固定 `WslDaemonRefreshResult` 的 Rust/TypeScript 序列化契约和 message ID。
2. 在 `WslRuntime`/`SystemWslRuntime` 中增加固定目标发行版的 daemon 探测与 restart
   执行能力。
3. 在 WSL 应用服务中把刷新放到配置状态提交和共享锁释放之后，并实现主结果与后置
   结果隔离。
4. 保证原本 Stopped 的发行版仍使用现有自然停止恢复流程。
5. 在 `apply_wsl_provider` command 中记录脱敏阶段日志并返回组合结果。
6. 在供应商页显示三类 daemon 结果和生命周期结果。
7. 补齐 fake runtime、guest harness、UI 和真实 WSL2 验收。

本方案明确不包含：

- Windows 桌面版进程树重启；
- WSL2 中 Codex CLI 的自动重启；
- WSL2 发行版级强制终止；
- 通过 `wsl.exe` 参数传递凭据或完整配置；
- 独立 Linux 脚本协议的重新设计；
- 应用重启后持久化 daemon 刷新任务；
- 为 daemon 刷新新增单独的前端按钮。

## 9. 验收标准

功能完成后至少满足：

- 在 Running WSL2 中选择供应商，配置成功提交后会尝试刷新目标发行版内已运行的
  managed Codex daemon；
- 在 daemon 未运行时不启动 daemon，并向用户显示无需刷新；
- daemon 刷新失败时，供应商切换仍显示成功，且可以看到明确的刷新失败提示；
- daemon 刷新失败不回滚 `config.toml`、凭据工件、备份或 WSL2 当前供应商状态；
- 原本 Stopped 的发行版不会为了刷新而被额外启动，且不会被强制终止；
- WSL2 内 Codex CLI、用户 app-server、Windows 桌面进程和 GPTEasy 会话服务均不受
  daemon 刷新控制；
- 日志能区分主切换阶段和 daemon 刷新阶段，且未泄漏真实凭据或完整配置；
- UI 能区分 `refreshed`、`not_running`、`failed` 和主切换失败；
- Linux 导出脚本现有 daemon restart 回归保持通过；
- 自动化测试和真实 WSL2 验收结果按实际执行情况记录，未执行项目不标记为通过。

## 10. 相关文档

- [`docs/plans/model-catalog-refresh-2026-10-06.md`](model-catalog-refresh-2026-10-06.md)
- [`docs/adr/0027-wsl2-provider-selection-scope.md`](../adr/0027-wsl2-provider-selection-scope.md)
- [`docs/adr/0030-shared-wsl-management.md`](../adr/0030-shared-wsl-management.md)
- [`docs/adr/0031-safe-wsl-lifecycle-restoration.md`](../adr/0031-safe-wsl-lifecycle-restoration.md)
- [`docs/adr/0048-force-standalone-linux-provider-switch.md`](../adr/0048-force-standalone-linux-provider-switch.md)
- [`docs/ui/PROVIDER-MANAGEMENT-SPEC.md`](../ui/PROVIDER-MANAGEMENT-SPEC.md)
