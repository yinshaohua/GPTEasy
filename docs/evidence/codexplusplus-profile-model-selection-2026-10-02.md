# CodexPlusPlus profile 模型选择与思考强度源码证据

## 调查基线

- 仓库：[`BigPizzaV3/CodexPlusPlus`](https://github.com/BigPizzaV3/CodexPlusPlus)。
- 调查日：2026-10-02。
- 核实的 `main` 最新 commit：`27d50a1a0413b3c445bc95fae081f16b6c7edbf0`，提交时间 2026-10-01 23:37:35 +0800（15:37:35 UTC）。以下链接全部固定到该 commit。
- 固定源码基线：[commit `27d50a1`](https://github.com/BigPizzaV3/CodexPlusPlus/tree/27d50a1a0413b3c445bc95fae081f16b6c7edbf0)。

本文只记录 CodexPlusPlus 源码行为，并与 GPTEasy ADR-0049、ADR-0051 做边界清晰的对照；不把 CodexPlusPlus 的启发式实现当作 GPTEasy 的实现要求，也不提出代码修改。

## 结论摘要

1. `model` 是 profile 的当前默认模型；`modelList` 是候选模型文本。生成 Codex catalog 时，当前 `model` 被放在第一位，`modelList` 条目随后加入并去重，因此当前模型即使不在 `modelList` 也会进入 catalog。
2. profile 的 `model` 为空时，CodexPlusPlus 在写入配置前从 `modelList` 第一条非空模型推导默认 `model`，并剥离自己的窗口后缀。
3. `modelMetadata` 不是模型发现列表，也不会新增模型条目。它按规范化 slug 覆盖已经生成的 catalog entry；窗口和压缩字段由专门字段保护。
4. 管理器返回的 `modelMetadata` 是 UI 元数据投影，不等于 Codex 实际加载的完整 catalog。它从兼容层、运行时缓存或 bundled metadata 读取 `supported_reasoning_levels`，映射成 UI 的 `supportedReasoningEfforts`。
5. Codex catalog 的可见模型来自 `/models` 来源与配置 `model_catalog_json` 的合并，并按 `supported_in_api` 与 `visibility == "list"` 过滤。写入 catalog 不证明供应商请求兼容。
6. 请求实际的 reasoning 处理在协议代理中独立进行：Chat 兼容路径按模型名推断 DeepSeek、Qwen、Kimi、MiniMax、StepFun 等方言，再把 Codex effort 映射为上游字段。源码没有用 profile 的 `modelMetadata` 验证供应商是否真的接受该参数。

## 1. profile、默认模型与 Codex catalog

### 1.1 字段定义

`RelayProfile` 使用 camelCase 序列化；`model` 是当前模型，`modelList` 是模型列表，`modelMetadata` 是 JSON map 字符串形式的每模型元数据覆盖。证据：[`settings.rs:26-89`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/settings.rs#L26-L89)。

这三个字段职责不同：`model`/`modelList` 提供候选 slug，`modelMetadata` 提供已存在条目的字段覆盖；字段定义不声明供应商官方模型能力。

### 1.2 `modelList` 如何影响可选模型

`collect_catalog_entries` 将 `model_list` 按换行、逗号拆分，去空、去重并解析窗口后缀；随后单独处理当前 `model`，把它放在第一位，并移除列表中同 slug 的重复项。证据：[`model_suffix.rs:107-177`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_suffix.rs#L107-L177)。

因此 `modelList` 通常决定 profile 生成的 Codex catalog 候选集合，但“生成条目”只证明 CodexPlusPlus 写入了模型，不证明模型已通过供应商 Responses、工具或 reasoning 验证。

写入 `config.toml` 前，如果 `model` 为空而 `modelList` 有非空项，代码取第一项作为默认模型；同时剥离 `[窗口]` 后缀，因为 Codex 不理解该后缀。证据：[`relay_config.rs:3727-3745`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/relay_config.rs#L3727-L3745)。

### 1.3 catalog 构建与未知模型

builder 优先使用显式模板或模型对应的 resolved metadata；找不到时使用 bundled catalog 第一条 entry 作为模板，再覆盖 slug、显示名、窗口、优先级、可见性等字段。证据：[`model_suffix.rs:506-608`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_suffix.rs#L506-L608)；[`model_suffix.rs:622-629`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_suffix.rs#L622-L629)。

因此新官方模型不在本地 metadata 时，可能仍被列入 catalog，但能力字段可能来自 bundled 模板或用户覆盖。这是“生成可供 Codex 读取的条目”，不是新模型官方能力证明。

## 2. `modelMetadata` 的作用边界

应用 profile 时，代码先解析列表和窗口，生成 catalog，再应用 metadata，最后写入 profile 对应的 `model-catalogs/*.json` 并写入 `model_catalog_json` 指针。证据：[`relay_config.rs:2332-2369`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/relay_config.rs#L2332-L2369)；[`relay_config.rs:2479-2496`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/relay_config.rs#L2479-L2496)。

metadata key 会去后缀并转为 ASCII 小写；覆盖阶段只遍历已经存在的 `models` 数组，所以不匹配已有 entry 的 key 不会创建新模型。证据：[`relay_config.rs:2556-2580`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/relay_config.rs#L2556-L2580)；[`relay_config.rs:2769-2803`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/relay_config.rs#L2769-L2803)。

覆盖明确跳过 `slug`、`context_window`、`max_context_window`、`auto_compact_token_limit`；reasoning 相关字段若存在于 metadata map，则可能被覆盖到 catalog entry，但这只是 CodexPlusPlus 写入的能力描述，不是上游请求兼容性证明。证据：[`relay_config.rs:2791-2800`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/relay_config.rs#L2791-L2800)。

## 3. UI 元数据、Codex catalog、请求兼容性

### 3.1 UI 元数据

管理器把 `modelList` 与当前 `model` 合并去重，并为每个模型调用 `model_ui_metadata`。证据：[`model_catalog.rs:83-155`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_catalog.rs#L83-L155)。实际配置读取路径还会抓取模型来源、读取 `model_catalog_json`、合并去重并按可见性过滤。证据：[`model_catalog.rs:158-260`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_catalog.rs#L158-L260)；[`model_catalog.rs:844-943`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_catalog.rs#L844-L943)。

`model_ui_metadata` 将 metadata 的 `supported_reasoning_levels` 投影为 UI 的 `supportedReasoningEfforts`，并把 `default_reasoning_level` 投影为 `defaultReasoningEffort`。证据：[`model_suffix.rs:324-365`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_suffix.rs#L324-L365)。解析优先级是兼容层、运行时缓存、bundled；兼容层叠加在运行时或 bundled 模板上。证据：[`model_suffix.rs:402-435`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_suffix.rs#L402-L435)。前端只校验数组和字符串形状，不验证上游官方列表。证据：[`model-metadata.ts:786-808`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/apps/codex-plus-manager/src/model-metadata.ts#L786-L808)。

### 3.2 Codex catalog 与可见性

配置 catalog 中只有 `supported_in_api` 不为 false 且 `visibility` 为 `list` 的条目会被纳入管理器模型列表。证据：[`model_catalog.rs:844-943`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_catalog.rs#L844-L943)。这影响 Codex/管理器可见候选，但不证明 wire 格式或供应商能力。

### 3.3 三层边界

| 层 | 源码事实 | 能说明什么 | 不能说明什么 |
|---|---|---|---|
| UI 元数据 | `model_ui_metadata`、前端 validator | 显示名、描述、默认/可选 reasoning 档位 | 不证明供应商接受该档位 |
| Codex catalog | `model_catalog_json`、`models[].slug`、可见性 | Codex 可读取哪些模型及其元数据 | 不证明请求 wire 格式或上游能力 |
| 请求实际兼容性 | `protocol_proxy.rs` 的协议转换 | CodexPlusPlus 实际发送哪些 reasoning 字段 | 不会自动修正 UI/catalog 中过时的列表 |

## 4. 新官方模型不在 profile 时的影响

1. **只是不在 `modelList`，但设为当前 `model`**：仍会被 catalog 收集并置首；profile 管理器视图也会把当前模型合并进模型 ID。证据：[`model_suffix.rs:150-176`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_suffix.rs#L150-L176)；[`model_catalog.rs:135-146`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_catalog.rs#L135-L146)。
2. **不在 profile，但来自供应商 `/models`**：管理器会把环境模型来源与 catalog 合并，因此可能出现在管理器列表；不等于它进入 profile 生成的 catalog。证据：[`model_catalog.rs:197-224`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/model_catalog.rs#L197-L224)。
3. **不在 profile、`/models`、配置 catalog，且无内置 metadata**：源码没有自动出现在选择器的保证。直接写入当前模型字符串也不等于获得 catalog 能力或实际请求兼容性。
4. **不在 profile metadata**：不会阻止模型进入 catalog；但 reasoning 元数据只能来自兼容层、运行时缓存、bundled 或其他覆盖。新官方模型未更新这些来源时，显示能力可能缺失或继承模板。

## 5. 官方 reasoning 列表不同时源码如何处理

### 5.1 请求映射

Chat 兼容路径先判断是否请求 reasoning，再按方言写字段；例如 Kimi 使用 `thinking.type`，只有特定模型才写 `reasoning_effort`。证据：[`protocol_proxy.rs:5812-5874`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/protocol_proxy.rs#L5812-L5874)。

`infer_chat_reasoning_style` 按模型名推断 DeepSeek、Qwen、Kimi、MiniMax、StepFun 等方言，未知模型回到 `Default`。证据：[`protocol_proxy.rs:5888-5919`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/protocol_proxy.rs#L5888-L5919)。`map_chat_reasoning_effort` 对方言使用不同列表和折叠：DeepSeek 的 `max`/`xhigh` -> `max`、其他非关闭值 -> `high`；LowHigh 只保留 `low`/`high`；OpenRouter 支持 `minimal` 到 `xhigh`；Thinking 将 `minimal/low` -> `low`、`medium/high` -> `high`、`xhigh/max` -> `max`。证据：[`protocol_proxy.rs:5921-5960`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/27d50a1a0413b3c445bc95fae081f16b6c7edbf0/crates/codex-plus-core/src/protocol_proxy.rs#L5921-L5960)。

### 5.2 不一致的含义

- UI 列表不同：管理器可能显示 bundled/兼容层列表；前端只做形状校验。
- catalog 列表不同：Codex 选择器所见范围可能受 `supported_reasoning_levels` 影响；这是元数据消费行为，不是请求兼容性验证。
- 请求列表不同：代理可能把 Codex effort 折叠为上游值，或因方言/模型条件不满足而不发送字段；最终由上游响应决定。

源码没有在这三层之间建立统一的“官方列表不一致即报错”机制，而是分别继续运行。这是 CodexPlusPlus 行为，不应直接当作 GPTEasy 设计。

## 6. 与 GPTEasy ADR-0049、ADR-0051 对照

GPTEasy ADR-0049 将供应商 `/models` 发现列表、Codex catalog、能力元数据分层，并要求未知模型不凭名称虚构能力。这个分层与本调查看到的 CodexPlusPlus 代码相吻合，但“未知能力的保守声明”“只为当前活动供应商生成 catalog”等是 GPTEasy ADR 的决策，不是本仓库源码事实。CodexPlusPlus 自身明确存在 profile 列表、catalog builder、UI metadata 和请求代理，且这些路径并非同一个能力事实源。

参考：[`docs/adr/0049-provider-model-catalog-and-reasoning-capabilities.md`](../adr/0049-provider-model-catalog-and-reasoning-capabilities.md)。

GPTEasy ADR-0051 区分“根级实际生效 effort”和“Codex 选择器可选范围”，并禁止仅按模型名猜测供应商能力。CodexPlusPlus 的 `infer_chat_reasoning_style`、`map_chat_reasoning_effort` 证明请求映射确实可能是独立的模型名启发式；但 ADR-0051 关于未知供应商不猜测、显式映射和不反向收窄选择器的规则，只是 GPTEasy 的比较启示，不是 CodexPlusPlus 已实现行为。

参考：[`docs/adr/0051-reasoning-effort-and-selector-range.md`](../adr/0051-reasoning-effort-and-selector-range.md)。

## 最终判断

- `model` 决定当前默认模型，并在 catalog 生成时置首；为空时可由 `modelList` 第一项回落。
- `modelList` 决定 profile 生成 catalog 的候选条目集合；不等同于供应商完整模型能力或请求成功。
- `modelMetadata` 只覆盖已生成条目的 catalog/UI 能力字段，不能凭空注册新模型；其 reasoning 列表会影响元数据投影/选择器所见，但不验证上游。
- 新官方模型可能因当前 `model`、供应商 `/models` 或配置 catalog 进入可见列表；没有任何来源时没有自动出现保证。
- 官方 reasoning 列表若与内部列表不同，UI、Codex catalog、请求兼容性可能分别表现不同；请求代理的模型名启发式不能作为 GPTEasy 的官方能力证据。
