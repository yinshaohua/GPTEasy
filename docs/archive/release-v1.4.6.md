# GPTEasy v1.4.6 变更归档

发布日期：2026-09-18

## 用户可见变更

- GPTEasy 会保存供应商完整验证时取得的模型发现列表，并为当前活动供应商生成 Codex 模型目录；DeepSeek、Qwen、Claude、Gemini、GLM、Kimi、Mistral、Llama 等提供 OpenAI 兼容接口的常见模型可进入当前供应商的模型选择目录。
- 供应商默认模型继续由用户在验证时选择。DeepSeek、GPT 和 OpenAI 推理模型家族默认使用 `high` 推理强度；Claude、Qwen、Gemini、GLM、Kimi、Mistral、Llama 等已识别家族默认使用 `medium`；未识别模型不强行声明推理强度。
- 当供应商的 `/models` 返回 HTML 落地页而不是模型列表时，模型发现会继续尝试同源的 `/v1/models`，兼容更多采用 OpenAI 风格路径的服务。
- Windows 与 WSL2 受管环境切换供应商时，模型目录文件、Codex 路由配置和凭据进入同一失败恢复流程；切回 OpenAI 登录模式时移除 GPTEasy 生成的模型目录。
- Bash 4+ 与 Zsh 5+ Linux 导出脚本采用相同的模型家族规则，并以大小写不敏感方式选择推理强度。

## 安全与兼容边界

- 模型目录快照绑定供应商 ID 和验证指纹，只有供应商完整验证或重新验证成功后才更新；验证失败不会替换上一份已验证列表。
- 供应商模型发现列表只证明服务端列出了模型 ID，不代表 GPTEasy 已精确探测每个模型的上下文窗口、输入模态、工具能力或全部推理档位。
- 当前 Codex 模型目录只包含当前活动供应商的模型，不把不同供应商的同名模型合并到无法确定路由的全局目录。
- 历史 GPTEasy 管理区块仍可识别；下一次显式应用供应商时会写入 `model_catalog_json` 并按默认模型更新或省略根级推理强度。
- 模型目录不保存 API Key；供应商切换仍遵循既有原子写入、备份、并发检查、Saga 回滚和待重启规则。

## 验证记录

- TypeScript 与 ESLint 检查通过；React 回归 111 项、Gitee 分发协议 29 项全部通过。
- 完整 Rust 回归通过；新增覆盖 schema 10 迁移、模型目录持久化、模型发现 HTML 回退、当前供应商目录生成、OpenAI 模式清理、失败回滚和历史配置兼容。
- Linux 导出 Bash/Zsh 黑盒回归 14 项通过；覆盖 DeepSeek `high`、Qwen `medium`、未知模型省略推理强度，以及大小写不敏感匹配。
- Linux/WSL 自动验收矩阵通过；当前 GNU Bash 5.3 与 WSL2 自动协议门禁通过。真实 GNU/Linux、一次性 WSL2、真实 Codex 和显式 opt-in 的真实供应商测试未执行，不记录为通过。
- 正式候选仍须从干净 `main` 构建，并通过双视口布局、Windows 自动验收、发布树、领域/UI 合同、更新信任根和 updater 签名门禁。
- 正式发布检查要求维护者明确确认已经完成与本次改动相称的人工测试；未执行的交互式 Windows UAT 不记录为通过。

## Issue 与发布范围

- 已核对全部开放 Issue，本批次没有直接关联且仍待关闭的 Issue；`#59`、`#60` 属于会话界面与全局标题栏工作，保持开放。
- 本次正式发布版本为 1.4.6；GitHub Release 是正式发布源，Gitee 标准 `.exe` 仍按既定流程由维护者通过网页上传。
- Windows 安装包保持 x64 当前用户 NSIS 安装和用户确认更新，不执行静默安装。
- Windows ARM64、macOS Intel、Apple Silicon、签名、公证和严格 `~/Applications` 安装仍需对应原生环境验证；本次 Windows x64 发布不宣称这些平台已完成交付。
