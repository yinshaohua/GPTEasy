# Architecture Decisions

ADR 记录难以逆转且有真实取舍的决定。重建遵循 ADR-0009 的“证据继承、决策重审”：旧 Spike 可以作为事实证据，但只有当前 ADR、`CONTEXT.md`、`docs/ui/UI-SPEC.md` 和活动 GSD 规划定义实现范围。

## Current Baseline

- ADR-0002、0004-0007：技术栈、直接配置、身份、SQLite 和本地模式。
- ADR-0009-0012：重建原则、Windows x64 范围、核心受管对象和验证/切换边界。
- ADR-0013-0016：SQLite 明文凭据、两种模式、无草稿和保存并应用。
- ADR-0017-0020：接管、恢复、外部修改和待重启。
- ADR-0021-0024：托盘生命周期、签名、单个未完成操作和数据库恢复。

## Historical Or Deferred

- ADR-0001 已被 ADR-0013 取代。
- ADR-0003 的 Linux function 属于后续候选，不进入当前路线图。
- ADR-0008 已被 ADR-0011 取代。
- `.planning/archive/pre-rebuild-2026-08-07/` 保存旧 GSD 研究和阶段执行记录，只作证据，不表示完成状态。
