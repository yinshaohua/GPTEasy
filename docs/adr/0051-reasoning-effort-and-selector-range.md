---
status: accepted
---

# 解耦思考强度生效值与 Codex 可选范围

此前 GPTEasy 曾为已识别的模型按模型家族生成固定的 `low`、`medium`、`high` 三档，并把结果写入 Codex 模型目录的 `supported_reasoning_levels`。该目录通过 `model_catalog_json` 被 Codex 读取，因此这项生成逻辑不只是描述默认值，还会改变 Codex 模型选择器可见的思考强度范围。

这混淆了两个不同问题：

1. **思考强度生效值**：当前模型请求实际使用的强度，由 `model_reasoning_effort` 表达。
2. **思考强度可选范围**：Codex 根据模型元数据和自身版本能力决定用户可选择的档位，不应由 GPTEasy 根据名称猜测并人为收窄。

本 ADR 的实现已落地。真实 Codex 选择器 UAT 和真实 DeepSeek Responses 请求仍属于独立验收，不在本次源码/schema 证据中宣称通过。

Codex 0.157.1 的 schema 验证记录见 [`docs/evidence/codex-model-catalog-contract-2026-09-27.md`](../evidence/codex-model-catalog-contract-2026-09-27.md)。当前实现采用完整 catalog 条目、`default_reasoning_level: null` 和 `supported_reasoning_levels: []`，语义是能力未知并匹配 Codex 未知模型 fallback，不是“无限制”标记。

## 决策

### 目录能力与实际生效值分离

GPTEasy 生成的 Codex 模型目录不得再根据模型家族无条件伪造 `low`、`medium`、`high`，也不得因为当前默认值是 `high` 就把其他 Codex 支持的档位排除在选择器之外。

模型目录中的 `supported_reasoning_levels` 必须遵循 Codex 当前版本要求的目录契约：

- 如果 Codex 明确要求该字段存在，则使用 Codex 官方允许的“不收窄选择范围”的表示；
- 如果当前 Codex 版本没有这种表示，则 GPTEasy 不得继续伪造供应商模型的完整能力范围，应优先移除 GPTEasy 对该字段的自定义覆盖，并让 Codex 使用其自身模型元数据和默认行为；
- 不得使用空数组作为“无限制”的隐含约定，除非当前 Codex 版本的官方 schema 明确规定空数组具有该语义；
- 不得把 `/models` 返回结果、模型名称或供应商名称当作推理能力证据；
- 不得为了让默认值 `high` 能写入而补造 `supported_reasoning_levels`。

实现必须先针对项目实际捆绑的 Codex 版本验证目录 schema 和选择器行为，再选择具体序列化方案。当前已按 Codex CLI 0.157.1 的 schema 使用完整条目、`default_reasoning_level: null` 和 `supported_reasoning_levels: []`；不能仅凭字段名称推断“省略字段”或“空数组”一定代表不限制。

### 根级配置只表达当前实际生效强度

GPTEasy 对受管 Codex 环境写入的 `model_reasoning_effort` 只表示当前默认模型的实际生效值，不承担能力声明、选择范围或供应商验证等级的职责。

供应商应用时必须按以下规则决定该值：

- OpenAI 模型：设置为 `high`；
- DeepSeek 模型：按照 DeepSeek 官方 Codex/Responses 兼容定义，将 OpenAI `high` 映射为 DeepSeek `high`；
- 其他供应商模型：只有存在可追溯的官方等效定义时才写入映射后的 Codex effort；
- 没有可靠官方映射，或供应商协议不支持 Codex 统一 effort 语义时，不得根据模型名称猜测并写入 `medium`、`high` 或其他值；应采用该供应商官方推荐的兼容配置，或保留为空并把原因记录为可诊断的非敏感状态。

本 ADR 不要求 GPTEasy 在配置中新增供应商原生思考参数。当前架构继续使用 Codex 的统一 `model_reasoning_effort`；供应商原生参数只有在 Codex 官方兼容协议要求且另有 ADR 决策时才引入。

### DeepSeek 的推荐语义

