## Agent skills

### Issue tracker

问题和 PRD 使用 `yinshaohua/GPTEasy` 的 GitHub Issues 管理，并通过 `gh` CLI 操作。参见 `docs/agents/issue-tracker.md`。

### Triage labels

使用默认的五种 triage 标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human` 和 `wontfix`。参见 `docs/agents/triage-labels.md`。

### Domain docs

采用 single-context 布局：根目录使用 `CONTEXT.md`，架构决策记录存放于 `docs/adr/`。参见 `docs/agents/domain.md`。

### Bug diagnostics

修复缺陷时同时评估并完善“问题日志”的最小脱敏证据，使同类失败能够区分关键阶段和状态；为新增日志补回归测试。

### Release authorization

功能修改完成后保持未发布状态。只有用户明确主动发起发布时，才执行打 tag、发布到 GitHub 或发布到 Gitee；单次功能修改或提交不构成发布授权，以便多次改动合并后统一发布。

正式发布要求干净 `main` 构建的候选通过完整自动门禁，并由维护者明确确认人工测试结果和发布授权；默认不要求一次性 Windows 账户生成交互式 UAT 证据。`uat:windows` 保留为按风险选择的深度验收，不得把未执行的 UAT 记录为已通过。

### Spike findings

- **Spike findings for GPTEasy** (implementation patterns, constraints, gotchas) → `Skill("spike-findings-gpteasy")`

### Codex Windows 终端约定

- 普通命令默认使用管道执行，不要强制设置 `tty=true`；在 Windows 上分配 PTY 可能创建可见控制台并导致闪窗。
- 只有需要交互式输入、实时终端控制或持续占用终端的命令才使用 `tty=true`；能使用非交互参数时优先使用非交互方式。
- 这条约定只影响模型选择终端工具的参数，不能修复 Codex 执行器本身的窗口创建问题；若非交互命令仍闪窗，应在 Codex/执行器实现层使用隐藏窗口或管道句柄解决。
