# 刷新模型目录方案

- 方案日期：2026-10-06
- 实施状态：后端、前端及自动回归已实现；真实 Windows 模型选择器验收待完成，未发布。契约差异及验收记录见 [实施记录](../testing/model-catalog-refresh-2026-10-06.md)。
- 适用范围：Windows 当前用户 Codex 环境、供应商切换、Codex 桌面版控制
- 目标入口：供应商管理页顶部现有“重启 Codex”控件旁

## 1. 背景

GPTEasy 切换供应商时会更新 `config.toml` 和
`gpteasy-model-catalog.json`。当前默认模型可以随配置变化生效，但已经运行的
Codex app-server daemon 可能继续使用启动时读取的旧模型目录。因此 Codex 中的模型
选择器仍显示切换前供应商的模型，直到用户执行：

```text
codex app-server daemon restart
```

本方案只处理 Codex 的共享 managed app-server daemon，不处理以下进程：

- ChatGPT/Codex 桌面版主进程；
- Codex CLI 用户进程；
- GPTEasy 为会话管理独占启动的 `codex app-server --stdio` 子进程。

现有“重启 Codex”按钮继续负责受信任的 Codex 桌面版进程树重启。本方案不改变其
进程身份复核、用户确认和 CLI 不受控等既有约束。

## 2. 产品决策

### 2.1 按钮名称和位置

在现有“重启 Codex”按钮旁增加独立按钮：

```text
刷新模型目录    重启 Codex
```

按钮名称固定为“刷新模型目录”。不使用“重启 Codex 服务”或“重载 app-server”，
避免用户把它与桌面版重启混淆。

该按钮属于当前 Codex 环境操作，不属于某个供应商列表行。模型目录是当前活动供应商
的全局 Codex 选择目录，按钮不接受供应商 ID，也不针对非当前供应商刷新。

### 2.2 刷新失败隔离

刷新模型目录是一个可失败的后置副作用，不是供应商切换或桌面重启的事务组成部分。

硬性要求：

- 模型目录刷新失败，不回滚已成功的供应商切换；
- 模型目录刷新失败，不把供应商切换报告为失败；
- 模型目录刷新失败，不阻止或取消“重启 Codex”；
- “重启 Codex”失败，不回滚已成功的模型目录刷新；
- “重启 Codex”失败，不把模型目录刷新报告为失败；
- 两个操作各自报告成功、失败、未执行和失败原因。

供应商切换结果至少需要能表达：

```text
供应商切换：成功
模型目录刷新：成功 / 未运行无需刷新 / 失败
```

桌面重启结果至少需要能表达：

```text
模型目录刷新：成功 / 未运行无需刷新 / 失败
Codex 桌面版重启：成功 / 失败
```

## 3. 三种触发方式

### 3.1 供应商切换后的自动刷新

供应商切换的安全配置 Saga 成功提交后，GPTEasy 尝试刷新模型目录。推荐顺序：

```text
验证供应商
→ 准备配置、catalog 和备份
→ 原子替换受管工件
→ 提交当前供应商状态
→ 检查 managed app-server daemon
→ daemon 正在运行时执行刷新
→ 读取刷新结果并返回切换报告
```

自动刷新不能发生在配置提交之前。配置提交成功后，即使刷新失败，也必须保留新供应商
和新模型目录。

以下情况视为“未执行，无需刷新”，不是失败：

- managed app-server daemon 当前没有运行；
- 当前环境没有发现可用的 Codex daemon 管理入口。

以下情况视为刷新失败：

- 已确认 daemon 正在运行，但 restart 命令失败；
- 命令返回后无法确认 daemon 恢复运行；
- 发现的 daemon 身份、`CODEX_HOME` 或管理状态不符合安全条件；
- 等待 daemon ready 超时；
- 版本或控制协议不兼容，无法可靠确认刷新结果。

自动刷新默认不启动未运行的 daemon。这样供应商切换不会为了刷新模型目录而凭空启动
Codex 后台服务。

### 3.2 手动“刷新模型目录”

按钮执行独立的刷新命令，不修改供应商配置、不重新验证供应商、不生成新的供应商
切换 Saga，也不重启桌面版。

建议流程：

```text
读取当前环境和 daemon 状态
→ 若 daemon 未运行，返回“未运行，无需刷新”
→ 若 daemon 可安全控制，执行 restart
→ 等待 daemon ready
→ 返回刷新结果
```

按钮操作期间只禁用自身和同一模型目录刷新操作，不能因为刷新进行中而禁用“重启
Codex”。如果两个操作同时开始，后端必须使用共享锁或幂等协调，避免并发执行两个
daemon restart；但“刷新失败不影响桌面重启”的错误隔离仍然成立。

### 3.3 “重启 Codex”中的联动刷新

用户点击现有“重启 Codex”并完成确认后，执行一次模型目录刷新，并继续执行既有的
桌面版重启流程。

推荐顺序：

```text
用户确认
→ 尝试刷新模型目录
→ 无论刷新成功或失败，继续执行 Codex 桌面版重启
→ 分别收集两个结果
→ 展示组合结果
```

