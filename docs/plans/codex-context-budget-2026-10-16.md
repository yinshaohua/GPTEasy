# Codex 模型上下文与自动压缩优化方案

- 初稿：2026-10-16；修订：2026-10-17
- 状态：已实现统一默认策略，并通过 Windows 与 WSL2 内 Linux Codex 隔离验收；维护者人工测试已通过，随 v1.5.2 发布
- 适用范围：当前用户 Codex 环境、WSL2、独立 Linux 导出
- 相关约束：[ADR-0049](../adr/0049-provider-model-catalog-and-reasoning-capabilities.md)、[ADR-0051](../adr/0051-reasoning-effort-and-selector-range.md)

## 1. 决策

GPTEasy 对全部已发现模型使用统一的上下文默认配置，取值复用本机 gpt-6.1-sol 的
当前 Codex 默认元数据。不根据模型名推断容量，不要求逐模型测试，不要求未知模型
补充声明或手工预算，不新增容量探测链路。供应商验证仍只对用户选定的默认模型
执行既有 Responses 流式工具闭环，其他模型来自发现列表。

这是用户明确选择的客户端运行默认策略，不是供应商容量的实测结论。该策略覆盖
此前初稿中的“精确元数据优先、未知模型要求预算”的建议；按用户要求采用统一缺省值。

## 2. 设置放在哪里

设置分为定义位置和生效位置：

- 定义位置：GPTEasy 的共享模型目录生成模块
  `src-tauri/src/provider/model_catalog.rs`，集中定义一组默认值。
- 生效位置：生成的 `gpteasy-model-catalog.json` 的每个 `models[]` 条目。
  每个模型写入相同默认值；按条目输出不意味着要逐模型探测或维护数值。
- `config.toml` 只继续写入 `model_catalog_json` 路径指针；正常生成流程不新增根级
  `model_context_window` 或 `model_auto_compact_token_limit`。
- 第一版作为内置默认生效，不增加供应商填写项，不新增数据库模型容量表。
  如以后需要允许用户调整，可在设置中提供统一的高级选项，再复用同一生成入口。

模型发现列表、验证指纹、推理能力快照和当前推理档位合并逻辑保持各自职责。

## 3. 默认值

| 模型条目字段 | 默认值 | 语义 |
| --- | ---: | --- |
| context_window | 272000 | 采用的默认窗口 |
| max_context_window | 872000 | 沿用参考模型的最大窗口元数据，不自动扩大当前窗口 |
| effective_context_window_percent | 95 | 沿用 Codex 的可用比例 |
| auto_compact_token_limit | null | 根据已知窗口交由目标 Codex 推导自动压缩阈值 |

完整模型条目继续保留现有必需字段，新增字段片段如下：

```json
{
  "slug": "任意发现的模型ID",
  "context_window": 272000,
  "max_context_window": 872000,
  "effective_context_window_percent": 95,
  "auto_compact_token_limit": null
}
```

这里 JSON 使用 `context_window`，不是 TOML 根级的 `model_context_window`。
272000 × 95% = 258400 是可用窗口参考值。按已核对的上游算法，自动压缩阈值参考值
为 272000 × 90% = 244800；目标定制构建的实际触发点仍须验收，不能记为已实测。
未知模型也写入上述默认窗口，因而不再因 GPTEasy 省略容量字段而保持容量未知。
统一默认值不构成所有供应商均支持该容量的保证；后续有实际不兼容反馈时可再设计
供应商级覆盖，本轮不引入逐模型配置。

## 4. 最小实现范围

1. 扩展 `ModelCatalogEntry`，增加上述四个字段；默认值只在共享生成模块定义一次，
   `entry()` 为每个模型赋相同值。
2. 保持目录排序、去重、默认模型补入和推理档位解析逻辑。默认容量不依赖推理快照
   是否可用；未知模型、快照缺失或探测失败仍获得相同容量默认值。
3. 原生、WSL2、Bash/Zsh 使用同一目录生成结果；shell 不额外维护一份容量常量。
4. 调整明确要求省略容量的旧测试，增加所有发现模型均输出同一默认值的回归验证。
5. 使用既有明确应用供应商的配置流程更新目录；纳入备份、原子写入、并发检查与
   失败恢复。已有外部或人工修改目录不在后台覆盖。
6. 保留已有旧根级容量键的迁移流程；变更预览应说明迁移后的统一默认值。根级外部
   覆盖可能影响目录值，需要在诊断中明确识别。
7. 已运行的 Codex 可能持有旧目录信息，应用后继续标记待重启，不自动终止 CLI。

## 5. 验收与诊断

- 目录测试覆盖已知模型、未知模型、缺失快照和多个模型，均检查四个字段；不能只
  检查默认模型而漏掉其他发现模型。
- 使用受支持的真实 Codex 构建在隔离环境加载目录，确认没有整体回退到内置目录，
  并通过 token_count 验证 model_context_window 非 null。
- 使用可控 fixture 和小窗口验证达到阈值后确实压缩，并确认 null 自动阈值在目标
  版本的行为。此项验证客户端压缩机制，不要求测试每个供应商模型的最大容量。
