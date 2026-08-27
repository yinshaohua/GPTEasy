# Issue #28 / #39 综合自动验收门禁

在 Windows x64 开发机上运行：

```powershell
npm run acceptance
```

门禁使用当前用户临时目录中的隔离 Codex 环境和本地模拟供应商。Rust 流程测试建立两个不同名称和 UUID 的供应商，覆盖缺失配置、OpenAI 登录、有效外部配置、损坏管理区块、验证失败/取消、并发修改、备份/多工件写入失败、六个未完成配置操作故障边界，以及 SQLite 缺失、损坏、未来 schema 和迁移失败恢复；消费者测试覆盖桌面版与 CLI 的路径/父子关系身份分类、运行中消费者产生被动待重启以及自然退出后的清除，并验证可信桌面启动、用户确认后的可信桌面进程树重启、失败不假报成功和 CLI 隔离；前端同时运行完整供应商管理验收矩阵，覆盖目录/详情、DayWay、顺序与托盘、BASE_URL 建议和环境迁移。

脚本在任何测试输出或证据写入前扫描运行时 API Key canary。含 canary 的输出会被丢弃，不会生成日志或证据文件；成功结果只写入 `src-tauri/target/acceptance/<session>/`，其中包括脱敏 `evidence.json`、Rust 测试日志、前端泄漏门禁日志和汇总 JSON。前端门禁运行真实 React DOM/HTML 快照辅助与 `window.confirm` 通知文本，并确认普通界面不会调用凭据揭示命令。目标 Codex 配置、凭据和备份留在临时工作区，不属于可导出的验收证据。

门禁还覆盖 #39 会话管理的公开边界：App Server JSONL fixture 核对 `thread/list`、`thread/read`、归档、取消归档和永久删除方法及筛选参数，混合来源验证 exec/子代理不会进入列表，异常退出验证只读请求至多恢复一次而 mutation 不自动重试，消费者状态验证 mutation 只读门禁和批量部分成功。进程实现合同检查无窗口启动、Job Object 树回收、stdin EOF 优雅关闭和精确所有权恢复。前端测试还覆盖统一切换确认、成功后当前供应商更新和失败后环境实际状态回读；`candidate:windows` 另行运行 Playwright，覆盖 `1120 × 620` 默认尺寸和 `680 × 520` 最小尺寸布局。真实供应商、打包 Windows GUI 生命周期和真实 Codex CLI 读取仍属于发布前 Windows x64 人工 UAT，UAT 使用 `windows-release-contract.json` 中的 `session_*` 检查 ID。