DeepSeek 通过 OpenAI-compatible Responses/Codex 方式接入时，采用统一 Codex 字段：

```toml
model_reasoning_effort = "high"
```

其含义是当前 DeepSeek 请求使用 DeepSeek 官方兼容定义中的 `high` 思考强度，不是把 DeepSeek 的可选范围限制为 `high`，也不是声称所有 DeepSeek 模型都支持相同档位。

DeepSeek 的 `max` 不能在 GPTEasy 中被当作 OpenAI `high` 的默认替代值。只有用户或后续明确的供应商映射规则要求最高强度时，才可以使用对应的 Codex effort 值；默认目标是与 OpenAI `high` 等效，而不是自动选择供应商的最高强度。

### 供应商映射规则

供应商映射必须是显式、可审计、可版本化的数据，而不是散落在模型名称判断分支中的启发式规则。每条映射至少包含：

- 供应商或协议身份；
- 官方文档或官方兼容说明的版本/发布日期；
- Codex effort 输入值；
- 供应商实际生效值；
- 不支持、未知或冲突时的处理方式。

映射表只能决定当前配置写入的生效值，不能自动生成或裁剪 Codex 选择器的可选范围。模型目录能力描述和供应商官方验证证据仍需分开保存。

#### DayWay-DS 精确兼容 profile

