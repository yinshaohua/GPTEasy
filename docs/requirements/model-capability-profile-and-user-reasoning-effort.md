# 模型能力快照与用户思考强度需求整理

- 整理日期：2026-10-02
- 整理基线：远端 `origin/main` 的 GPTEasy 1.4.10（`0780407`）
- 来源：本地未提交改动、`docs/adr/0052-model-capability-profile-and-permissive-unknown-models.md`、`docs/evidence/codexplusplus-profile-model-selection-2026-10-02.md`
- 当前状态：高级 profile、结构化快照和逐模型用户覆盖仍为后续需求；当前先实施免逐模型配置的通用选择器修复（ADR-0053）

## 2026-10-02 当前优先方案

用户明确优先选择简单方案：恢复 Codex内的思考深度选择，不要求逐模型 profile。按 [ADR-0053](../adr/0053-common-reasoning-selector-defaults.md)，当前原生目录统一生成 `low/medium/high/xhigh`、默认 `high`，属于客户端兼容声明，不是已验证能力。真实 Codex 0.159.3对照实验证明空数组取消选项、删除字段使目录解析失败，根级 none本身不会移除非空目录档位。

本次不恢复下述探索代码，不增加数据库快照 schema或 GPTEasy页面控件。下述高级需求保留供后续设计，不能再把“未知能力”解释为必须清空选择器，也不作为本次简单修复的验收条件。根级官方映射、备份和受控应用生命周期沿用现有实现。证据见 [选择器验证](../evidence/codex-reasoning-selector-defaults-2026-10-02.md)。

## 配置更新后的共享后台服务刷新

2026-10-02，本机在新目录已有通用四档时，默认共享后台服务仍跳过思考强度菜单；用户重启同一版本服务后恢复。此问题需要补充服务生命周期与待刷新协调，不以逐模型 profile 或自动升级解决。后续实现按 [共享后台服务配置刷新设计](../design/codex-shared-daemon-refresh.md) 和提议中的 [ADR-0054](../adr/0054-shared-daemon-refresh-and-confirmed-restart.md)安排；本次只记录设计，配置保存仍不自动触发重启。

## 背景

GPTEasy 已经能够从供应商模型发现接口取得模型 ID，并生成当前供应商的 Codex 模型目录。模型发现只能证明供应商列出了模型，不能证明该模型支持哪些思考强度。此前实现容易把模型名称、供应商名称或固定规则误当成能力证据，并且把目录中的可选范围和当前请求实际使用的思考强度混在一起。

新需求分为两层：

1. 为每个已发现模型保存可追溯的能力快照，未知能力保持未知，但不阻止模型被保存、设为默认模型或写入目录。
2. 在供应商页面允许用户为当前默认模型明确选择思考强度；该选择是用户声明，按模型保存，并在重新验证、应用、重启及 WSL2 读取时保持一致。

## 必须保持的行为

### 1. 模型发现与快照

- 供应商 `/models` 返回的模型 ID先去除首尾空白、丢弃空字符串并按原发现顺序精确去重。
- 默认模型必须来自本次发现结果。默认模型在 Codex 目录中排在第一位，其他模型保留发现顺序。
- 模型目录快照绑定供应商和验证指纹，只有供应商验证成功后更新；验证失败继续使用上一次有效快照。
- 快照使用版本化结构化 JSON。建议至少保留：`schema`、模型 ID、发现来源、是否完成能力验证、能力 profile、用户覆盖。
- 旧版字符串数组快照继续兼容读取，并转换为“能力未知”的快照；损坏 JSON 或不支持的 schema 必须报错，不能静默变成空模型列表。

### 2. 未知能力

