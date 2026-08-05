# Phase 1：可信本地状态与实现契约 - Pattern Map

**Mapped:** 2026-08-05  
**范围来源:** `CONTEXT.md`、`docs/adr/0001-0008`、`.planning/REQUIREMENTS.md`、`01-RESEARCH.md`、`01-VALIDATION.md`  
**拟建路径:** 52 个物理文件/工件，归并为 38 个模块  
**现有类比:** 5 个 Spike 类比族，覆盖 28 / 38 个模块  
**生产代码现状:** 根目录没有 `package.json`、`src/`、`src-tauri/`、`tests/` 或 `scripts/`；所有产品代码都需要绿色脚手架。现有实现证据仅来自 `.planning/spikes/`。

## 锁定边界

- 不重新设计 ADR：Tauri 2 + Rust + TypeScript/React；SQLite 只能由 Rust 后端访问。
- 不把 Phase 1 扩展为供应商网络验证、Codex 配置写入、WSL2 实际切换、完整托盘 UI、诊断导出或 updater 业务。
- SQLite 必须保存供应商明文 API Key；不得照搬 Spike 012 的“数据库不存 Key”实验 schema。
- 旧应用遇到更高 schema 时，必须在任何 read-write open 前拒绝写入。
- 所有待执行迁移必须在同一个 `BEGIN IMMEDIATE` 事务中完成；不能逐版本提交。
- 数据库升级备份必须使用 SQLite Online Backup API，不能复制单个 WAL 模式主库文件。
- 正式 contract evidence 必须使用字段允许清单；不得保存完整配置、完整 app-server 响应、完整命令行或凭据。
- `app_local_data_dir()` 是 Windows/macOS 产品状态根；Spike 017 的 `app_data_dir()` 只能作为 Tauri PathResolver 调用形状参考，不能原样复制。

## 最强现有类比

| 类比族 | 主要路径 | 可复用内容 | 不可照搬内容 |
|---|---|---|---|
| Spike 012：桌面供应商切换 E2E | `.planning/spikes/012-desktop-provider-switch-e2e/` | Tauri composition root、窄 command、`State`、`spawn_blocking`、rusqlite 参数化 SQL、`TransactionBehavior::Immediate`、矩阵测试、证据字节扫描 | Spike schema 不含 API Key；`Connection::open` 先打开 RW；错误直接转字符串；有 Phase 1 范围外的配置写入与进程逻辑 |
| Spike 009：WSL2 生命周期 | `.planning/spikes/009-wsl2-environment-lifecycle/` | 无副作用 PowerShell 探针、结构化 JSON evidence、备份排序/裁剪/恢复、fixture matrix | 备份对象是 TOML 文件，不是 SQLite snapshot；保留数是 5 而非数据库备份的 3 |
| Spike 001：Codex 原生配置契约 | `.planning/spikes/001-codex-native-config-contract/` | 隔离 harness、隐藏子进程、允许清单式请求摘要、版本/路径探针 | `inspect-windows.ps1` 保存了完整 `command_line`；`fingerprint()` 暴露头 8 字节；目标版本是历史 0.146.0 |
| Spike 005：安装/更新矩阵 | `.planning/spikes/005-desktop-install-update-matrix/` | Windows current-user NSIS 构建与安装/卸载 smoke、隐藏安装进程、结构化 summary、最小 capability | updater 业务不属于 Phase 1；updater `.sig` 不能替代 Authenticode |
| Spike 017：macOS 宿主契约 | `.planning/spikes/017-macos-real-host-contract/` | `evidence_level`/`limitations`、平台真伪分级、`~/Applications` smoke、codesign/Gatekeeper 检查、状态 canary | Windows 上的 partial summary 不能冒充真实 macOS；`app_data_dir()` 不是本阶段产品状态根 |

## 文件分类

“精确”表示角色和数据流都接近；“角色匹配”表示可复制结构但必须替换业务语义；“部分”表示只能复制一个局部 primitive；“无”表示必须按 `01-RESEARCH.md` 绿色实现。

