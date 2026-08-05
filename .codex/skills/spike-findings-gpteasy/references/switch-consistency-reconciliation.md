# 切换一致性与外部协调

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- 供应商使用不可变 ID；地址、凭据或默认模型变化时，必须验证新组合后再替换，失败保留旧配置。
- SQLite 与 Codex 配置文件之间的切换必须能在失败或崩溃后恢复到完整旧状态、完整新状态或明确的外部配置状态。
- 外部工具修改用户配置、项目/会话层覆盖或供应商身份歧义时，不得自动覆盖。
- 取消和验证失败必须发生在任何 Saga、备份或配置写入之前。
- 重启失败不得回滚已经成功提交的供应商配置；CLI 不得被静默终止。

## How to Build It

### 1. 接受“跨资源不存在 ACID”

SQLite、Codex TOML 和操作系统进程不能共享一个事务。正确模型是持久化 Saga，而不是让数据库单方面宣称切换成功。

最小持久化实体：

```sql
CREATE TABLE switch_operations(
    id TEXT PRIMARY KEY,
    environment_id TEXT NOT NULL,
    old_provider_id TEXT NOT NULL,
    new_provider_id TEXT NOT NULL,
    old_hash TEXT NOT NULL,
    new_hash TEXT NOT NULL,
    backup_path TEXT NOT NULL,
    decision TEXT NOT NULL,
    desktop_present INTEGER NOT NULL,
    cli_present INTEGER NOT NULL,
    phase TEXT NOT NULL,
    restart_attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);
```

SQLite 连接使用：

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA busy_timeout = 5000;
```

写事务使用 `BEGIN IMMEDIATE` 或 `rusqlite::TransactionBehavior::Immediate`，让写冲突尽早暴露。WAL 和 FULL 只提高 SQLite 自身恢复能力，不能替代 Saga。

### 2. 固定切换顺序

推荐阶段：

```text
validated
  → prepared
  → config_replaced
  → state_committed
  → completed | pending_restart