- 未发现可追溯官方能力证据、且没有用户覆盖的模型，状态为“未知模型”。
- 未知模型仍可以被发现、保存、设为默认模型、切换供应商并写入 Codex 模型目录。
- 未知模型不能因为 ID 含有 `gpt`、`deepseek`、`claude` 等字符串而自动获得 reasoning 档位、上下文窗口、工具或图片能力。
- Codex 目录对未知模型应省略或按已验证 Codex schema 表达未知能力，不能伪造“支持全部档位”。具体字段形态必须以项目实际使用的 Codex 版本验证结果为准。

### 3. 能力来源与优先级

能力来源必须可审计并相互区分：

1. 用户针对具体模型的明确覆盖；
2. 有出处的官方模型能力 profile；
3. 未知能力。

供应商模型发现列表不是能力证据。外部项目的实现方式可以作为调查材料，但不能直接当作 GPTEasy 的官方能力来源。

探索代码已经预留 `source`、`source_ref`、支持档位和默认档位字段，但没有建立真正的官方 profile 数据源。后续实现必须明确 profile 的维护位置、版本、来源链接和更新方式；在此之前，除用户明确覆盖外都应保持未知。

### 4. 用户思考强度覆盖

供应商页面为当前默认模型提供以下选择：

- 自动：清除当前模型的用户覆盖，交由 Codex 或既有供应商映射决定；
- `low`、`medium`、`high`、`xhigh`：保存为该模型的用户声明。

保存接口需要区分三种请求状态：

- 字段未提供：兼容旧调用，保持当前默认模型的覆盖；
- 显式 `null`：清除当前默认模型的覆盖；
- 字符串：设置指定档位，非法值拒绝保存。

覆盖只作用于同一供应商快照中已经存在的模型。切换默认模型时不能把旧模型的覆盖值带给新模型；旧模型的覆盖可以留在快照中，用户切回时恢复。重新验证如果仍然发现同一模型，应保留该模型的覆盖。

### 5. 实际生效值与目录可选范围

必须分开处理：

- Codex 模型目录能力字段描述选择器可见的模型元数据；
- 根级 `model_reasoning_effort` 描述当前默认模型请求的实际生效值。

实际生效值建议按以下顺序计算：

1. 当前默认模型的用户覆盖；
2. 有明确官方依据的供应商或模型映射；
3. 未知时为空，不能继承上一个默认模型的值。

目录的 `supported_reasoning_levels` 不能仅为了写入默认值而伪造，也不能由根级 effort 反向收窄。OpenAI、DeepSeek 等映射规则必须有独立的官方依据、规则 ID和版本记录；没有可靠映射时保持为空并留下可诊断的非敏感状态。

### 6. 保存、重新验证和应用

以下路径必须使用同一套快照和生效值计算：

- 新建并保存供应商；
- 更新已保存供应商；
- 保存并立即应用供应商；
- 供应商重新验证；
- Windows 当前用户 Codex 环境应用；
- WSL2 发行版应用；
- Linux 脚本导出。

应用启动不能因为新增 schema 或发现新模型而静默改写用户配置。只有用户明确保存、重新验证或应用供应商时，才生成新的快照和 Codex 目录。目录、配置、凭据和数据库状态继续遵循现有的备份、原子写入、并发检查、Saga 和失败恢复约束。

### 7. 前端交互

供应商页面在默认模型下方增加“思考强度”选择器，并显示“未知模型默认交给 Codex 判断；明确选择后会作为该模型的用户声明保存”一类说明。

页面状态需要在以下操作中正确刷新：

- 打开已保存供应商；
- 新建供应商并完成模型发现；
- 修改服务地址、API Key或默认模型；
- 重新验证；
- 保存失败后重新编辑；
- 模型从 A 切换为 B。

基于服务地址的默认建议只能是 UI 初始提示，不能替代后端的官方映射和用户覆盖持久化。

### 8. 诊断和失败

至少区分以下阶段：

- 快照读取失败；
- 旧快照兼容迁移；
- schema 不兼容；
- 默认模型不在发现列表；
- 用户档位非法；
- 官方映射缺失；
- Codex 目录生成失败；
- 配置或凭据写入失败；
- Codex 尚未重启而仍在使用旧配置。