\`DayWay-DS\` 使用版本化规则 \`dayway-ds-deepseek-effort-v1\` 将 Codex 的 \`high\` 生效值映射到该供应商的兼容接口。该规则只在供应商名称不区分大小写匹配 \`DayWay-DS\` 时启用，并只对精确模型 ID \`deepseek-v4-flash\` 和 \`deepseek-v4-pro\` 生效；不会根据任意模型名包含 \`deepseek\` 推断能力，也不会影响普通 \`DayWay\` 或预览模型。

当 Codex 离线 \`model/list\` 将上述模型报告为 \`not_found\` 时，能力快照才允许由该兼容 profile 补充 \`low\`、\`medium\`、\`high\`，默认值为 \`high\`，来源记录为 \`vendor_compatibility\`。这条兜底同时写入脱敏审计证据，区分 Codex 原生能力和供应商兼容声明。

### 未知模型与模型切换

- 未知模型仍可进入当前供应商目录，但不能仅凭模型 ID 声明推理档位；
- 模型 ID 包含 `gpt`、`deepseek`、`claude` 等字符串不构成官方能力证据；
- 用户在 Codex 中切换模型后，GPTEasy 写入的根级 `model_reasoning_effort` 不应被解释为已为新模型自动完成官方映射；
- 如果 Codex 将该根级值应用于新模型，而新模型不接受该值，错误应由 Codex/供应商返回并可诊断；后续若需要按模型切换自动重算，必须另行设计模型级配置生命周期；
- GPTEasy 不得因为无法确认新模型能力而偷偷收窄选择器范围。

### 跨环境一致性

同一已验证供应商和默认模型的思考强度语义必须在以下路径保持一致：

- Windows 当前用户 Codex 配置；
- WSL2 发行版 Codex 配置；
- 独立 Linux 导出脚本及其 Bash/Zsh 写入路径。

这些路径必须共享同一套映射和目录渲染规则，不得继续出现桌面路径按模型推导、WSL2 固定写 `high`、Linux 脚本按另一套家族规则写入的分叉行为。导出脚本只能写入由快照确定的生效值，不得在 shell 中重新猜测模型家族。

### 生命周期、迁移和失败恢复

- 新建、重新验证、切换、受控重建和明确确认的兼容迁移，使用新规则生成目录和配置；应用启动不得静默改写现有用户配置；
- 旧版目录中由 GPTEasy 生成的三档 `supported_reasoning_levels` 不应被继续视为供应商能力证据；迁移时只有在能严格证明目录是 GPTEasy 旧模板且未被外部修改时，才允许由用户确认后更新；否则保留原文件并要求重新应用供应商；
- 配置、目录、凭据和数据库状态仍纳入现有备份、原子写入、并发检查、Saga 和失败恢复；
- 目录 schema 不兼容、映射缺失或生成失败时，整个配置切换失败并保留上一份可用状态，不写入半成品；
- Codex 消费者重启后才读取新的目录和根级配置，GPTEasy 不静默终止 Codex CLI；
- 诊断日志只记录供应商/模型标识的脱敏值、映射规则 ID、目标 effort、目录 schema 分支和失败阶段，不记录 API Key、请求内容或完整凭据。

## 不在本 ADR 范围内

- 为 Anthropic、Gemini、Qwen 或其他供应商新增原生 thinking budget/extended thinking 请求结构；
- 通过模型名称推断上下文窗口、工具、图片或推理能力；
- 修改 OpenAI 登录模式；
- 让多个供应商的模型合并到同一个可路由的全局模型目录；
- 自动升级 Codex 或依赖未验证的未来目录字段语义。

## 已实施契约

1. 已固定并验证项目实际使用的 Codex CLI 0.157.1 的模型目录 schema。
2. 已删除 `src-tauri/src/provider/model_catalog.rs` 中按模型家族生成 `low/medium/high` 的逻辑。
3. 已使用 `null` 和空数组表达未知能力，且不把空数组解释为无限制范围。
4. 已建立共享的供应商 effort 映射模块，实现 OpenAI 和 DeepSeek 的 `high` 映射，缺失映射时省略根级 effort。
5. Windows、WSL2、Linux 导出共用同一生效值计算结果；shell 不再按模型名称重新推导。
6. 已保留根级 `model_reasoning_effort` 的唯一性，并在旧管理区块迁移时清理过时字段。
7. 已补充目录渲染、映射、迁移、跨环境一致性和失败阶段审计的回归测试。
8. 供应商验证闭环保持不变：模型发现不等于推理能力验证，目录声明不等于供应商官方认证。

## 验收标准

- Codex 选择器不再因 GPTEasy 的固定三档生成逻辑而失去 Codex 官方支持的其他档位；
- OpenAI 默认模型应用后实际配置为 `model_reasoning_effort = "high"`；
- DeepSeek 默认模型应用后实际配置为 `model_reasoning_effort = "high"`，并可在真实 Responses/Codex 请求中验证其对应 DeepSeek `high`；
- 未知模型不会因名称匹配获得虚构的 `medium` 或 `high`；
- 没有官方等效映射的供应商不会被静默写入猜测的强度；
- Windows、WSL2、Bash 和 Zsh 生成的配置对同一供应商和模型使用相同的生效值；
- 重新应用供应商后目录和配置按新规则更新，旧目录或人工修改目录不会被静默覆盖；
- 配置切换失败时模型目录、根级配置、凭据和数据库状态保持上一致状态；
- 诊断证据能区分“目录 schema 不兼容”“映射缺失”“配置写入失败”“Codex 尚未重启”等阶段，且不泄露敏感数据；
- OpenAI 登录模式、现有供应商验证和非相关模型目录能力保持不变。

## 结果与取舍

该设计消除了 GPTEasy 以模型家族名义限制 Codex 思考强度选择范围的问题，同时保留对默认实际生效值的明确控制。DeepSeek 采用与 OpenAI `high` 等效的 `high`，不会错误地把最大强度 `max` 当成默认值。

代价是 GPTEasy 必须针对实际 Codex 版本验证目录 schema，并维护有官方出处的供应商映射；对于没有可靠官方定义的供应商，系统可能无法自动写入推理强度，需要沿用供应商默认行为或等待后续兼容设计。这比把未知模型静默伪造成三档更可审计，也更符合能力声明与配置生效值分离的原则。

## 后续规划（未实施）

2026-10-06 的本机差分证据表明，统一空范围会覆盖 Codex 对已知模型的可选档位。
当前实现已按 [推理档位能力发现与兼容兜底方案](../plans/reasoning-capability-discovery-2026-10-06.md) 接入精确模型元数据，并新增上述 DayWay-DS 显式兼容 profile；未知模型仍保持明确降级。真实安装包 UI 复测和真实供应商请求仍需独立验收。
