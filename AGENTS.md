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

功能修改完成后保持未发布状态。只有用户明确主动发起发布时，才执行打 tag、发布到 GitHub 或发布到 GitCode；单次功能修改或提交不构成发布授权，以便多次改动合并后统一发布。

### Spike findings

- **Spike findings for GPTEasy** (implementation patterns, constraints, gotchas) → `Skill("spike-findings-gpteasy")`
