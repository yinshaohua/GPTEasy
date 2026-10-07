# 模型目录刷新实施与验收记录

- 日期：2026-10-06
- 状态：代码与自动回归已实现，真实 Windows 模型选择器验收待完成；未发布。
- 设计：[刷新模型目录方案](../plans/model-catalog-refresh-2026-10-06.md)。

## 实施范围

- 新增独立“刷新模型目录”按钮；刷新忙碌不禁用桌面重启，桌面失败后仍能手动补偿。
- 原生供应商应用、强制应用、当前供应商保存并应用，在原有配置操作成功后刷新；失败提交不执行刷新。未扩大到 WSL2、Linux 导出或 OpenAI 登录/恢复入口。
- 确认桌面重启后先刷新，再执行既有可信桌面重启。保留会话可见性硬性保护，保护拒绝时刷新尚未执行。
- 主结果沿用已有 JSON 顶层字段，新增 modelCatalogRefresh；桌面失败报告也携带同一刷新结果，未执行时字段缺省。
- 后端共享锁串行刷新，锁等待、控制命令和 ready 检查有界。仅执行原生 Codex 官方 daemon 管理命令，不执行 shell shim，不枚举或终止消费者进程。
- 日志记录 operation ID、来源、阶段、版本摘要、失败类别及真实后续桌面结果；不记录原始响应、socket 正文、命令行或供应商配置。

## 真实 Windows 只读契约探测

已在本机 Codex CLI 0.160.1 上执行官方帮助与 version 查询；没有执行生产 daemon restart/start/stop。

1. 该版本没有 daemon status --json；只读接口是 codex app-server daemon version。
2. 运行中接口返回 running、backend=pid、managedCodexPath、socketPath、cliVersion=0.160.1、appServerVersion=0.160.0。
3. 官方响应没有 PID，因此 before/after.pid 为 null，绝不按进程名猜测 PID，也不读取 socket 正文。
4. 隔离 CODEX_HOME 的未运行实例，version 返回非零连接错误，而非 stopped JSON。实现首先用 symlink_metadata 判断标准控制 socket 缺失，缺失时返回无需刷新；存在但探测失败时保守报告失败。
5. Windows 验证控制目录和 managed daemon 包目录的所有者 SID 等于当前进程用户；同时核对 CODEX_HOME、官方 backend、规范化的受管可执行文件路径、标准 socket 路径与安全版本摘要。
6. 非 Windows 的现存 daemon 暂不声称已完成所有权验证，返回 unsafe；本迭代设计范围为 Windows。

## 已知边界与设计偏差

- PID 字段不可从已验证的官方接口取得，当前以官方管理接口、路径与当前用户所有权作为控制边界。需要维护者确认该替代证据是否满足 PID 要求；不得把 null 记录成“已验证 PID”。
- 官方 restart 不提供“仅当运行才重启”的原子参数。实现初次探测及 restart 前再次检查，已观察到未运行时不调用 restart，但无法原子排除外部在最后检查与 restart 之间停止 daemon 的竞态；此时官方 restart 可能启动 daemon。若要求绝对杜绝此竞态，需 Codex 提供条件重启接口，不能通过杀消费者绕过。
- ready 只表示官方接口重新报告 running，不表示桌面模型选择器已更新，也不证明活动任务未受 daemon restart 影响；任务检测字段为 unknown。
- 锁只协调本 GPTEasy 实例，不控制外部 Codex 命令。对旧/未知协议失败关闭，不输出原始错误。
- 超时仅终止本次新启动的原生控制命令，不结束 daemon、CLI、桌面或 GPTEasy stdio 会话。

## 自动验证

自动测试覆盖：未提交不刷新、刷新失败保留提交主结果、三态均继续桌面操作、桌面失败保留刷新成功、日志分别记录两个结果、未运行不重启、共享锁、ready/命令超时、路径/版本拒绝、当前用户所有权，以及按钮补偿和迟到响应不覆盖结果。

最终命令与通过数量见本次实现回复；全量 Rust 测试中的 Linux shell 黑盒失败需要单独复核环境，不把未通过项标为通过。

## 待完成的真实 Windows 验收

应在可安全重启、无重要任务的独立测试环境中执行，不能重启承载当前 Codex 工作任务的共享 daemon。

- [ ] 未运行 daemon：真实按钮返回无需刷新，操作前后无新 daemon。
- [ ] 运行 daemon：手动刷新成功后模型选择器显示磁盘上的新目录。
- [ ] 供应商切换：自动刷新成功后显示新目录；注入刷新失败后配置与数据库仍保留新供应商，并能手动补偿。
- [ ] 确认重启：刷新成功、失败、超时均继续既有桌面重启；各自展示结果。
- [ ] 桌面重启失败：成功的目录结果不被改写为失败，补偿按钮可用。
- [ ] 实际 CLI 和 GPTEasy stdio 会话未被本功能终止或重启；另外记录共享 daemon 重启对活动任务的实际影响。
- [ ] 记录 daemon 已 ready 但桌面选择器仍过滤/缓存的消费者兼容问题，不归因于供应商配置切换失败。

未完成这些人工项前，不声明真实 Windows 端到端验收通过，不执行发布。