| 新建/修改文件或模块 | 角色 | 数据流 | 最近类比 | 匹配质量 |
|---|---|---|---|---|
| `package.json`、`package-lock.json` | config | build/batch | Spike 012 `package.json` | 角色匹配 |
| `vite.config.ts`、`tsconfig.json`、`index.html` | config | transform/build | 无 React TS 产品类比 | 无 |
| `src/main.tsx` | component/bootstrap | event-driven | 无 React TS 产品类比 | 无 |
| `src/App.tsx` | component | request-response | Spike 012 `web/app.js` | 部分 |
| `src/state-api.ts` | service/client adapter | request-response | Spike 012 `web/app.js` | 部分 |
| `src-tauri/build.rs`、`Cargo.toml`、`Cargo.lock` | config | build/batch | Spike 012 `src-tauri/build.rs`、`Cargo.toml` | 精确 |
| `src-tauri/tauri.conf.json` | config | build/request-response | Spike 005/012 `tauri.conf.json` | 角色匹配 |
| `src-tauri/capabilities/default.json` | config/guard | request-response | Spike 005 capability | 精确 |
| `src-tauri/src/main.rs` | config/bootstrap | event-driven | Spike 012 `src/main.rs` | 精确 |
| `src-tauri/src/lib.rs` | provider/composition root | event-driven + request-response | Spike 012 `src/lib.rs` | 精确 |
| `src-tauri/src/commands/bootstrap.rs` | command/controller | request-response | Spike 012 commands | 部分 |
| `src-tauri/src/commands/settings.rs` | command/controller | CRUD/request-response | Spike 012 commands + core SQL | 部分 |
| `src-tauri/src/commands/recovery.rs` | command/controller | file-I/O/request-response | Spike 012 command boundary | 部分 |
| `src-tauri/src/domain/provider.rs` | model | transform/CRUD | Spike 012 validation/provider structs | 部分 |
| `src-tauri/src/domain/environment.rs` | model | transform/CRUD | Spike 009 `Distribution`/`DetectedEnvironment` | 部分 |
| `src-tauri/src/domain/settings.rs` | model | transform/CRUD | 无产品设置模型类比 | 无 |
| `src-tauri/src/state/mod.rs`、`error.rs` | service + utility | request-response/transform | Spike 012 module split；错误模型无生产类比 | 部分 |
| `src-tauri/src/state/paths.rs` | utility | file-I/O | Spike 017 Tauri path calls | 部分 |
| `src-tauri/src/state/preflight.rs` | service | file-I/O/request-response | 无只读 SQLite preflight 类比 | 无 |
| `src-tauri/src/state/connection.rs` | service | file-I/O/CRUD | Spike 012 `open_db` | 部分 |
| `src-tauri/src/state/backup.rs` | service | file-I/O/batch | Spike 009 retention/restore；无 Online Backup 类比 | 部分 |
| `src-tauri/src/state/recovery.rs` | service | file-I/O/request-response | Spike 012 recovery state machine | 部分 |
| `src-tauri/src/state/migrations/mod.rs` | service/migration registry | batch/transform | 无顺序 migration registry 类比 | 无 |
| `src-tauri/src/state/migrations/0001_initial.sql` | migration | batch/CRUD | 仅 `01-RESEARCH.md` 初始 schema | 无 |
| `src-tauri/src/state/repositories/providers.rs` | repository/service | CRUD | Spike 012 参数化 provider SQL | 角色匹配 |
| `src-tauri/src/state/repositories/environments.rs` | repository/service | CRUD | Spike 012 environment SQL | 角色匹配 |
| `src-tauri/src/state/repositories/settings.rs` | repository/service | CRUD | 无产品 settings repository 类比 | 无 |
| `src-tauri/tests/state_persistence.rs` | integration test | CRUD/restart | Spike 012 scenario + snapshot | 部分 |
| `src-tauri/tests/local_only_boundary.rs` | integration/static test | file-I/O/transform | Spike 012 evidence canary scan | 部分 |
| `src-tauri/tests/migration_matrix.rs` | fixture-matrix test | batch | 无历史 DB 升级类比 | 无 |
| `src-tauri/tests/migration_failure.rs` | fault integration test | batch/CRUD | Spike 012 `Injection` matrix | 角色匹配 |
| `src-tauri/tests/backup_restore.rs` | integration test | file-I/O/batch | Spike 009 backup/restore matrix | 角色匹配 |
| `src-tauri/tests/higher_schema_refusal.rs` | recovery integration test | file-I/O/request-response | 无 higher-schema non-mutating 类比 | 无 |
| `tests/fixtures/databases/v001/state.sqlite3`、`manifest.json` | historical fixture/config | file-I/O/batch | Spike scenario directories only | 无 |
| `tests/fixtures/contracts/**/manifest.json` 及脱敏 JSON evidence | contract fixture/config | batch/transform | Spike 017 snapshot + Spike 001/009 summaries | 部分 |
| `scripts/contracts/run-phase1-contracts.ps1`（含 canary 扫描） | batch/orchestrator | batch/file-I/O | Spike 001/005/009/017 runners | 角色匹配 |
| `scripts/contracts/probe-codex.ps1`、`probe-windows-host.ps1`、`probe-wsl2.ps1` | probe/utility | request-response + file-I/O | Spike 001、009 | 精确/角色匹配 |
| `scripts/contracts/verify-windows-package.ps1`、`run-macos.sh` | platform smoke | batch/file-I/O | Spike 005、017 | 精确 |

## Pattern Assignments

### 1. Tauri/Rust composition root

**应用到：**

- `src-tauri/build.rs`
- `src-tauri/src/main.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/commands/*.rs`
- `src-tauri/capabilities/default.json`

**主类比：** `.planning/spikes/012-desktop-provider-switch-e2e/src-tauri/src/lib.rs`

**模块与 composition root 形状**（`lib.rs:1-3,165-186`）：

```rust
mod appserver;
mod core;
mod validation;

pub fn run() {
    tauri::Builder::default()
        .manage(UiState::default())
        .invoke_handler(tauri::generate_handler![
            scan_processes,
            run_demo,
            export_latest_report
        ])
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build desktop provider switch spike")
        .run(|_app, event| {
            // lifecycle handling
        });
}
```

**复制方式：**

1. 保留 `lib.rs` 作为可测试 composition root，`main.rs` 只调用库入口。
2. 把 `UiState` 替换为只暴露应用启动状态的托管对象，例如 `ApplicationState`：
   - `Ready(Arc<StateStore>)`
   - `RecoveryRequired(Arc<RecoveryService>)`