日志只能记录脱敏后的供应商/模型标识、快照 schema、能力来源、映射规则 ID、目标 effort和失败阶段，不得记录 API Key、请求正文或完整配置凭据。新增日志应配套回归测试，确保同类失败能够区分关键阶段和状态。

## 探索代码做过的改动范围

这些内容用于后续实现时定位工作面，不代表已经合并：

- `src-tauri/src/provider/model_capability.rs`：新增快照、能力 profile、用户覆盖和三态更新模型。
- `src-tauri/src/provider/catalog.rs`：把快照保存到 `provider_model_catalog`，读取时兼容旧字符串数组，并在供应商摘要中返回当前默认模型的用户覆盖。
- `src-tauri/src/provider/model_catalog.rs`：按快照生成 Codex 目录，默认模型置首，未知能力不生成 reasoning 字段。
- `src-tauri/src/provider.rs`、`src-tauri/src/commands.rs`：为保存、更新和保存并应用命令传递可选 reasoning effort。
- `src-tauri/src/environment.rs`、`src-tauri/src/wsl.rs`：Windows 与 WSL2 从同一快照读取目录和当前 effort。
- `src-tauri/src/provider/reasoning.rs`：用户模型覆盖优先于基于服务地址的既有映射。
- `src-tauri/src/provider/validation.rs`：默认模型 trim 后参与发现校验，模型 ID trim、过滤空值并去重。
- `src/ProviderPage.tsx`、`src/contracts/provider.ts`、`src/messages.ts`、`src/global.css`：增加选择器、三态请求参数和交互提示。
- `src-tauri/tests/provider_workflow.rs`：覆盖覆盖值保存、修改、保持、清除、模型切换不继承、重启和重新验证。

探索实现中还修改了 `CONTEXT.md` 并新增 ADR-0052；这些是需求和领域词汇文档，不属于要恢复的代码。

## 后续实现的验收标准

- 从旧版字符串数组恢复的模型全部能力未知，损坏快照不会被当作空列表接受。
- 未知模型可以保存和应用，但目录不会凭模型名生成 reasoning 档位。
- 当前默认模型总是出现在目录中并排在第一位；发现列表中的重复和空白 ID不会污染目录。
- 用户选择 `low`、`medium`、`high` 或 `xhigh` 后，摘要、数据库快照、Codex 目录和实际根级 effort 的语义一致。
- 选择“自动”可以清除当前默认模型的覆盖；省略参数可以保留已有覆盖。
- 默认模型从 A 切到 B时，A的覆盖不泄漏到 B；切回 A时可按设计恢复 A的覆盖。
- Windows、WSL2和Linux导出对同一供应商快照得到一致的模型能力和实际 effort结果。
- 保存、重新验证、应用或重启后，不能出现数据库、目录和配置只更新一部分的状态。
- 诊断日志能区分快照、映射、目录生成和写入阶段，且不泄露敏感信息。

## 相关文档

- [`docs/adr/0049-provider-model-catalog-and-reasoning-capabilities.md`](../adr/0049-provider-model-catalog-and-reasoning-capabilities.md)：供应商模型发现、模型目录和实际 effort 的分层。
- [`docs/adr/0051-reasoning-effort-and-selector-range.md`](../adr/0051-reasoning-effort-and-selector-range.md)：实际生效值与 Codex 可选范围的边界。
- [`docs/adr/0052-model-capability-profile-and-permissive-unknown-models.md`](../adr/0052-model-capability-profile-and-permissive-unknown-models.md)：模型能力 profile 和未知模型的决策草案。
- [`docs/evidence/codexplusplus-profile-model-selection-2026-10-02.md`](../evidence/codexplusplus-profile-model-selection-2026-10-02.md)：外部项目源码调查证据，不是 GPTEasy 的实现规范。
