# 当前用户 Codex 环境支持供应商模式与 OpenAI 登录模式

当前用户 Codex 环境具有互斥的供应商模式和 OpenAI 登录模式。供应商模式使用一个已验证供应商；OpenAI 登录模式复用 Codex 已有的 ChatGPT 账户凭据，而非 API Key 登录，GPTEasy 不把它建模为供应商。

切换到 OpenAI 登录模式前，GPTEasy 必须确认当前为 ChatGPT 登录，或确认 `auth.json`、当前用户 Codex 目录中的临时恢复快照之一仍保留可恢复的 ChatGPT token。纯 API Key 登录、缺失凭据或无法确认的登录状态均拒绝写入并提示用户先在 Codex 中完成 ChatGPT 账户登录。

从 OpenAI 登录模式进入供应商模式前，GPTEasy 在当前用户 Codex 目录中以原子写入建立一个临时、私有的完整 ChatGPT 凭据恢复快照；它不进入通用配置备份或诊断输出。这样 Codex 在供应商模式下重写 `auth.json` 后，返回操作仍能恢复原账户。

从供应商模式返回时，GPTEasy 将 `config.toml` 与必要的 `auth.json` 作为同一可恢复事务：先备份两个受影响工件，再移除或停用 GPTEasy 管理的供应商配置；若本机或恢复快照保留 ChatGPT token，则将凭据切回 `auth_mode: "chatgpt"` 并删除供应商 API Key，同时保留 token、刷新元数据和未知字段。返回成功后删除恢复快照；失败回滚时恢复快照。GPTEasy 不执行远程登录、刷新或注销。该操作只能在设置窗口中执行，托盘菜单仍只用于已验证供应商之间的快捷切换，但可以确认后返回供应商模式。

模式已生效后若用户在 Codex 外部注销或删除凭据，GPTEasy 保留磁盘上的 OpenAI 登录模式，只在设置页显示登录不可用警告，不改写配置或将其误判为管理冲突。
