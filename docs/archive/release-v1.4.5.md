# GPTEasy v1.4.5 变更归档

发布日期：2026-09-17

## 用户可见变更

- GPTEasy 管理的 Windows、WSL2 和独立 Linux Codex 配置新增统一状态栏，显示当前目录、模型与推理强度、上下文使用量以及输入输出 token 统计。
- 应用或切换供应商时，GPTEasy 将 Codex 默认推理强度设为 `high`，覆盖已有的根级 `model_reasoning_effort`，并避免生成重复配置项。
- 已有 GPTEasy 管理区块会在下一次应用供应商时兼容升级；历史区块和备份仍可识别，独立 Linux 的强制切换与 `gpteasy restore` 行为保持不变。

## 安全与兼容边界

- 状态栏更新保留 `[tui]` 中由用户设置的其他字段；无法安全解析或合并的配置继续失败关闭，不会绕过既有接管、并发修改、备份和恢复门禁。
- 默认推理强度属于 GPTEasy 管理的供应商选择状态。首次接管和受控重建会移除外部同名根字段后写入唯一的受管值，OpenAI 登录模式仍遵循原有配置归还规则。
- GitHub Release 仍是正式发布源；Gitee 同步遇到安装包自动上传限制时，只生成待上传附件并等待维护者人工上传，确认附件一致后才推进公开更新清单。

## 验证记录

- 维护者已完成与本次变更相称的人工测试并明确确认通过。
- TypeScript 与 ESLint 检查通过；React 回归 111 项、Gitee 分发协议 29 项全部通过。
- 完整 Rust 回归通过；新增覆盖 Windows 接管、WSL2 应用、Bash/Zsh 导出、历史管理区块升级、配置去重和恢复兼容。
- 正式候选继续执行 Playwright 双视口布局、Windows 自动验收、Linux/WSL 自动矩阵、发布树、领域/UI 合同、Gitee 更新信任根和 updater 签名门禁。
- 可选的交互式 Windows UAT 不重复执行，不将其记录为新的自动化结果。

## 发布与平台范围

- 本次正式发布版本为 1.4.5；跳过 1.4.4，不创建对应 Tag 或 Release。
- Windows 安装包保持 x64 当前用户 NSIS 安装和用户确认更新，不执行静默安装。
- Windows ARM64、macOS Intel、Apple Silicon、签名、公证和严格 `~/Applications` 安装仍需对应原生环境验证；本次 Windows x64 发布不宣称这些平台已完成交付。
