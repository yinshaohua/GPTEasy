# #68：有效模型快照驱动 Windows 供应商应用

- Issue：[yinshaohua/GPTEasy#68](https://github.com/yinshaohua/GPTEasy/issues/68)
- 父任务：[#67](https://github.com/yinshaohua/GPTEasy/issues/67)
- 实施日期：2026-10-03
- 范围：原生 Windows 当前用户供应商应用；不包含共享后台服务控制、逐模型 profile、用户能力覆盖或 WSL/Linux 新模型目录。
- 决策：[ADR-0053](../adr/0053-common-reasoning-selector-defaults.md)

## 权威与输出边界

GPTEasy 自己的 SQLite 继续保存 `providers` 和 `provider_model_catalog`：前者是供应商配置，后者是与供应商 ID 及服务地址/API Key/默认模型验证组合绑定的完整发现快照。应用前严格校验当前组合、绑定指纹、快照格式和非空集合，不用默认模型或现有目录回退掩盖快照故障。

应用时生成 Codex 使用的 `config.toml`、凭据工件和 `gpteasy-model-catalog.json`，由 `model_catalog_json` 指向派生目录。它们与 GPTEasy 持久化承担不同职责；本任务不删除、迁移或替代 Codex 自身的 SQLite。目录原字节仍参与备份、并发修订和恢复，即使旧现场不是有效验证快照。

## 实现与验收对应

| #68 验收点 | 实现与回归切面 |
| --- | --- |
| 有效快照与验证绑定 | 统一严格读取与生成；缺失、损坏、空集合、不识别格式、陈旧绑定阻止普通/强制应用，保留三工件。 |
| 重新验证可修复 | 环境只读检查不依赖严格快照读取；验证成功事务更新快照，失败保留旧数据。规范化地址与验证指纹同步持久化。 |
| 完整且供应商隔离的目录 | 去首尾空白、过滤空 ID、确定性排序与精确去重，补入已验证默认模型；包含全部发现模型，同名模型仍绑定当前路由。 |
| 完整 schema 与四档 | 所有模型统一 `low/medium/high/xhigh`、默认 `high`；保留能力未识别说明、text/image 兼容声明和 original detail 禁用，不按名称认证能力。 |
| 根级 effort 独立 | 官方 OpenAI/DeepSeek 映射保持 `high`，未知供应商省略 root effort；只读不改旧 `none`，明确应用才按受管迁移处理。 |
| 各应用路径一致 | 普通应用、保存并应用、强制设置及历史模板兼容修复共享校验/生成规则；人工能力声明不作为历史模板接管。 |
| 跨资源提交与恢复 | 目录纳入 revision、备份 manifest、pending 指纹及恢复 preview；三工件原子替换和复读，回滚后复读才报告恢复。SQLite schema v11 为 pending 增加目录旧/新指纹，保留旧备份兼容。 |
| 错误反馈与脱敏诊断 | 区分快照、目录生成/schema、工件提交及重启阶段；记录策略/schema、模型数量、默认模型存在性与脱敏引用，不含 key、地址、目录正文或原始模型 ID。 |
| 持久化回归 | 环境/供应商/诊断/启动/迁移测试覆盖上述状态与故障；另显式运行真实 Codex 目录消费者合约。 |

## 审查修正

- **Standards**：旧备份未涉及目录或凭据时，恢复使用磁盘实际工件，不把缺少备份字段误判为磁盘缺失；提取组合绑定校验；移除无效 fixture 参数。此前 1 项硬违规、2 项启发式发现已修正。
- **Spec**：pending 原生 Saga 期间拒绝改名、配置替换或重新验证覆盖目标；目录兼容修复完成与恢复在 Immediate 事务内严格重验；保存并应用的正常提交和中断恢复均核对原 SQLite 配置及原始快照修订，避免覆盖较新数据；保存并应用补齐目录审计日志。
- 新增并发回归在最终提交前或中断后直接替换 SQLite 快照，分别断言三工件回滚且保留较新快照、或进入 conflict 而不覆盖。无法证明旧/新一致的部分写入按既有 Saga 进入冲突，不盲目覆盖外部改动。
- 重新验证先读取供应商基础记录，再在验证成功后更新模型快照；缺失或损坏的派生快照不会把可重新验证的供应商误判为不存在，严格目录校验仍保留在应用和导出路径。

Standards 与 Spec 分别完成只读复核：此前发现均已关闭，无剩余发现；复核不代替下面实际运行的自动测试。

## 自动验证

- `cargo test --locked --manifest-path src-tauri/Cargo.toml`：**453 项通过，7 项默认忽略，0 失败**（包含库测试 141 项、环境 74 项、供应商 35 项、诊断 15 项、启动 23 项、状态存储/迁移 20 项及其余集成测试）。
- 显式真实 Codex 合约：**1 项通过**；本机 `codex-cli 0.160.0`，省略 root effort 和显式 `none` 两组均返回完整四档/默认 `high`。
- `npm run check`：通过。
- `npm test`：**113 项 Vitest + 29 项 Node 测试通过**。
- `npm run build`：通过。
- `cargo fmt -- --check`、`cargo check --locked`、`git diff --check`：通过。
- ignored 宿主/端到端测试未因常规套件完成而记为通过；仅上面的原生 Codex 合约单独显式运行。

可重复运行的原生命令与假凭据边界见 [消费者证据](../evidence/codex-reasoning-selector-defaults-2026-10-02.md)。

## 未执行与发布边界

- 原生 Codex 契约使用隔离 CODEX_HOME 和假凭据，只调用初始化/模型列表，不发送真实供应商请求。
- 未执行 Windows 桌面交互 UAT、真实供应商四档能力探测或深度 `uat:windows`，不把它们记录为通过。
- 本任务保持未发布；不更新版本、不打 tag、不发布 GitHub/Gitee。
- 父任务及后续任务的未提交设计文档保留在工作区，不合并为本任务已实施内容。
