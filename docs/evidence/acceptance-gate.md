# Issue #10 集成验收门禁

在 Windows x64 开发机上运行：

```powershell
npm run acceptance
```

门禁使用当前用户临时目录中的隔离 Codex 环境和本地模拟供应商。测试建立两个不同名称和 UUID 的供应商，覆盖缺失配置、OpenAI 登录、有效外部配置、损坏管理区块、验证失败/取消、并发修改、备份/多工件写入失败、六个未完成配置操作故障边界，以及 SQLite 缺失、损坏、未来 schema 和迁移失败恢复。

脚本在任何测试输出或证据写入前扫描运行时 API Key canary。含 canary 的输出会被丢弃，不会生成日志或证据文件；成功结果只写入 `src-tauri/target/acceptance/<session>/`，其中包括脱敏 `evidence.json`、Rust 测试日志、前端泄漏门禁日志和汇总 JSON。前端门禁运行真实 React DOM/HTML 快照辅助与 `window.confirm` 通知文本，并确认普通界面不会调用凭据揭示命令。目标 Codex 配置、凭据和备份留在临时工作区，不属于可导出的验收证据。

门禁只断言 Provider/Environment/State 的公开应用结果、最终文件/数据库状态和故障恢复分类，不依赖内部函数或暂存文件名。真实供应商和真实 Codex 消费者读取仍属于发布前 Windows x64 人工 UAT。