3. `invoke_handler!` 只注册 `bootstrap_state`、`update_app_settings`、`list_compatible_backups`、`restore_database_backup`。
4. Phase 1 不复制托盘、进程扫描或供应商切换 command。
5. 初始化失败不得 `expect` 后退出或创建新库；需要转成可投影的 recovery-only 启动状态。

**薄 `main.rs`**（Spike 012 `src/main.rs:1-3`）：

```rust
fn main() {
    gpteasy_spike_012_lib::run();
}
```

**薄 `build.rs`**（Spike 012 `build.rs:1-3`）：

```rust
fn main() {
    tauri_build::build()
}
```

**最小 capability**（Spike 005 `capabilities/default.json:1-6`）：

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability for custom updater commands",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

产品文件应保留 `core:default` 的最小形状，但 description 改成 Phase 1 状态 command；不要增加 fs、shell、http 或 SQL plugin 权限。

**command 形状**（Spike 012 `lib.rs:47-65,109-112`）：

```rust
#[tauri::command]
async fn run_demo(
    decision: String,
    state: State<'_, UiState>,
) -> Result<PipelineReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // blocking Rust work
    })
    .await
    .map_err(|error| error.to_string())?
}
```

只复制 `State<'_>` 注入和 blocking 工作离开 async executor 的结构。**不要复制** `Result<_, String>` 和任意 `error.to_string()` 直接公开的做法；产品 command 必须映射到脱敏 `PublicStoreError`。

---

### 2. SQLite open preflight 与 ready connection

**应用到：**

- `src-tauri/src/state/preflight.rs`
- `src-tauri/src/state/connection.rs`
- `src-tauri/src/state/mod.rs`
- `src-tauri/tests/higher_schema_refusal.rs`

**生产类比：无。必须绿色实现。**

Spike 012 的下列代码（`core.rs:1263-1273`）只能作为“兼容后配置连接”的局部参考：

```rust
fn open_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "
        PRAGMA foreign_keys=ON;
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=FULL;
        PRAGMA busy_timeout=5000;
        ",
    )?;
    Ok(conn)
}
```

**禁止原样复制：** 它先以 RW 打开并立刻设置 WAL，不满足 STATE-05。

**绿色实现契约来自** `01-RESEARCH.md:410-427`：

```rust
fn inspect_existing(path: &Path) -> Result<DbHeader, StoreError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let application_id =
        conn.query_row("PRAGMA application_id", [], |row| row.get::<_, i64>(0))?;
    let user_version =
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    Ok(DbHeader { application_id, user_version })
}
```

**实现顺序必须固定：**

1. `state.sqlite3` 不存在：创建父目录，再在一个事务内创建 v1 schema。
2. 已存在：只读打开，读取 `application_id` 和 `user_version`。
3. `application_id` 不符：返回 recovery-only 错误，不打开 RW。
4. `user_version > CURRENT_SCHEMA_VERSION`：返回 `DatabaseTooNew`，不设置 WAL、不建 migration 表、不创建备份。
5. 版本相等：再 RW open，并配置 `busy_timeout`、`foreign_keys=ON`、`trusted_schema=OFF`、`synchronous=FULL`、WAL。
6. 版本较旧：先创建并验证 SQLite snapshot，再打开兼容的 RW migration 路径。

**higher-schema 测试必须额外记录：**

- 主库、`-wal`、`-shm` 是否存在；
- 每个文件的 hash、长度和 mtime；
- 调用后全部保持不变；
- backup 目录没有新增文件；
- 普通 CRUD command 不可用。

---

### 3. 顺序 migration transaction

**应用到：**

- `src-tauri/src/state/migrations/mod.rs`
- `src-tauri/src/state/migrations/0001_initial.sql`
- `src-tauri/tests/migration_matrix.rs`
- `src-tauri/tests/migration_failure.rs`

**最近局部类比：** Spike 012 `core.rs:299-336,483-493`

```rust
let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
tx.execute(
    "INSERT INTO providers(...) VALUES (?1, ?2, ?3, ?4, ?5, 'validated')",
    params![/* typed values */],
)?;
tx.execute(
    "INSERT INTO switch_operations(...) VALUES (...)",
    params![/* typed values */],
)?;
tx.commit()?;
```

可复制：

- `TransactionBehavior::Immediate`
- 参数化 SQL
- 相关写入在一个事务中
- 只在最后 `commit`

不可复制：

- Spike 是业务 Saga 的多个短事务，不是“全部 pending migrations 一个事务”。
- Spike 没有 `user_version`、`schema_migrations`、checksum 或历史 migration registry。

**绿色 migration runner 契约来自** `01-RESEARCH.md:590-627`：

```rust
let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

let observed: u32 =
    tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
if observed != from {
    return Err(StoreError::ConcurrentSchemaChange { expected: from, observed });
}

for migration in migrations.iter().filter(|m| m.version > from) {
    tx.execute_batch(migration.sql)?;
    tx.execute(
        "INSERT INTO schema_migrations(version, name, checksum, applied_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![migration.version, migration.name, migration.checksum, applied_at],
    )?;
    tx.pragma_update(None, "user_version", migration.version)?;
}

ensure_no_rows(&tx, "PRAGMA foreign_key_check")?;
let quick: String = tx.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
if quick != "ok" {
    return Err(StoreError::IntegrityCheckFailed);
}
tx.commit()?;
```