```

执行顺序：

1. 完整验证目标地址、Key 和模型。
2. 用户选择取消时直接返回；不创建操作、备份或写文件。
3. 读取旧配置，渲染新管理区块并重新解析。
4. 创建同步备份，计算旧/新配置 SHA-256。
5. 在 `BEGIN IMMEDIATE` 中插入 `phase = prepared`，保存哈希、备份路径、决策和进程快照。
6. 使用 `safe-config-editing.md` 的并发检查和平台原子替换写入新配置。
7. 在一个 SQLite 事务中更新环境当前供应商，并把操作推进到 `state_committed`。
8. 执行桌面重启计划；CLI 只记录人工重启要求。
9. 结束为 `completed` 或 `pending_restart`。

持久化意图必须先于配置替换：

```rust
let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
tx.execute(
    "INSERT INTO switch_operations(
        id, environment_id, old_provider_id, new_provider_id,
        old_hash, new_hash, backup_path, decision,
        desktop_present, cli_present, phase
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'prepared')",
    params![/* ... */],
)?;
tx.commit()?;
```

### 3. 恢复时以配置哈希为磁盘事实

应用启动时扫描所有非终态操作，并计算当前配置哈希：

| 当前哈希 | 恢复动作 |
|----------|----------|
| 等于 `old_hash` | 数据库回到旧供应商，操作标记 `rolled_back` |
| 等于 `new_hash` | 数据库前滚到新供应商，继续重启阶段 |
| 两者都不等 | 环境当前供应商置空，操作标记 `needs_attention` |

核心判定：

```rust
if current_hash == old_hash {
    rollback_database_to_old_provider();
} else if current_hash == new_hash {
    commit_database_to_new_provider();
    continue_restart_phase();
} else {
    clear_current_provider();
    mark_needs_attention("config hash matches neither old nor new");
}
```

第三种哈希表示用户或外部工具已经编辑配置。不能用备份或候选配置覆盖它；转入外部配置协调。

### 4. 把重启从配置事务中分离

配置与数据库一致后：

- `later`：只要桌面或 CLI 仍运行，就进入 `pending_restart`。
- `immediate`：只尝试重启已可靠识别的桌面宿主进程树。
- 桌面重启失败：保留新配置，进入 `pending_restart`。
- CLI 存在：不终止，记录“请在原终端退出并重新运行”。

重启失败不是供应商切换失败，也不能触发配置回滚。

### 5. 用不可变 ID 协调供应商身份

供应商目录中的 ID 一旦创建就不变。管理区块写入：

```toml
# GPTEasy provider-id: immutable-provider-id
```

解析规则：

1. 管理区块有且仅有一个非空 ID 注释。
2. 已知 ID 的地址或模型不同：`managed_drifted`，要求重新验证。
3. ID 不在供应商目录：`external_unknown_id`。
4. 无 ID 的旧配置只按“地址 + 模型”精确匹配。
5. 唯一匹配：`legacy_unique_match`，只作为待用户确认的迁移候选。
6. 多个匹配：`external_ambiguous`。
7. 没有匹配：`external_unmatched`。

不要把名称、地址、模型或 Key 当主键；这些字段都可能变化或重复。

### 6. 通过 Codex app-server 读取最终有效配置

不要自己重新实现 Codex 的配置层合并。启动隔离的 `codex app-server`：

1. 发送 `initialize`。
2. 发送 `initialized`。
3. 调用：

```json
{
  "id": 2,
  "method": "config/read",
  "params": {
    "cwd": "目标工作目录",
    "includeLayers": true
  }
}
```

只在内存中提取：

- `config.model`
- `config.model_provider`
- `origins.model.name.type`
- `origins.model_provider.name.type`
- `layers[].name.type`

不要保存原始响应，因为 effective config 可能包含 `experimental_bearer_token`。

app-server 子进程要求：

- stdin/stdout 使用逐行 JSON-RPC。
- stderr 丢弃或经过严格脱敏。
- 请求设置明确超时。
- Windows 使用隐藏窗口启动。
- 超时、异常退出或协议不兼容时降级为“无法确认最终有效层”，不能改写用户文件。

### 7. 形成稳定的协调状态

| 状态 | 含义 | 自动写入 |
|------|------|----------|
| `managed_current` | ID、用户字段和最终有效来源都匹配 | 否 |
| `managed_overridden` | 用户管理区块正确，但项目/会话/托管层覆盖 | 否 |
| `managed_drifted` | 已知 ID 的地址或模型与已验证目录不同 | 否，先重新验证 |
| `external_unknown_id` | 管理区块 ID 不在目录 | 否 |
| `legacy_unique_match` | 无 ID，但地址和模型唯一匹配 | 否，等待用户确认迁移 |
| `external_ambiguous` | 无 ID，存在多个候选 | 否 |
| `external_unmatched` | 无 ID，没有候选 | 否 |
| `needs_attention` | TOML、标记或 ID 元数据损坏，或 Saga 哈希未知 | 否 |

用户文件与最终有效配置必须分别建模。`managed_overridden` 不是写入失败，也不能通过重复改写用户层来“修复”。

### 8. 记录可恢复事实，不记录敏感正文

操作日志记录：

- operation ID、environment ID
- phase、decision、restart attempts
- old/new/current SHA-256
- provider ID
- 配置路径和备份路径
- effective model/provider 的来源类型
- 脱敏错误类别

不要记录完整配置、API Key、app-server 原始响应或完整进程命令行。

## What to Avoid

- **不要先提交 SQLite 再写配置。** 文件失败后数据库会谎称已切换。
- **不要先写配置而不持久化意图。** 崩溃后无法判定前滚或回滚。
- **不要把配置正文复制进 SQLite 作为唯一真相。** 外部工具仍会改文件，且会重复存储明文凭据。
- **不要把 WAL 当跨文件事务。**
- **不要在未知哈希时自动恢复备份。** 这会覆盖外部编辑。
- **不要因桌面重启失败回滚新配置。**
- **不要静默终止 CLI。**
- **不要用地址、名称或模型作为供应商身份。**
- **不要在已知 ID 漂移时退回模糊匹配并静默接受。**
- **不要重新实现 Codex 配置层优先级。**
- **不要保存 app-server 完整 `config/read` 响应。**
- **不要自动争夺项目层、会话层或托管层覆盖。**

## Constraints

- 007 的 13 个正常、失败和崩溃场景已验证 `prepared → state_committed → completed/pending_restart` 恢复路径。
- 进程状态使用 fixture；真实桌面进程识别和应用激活沿用 Spike 004。
- 007 的实验配置曾把 ID 写成 provider 未知字段；008 已收敛为管理区块注释，正式实现必须使用注释方案。
- 008 在 Windows 和 `codex-cli 0.146.0` 上验证了 user/project/sessionFlags 来源与 10 个协调场景。
- 未来 Codex 升级需要对 app-server schema、字段来源和配置层行为做回归。
- 没有真实 MDM、enterprise managed 或系统 managed config，因此这些层只验证了协议可表达性。
- `legacy_unique_match` 只是迁移候选，不代表供应商已经重新验证。
- 未覆盖系统关机、磁盘损坏、SQLite 文件丢失或多进程同时写数据库。

## Origin

Synthesized from spikes: 007, 008
Source files available in: `sources/007-provider-switch-saga/`, `sources/008-external-config-reconciliation/`