特别要求：

- 刷新模型目录失败，不能短路桌面重启；
- 刷新模型目录超时，不能短路桌面重启；
- 刷新模型目录未执行，仍继续桌面重启；
- 桌面版重启失败，不能重新执行或回滚模型目录刷新；
- 现有桌面重启的确认文案必须继续说明可能结束 Codex 桌面进程及其中任务；
- 不因新增刷新步骤而扩大桌面进程关闭集合；
- 不因刷新失败而终止、重启或接管 Codex CLI。

如果后续验证发现 daemon restart 会影响桌面版重启的稳定性，可以调整为先执行桌面
版重启，再在桌面版启动完成后执行模型目录刷新，但仍必须保持两个结果独立。第一版
实现应先按上面的顺序落地，并通过真实 Windows 验收确认。

## 4. 运行和安全边界

### 4.1 只操作 Codex managed daemon

后端不得按进程名、可执行文件名或命令行中出现 `app-server` 批量查找和终止进程。
只允许使用 Codex 官方 daemon 管理命令或等价的、可验证的控制协议，并在执行前确认：

- 当前用户 `CODEX_HOME`；
- daemon 管理状态；
- daemon 版本和 PID；
- 控制 socket 或官方状态接口属于当前用户；
- 目标不是 GPTEasy 自有会话 app-server；
- 目标不是桌面版或 CLI 用户进程。

### 4.2 不主动启动 daemon

“刷新模型目录”只重启已经运行的 managed daemon。daemon 未运行时返回无需刷新，
不主动执行 `daemon start`。

原因是模型目录文件已经写入磁盘，未来 Codex 启动或 daemon 首次启动时会读取新目录；
GPTEasy 不应因用户切换供应商而无提示启动后台服务。

### 4.3 不扩大现有重启权限

本方案不修改 ADR-0041 的桌面进程控制边界：

- 桌面版重启仍需用户二次确认；
- 只控制可信 OpenAI 桌面安装对应的进程树；
- Codex CLI 不启动、不关闭、不终止、不重启；
- GPTEasy 自有会话 app-server 由其现有生命周期管理；
- daemon 刷新不得复用桌面根进程的 PID 集合。

## 5. 后端接口设计方向

本节只固定后续实现的契约，不在本次方案中新增接口。

建议新增独立的后端操作，例如 `refresh_model_catalog`，返回结构化结果：

```text
status: refreshed | not_running | failed
daemon: managed | unavailable | unsafe | unknown
before: { pid, version }
after: { pid, version }
message_id: string
```

不得把原始命令行、控制 socket 路径、API Key、配置正文或 daemon 原始响应返回给前端。
前端只根据 `status` 和脱敏 `message_id` 选择文案。

供应商切换结果和桌面重启结果应增加可选的模型目录刷新结果，而不是把刷新错误编码
成供应商或桌面操作的主错误。例如：

```text
ProviderSwitchResult {
    environment: ...,
    model_catalog_refresh: ...
}
```

```text
DesktopRestartResult {
    desktop: ...,
    model_catalog_refresh: ...
}
```

如果现有 command 失败类型不适合承载“主操作成功、刷新失败”的组合结果，应优先
改为正常返回包含子结果的报告，而不是抛出刷新错误。

## 6. 前端交互设计

### 6.1 按钮状态

“刷新模型目录”按钮使用自己的忙碌状态和刷新图标。它不复用桌面版启动/重启按钮
的 `loading` 状态，也不显示桌面版进程状态。

建议状态文案：

- `刷新模型目录`
- `正在刷新模型目录`
- `模型目录已刷新`
- `当前没有运行中的 Codex 服务，无需刷新`
- `模型目录刷新失败，请重试`

按钮在以下情况下禁用：

- 应用正在读取启动状态；
- 同一次模型目录刷新正在进行；
- 当前环境状态不可安全确认。

供应商切换进行时是否禁用手动按钮，应由后端并发协调决定；前端不能通过按钮状态
假定供应商切换已经提交或未提交。推荐在切换的配置写入阶段禁用，切换提交后恢复，
由自动刷新结果更新提示。

### 6.2 与“重启 Codex”并列

桌面版状态、按钮和模型目录按钮保持同一操作区域，但状态文案分开：

```text
Codex 桌面版运行中    [刷新模型目录] [重启 Codex]
```

不要把模型目录结果写入桌面版状态标签，也不要把“刷新模型目录”做成“重启 Codex”
的下拉选项。两个操作都可以在日志中关联同一个 operation ID，但用户界面必须能看出
它们是两个不同动作。

### 6.3 联动结果反馈

“重启 Codex”完成后，组合反馈应覆盖四种主要结果：

| 模型目录刷新 | 桌面版重启 | 展示 |
| --- | --- | --- |
| 成功 | 成功 | 模型目录已刷新，Codex 已重新启动。 |
| 失败 | 成功 | Codex 已重新启动，但模型目录刷新失败，请点击“刷新模型目录”重试。 |
| 未运行无需刷新 | 成功 | Codex 已重新启动，当前没有运行中的 Codex 服务，无需刷新模型目录。 |
| 任意 | 失败 | 分别展示模型目录结果和桌面版重启失败原因，不合并成单一失败。 |

