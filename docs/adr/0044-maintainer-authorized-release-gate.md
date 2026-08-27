# 正式发布采用自动门禁与维护者明确验收授权

Windows 正式候选必须从干净 `main` 构建，并通过前端检查与测试、双视口布局、完整 Rust 回归、综合 acceptance、发布树、领域/UI 合同、更新信任根和 updater 签名验证。候选 manifest 绑定 commit、平台、安装包路径、大小、SHA-256、Authenticode 状态和各项门禁结果；正式发布检查重新核对候选、安装包、当前 HEAD、发布树和合同。

正式发布还必须由维护者明确确认已经完成与本次变更相称的人工测试并授权发布，通过 `-ConfirmMaintainerAcceptance` 进入发布检查。默认不要求在一次性 Windows 账户运行全量交互式 UAT，也不要求生成 `evidence.json`；未执行的 UAT 不得在 Release、Issue 或日志中冒充通过。

`uat:windows` 和 `release:check -Mode Acceptance` 继续保留为高风险变更、安装生命周期重构或维护者主动要求时的深度验收。提供 UAT 证据时仍必须绑定同一候选并满足完整结构化检查；这个可选路径不会削弱正式候选的自动门禁、哈希、签名、稳定版本或发布授权要求。
