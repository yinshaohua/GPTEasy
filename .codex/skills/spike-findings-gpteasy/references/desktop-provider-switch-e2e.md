# 桌面供应商切换端到端

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- Tauri UI 只能发起意图；供应商验证、配置迁移、SQLite Saga、最终有效配置协调和重启计划由同一个 Rust 后端流程完成。
- 供应商保存前必须完成模型发现、Responses API 流式响应、strict function call 和工具结果回传闭环。
- 验证结果必须绑定同一组服务地址、API Key 和默认模型；任一字段变化都必须重新验证。
- 配置修改必须保留非 GPTEasy 字段，先备份，再原子替换，并默认只保留最近五份备份。
- 首次接管使用结构化 TOML 迁移；管理区块建立后只替换唯一 dotted-key 管理区块。
- SQLite 与 Codex 配置文件之间必须使用可恢复 Saga；未知配置哈希不得被自动覆盖。
- 取消必须发生在验证、备份、数据库意图和配置写入之前。
- 桌面重启失败不得回滚已生效配置；CLI 不得被静默终止。
- API Key 不得进入 SQLite、事件日志、诊断 evidence 或进程命令行。

## How to Build It

### 1. 只暴露一个后端切换入口

不要让前端依次调用“验证”“保存”“写配置”“重启”等松散 command。UI 收集供应商输入和用户决策后，调用一个 Rust 入口：

```rust
pub fn switch_provider(
    input: ProviderInput,
    decision: RestartDecision,
) -> Result<PipelineReport> {
    if decision == RestartDecision::Cancel {
        return Ok(PipelineReport::cancelled());
    }

    let verified = validate_provider(input)?;
    execute_switch(&verified, decision)
}
```

这样可以保证：

- UI 不能伪造“已验证”状态。
- 验证后使用的 Key、地址和模型仍是同一组合。
- 重复点击或字段修改不会绕过后端门禁。
- 取消路径不创建备份、不打开写事务、不改变配置。

耗时验证应在 Tauri 异步 command 或后台任务中运行，并把阶段进度作为事件发送给 UI；不能为了阶段展示而拆散安全边界。

### 2. 用类型和组合指纹封住验证—保存接缝

核心类型：

```rust
pub struct ProviderInput {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

pub struct VerifiedProvider {
    pub input: ProviderInput,
    pub evidence: ValidationEvidence,
}
```

组合指纹使用版本化域分隔：

```rust
pub fn combination_fingerprint(input: &ProviderInput) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-provider-combination-v1\0");
    hasher.update(input.base_url.as_bytes());
    hasher.update(b"\0");
    hasher.update(input.model.as_bytes());
    hasher.update(b"\0");
    hasher.update(input.api_key.as_bytes());
    hex::encode(hasher.finalize())
}
```

进入 Saga 前重新计算并比较：

```rust
let actual = combination_fingerprint(&verified.input);
ensure!(
    verified.evidence.ok
        && verified.evidence.category == "validated"
        && actual == verified.evidence.combination_fingerprint,
    "validated provider combination changed after validation"
);
```

SQLite 保存不可变供应商 ID、地址、模型和组合指纹，不保存 API Key。明文 Key 只存在于：

- 当前 Rust 调用的内存。
- 目标 Codex 配置及其备份。
- 用户明确导出的敏感 Linux 脚本。

### 3. 验证通过后先准备完整配置事务

在修改 SQLite 当前状态前：

1. 读取原配置字节并计算 `old_hash`。
2. 根据管理区块状态执行首次结构化迁移或后续区块替换。
3. 重新解析候选 TOML。
4. 计算 `new_hash`。
5. 创建带时间戳的备份并裁剪到五份。
6. 记录配置文件并发指纹。

准备结果至少包含：

```rust
struct PreparedConfig {
    old_hash: String,
    new_hash: String,
    backup: PathBuf,
    candidate: Vec<u8>,
}
```

首次接管必须复用安全配置写入蓝图中的单事务迁移，不能假定管理区块已经存在。

### 4. 先持久化意图，再替换配置

使用 `BEGIN IMMEDIATE` 把以下事实写入 `switch_operations`：

- operation ID、environment ID
- old/new provider ID
- old/new 配置哈希
- 备份路径
- 用户的 restart decision
- 桌面宿主和 CLI 是否存在
- `phase = prepared`

同一个事务可 upsert 已验证供应商元数据和组合指纹，但不能把环境的当前供应商提前改成新值。

固定执行顺序：

