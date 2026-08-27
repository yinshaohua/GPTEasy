# “返回 OpenAI 登录模式”流程问题分析

日期：2026-08-25，2026-08-27 补充
范围：分析用户在 Windows PowerShell 环境中的 Codex Desktop、Codex CLI 与 GPTEasy 交互，并记录已实施的修复；不发布版本。

## 结论摘要

问题确认是 GPTEasy 不能依赖供应商模式下当前 `auth.json` 持续保留 ChatGPT token。Codex 0.149 及后续版本可能按自身逻辑将它重写成纯 API Key 凭据；此前返回流程因此拒绝切换，保留了代理配置，Desktop 继续报告“已通过 API 秘钥登录”。已实施的设计是：

- OpenAI 登录模式与 API Key 供应商模式互斥。它可以已登录或等待用户登录，但不能把“已通过 API 秘钥登录”当作 OpenAI 账户登录。
- 从 ChatGPT 账户进入供应商模式前，GPTEasy 在当前用户 Codex 目录写入一个临时、私有的完整凭据恢复快照。该快照不进入普通配置备份或诊断输出，返回成功即删除、失败则回滚保留。
- 返回切换把 `config.toml` 和必要的 `auth.json` 作为一个可回滚事务；存在当前 token 或恢复快照 token 时，恢复 `auth_mode: "chatgpt"`、删除供应商 `OPENAI_API_KEY`，并保留 token、刷新元数据和未知字段。GPTEasy 不负责远程登录、刷新或注销。
- 2026-08-27 补充：没有当前 token 或恢复快照不再阻止退出供应商模式。事务会移除纯供应商凭据，使 Codex 下次启动进入未登录流程，由用户自行完成 ChatGPT 登录。
- 运行中的 Desktop/CLI 仍不会被 GPTEasy 重启，必须从原入口退出并重新运行后才会读取新配置；但这不是本事件中 API Key 状态残留的唯一解释。

“Token exchange failed ... 403 ... Country, region, or territory not supported” 是 OpenAI 登录 token endpoint 返回的地区限制错误。它发生在浏览器登录回调的 token 交换阶段，通常不是 GPTEasy 对供应商配置写入造成的；但它会使 Codex 的 OpenAI 登录凭据无法建立或刷新，从而导致返回 OpenAI 模式后仍不可用。

## 逐项判断

### 1. 左下角既不显示代理商名称，也不显示 OpenAI 名称

这两个名称属于 Codex Desktop 对当前会话/认证状态的 UI 展示，不是 GPTEasy 配置文件中的可控字段。GPTEasy 只能识别环境模式并更新 Codex 原生配置，不能伪造或恢复 Desktop 的账户显示名。

若 OpenAI token 文件仍存在但 Desktop 进程未重新加载，UI 可能继续显示旧状态；若 token 交换失败或凭据已过期，则可能没有可显示的 OpenAI 账户。

### 2. 设置页显示“已通过 API 秘钥登录”，对话不再回答

这表示 Desktop 当前实际读取到 API Key 凭据，或者 API Key 请求链路不可用。本事件中，旧实现返回时没有把 `auth.json` 切回 ChatGPT 凭据形态，是直接原因；切换文件成功也不等于运行中的消费者立即切换，当前合同明确禁止 GPTEasy 主动关闭、激活或重启 Desktop/CLI。

应先完全从原入口退出 Desktop 和 CLI，再重新启动，并确认它们使用的是同一个 Windows 用户和同一个 `CODEX_HOME`。如果重启后仍是 API Key 模式，核对 `auth.json` 的 `auth_mode`、是否错误保留 `OPENAI_API_KEY` 以及实际读取路径；`auth_mode` 不属于 `config.toml`。

### 3. codex-cli 回到登录选择提示

这说明 CLI 进程本身可能已经重新读取了配置，或其 `auth.json` 不存在/不可识别。它不能证明 Desktop 已同步，也不能证明 OpenAI 账户登录成功。CLI 和 Desktop 是两个消费者，必须分别从各自原入口重启并分别观察。

### 4. 点击“退出登录”出现 “Oops, an error has occurred”

GPTEasy 不实现 Codex Desktop 的退出登录动作，因此该错误应归属于 Desktop 自身的注销流程或其 token 服务调用。重启后重新出现登录选项，符合 Desktop 本地会话状态被清理或加载失败后的表现，但不代表 token endpoint 可用。

### 5. Chrome 报 403：Country/region not supported

这是服务端明确拒绝 token exchange 的结果。它和 GPTEasy 的本地配置切换没有直接因果关系；本地程序最多只能触发浏览器登录，不能绕过 OpenAI 地区策略。若该地区/出口 IP 不受支持，反复点击登录或注销不会修复问题。

## 目前流程中不应期待的行为

“切换到 OpenAI 登录模式”不应被理解为把当前 API Key 供应商转换成一个可显示名称的 OpenAI 账户，也不应自动完成远程登录。正确语义是：退出 GPTEasy 供应商配置；有可恢复 ChatGPT token 时恢复为 Codex 可识别的 ChatGPT 形态，没有时进入未登录状态并由用户随后从 Codex 官方入口登录。对于修复安装前已丢失 token 的历史环境，本地程序不能伪造或找回账户。

## PowerShell 环境建议采集的证据

以下检查应由 Windows PowerShell 环境执行，值班记录中只保留路径、模式和状态，不复制 token 或 API Key：

1. 记录 Desktop、CLI 的进程是否在切换前后持续存在；确认从原入口退出后 PID 已变化。
2. 确认 GPTEasy、Desktop、CLI 的 Windows 用户一致，并确认 `CODEX_HOME`（若未设置则为默认用户 `.codex`）一致。
3. 退出所有消费者后读取 `config.toml` 的 GPTEasy 管理区块，并单独检查 `auth.json` 的认证模式，确认目标是 OpenAI 模式而不是 API Key 模式。
4. 仅检查 `auth.json` 是否存在、JSON 结构是否可识别、登录状态是否为有效/过期/缺失；严禁输出 `access_token`、refresh token 或 API Key。
5. 分别重新启动 Desktop 与 CLI，记录每个消费者读取后的模式和首个请求结果。
6. 若重新登录仍返回同一 403，记录发生时间、HTTP 状态和错误码即可，不要把 token endpoint 响应正文或凭据写入 Issue/日志。

## 交给 PowerShell 环境 AI 的处理建议

- 先做上述证据采集，再判断是“切换后未重启/路径不一致”“OpenAI 凭据无效”还是“地区策略拒绝”。
- 不要通过伪造 token 或把 API Key 供应商名称写成 OpenAI 账户名称来制造“原账户”；返回切换可在保留 token 已存在时安全恢复 `auth.json` 的认证形态。
- 若目标是改善体验，应另行设计只读的消费者重载提示和登录状态诊断；这不等同于 GPTEasy 代替 Codex Desktop 完成登录或注销。
- 403 问题应按 OpenAI 官方支持的地区、网络出口和账户登录条件处理；在服务端拒绝未解除前，本地代码修改不能保证恢复登录。

## 依据

- `docs/adr/0014-provider-and-openai-login-modes.md`
- `docs/adr/0026-no-active-desktop-control.md`
- `docs/ui/PROVIDER-MANAGEMENT-SPEC.md`
- `docs/evidence/windows-x64-uat.md`
- `.codex/skills/spike-findings-gpteasy/references/codex-provider-compatibility.md`