**registry 文件模式：**

```rust
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
    pub checksum: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("0001_initial.sql"),
        checksum: "<compile-time checked SHA-256>",
    },
];
```

这是绿色脚手架建议，字段必须与 `schema_migrations(version, name, checksum, applied_at)` 及研究中的双账本契约一致。已发布 migration 只能追加，不能编辑。

---

### 4. SQLite online backup、三份 retention 与 restore

**应用到：**

- `src-tauri/src/state/backup.rs`
- `src-tauri/src/state/recovery.rs`
- `src-tauri/tests/backup_restore.rs`

**Online Backup 生产类比：无。**

**必须采用的绿色 primitive 来自** `01-RESEARCH.md:429-451`：

```rust
fn create_verified_backup(source: &Connection, target: &Path) -> Result<(), StoreError> {
    let mut destination = Connection::open(target)?;
    let backup = rusqlite::backup::Backup::new(source, &mut destination)?;
    backup.run_to_completion(128, Duration::from_millis(10), None)?;
    drop(backup);
    drop(destination);

    let check = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let quick_check: String =
        check.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        return Err(StoreError::BackupInvalid);
    }
    Ok(())
}
```

**retention/restore 局部类比：** Spike 009 `src/main.rs:566-596`

```rust
fn restore_latest(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let latest = backup_files(path)?
        .pop()
        .ok_or("no backup available for restore")?;
    let bytes = fs::read(latest)?;
    let temp = write_temp(path, &bytes)?;
    atomic_replace(path, &temp)?;
    Ok(())
}

fn prune_backups(path: &Path, limit: usize) -> Result<(), Box<dyn std::error::Error>> {
    let backups = backup_files(path)?;
    let remove_count = backups.len().saturating_sub(limit);
    for old in backups.into_iter().take(remove_count) {
        fs::remove_file(old)?;
    }
    Ok(())
}

fn backup_files(path: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = /* enumerate */;
    files.sort();
    Ok(files)
}
```

**复制方式与差异：**

- 复制“可排序文件名 → 排序 → 删除最老项”的 retention 结构。
- 数据库保留数改为 3。
- 不复制 `fs::read` 作为 SQLite backup/restore 的一致性模型。
- backup 名称必须包含 UTC 可排序时间、源 schema、目标 schema 和 opaque ID；不要依赖 mtime。
- 新 backup 只有在独立 read-only open、`application_id`、`user_version`、`quick_check` 全部通过后，才能参与裁剪。
- 同一未变化源库 + 同一目标 migration 的重复失败启动，应复用最近已验证 backup，避免挤掉真正历史备份。

**restore 固定顺序：**

1. UI 只传后端列出的 opaque backup ID。
2. 后端 canonicalize，确认仍位于 backup root。
3. 只读验证 `application_id`、兼容 schema、`quick_check`。
4. 把当前更高版本数据库保留为 quarantine 副本。
5. 用同目录临时文件/平台原子替换恢复。
6. 重新走完整 preflight，不直接复用旧 connection。

---

### 5. Recovery-only projection

**应用到：**

- `src-tauri/src/state/recovery.rs`
- `src-tauri/src/commands/bootstrap.rs`
- `src-tauri/src/commands/recovery.rs`
- `src/state-api.ts`
- `src/App.tsx`

**生产类比：无完整实现；Tauri/前端调用结构可部分复制。**

**启动投影锁定形状来自** `01-RESEARCH.md:454-479`：

```rust
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BootstrapState {
    Ready {
        schema_version: u32,
        snapshot: StateSnapshot,
    },
    DatabaseTooNew {
        found: u32,
        supported: u32,
        compatible_backups: Vec<BackupSummary>,
    },
    MigrationFailed {
        from: u32,
        to: u32,
        backup: BackupSummary,
        error_code: String,
    },
}
```

**窄恢复 command 来自** `01-RESEARCH.md:630-646`：

```rust
#[tauri::command]
fn restore_database_backup(
    backup_id: String,
    recovery: tauri::State<'_, RecoveryService>,
) -> Result<BootstrapState, PublicStoreError> {
    recovery.restore_compatible(&backup_id).map_err(Into::into)
}
```

**前端 invoke 局部类比：** Spike 012 `web/app.js:70-86`

```javascript
async function runDemo() {
  const button = document.querySelector("#run");
  button.disabled = true;
  try {
    const report = await invoke("run_demo", {
      decision: document.querySelector("#decision").value,
    });
    renderReport(report);
  } catch (error) {
    document.querySelector("#report").textContent = `运行失败：${error}`;
  } finally {
    button.disabled = false;
  }
}
```

`src/state-api.ts` 应把 `invoke` 集中为类型化 wrapper；`App.tsx` 只根据 `BootstrapState.kind` 渲染：

- `ready`：显示最小状态 tracer 和设置修改入口；
- `database_too_new`：只显示 found/supported 与兼容备份；
- `migration_failed`：只显示脱敏错误码和已经验证的备份；
- recovery 分支不得渲染普通 provider/environment/settings 写入操作。

