---
status: accepted
---

# 按当前供应商生成 Codex 模型目录并按模型选择推理档位

GPTEasy 的供应商模型发现和 Codex 的模型元数据是两种不同的数据。供应商验证目前请求供应商的 `/models` 接口，并从响应中取得模型 ID；`model_providers` 只定义请求端点，不会把这些 ID 注册为 Codex 的模型。Codex 找不到自定义模型的元数据时会使用内置目录的 fallback，因此兼容模型会出现 `Model metadata ... not found`，也不会出现在 `/model` 选择器中。

## 决策

### 分离三层模型数据

1. **供应商模型发现列表**是某个供应商 `/models` 返回的候选模型 ID。它只证明模型可以被该供应商列出，不证明上下文窗口、工具、Responses 或推理能力。
2. **模型目录快照**保存一次完整供应商 `/models` 得到的发现列表，并绑定供应商 ID 与验证指纹。验证闭环只针对用户选定的默认模型；验证成功时把本次发现的全部模型写入快照，并在响应遗漏默认模型时补入默认模型。只有重新验证成功才更新；失败时继续使用上一份快照。
3. **Codex 模型元数据目录**是当前活动供应商提供给 Codex 的模型及能力描述。GPTEasy 根据模型目录快照和模型能力元数据生成它，并通过 Codex 支持的 `model_catalog_json` 指针让 Codex 在启动时读取。

这三层不互相冒充：不能把 `/models` 的 ID 列表直接当作 Codex 完整模型元数据，也不能把 Codex 的内置 OpenAI 模型目录当作兼容供应商目录。

### 只为当前活动供应商生成目录

Codex 的 catalog 是当前环境的全局选择目录，不携带“模型 ID 到供应商端点”的切换映射。因此 catalog 只包含当前活动、已验证供应商的模型。切换供应商时，GPTEasy 原子更新该供应商的活动 catalog 和路由配置，并要求 Codex 消费者在重启后读取新目录。

不把所有已保存供应商的模型合并到一个全局列表：同名模型会冲突，用户在 `/model` 中选择后也无法仅凭模型 ID 把请求路由到正确的供应商。

### 目录条目的能力来源

目录生成必须产出当前 Codex 版本要求的完整条目，而不是只写入 `slug`。目录条目代表“已发现”，不代表该模型单独完成了 Responses 或工具调用验证。能力元数据按以下顺序取得：

- 已知供应商/模型的静态能力 profile；
- 随当前 Codex 版本提供的 bundled 条目作为结构模板，再覆盖真实供应商模型 ID 和已知能力；
- 对未知模型使用完整但保守的兼容模板，并在 GPTEasy 中标记“能力未识别”。

未知模型可以出现在 `/model`，但它仍只是“已发现”，不因此获得虚构的上下文窗口、工具或推理能力。供应商验证的 Responses 流式工具闭环继续以用户选定的默认模型为准。

### 目录能力与实际推理强度

供应商验证时用户选定的模型就是该供应商的默认模型，但模型发现只证明供应商列出了该模型，不证明其推理能力。GPTEasy 不再根据模型 ID 或模型家族生成 `low`、`medium`、`high`，也不把供应商名称或 `/models` 响应当作 `supported_reasoning_levels` 的证据。

当前 Codex 版本要求 catalog 条目保留 `default_reasoning_level` 和 `supported_reasoning_levels` 字段，因此未知能力条目使用 `default_reasoning_level: null` 与 `supported_reasoning_levels: []`。这表示 GPTEasy 没有能力声明，不把空数组解释为“无限制”；字段契约和未知模型 fallback 的版本证据记录在 [`docs/evidence/codex-model-catalog-contract-2026-09-27.md`](../evidence/codex-model-catalog-contract-2026-09-27.md)。

根级 `model_reasoning_effort` 是当前默认模型请求使用的实际 effort，与 catalog 的能力字段分离。只有存在可追溯的供应商官方等效定义时才写入映射值：OpenAI 和 DeepSeek 当前均映射为 `high`；未知或未确认的供应商省略该字段。映射规则由 ADR-0051 统一维护，不散落在 Windows、WSL2 或 Linux 导出分支中。

### 生命周期和失败恢复

- 供应商保存或重新验证成功后，更新模型目录快照和该供应商的 catalog；本次 `/models` 返回的全部模型都进入活动目录，并确保默认模型有目录条目，不能只保留验证闭环实际调用的默认模型。
- 目录文件、配置指针和路由配置属于同一次受管配置变更，纳入既有备份、原子写入、并发检查、失败回滚和待重启状态。
- 生成失败、Codex schema 不兼容或切换中断时，保留上一份可用 catalog 和环境实际状态，不留下半成品指针。
- catalog 不保存 API Key；模型 ID、显示名和供应商扩展元数据必须按不可信输入序列化，不能影响路径或配置边界。
- 旧版 GPTEasy 管理区块可能缺少 `model_catalog_json`，而外部 Codex 配置仍在根级保留同名路径。下一次用户明确应用供应商时，迁移必须先移除这个旧根级键，再由现行管理区块写入 `gpteasy-model-catalog.json`；不得生成重复 TOML 键，也不得因为该兼容场景误报“无法安全迁移”。

## 结果与取舍

该决策消除已登记兼容模型的 metadata fallback，并使 `/model` 能列出当前供应商的已发现模型；同时把供应商模型发现、Codex catalog 能力声明和根级实际 effort 分开。代价是 GPTEasy 必须维护版本相关的 catalog schema 和有官方依据的供应商 effort 映射；未知模型保持能力未知，模型列表变化仍需通过重新验证刷新，Codex 消费者需要重启后才能看到目录更新。

本 ADR 不要求每次 Codex 启动都实时请求供应商 `/models`，不从 `/models` 猜测工具或推理能力，也不把不同供应商的模型合并为一个无路由命名空间。
