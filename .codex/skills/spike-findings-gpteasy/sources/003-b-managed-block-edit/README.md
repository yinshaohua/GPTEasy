---
spike: 003b
name: managed-block-edit
type: comparison
validates: "Given 含未知字段、注释、损坏或重复管理区块的 Codex TOML，when 只替换 GPTEasy 管理区块，then 能保留文件其余字节并在歧义时停止修改"
verdict: PARTIAL
related: [001, 003a, 004]
tags: [rust, managed-block, atomic-write, backup, comparison]
---

# Spike 003b: GPTEasy 管理区块替换

## What This Validates

**Given** 包含未知字段、注释、不同换行或损坏管理标记的 Codex TOML，  
**when** Rust 只插入或替换带明确边界的 GPTEasy 区块，  
**then** 已建立区块之外的字节保持不变，并在标记缺失、重复、倒置或配置冲突时停止修改。

## Research

### 方案比较

| 区块格式 | 优点 | 致命问题 | 状态 |
|---|---|---|---|
| 区块中使用 `[model_providers.gpteasy]` 表头 | 可读性好 | 表头会改变其后裸键的 TOML 归属，不能安全插到任意位置 | 淘汰 |
| 区块中使用顶层键与 dotted keys | 不改变 TOML 当前表上下文，可安全放在文件开头 | 已存在 `model`/`model_provider` 时会形成重复定义 | **采用，但首次迁移受限** |
| 先结构化迁移，再长期替换区块 | 首次接管安全；后续区块外字节完全不动 | 实际上是 003a + 003b 的混合方案 | **正式实现推荐** |

采用的区块只使用顶层键和 dotted keys：

```toml
# >>> GPTEasy managed provider >>>
model = "provider-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "Provider"
model_providers.gpteasy.base_url = "https://provider.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.supports_websockets = false
model_providers.gpteasy.experimental_bearer_token = "..."
# <<< GPTEasy managed provider <<<
```

## How to Run

```powershell
.\.planning\spikes\003-b-managed-block-edit\run.ps1
```

macOS 两种目标的编译检查：

```powershell
cargo check --manifest-path .planning/spikes/003-b-managed-block-edit/Cargo.toml --target x86_64-apple-darwin
cargo check --manifest-path .planning/spikes/003-b-managed-block-edit/Cargo.toml --target aarch64-apple-darwin
```

## What to Expect

`.run/summary.json` 应显示 11/11 场景通过：

- 无冲突配置首次插入区块并保持 TOML 有效。
- 已存在受管顶层键时返回“需要结构化迁移”，不写文件。
- 已建立区块更新后，区块前后字节完全一致。
- 缺失结束标记、重复开始标记、倒置标记均拒绝修改。
- CRLF 保留。
- 原子替换前故障与并发外部编辑不覆盖目标。
- 只保留最近五份备份，并可从最新备份恢复。

## Observability

- 标记扫描、冲突检查、备份、临时写入和替换结果可分别记录。
- 不记录管理区块正文，因为其中包含明文 Key。
- 标记错误必须输出可定位的错误类别，但不自动删除、合并或修复区块。

## Investigation Trail

1. **表头区块不可任意插入**：TOML 表头会改变后续键的作用域，因此管理区块不能简单包含 `[model_providers.gpteasy]` 后追加到文件。
2. **dotted keys 解决作用域问题**：使用 `model_providers.gpteasy.*` 可让整个区块位于根上下文，并安全放在文件开头。
3. **首次接管冲突**：多数真实 Codex 配置已经有 `model` 或 `model_provider`。纯区块插入会产生重复键，所以实现选择停止并要求 003a 结构化迁移，而不是猜测性删除。
4. **后续更新优势**：一旦区块存在，替换可精确限定在两个完整行标记之间，区块外字节保持不变。
5. **损坏处理**：缺失、重复或倒置标记全部视为歧义，原文件保持不变；这符合 CONTEXT.md 的“损坏、重复或存在歧义时停止修改”。
6. **字符串中的伪标记**：扫描器只识别独占整行的精确标记，降低误报；若用户在 TOML 多行字符串中放入完全相同的独占行，仍会安全停止而不是误写。
7. **原子写入和备份**：采用与 003a 相同的同目录临时文件、同步、并发字节检查、Windows `ReplaceFileW`、Unix rename 和最近五份备份。
8. **macOS 证据边界**：Intel/Apple Silicon 目标均编译通过，真实文件系统替换仍待 Mac 执行。

## Results

### Verdict: PARTIAL ⚠️

管理区块对**已经完成接管的配置**非常可靠：区块外字节精确保留，标记损坏时安全停止，11 个 Windows 场景全部通过。

但它不能单独安全接管已经存在 `model`、`model_provider` 或同名 provider 表的真实配置。因此，纯管理区块不是完整首版方案。

### Head-to-head

| 维度 | 003a 结构化编辑 | 003b 管理区块 |
|---|---|---|
| 首次接管已有 Codex 配置 | **胜** | 需要迁移 |
| 损坏 TOML | 安全拒绝 | 安全拒绝 |
| 区块外字节完全不变 | 近似保留 | **胜** |
| 重复/损坏标记 | 不适用 | **胜，明确拒绝** |
| 现有顶层 `model`/`model_provider` | **可更新** | 不能直接插入 |
| 长期切换可审计性 | 中 | **高** |

### 推荐组合

1. 首次接管或发现外部配置时，用 003a 解析并结构化迁移受管键。
2. 迁移成功并通过重新解析后，写入 003b 的 dotted-key 管理区块。
3. 后续切换只替换区块；标记异常时停止并要求用户恢复备份或人工处理。
