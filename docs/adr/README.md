# Architecture Decisions

ADR 记录难以逆转且有真实取舍的决定。重建遵循 ADR-0009 的“证据继承、决策重审”：旧 Spike 可以作为事实证据，但只有当前 ADR、`CONTEXT.md`、`docs/ui/UI-SPEC.md`、GitHub Issues 和 grilling 结论定义实现范围。

## Current Baseline

- ADR-0002、0004-0007：技术栈、直接配置、身份、SQLite 和本地模式。
- ADR-0009-0012：重建原则、Windows x64 范围、核心受管对象和验证/切换边界。
- ADR-0013-0016：SQLite 明文凭据、两种模式、无草稿和保存并应用。
- ADR-0017-0019：接管、恢复和外部修改。
- ADR-0021、0023-0024：托盘生命周期、单个未完成操作和数据库恢复。
- ADR-0026、0033-0034：禁止控制用户的 Codex 消费者，允许无窗口启动并严格回收 GPTEasy 自有的会话服务进程，并只通过官方 App Server 管理会话。
- ADR-0027-0031：WSL2 单发行版管理、Linux 命令式凭据、最小 shell 写入协议、共同管理和安全生命周期恢复。
- ADR-0032、0035-0036：会话管理定位为用户交互会话的历史管理器，同时提供归档和永久删除；外部 Codex 消费者运行状态不阻断官方会话修改。
- ADR-0037：Windows 正式发布允许未签名安装包，但必须保留完整性验证并明确披露系统警告风险。
- ADR-0038：应用只信任单一 GitCode Raw 更新端点和内置 updater 公钥；同一构建双平台分发，并在附件匿名验证后最后推进正式清单。

## Historical Or Deferred

- ADR-0001 已被 ADR-0013 取代。
- ADR-0010 仍描述首个 Windows x64 垂直切片的历史边界；其中延期的 WSL2 与 Linux 导出已由 ADR-0027 至 ADR-0031 纳入当前路线。
- ADR-0008 已被 ADR-0011 取代。
- ADR-0020 的被动待重启原则由 ADR-0026 恢复；ADR-0025 已被 ADR-0026 取代。
- ADR-0022 已被 ADR-0037 取代。
- 旧 GSD 管理文件已删除；当前代码曾交付的能力摘要见 [`docs/archive/implemented-features-2026-08-09.md`](../archive/implemented-features-2026-08-09.md)。