```text
validated combination
  → prepare candidate and backup
  → persist prepared intent
  → atomically replace config
  → commit environment current provider
  → reconcile effective Codex config
  → execute or defer restart plan
```

事件日志只记录 operation ID、provider ID、组合指纹、哈希、阶段和脱敏错误分类。

### 5. 用配置哈希完成崩溃恢复

启动时扫描未完成操作，并读取当前配置哈希：

| 当前哈希 | 恢复动作 |
|----------|----------|
| `old_hash` | 数据库回滚为旧供应商，操作标记 `rolled_back` |
| `new_hash` | 数据库前滚为新供应商，然后继续重启收尾 |
| 其他哈希 | 环境和操作进入 `needs_attention`，保留外部编辑 |

不要根据最后一条日志、SQLite phase 或备份是否存在猜测磁盘状态。配置文件哈希是恢复分支的事实来源。

### 6. 把重启变成配置提交后的可重试副作用

重启结果不属于配置原子事务：

| 决策/进程状态 | 结果 |
|---------------|------|
| `later` 且存在桌面或 CLI | `pending_restart` |
| `immediate` 且桌面重启成功、无 CLI | `completed` |
| `immediate` 且桌面重启失败 | `pending_restart` |
| `immediate` 且 CLI 存在 | 即使桌面成功也保持 `pending_restart` |

桌面宿主只按已验证根 PID 和平台激活方式重启。CLI 仅提示用户回到原终端退出并重新运行，不尝试恢复 TTY、cwd、stdin 或会话。

### 7. 用 Codex app-server 确认最终有效状态

配置写入和数据库提交后，调用：

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

- effective `model`
- effective `model_provider`
- 两个字段的 origin 类型
- 配置层名称摘要

Windows 的 cwd 和 trust 路径必须去除 `\\?\` verbatim 前缀；可使用 `dunce::canonicalize`。否则项目 trust 键与 app-server cwd 可能看似相同却不能匹配。

启动 app-server 时优先定位 npm 包中的原生 `vendor/.../codex.exe`，并在 Windows 隐藏窗口运行。`codex.cmd` 的 stdio 包装层不是稳定的 JSON-RPC 启动面。

协调状态沿用：

- `managed_current`
- `managed_overridden`
- `managed_drifted`
- `external_*`
- `needs_attention`

项目层或会话层覆盖只产生 `managed_overridden`，不得反复改写用户配置争夺优先级。

### 8. 分离 operational workspace 与可导出 evidence

含真实 Key 的隔离 Codex 配置和备份属于 operational workspace；允许泄漏扫描和诊断导出的只有：

- 脱敏事件 JSONL
- SQLite 数据库
- 阶段摘要
- 配置哈希和来源摘要

不要扫描 operational workspace 并把“目标配置含 Key”误判为诊断泄漏；也不要为了通过扫描而把真实目标配置复制到 evidence。

## What to Avoid

- **不要让 UI 保存布尔型 `validated = true`。**
- **不要把验证和保存拆成可由用户交错调用的 command。**
- **不要只按供应商 ID 复用旧验证。** 地址、模型或 Key 变化都必须使组合指纹变化。
- **不要把 API Key 放进 SQLite。**
- **不要在 `prepared` 意图持久化前替换配置。**
- **不要在配置替换前把数据库当前供应商改成新值。**
- **不要在未知哈希时恢复备份或重写配置。**
- **不要因桌面重启失败回滚新配置。**
- **不要终止本机 CLI。**
- **不要保存 app-server 原始响应或完整进程命令行。**
- **不要让项目层覆盖触发用户层自动争夺。**
- **不要把包含受管配置的 workspace 当成可导出诊断。**

## Constraints

- Spike 012 的 15/15 确定性矩阵、真实供应商闭环、隔离配置切换、SQLite Saga、app-server 协调和 Tauri release smoke 已通过。
- 真实 Windows 进程扫描识别了桌面宿主、bundled Codex 和 CLI，但为避免中断当前会话，没有执行真实终止和重新激活。
- 真实供应商结论只绑定 2026-08-05 执行时的单个地址、Key 和模型组合，不代表持续健康。
- app-server 行为基于 `codex-cli 0.146.0`，升级后必须回归 schema、stdio 启动方式、字段 origins 和配置层行为。
- 多 GPTEasy 实例同时写 SQLite/配置、系统关机、磁盘损坏、SQLite 丢失和真实断电尚未覆盖。

## Origin

Synthesized from spikes: 012
Source files available in: `sources/012-desktop-provider-switch-e2e/`