**不要复制：**

- Spike 012 直接把任意错误字符串展示给 UI。
- nullable `StateStore` 加“调用时再报错”的结构。
- 允许 UI 传任意文件路径。

---

### 6. Domain models 与 repositories

**应用到：**

- `src-tauri/src/domain/provider.rs`
- `src-tauri/src/domain/environment.rs`
- `src-tauri/src/domain/settings.rs`
- `src-tauri/src/state/repositories/*.rs`
- `src-tauri/tests/state_persistence.rs`

**SQL/transaction 类比：** Spike 012 `core.rs:299-335`

```rust
tx.execute(
    "INSERT INTO providers(id, name, base_url, model, combination_fingerprint, validation_state)
     VALUES (?1, ?2, ?3, ?4, ?5, 'validated')
     ON CONFLICT(id) DO UPDATE SET
       name=excluded.name, base_url=excluded.base_url, model=excluded.model,
       combination_fingerprint=excluded.combination_fingerprint,
       validation_state='validated'",
    params![
        verified.input.id,
        verified.input.name,
        verified.input.base_url,
        verified.input.model,
        verified.evidence.combination_fingerprint
    ],
)?;
```

复制参数化 SQL、不可变 ID 和同事务关联写入的形状；产品 repository 必须改成研究 schema：

- `providers` 包含 `api_key` 明文字段。
- `provider_verifications` 独立保存 fingerprint、验证时间与 contract version。
- `managed_environments` 使用不可变 `id` 和 opaque `platform_identity`，显示名不是主键。
- `app_settings` 是 `singleton_id = 1` 的类型化单行表，不做 JSON/EAV。

**状态 round-trip 测试模式：**

1. 临时目录创建 v1 DB。
2. 写两个供应商及其不同 API Key。
3. 写一个验证记录。
4. 写 native + WSL 环境，并关联不同当前供应商。
5. 修改 locale/theme/launch-at-login 等设置。
6. drop 所有 repository/store/connection。
7. 从同一路径重新 `StateStore::open`。
8. 对公开领域值逐字段深比较，包括 API Key；测试输出和 panic 信息不得打印 API Key。

Spike 012 的 `create_scenario()` + `snapshot()`（`core.rs:135-179,683-717`）可作为“建立隔离场景、通过公开读取投影断言”的结构参考，但不能复用其 schema。

---

### 7. Historical DB fixtures

**应用到：**

- `tests/fixtures/databases/v001/state.sqlite3`
- `tests/fixtures/databases/manifest.json`
- `src-tauri/tests/migration_matrix.rs`

**生产类比：无。**

Spike 009/012 只有运行时创建的 scenario matrix，没有“随正式版本永久提交的可打开 SQLite 文件”。可复制的只有矩阵执行形状。

**矩阵入口类比：** Spike 009 `src/main.rs:94-120,288-297`

```rust
fn run_matrix(output: &Path, evidence_path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    fs::create_dir_all(output)?;
    let mut results = Vec::new();

    results.push(case(
        "real-detection-does-not-start-or-enter-distros",
        /* predicate */,
        json!({ /* allowlisted evidence */ }),
    ));

    let passed = results
        .iter()
        .filter(|entry| entry["passed"] == true)
        .count();
    let summary = json!({"passed": passed, "total": results.len(), "results": results});
    fs::write(output.join("summary.json"), serde_json::to_vec_pretty(&summary)?)?;
    Ok(summary)
}
```

**`manifest.json` 至少需要：**

```json
{
  "fixtures": [
    {
      "schema_version": 1,
      "path": "v001/state.sqlite3",
      "application_id": "<locked integer>",
      "sha256": "<64 hex>",
      "created_by_release": "0.1.0",
      "contains_only_fixed_test_credentials": true
    }
  ]
}
```

字段名可由实现固定，但必须满足：

- 测试从 manifest 枚举，不能在 Rust 测试中手写版本列表。
- fixture 复制到 `tempfile::TempDir` 后迁移，绝不能就地修改仓库文件。
- 每个 fixture 必须 read-only 可打开，并在迁移前验证 manifest hash。
- 迁移后验证 `schema_migrations`、checksum、`user_version`、FK 与 `quick_check`。
- 正式发布后 fixture 和对应 migration 都是 append-only。

---

### 8. Contract evidence manifests 与 canary scanning

**应用到：**

- `tests/fixtures/contracts/codex/0.146.1/manifest.json`
- `tests/fixtures/contracts/windows-host/manifest.json`
- `tests/fixtures/contracts/wsl2/manifest.json`
- `tests/fixtures/contracts/packaging/**/manifest.json`
- `scripts/contracts/run-phase1-contracts.ps1`
- `src-tauri/tests/local_only_boundary.rs`

**evidence level 类比：** Spike 017 `src/lib.rs:52-68`

```rust
pub struct ContractSnapshot {
    pub captured_at: String,
    pub os: String,
    pub arch: String,
    pub current_app_scope: String,
    pub app_bundles: Vec<BundleEvidence>,
    pub processes: Vec<ProcessEvidence>,
    pub evidence_level: String,
    pub limitations: Vec<String>,
}
```

**真实/非真实平台分级**（Spike 017 `src/lib.rs:315-330`）：

