---
spike: 007
name: provider-switch-saga
type: standard
validates: "Given SQLite 中的已验证供应商、当前 Codex 配置和相关进程，when 在验证、状态写入、配置替换及重启边界发生失败或崩溃，then 系统重启后能收敛到完整旧状态或完整新状态且不静默终止 CLI"
verdict: VALIDATED
related: [002, 004, 006, 008]
tags: [rust, sqlite, config, saga, recovery, restart, integration]
---

# Spike 007: 供应商切换 Saga

## What This Validates

**Given** SQLite 中的已验证供应商、带管理区块的 Codex 配置，以及可能运行中的桌面 Codex 和 CLI，  
**when** 用户选择立即重启、稍后重启或取消，并在 Saga 的准备、配置替换、状态提交和重启边界发生失败或崩溃，  
**then** GPTEasy 重启后能依据持久化意图和配置哈希收敛到完整旧状态、完整新状态或明确的外部配置状态，而不会静默终止 CLI。

## Research

### 已检查的资料

- SQLite 事务语义：`https://sqlite.org/lang_transaction.html`
- SQLite 原子提交原理：`https://sqlite.org/atomiccommit.html`
- SQLite WAL：`https://sqlite.org/wal.html`
- `rusqlite::TransactionBehavior`：`https://docs.rs/rusqlite/0.40`

### 方案比较

| 方案 | 优点 | 致命问题 | 状态 |
|---|---|---|---|
| 先提交 SQLite，再写配置 | 数据库事务简单 | 配置失败后数据库会谎称已切换 | 淘汰 |
| 先写配置，再提交 SQLite，不记录意图 | 正常路径简单 | 写配置后崩溃无法判断应前滚还是回滚 | 淘汰 |
| 把配置正文存进 SQLite 并由数据库作为唯一真相 | 可用单库事务 | 外部工具仍会直接修改 Codex 文件；明文凭据和正文重复存储 | 不采用 |
| 持久化 Saga 意图、旧/新哈希和备份路径，再写文件并提交状态 | 每个崩溃点都可根据磁盘事实恢复 | 需要恢复状态机和长期操作记录 | **采用** |

### 选定顺序

1. 完整供应商验证成功；取消和验证失败在任何持久化前返回。
2. 读取旧配置，渲染新管理区块，计算旧/新 SHA-256，并创建同步备份。
3. 使用 SQLite `BEGIN IMMEDIATE` 写入 `switch_operations.phase = prepared`。
4. 写同目录临时文件并原子替换 Codex 配置。
5. 在一个 SQLite 事务中更新环境当前供应商并把操作推进到 `state_committed`。
6. 执行桌面重启计划；CLI 只记录人工重启要求。
7. 操作结束为 `completed` 或 `pending_restart`。

SQLite 保证数据库自身的原子提交，但不能与外部 TOML 文件和操作系统进程组成全局事务。因此 Saga 必须把**意图和判据**写入数据库，而不是假装三个资源可以同时提交。

## How to Run

```powershell
.\.codex\skills\spike-findings-gpteasy\sources\007-provider-switch-saga\run.ps1
```

## What to Expect

`.run/summary.json` 应显示 13/13：

- 立即重启桌面进程后完成切换。
- CLI 存在时保持 `pending_restart`。
- 稍后重启写配置但不尝试重启。
- 无相关进程时直接完成。
- 取消和验证失败不创建 Saga，也不写配置。
- `prepared` 后崩溃且文件仍是旧哈希时恢复为旧状态。
- 配置替换失败恢复为旧状态。
- 配置已是新哈希但数据库仍是旧状态时前滚数据库并继续重启。
- 状态提交后崩溃时继续执行重启阶段。
- 重启失败不回滚已经成功写入的配置，只进入待重启。
- 文件哈希既非旧也非新时进入外部配置 `needs_attention`，环境当前供应商置空。

## Observability

每个场景包含：

- `state.db`：最小供应商、环境和 Saga 操作表。
- `config.toml` 与 `.gpteasy-backups/`：只使用假凭据。
- `events.jsonl`：阶段事件、操作 ID、决策、哈希和状态，不记录配置正文或 API Key。
- 总结只记录供应商 ID、配置 SHA-256、操作阶段和重启尝试次数。

## Investigation Trail

1. **不存在跨资源 ACID**：SQLite、TOML 和进程无法共享事务；正确问题是“崩溃后如何判定磁盘事实并收敛”。
2. **意图必须先于配置替换持久化**：`prepared` 行保存旧/新哈希、备份路径、决策和进程存在性。配置替换后即使立刻崩溃，恢复也能识别新哈希。
3. **文件哈希决定前滚或回滚**：恢复时旧哈希意味着配置未生效，操作标记 `rolled_back`；新哈希意味着配置已生效，数据库前滚到新供应商。
4. **第三种哈希是外部配置**：若文件既不匹配旧值也不匹配新值，不能用备份或新配置覆盖用户编辑。环境当前供应商置空，操作进入 `needs_attention`，交给 Spike 008 协调。
5. **重启不是配置事务的一部分**：配置和数据库已经一致后，桌面重启失败不应回滚供应商，只应进入待重启。
6. **CLI 状态不可补偿**：CLI 无法透明恢复原终端，因此无论正常执行还是崩溃恢复，只要 CLI 存在就保留 `pending_restart`。
7. **取消和验证失败是前置门禁**：两者都不会创建操作、备份或修改配置。
8. **`BEGIN IMMEDIATE` 提前获取写事务**：正式应用由 Rust 后端独占数据库访问，使用 immediate transaction 能让写冲突尽早暴露。
9. **WAL 不是跨文件事务**：WAL 与 `synchronous = FULL` 提升 SQLite 自身恢复能力，但不能替代 Saga 日志和配置哈希。

## Results

### Verdict: VALIDATED ✓

13 个正常、失败和崩溃场景全部通过。供应商切换可以通过持久化 Saga 意图和配置哈希，在 SQLite、Codex 配置文件和重启计划之间实现可恢复的一致性。

### 关键结论

- 配置文件是判断 Codex 实际供应商状态的外部事实，SQLite 不能单方面覆盖它。
- `prepared → config replaced → state_committed → completed/pending_restart` 是可恢复的最小阶段。
- 恢复必须支持旧哈希回滚、新哈希前滚和未知哈希转外部配置三条路径。
- 重启失败不会撤销已成功切换的配置。
- CLI 永远不会被 Saga 静默终止。

### 限制

- 本 Spike 使用 fixture 进程状态，没有终止真实桌面 Codex；实际进程识别和激活沿用 Spike 004。
- 数据库只包含验证 Saga 所需的最小 schema，不是完整应用 schema 或迁移矩阵。
- 只模拟进程重启结果，没有覆盖系统关机、磁盘损坏和 SQLite 文件丢失。
- 配置备份包含明文假 Key；正式备份与数据库都需要当前用户访问控制。