- 检查原生、WSL2、Linux 导出和旧配置迁移不产生全局容量覆盖冲突。
- 最小脱敏问题日志记录策略标识、采用数值、目标 Codex 版本、目录更新阶段和
  待重启状态；不记录 API Key、请求正文或模型指令。新增日志补回归测试。

## 6. 已有证据与验证限制

2026-10-16 实际执行本机 Codex 0.160.0 和 0.163.0 的
`debug models --bundled`，两者 gpt-6.1-sol 均返回 272000、872000、95，自动压缩
阈值未显式指定。这是本次选择缺省值的来源，不是逐供应商容量测试。

原实现 `ModelCatalogEntry` 不包含容量字段，旧测试
`catalog_uses_high_instead_of_low_and_omits_context_windows` 明确断言省略容量。
现有推理能力快照仅保存推理档位，本方案无需扩展它。

初稿记录了 2026-10-16 一次被自动审批审查拒绝的隔离 Codex 子进程尝试；该历史记录
不代表最终实现状态。后续在 2026-10-17 已完成隔离验收，具体结果见下一节；本节不把
初稿时的阻断误写成当前缺陷。

## 7. 2026-10-17 实现与验收

已在共享模型目录生成器定义统一默认值，覆盖 Windows 原生、WSL2、Bash 和 Zsh
导出。手工加入导出脚本供应商目录的模型也通过共享序列化模板生成完整目录条目，
shell 不另存容量常量。已知模型、未知模型、缺失/失败快照以及补入的默认模型
均获得同一组容量字段；推理能力快照及其信任边界保持原有职责。

沿用明确应用供应商时的备份、原子替换、并发检查、恢复和待重启流程。Windows
应用确认与 WSL2 预览显示统一缺省策略，旧根级容量键仍在应用时迁移。诊断只读
提示根级容量覆盖，不输出覆盖值、不自动修改配置。普通应用、WSL2 重新接管及
Linux 导出的既有审计日志增加策略、默认数值、更新阶段、数字版本或 unknown、
待重启状态；日志使用 auto_compact_limit 表示目录自动压缩字段，避免现有凭据
脱敏器将含 token 的字段名误识别为令牌；凭据脱敏规则保持原样。

自动验证结果：

- Rust 单元测试 193 项通过；另有 2 项默认忽略的真实环境测试，其中新增真实
  Codex 上下文测试已单独显式运行通过。
- 原生环境集成测试 69 项、诊断集成测试 16 项、Linux 导出集成测试 19 项通过。
  导出黑盒分别实际执行 Bash 与标准 Zsh 5.9，覆盖 source 无写入、手工新增
  供应商、备份、并发冲突、恢复及凭据隔离。
- npm run check 通过；前端测试 127 项与现有脚本测试 29 项通过。

真实 Codex 验收脚本为 `scripts/test-codex-context-budget.mjs`，Rust 入口为
real_codex_loads_generated_context_and_auto_compacts（默认忽略，显式选择二进制）。
脚本读取 Rust 生产生成器输出的双模型目录，使用临时 CODEX_HOME 和回环 Responses
fixture，不读取真实供应商凭据，不修改现有用户配置。验收结果：

| 执行环境 | Codex 版本 | 双模型目录加载 | 默认有效窗口 | null 自动压缩小窗口验证 |
| --- | --- | --- | ---: | --- |
| Windows 原生 | 0.163.0 | 通过 | 258400 | 通过 |
| WSL2 内 Linux 二进制 | 0.147.0 | 通过 | 258400 | 通过 |
| WSL2 内 Linux 二进制 | 0.162.1 | 通过 | 258400 | 通过 |

目录加载由 model/list 确认两个自定义 fixture 模型均存在，排除整体回退到内置目录；
thread/tokenUsage/updated 确认 modelContextWindow 非 null。小窗口对照只在隔离
fixture 中把窗口改为 20000，保持自动阈值为 null：首轮总用量 18510，第二轮出现
contextCompaction 完成事件，服务端总计收到两轮请求与一次压缩请求，有效窗口为
19000。该结果验证客户端自动压缩机制，不表示供应商最大容量已实测，也不表示
默认大窗口的精确压缩触发点或桌面状态栏已完成人工 UAT。

PowerShell 复验入口（将变量值换成目标二进制的绝对路径）：

~~~powershell
$env:GPTEASY_REAL_CODEX = 'C:/path/to/codex.exe'
cargo test --manifest-path src-tauri/Cargo.toml --lib real_codex_loads_generated_context_and_auto_compacts -- --ignored --nocapture
~~~

可选设置 `GPTEASY_CONTEXT_ACCEPTANCE_REPORT` 为隔离证据目录，保存不含凭据的
catalog.json 与 results.json；也可把该 catalog.json 交给脚本在另一环境复验。
本次证据位于 Git 忽略的 src-tauri/target/acceptance/context-budget/。

本次 Linux 二进制在 WSL2 中运行，未把它记录为独立原生 Linux 宿主验收；未执行
真实供应商容量压测，也不把默认容量策略表述为供应商容量保证。维护者已确认与本次
变更相称的人工测试通过；完整发布授权和 Gitee 国内分发仍按发布流程单独执行。