```rust
evidence_level: if is_macos {
    "native_host_probe".to_string()
} else {
    "non_macos_development_host".to_string()
},
limitations: if is_macos {
    vec![/* remaining real-host checks */]
} else {
    vec![
        "当前执行环境不是 macOS，不能授予真实宿主验证结论".to_string(),
        "交叉编译或 fixture 不能替代 LaunchServices、APFS 和 WindowServer 实测".to_string(),
    ]
},
```

**正式 manifest 必须补足 Spike 017 没有的字段：**

```json
{
  "contract_name": "codex-native-config",
  "observed_version": "0.146.1",
  "evidence_level": "native_runtime_probe",
  "captured_at": "2026-08-05T00:00:00Z",
  "artifact_sha256": "<64 hex>",
  "assertions": {},
  "redactions": []
}
```

这组字段来自 `01-RESEARCH.md:481-484`。`artifact_sha256` 必须指向实际探针二进制、schema 或打包工件，而不是 README。

**允许清单式请求摘要类比：** Spike 001 `src/main.rs:179-189`

```rust
serde_json::json!({
    "method": method,
    "path": path,
    "authorization_present": !authorization.is_empty(),
    "authorization_bearer": authorization.to_ascii_lowercase().starts_with("bearer "),
    "model": body.as_ref().and_then(|value| value.get("model")).and_then(Value::as_str),
    "stream": body.as_ref().and_then(|value| value.get("stream")).and_then(Value::as_bool),
    "tools_count": body.as_ref().and_then(|value| value.get("tools")).and_then(Value::as_array).map_or(0, Vec::len),
})
```

**不要复制**同文件 `fingerprint()` 的 `head:前8字节`；正式 evidence 只记录长度、布尔值或不可逆 SHA-256。

**canary 字节扫描类比：** Spike 012 `core.rs:1185-1216`

```rust
let bytes = serde_json::to_vec_pretty(&summary)?;
if bytes.windows(secret.len()).any(|window| window == secret) {
    bail!("live API key leaked into evidence summary");
}

fn files_contain(paths: &[PathBuf], needle: &[u8]) -> Result<bool> {
    for path in paths {
        if path.exists() {
            let bytes = fs::read(path)?;
            if bytes.windows(needle.len()).any(|window| window == needle) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
```

`run-phase1-contracts.ps1` 和 `local_only_boundary.rs` 必须扫描：

- 固定假 API Key canary；
- `experimental_bearer_token`；
- `Authorization`；
- `command_line`；
- 完整 TOML/config/app-server raw response 标志；
- SQLite dump 或真实 DB 复制进 contract fixture 的迹象。

**明确禁止复制的反例：** Spike 001 `inspect-windows.ps1:9-17` 把 `command_line = $_.CommandLine` 写入 evidence。正式 `probe-windows-host.ps1` 只能保存：

- PID/PPID；
- 可执行文件 hash 或受限路径类别；
- 进程角色；
- `command_line_has_codex_home`、`command_line_has_auth_value` 等布尔判据；
- 不得保存完整命令行。

---

### 9. Platform contract probes 与 smoke scripts

#### 9.1 `probe-codex.ps1`

**主类比：** Spike 001 `run.ps1`

可复制：

- 隔离 `CODEX_HOME`；
- 后台 helper 用 `Start-Process -WindowStyle Hidden`；
- 等待端口文件；
- `try/finally` 清理环境变量和子进程；
- 结构化 summary，失败返回非零。

关键片段（`run.ps1:14-21,72-77`）：

```powershell
$server = Start-Process -FilePath $binary `
  -ArgumentList @('serve',$portFile,$serverLog,'2') `
  -WorkingDirectory $root -WindowStyle Hidden -PassThru
try {
  for ($i = 0; $i -lt 100 -and -not (Test-Path -LiteralPath $portFile); $i++) {
    Start-Sleep -Milliseconds 100
  }
}
finally {
  Remove-Item Env:CODEX_HOME -ErrorAction SilentlyContinue
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
  }
}
```

必须替换：

- 目标版本固定为 0.146.1，并记录二进制 SHA-256。
- 生成 app-server JSON schema 并记录 schema SHA-256。
- 运行 initialize/initialized/config-read fixture。
- 不把 Codex 原始 `--json` 输出、配置正文或 Key 写进 committed evidence。
- 0.146.0 只能保留 historical fixture，不能作为 Phase 1 target verdict。

#### 9.2 `probe-windows-host.ps1`

**类比：** Spike 001 `inspect-windows.ps1`

复制 AppX/package 与 PID/PPID 发现形状；删除 `command_line` 字段，增加：

- package family/bundle version；
- exe SHA-256；
- 角色分类理由布尔值；
- `evidence_level`、`limitations`；
- canary scan 结果。

#### 9.3 `probe-wsl2.ps1`

**精确类比：** Spike 009 `inspect-wsl.ps1:9-33,48-90`

```powershell
$info = [System.Diagnostics.ProcessStartInfo]::new()
$info.FileName = $wsl
$info.UseShellExecute = $false
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$info.StandardOutputEncoding = [System.Text.Encoding]::Unicode
$info.StandardErrorEncoding = [System.Text.Encoding]::Unicode
$info.CreateNoWindow = $true
```

