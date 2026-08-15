# Architecture Decisions

ADR 记录难以逆转且有真实取舍的决定。重建遵循 ADR-0009 的“证据继承、决策重审”：旧 Spike 可以作为事实证据，但只有当前 ADR、`CONTEXT.md`、`docs/ui/UI-SPEC.md`、GitHub Issues 和 grilling 结论定义实现范围。

## Current Baseline

- ADR-0002、0004-0007：技术栈、直接配置、身份、SQLite 和本地模式。
- ADR-0009-0012：重建原则、Windows x64 范围、核心受管对象和验证/切换边界。
- ADR-0013-0016：SQLite 明文凭据、两种模式、无草稿和保存并应用。
- ADR-0017-0019：接管、恢复和外部修改。
- ADR-0021-0024：托盘生命周期、签名、单个未完成操作和数据库恢复。
- ADR-0026：移除所有主动桌面控制入口。
- ADR-0027-0031：WSL2 单发行版管理、Linux 命令式凭据、最小 shell 写入协议、共同管理和安全生命周期恢复。

## Historical Or Deferred

- ADR-0001 已被 ADR-0013 取代。
- ADR-0010 仍描述首个 Windows x64 垂直切片的历史边界；其中延期的 WSL2 与 Linux 导出已由 ADR-0027 至 ADR-0031 纳入当前路线。
- ADR-0008 已被 ADR-0011 取代。
- ADR-0020 的被动待重启原则由 ADR-0026 恢复；ADR-0025 已被 ADR-0026 取代。
- 旧 GSD 管理文件已删除；当前代码曾交付的能力摘要见 [`docs/archive/implemented-features-2026-08-09.md`](../archive/implemented-features-2026-08-09.md)。