供应商切换后的反馈同理，但不应提示用户“供应商切换失败”。

## 7. 日志和最小脱敏证据

刷新功能需要补充最小结构化日志，区分以下阶段：

- `model_catalog_refresh.inspect`
- `model_catalog_refresh.start`
- `model_catalog_refresh.restart_requested`
- `model_catalog_refresh.ready`
- `model_catalog_refresh.not_running`
- `model_catalog_refresh.failed`

每条日志至少记录：

- operation ID；
- 触发来源：`provider_switch`、`manual_button` 或 `desktop_restart`；
- 阶段；
- 结果分类；
- CLI/daemon 版本摘要；
- 是否检测到运行中的任务（若官方接口能够安全提供）；
- 是否继续执行了后续独立操作；
- 脱敏错误类别和稳定 message ID。

禁止记录：

- API Key；
- 完整 `config.toml`；
- 完整命令行；
- 控制 socket 原始内容；
- daemon 原始响应正文；
- 供应商服务地址中的敏感信息。

重点回归日志证据：刷新失败后，供应商切换仍为成功；刷新失败后，桌面版重启仍被
调用；桌面版重启失败不会被错误归因于模型目录刷新。

## 8. 实现分层和建议顺序

后续实现建议拆成以下层次：

1. **Codex daemon 探测和命令封装**
   - 定位当前用户 Codex CLI；
   - 读取 daemon 状态；
   - 只执行官方 managed daemon restart；
   - 等待并确认 ready；
   - 统一映射脱敏失败类型。
2. **后端刷新应用服务**
   - 单次刷新锁；
   - `not_running`、`refreshed`、`failed` 三态；
   - 结构化诊断日志。
3. **供应商切换后置协调**
   - 配置 Saga 提交后调用刷新；
   - 刷新失败只附加到结果，不改变主结果；
   - 保留已有配置恢复语义。
4. **桌面重启联动**
   - 在用户确认后调用刷新；
   - 无论刷新结果如何继续现有桌面重启；
   - 返回两个独立子结果。
5. **前端按钮和反馈**
   - 在“重启 Codex”旁增加按钮；
   - 增加独立 busy/error/success 状态；
   - 展示组合结果。
6. **回归测试和真实 Windows 验收**
   - 先覆盖后端错误隔离；
   - 再覆盖 UI 结果矩阵；
   - 最后在真实 Codex CLI 和桌面版中验证模型选择器。

不建议在第一版实现中引入持久化的 `pending_catalog_refresh`。当前需求只要求失败
不影响主操作，并提供手动按钮补偿。若后续需要在应用重启后持续展示未刷新状态，再
单独新增 ADR 和持久化状态，避免把临时 daemon 操作与现有 `pending_restart` 混为一谈。

## 9. 验收标准

### 9.1 手动按钮

- 按钮显示在“重启 Codex”旁；
- daemon 运行时，点击后模型目录重新加载；
- daemon 未运行时显示无需刷新，不自动启动 daemon；
- 刷新失败时可以再次点击；
- 刷新失败不修改供应商、配置或 catalog 文件。

### 9.2 供应商切换

- 切换成功后自动尝试刷新；
- 刷新成功时，切换结果明确包含成功；
- 刷新失败时，供应商切换仍报告成功；
- 刷新失败后可使用手动按钮补偿；
- 不因刷新失败回滚配置或供应商数据库状态。

### 9.3 重启 Codex

- 点击并确认“重启 Codex”时会尝试刷新模型目录；
- 刷新失败时仍执行桌面版重启；
- 桌面版重启成功时，不因刷新失败报告整体为失败；
- 桌面版重启失败时，不伪造刷新失败原因；
- Codex CLI 和 GPTEasy 自有 app-server 不被终止或重启。

### 9.4 模型选择器

- 切换供应商后自动刷新成功时，Codex 模型选择器显示新供应商目录；
- 手动刷新成功后，模型选择器显示新供应商目录；
- 真实桌面版需要重启才能重新读取目录的场景，必须在真实 Windows 验收中记录；
- 如果 daemon 已刷新但桌面选择器仍过滤模型，结果必须单独记录为 Codex 消费者兼容
  问题，不能把它误判为 GPTEasy 供应商切换失败。

## 10. 相关文档

- [`docs/adr/0041-trusted-desktop-start-and-confirmed-restart.md`](../adr/0041-trusted-desktop-start-and-confirmed-restart.md)
- [`docs/adr/0049-provider-model-catalog-and-reasoning-capabilities.md`](../adr/0049-provider-model-catalog-and-reasoning-capabilities.md)
- [`docs/ui/PROVIDER-MANAGEMENT-SPEC.md`](../ui/PROVIDER-MANAGEMENT-SPEC.md)
- [`docs/plans/provider-save-flow-fix-2026-09-26.md`](provider-save-flow-fix-2026-09-26.md)