```powershell
$all = @(Get-Names -Arguments @('--list', '--quiet'))
$runningBefore = @(Get-Names -Arguments @('--list', '--running', '--quiet'))
# 只读 HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss
$runningAfter = @(Get-Names -Arguments @('--list', '--running', '--quiet'))

$evidence = [ordered]@{
    running_before = $runningBefore
    running_after = $runningAfter
    running_set_unchanged = (@(Compare-Object $runningBefore $runningAfter).Count -eq 0)
    commands_that_entered_a_distribution = 0
}
```

Phase 1 fixture 要补：

- registration GUID；
- `DistributionName`；
- `DefaultUid`；
- `command_target_resolvable`；
- 重复名称时必须 false/needs_attention；
- 不运行 `wsl.exe -d NAME -- ...`，不启动已停止发行版。

#### 9.4 `verify-windows-package.ps1`

**精确类比：** Spike 005 `verify-current-user-install.ps1:7-24,39-42`

```powershell
$process = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "installer failed with exit code $($process.ExitCode)" }

$installRoot = (Resolve-Path -LiteralPath $candidates[0]).Path
$localRoot = (Resolve-Path -LiteralPath $env:LOCALAPPDATA).Path
if (-not $installRoot.StartsWith($localRoot, [StringComparison]::OrdinalIgnoreCase)) {
  throw "installer escaped current-user LocalAppData: $installRoot"
}

$uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
if ($uninstall.ExitCode -ne 0) { throw "uninstaller failed with exit code $($uninstall.ExitCode)" }
```

Phase 1 必须增加：

- Authenticode 验证，不接受 updater `.sig` 代替；
- x64 与 ARM64 分开记录；
- 启动应用写设置 canary；
- 退出/重开后读回；
- 安装、升级、降级恢复路径分别记录；
- 未取得签名 runner/证书时 manifest verdict 保持 blocked/partial。

#### 9.5 `run-macos.sh`

**精确类比：** Spike 017 `run-macos.sh:9-18,26-45,48-68`

```zsh
if [[ "$(uname -s)" != "Darwin" ]]; then
  print -u2 "Spike 017 must run on macOS."
  exit 1
fi

major=$(/usr/bin/sw_vers -productVersion | /usr/bin/awk -F. '{print $1}')
if (( major < 14 )); then
  print -u2 "macOS 14 or newer is required."
  exit 1
fi
```

```zsh
signature=$(/usr/bin/codesign --verify --deep --strict "$USER_APP" >/dev/null 2>&1 && print verified || print unverified)
gatekeeper=$(/usr/sbin/spctl --assess --type execute "$USER_APP" >/dev/null 2>&1 && print accepted || print rejected)
/usr/bin/open "$USER_APP"
```

产品 smoke 固定目标：

- `~/Applications/GPTEasy.app`；
- Intel 与 Apple Silicon 分开运行；
- codesign、notary、Gatekeeper、bundle ID、minimum macOS 14；
- 写状态 canary，关闭/重开读回；
- Windows 或 cross-build 结果的 `evidence_level` 必须为非原生/partial，不能关闭 gate。

## Shared Patterns

### Rust 后端是唯一状态权威

**来源：** ADR-0002、ADR-0006；Spike 012 composition root。  
**应用到：** 所有 command、repository、React API。

- React 不缓存第二份权威 provider/environment/settings 状态。
- 不引入 Tauri SQL/fs/shell/http plugin。
- command 接收 typed DTO，不接收 SQL、任意数据库路径或任意 backup 路径。

### 先兼容性门禁，再任何写路径

**来源：** `01-RESEARCH.md:410-427`。  
**应用到：** startup、restore 后 reopen、fixture migration。

- existing DB 第一连接必须 `SQLITE_OPEN_READ_ONLY`。
- higher schema 分支不得经过 `Connection::open`、WAL、migration ledger 或 backup。

### 单一迁移事务

**来源：** Spike 012 immediate transaction primitive + `01-RESEARCH.md:590-627`。  
**应用到：** 所有 pending migrations。

- 事务内重读 `user_version`。
- 所有 pending migration、ledger、`user_version` 更新、FK/quick check 在同一事务。
- 任一失败不 commit，数据库仍保持原版本。

### 错误分类与公开脱敏

**来源：** Research Standard Stack (`thiserror`) 与 Phase 1 threat map。  
**应用到：** `state/error.rs`、所有 commands、contract scripts。

至少区分：

- `DatabaseTooNew`
- `ApplicationIdMismatch`
- `MigrationFailed`
- `MigrationChecksumMismatch`
- `BackupInvalid`
- `NoCompatibleBackup`
- `ConcurrentSchemaChange`
- `IntegrityCheckFailed`
- `RecoveryReplaceFailed`

`PublicStoreError` 只公开稳定 error code 和非敏感参数；不得公开 raw SQL、API Key、完整路径内容、SQLite dump 或底层错误链。

### 证据允许清单

**来源：** Spike 001/012/017。  
**应用到：** 所有 JSON manifests、CI artifacts、脚本 summary。

允许：

- version、hash、长度、布尔判据、角色、来源类型、计数；
- `evidence_level`、`limitations`、`redactions`。

禁止：

