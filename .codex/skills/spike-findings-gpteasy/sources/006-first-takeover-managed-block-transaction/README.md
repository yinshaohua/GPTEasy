---
spike: 006
name: first-takeover-managed-block-transaction
type: standard
validates: "Given 已有受管键、未知字段、注释和不同换行的 Codex TOML，when 首次接管在一个事务中完成结构化迁移并建立 dotted-key 管理区块，then 最终配置有效、非受管配置保留、后续切换区块外字节不变且故障可恢复"
verdict: VALIDATED
related: [003a, 003b, 007, 008]
tags: [rust, toml, migration, managed-block, atomic-write, integration]
---

# Spike 006: 首次接管与管理区块事务

## What This Validates

**Given** 已有 `model`、`model_provider`、旧 provider、未知字段、注释和 LF/CRLF 的 Codex TOML，  
**when** GPTEasy 在一次写入事务中移除旧受管键、建立唯一 dotted-key 管理区块，并在后续切换时只替换该区块，  
**then** 最终配置始终有效，非受管配置得到保留，后续切换区块外字节不变，并且故障、并发修改、备份裁剪和恢复都安全。

## Research

### 已检查的资料

- `toml_edit::DocumentMut` 与 `Table::set_implicit`：`https://docs.rs/toml_edit/0.23`
- TOML 1.0 dotted keys 与 table 定义规则：`https://toml.io/en/v1.0.0`
- Windows `ReplaceFileW`：`https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew`
- Rust `std::fs::rename`：`https://doc.rust-lang.org/std/fs/fn.rename.html`

### 方案比较

| 方案 | 优点 | 缺点 | 状态 |
|---|---|---|---|
| 先运行 003a，再直接运行 003b | 可复用已有代码 | 003b 会把 003a 写出的受管键识别为冲突，无法建立区块 | 淘汰 |
| 纯文本删除旧键后插入区块 | 容易保持大部分字节 | 很难正确处理 TOML 表作用域、引号和注释 | 淘汰 |
| 单事务结构化迁移：移除旧受管节点、把空父表转为 implicit、插入区块并重新解析 | 首次迁移安全，后续可进入字节级区块替换 | 首次迁移会重新渲染受管节点附近格式；不支持 `model_providers` 父表中的非 table 直属值 | **采用** |

### 关键 TOML 约束

`model_providers.gpteasy.*` dotted keys 会隐式定义 `model_providers`。如果文件后面仍有显式 `[model_providers]`，TOML 会把它视为重复定义。因此首次迁移必须：

1. 删除旧 `model`、`model_provider` 和 `model_providers.gpteasy`。
2. 若显式 `[model_providers]` 只包含子 provider 表，把父表转换为 implicit。
3. 若父表含直属非 table 值，停止迁移，不能猜测性改变其语义。
4. 在根上下文插入 dotted-key 管理区块。
5. 对最终文本重新解析后才允许备份和写入。

### Windows 替换标志修正

Microsoft 当前文档将 `REPLACEFILE_WRITE_THROUGH` 标记为“不支持”。003a/003b 虽然在当前 Windows 上使用该值通过测试，正式模式不应依赖未支持标志。本 Spike 改为：

- 临时文件先 `sync_all`
- `ReplaceFileW` 的 flags 传 `0`
- 不把该 API 描述成提供额外的 write-through 保证

## How to Run

```powershell
.\.planning\spikes\006-first-takeover-managed-block-transaction\run.ps1
```

## What to Expect

`.run/summary.json` 应显示 13/13：

1. 已有顶层受管键和旧 `gpteasy` provider 被迁移为唯一管理区块。
2. 只含子 provider 的显式 `[model_providers]` 父表转换为 implicit。
3. 父表含直属未知值时在备份和写入前安全停止。
4. 无冲突配置可首次建立区块。
5. 第二次切换保持区块外字节完全一致。
6. 缺失、重复和倒置标记均停止修改。
7. 区块外重复受管键导致最终解析失败并停止。
8. 替换前故障保留原文件。
9. 并发外部编辑不被覆盖。
10. 只保留最近五份备份。
11. 最新备份可通过原子替换恢复。

## Observability

- 每次运行在 `.run/session-*` 沙盒中生成独立配置和备份。
- `.run/summary.json` 只记录场景名、通过状态、长度和脱敏错误类别。
- 测试使用明显的假 Key；配置正文和管理区块不进入汇总日志。

## Investigation Trail

1. **组合不能简单串联**：003a 写出结构化受管键后，003b 会正确拒绝再次插入重复键，所以必须在一个结构化事务中先移除旧受管节点。
2. **dotted key 与显式父表冲突**：`model_providers.gpteasy.*` 会隐式创建父表；后续显式 `[model_providers]` 会构成重复表。
3. **空父表可安全隐式化**：使用 `Table::set_implicit(true)` 后，旧 provider 子表继续保留，显式父表头消失，最终 TOML 可重新解析。
4. **未知直属值不猜测迁移**：若 `[model_providers]` 中存在 `custom = ...` 这类非 provider 子表值，实验在创建备份前停止。这类形状不符合当前 Codex provider 目录结构，安全拒绝优于丢失或改变语义。
5. **首次迁移保留语义**：旧 provider、项目 trust 配置、未知顶层布尔值、注释和 CRLF 均保留；旧受管 provider 被唯一新区块替代。
6. **后续切换达到字节边界**：管理区块建立后，第二次切换只替换两个标记之间的字节，区块外前后内容逐字节一致。
7. **最终解析是写入门禁**：即使标记本身正常，只要区块外仍有重复 `model`，最终解析就在备份前失败。
8. **原子写入边界保持**：替换前故障、并发编辑、备份裁剪和恢复继续沿用 003a/003b 已验证协议。
9. **Windows 标志修正**：依据 Microsoft 文档移除未支持的 `REPLACEFILE_WRITE_THROUGH`，依赖临时文件同步与 `ReplaceFileW` 的替换语义。

## Results

### Verdict: VALIDATED ✓

Windows 上 13 个场景全部通过。003a 与 003b 之间的关键接缝已经闭合：首次接管可以在单事务中建立唯一管理区块，后续切换可以保持区块外字节完全不变。

### 已验证

- 常见 Codex provider 结构中的已有顶层键、旧 provider 和旧 GPTEasy 表可安全迁移。
- 显式但只包含 provider 子表的 `[model_providers]` 可转换为 implicit 父表。
- 最终 TOML 在备份和写入前重新解析。
- 损坏标记、重复受管键和不支持的父表形状都在修改前安全停止。
- 失败、并发、最近五份备份和恢复协议保持成立。

### 限制

- 首次迁移会重新渲染被移除受管节点附近的局部格式；字节级保留从管理区块建立后的第二次切换开始。
- `[model_providers]` 中的直属非 table 值不会自动迁移。这种未知形状必须作为外部配置交由用户处理。
- 尚未在真实 macOS/APFS 上执行替换和崩溃测试。
- 结论不等同于真实断电一致性认证。