- API Key；
- `Authorization` 值；
- `experimental_bearer_token` 的值或配置正文；
- 完整命令行；
- app-server raw response；
- SQLite 数据库或 dump；
- 用户真实 Codex 配置。

### 测试先隔离复制，再执行

**来源：** Spike 009/012 scenario matrix。  
**应用到：** 所有 integration/fixture tests。

- 使用 `tempfile::TempDir`。
- committed fixture 只读校验后复制。
- 测试通过公开 `StateStore`/repository API，不直接篡改内部状态；只有故障注入和 higher-schema fixture setup 可以直接构造数据库。
- 凭据使用固定假 canary，断言失败消息不得输出其内容。

## 明确不可复制的 Spike 代码

| 路径/模式 | 原因 |
|---|---|
| Spike 012 `core.rs:1263-1273` 直接 `Connection::open` 后设置 WAL | 违反 higher-schema 先只读拒写 |
| Spike 012 `initialize_schema()` | schema 不包含 API Key、migration ledger、settings 等产品事实 |
| Spike 012/017 command 的 `map_err(|e| e.to_string())` | 可能把路径、SQL 或敏感上下文直接公开给 UI |
| Spike 009 `create_backup()` 的普通文件复制 | 不能替代 SQLite Online Backup API |
| Spike 001 `inspect-windows.ps1` 的 `command_line` 字段 | 正式 evidence 泄露面过大 |
| Spike 001 `fingerprint()` 的 `head:前8字节` | 不是不可逆脱敏 |
| Spike 017 `app_data_dir()` | Research 已锁定产品使用 `app_local_data_dir()` |
| Spike 005 updater 逻辑 | Phase 1 只做打包/签名/状态 smoke，不实现 updater 业务 |

## No Analog Found / Greenfield Scaffolding

| 文件/模块 | 角色 | 数据流 | 无类比原因 | 实现依据 |
|---|---|---|---|---|
| `state/preflight.rs` | service | file-I/O | 所有 Spike 都直接 RW open SQLite | `01-RESEARCH.md:410-427` |
| `state/migrations/mod.rs` | migration registry | batch | 没有永久 migration/checksum 双账本 | `01-RESEARCH.md:590-627` |
| `migrations/0001_initial.sql` | migration | CRUD | Spike schema 与 ADR-0006 冲突 | `01-RESEARCH.md:334-408` |
| `state/backup.rs` 的 Online Backup 部分 | service | file-I/O | Spike 只备份普通配置文件 | `01-RESEARCH.md:429-451` |
| higher-schema recovery-only gate | service/provider | request-response | Spike 没有“不开 RW connection”的启动模式 | `01-RESEARCH.md:454-479,630-646` |
| `domain/settings.rs`、settings repository | model/service | CRUD | 没有类型化单行应用设置类比 | 初始 schema `app_settings` |
| `migration_matrix.rs` | fixture test | batch | 没有 committed historical DB fixture | STATE-03、Validation Required Scenarios |
| `tests/fixtures/databases/**` | fixture | file-I/O | Spike scenario 都是运行时临时创建 | ADR-0006、STATE-03 |
| React TS `main.tsx`/基础配置 | component/config | event-driven/build | Spike UI 是 vanilla JS；产品树完全绿色 | Research Standard Stack/official scaffold |
| 完整 contract manifest schema + 多 needle canary scanner | config/test | batch/transform | Spike 只有部分 evidence 字段或单 secret 扫描 | `01-RESEARCH.md:481-484,561-565` |

## Planner 采用顺序

1. 先创建官方 Tauri/React TS 绿色骨架，但依赖版本以 `01-RESEARCH.md:94-175` 为准，不从 Spike 的宽松版本范围复制。
2. 复制 Spike 012 的 `lib.rs`/`main.rs`/`build.rs` 组合形状，替换为 typed bootstrap/recovery commands。
3. 绿色实现 `paths -> preflight -> ready connection -> repositories`，先完成 restart round-trip。
4. 绿色实现 Online Backup + 单事务 migration + recovery-only；仅复用 Spike 009 的排序/裁剪/矩阵组织方式。
5. 提交 v001 historical DB fixture 和 manifest 后，再写 migration/failure/higher-schema 矩阵。
6. 最后移植 Spike 001/005/009/017 的 contract/smoke harness，并在统一 runner 中执行 canary 扫描。
7. Codex 0.146.1、Windows signed x64/ARM64、真实 macOS Intel/Apple Silicon 证据未齐时，Phase 1 必须保持未完成。

## Metadata

**Analog search scope:**

- `.planning/spikes/001-codex-native-config-contract/`
- `.planning/spikes/005-desktop-install-update-matrix/`
- `.planning/spikes/009-wsl2-environment-lifecycle/`
- `.planning/spikes/012-desktop-provider-switch-e2e/`
- `.planning/spikes/017-macos-real-host-contract/`
- `.codex/skills/spike-findings-gpteasy/` 及 references

**覆盖统计（按 38 个模块）：**

- 精确类比：8
- 角色匹配或部分类比：20
- 无类比、必须绿色实现：10

**Pattern extraction date:** 2026-08-05  
**注意:** 工作树在分析前已有 `.planning/research/.cache/*.json` 未跟踪文件；本次未修改、删除或纳入这些文件。
