# 安全配置写入

## 目录

- [Requirements](#requirements)
- [How to Build It](#how-to-build-it)
- [What to Avoid](#what-to-avoid)
- [Constraints](#constraints)
- [Origin](#origin)

## Requirements

- 只修改 GPTEasy 管理的供应商字段，保留所有非 GPTEasy 配置。
- 写入前创建带时间戳的备份，每个受管环境默认只保留最近五份，并支持恢复。
- 使用同目录临时文件和平台原子替换，不能让目标文件暴露在部分写入或先删后建状态。
- 首次接管已有配置时使用结构化 TOML 迁移；管理区块建立后只替换 dotted-key 管理区块。
- 损坏、歧义、不支持的 TOML 形状或并发外部修改必须在替换前停止。

## How to Build It

### 1. 把配置处理建模为显式状态机

读取原始字节后，先识别状态，再决定写入策略：

| 状态 | 处理 |
|------|------|
| TOML 无法解析 | 停止，不创建备份或临时文件，不自动修复 |
| 没有管理区块 | 执行单事务首次结构化接管 |
| 恰好一对顺序正确的管理标记 | 只替换整个管理区块 |
| 标记缺失、重复或倒置 | 停止并提示恢复备份或人工处理 |
| 最终候选 TOML 无法重新解析 | 在备份和写入前停止 |

管理标记是独占整行的精确文本：

```text
# >>> GPTEasy managed provider >>>
# <<< GPTEasy managed provider <<<
```

不可变供应商 ID 是区块内的唯一注释：

```text
# GPTEasy provider-id: immutable-provider-id
```

重复 ID 注释、空 ID 或损坏标记都进入 `needs_attention`，不猜测修复。

### 2. 首次接管必须在一个结构化事务中完成

003a 与 003b 不能简单串联。结构化写入旧受管键后，再插入 dotted-key 区块会造成重复定义。已验证的首次接管顺序是：

1. 用 `toml_edit::DocumentMut` 解析原文件。
2. 删除旧顶层 `model` 和 `model_provider`。
3. 从 `model_providers` 中删除旧 `gpteasy` provider。
4. 检查 `model_providers` 父表形状。
5. 渲染剩余文档并保持原 LF/CRLF 风格。
6. 在根上下文建立唯一 dotted-key 管理区块。
7. 对最终文本再次执行 `DocumentMut` 解析。
8. 最终候选有效后，才创建备份并进入原子替换。

核心迁移模式：

```rust
let mut doc = original.parse::<DocumentMut>()?;
doc.remove("model");
doc.remove("model_provider");

if let Some(providers) = doc
    .get_mut("model_providers")
    .and_then(|item| item.as_table_mut())
{
    providers.remove("gpteasy");
    if providers.iter().any(|(_, item)| !item.is_table()) {
        return Err("refusing lossy migration".into());
    }
    if providers.is_empty() {
        doc.remove("model_providers");
    } else {
        providers.set_implicit(true);
    }
}
```

`model_providers.gpteasy.*` dotted keys 会隐式定义 `model_providers`。若剩余文件仍输出显式 `[model_providers]`，TOML 会把它视为重复表。因此：

- 父表只包含 provider 子表时，调用 `set_implicit(true)`。
- 父表为空时，删除父表。
- 父表包含直属非 table 值时，停止迁移，不能猜测改变语义。

### 3. 后续切换只替换 dotted-key 区块

不要使用 `[model_providers.gpteasy]` 表头作为可移动区块；表头会改变其后裸键作用域。使用根级 dotted keys：

```toml
# >>> GPTEasy managed provider >>>
# GPTEasy provider-id: immutable-provider-id
model = "provider-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "Provider"
model_providers.gpteasy.base_url = "https://provider.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.supports_websockets = false
model_providers.gpteasy.experimental_bearer_token = "API Key"
# <<< GPTEasy managed provider <<<
```

区块替换算法：

1. 按完整行扫描开始和结束标记。
2. 要求有且仅有一对，且开始位置早于结束位置。
3. 用新块替换 `[start, end]` 整段。
4. 保留区块之外的原始字节。
5. 替换后重新解析最终 TOML。

已验证的核心实现：

```rust
match (starts.as_slice(), ends.as_slice()) {
    ([(start, _)], [(_, end)]) if start < end => {
        let mut rendered = String::with_capacity(original.len() + block.len());
        rendered.push_str(&original[..*start]);
        rendered.push_str(&block);
        rendered.push_str(&original[*end..]);
        rendered.parse::<DocumentMut>()?;
        Ok(rendered)
    }
    _ => Err("managed block markers are missing, duplicated, or reversed".into()),
}
```

字节级“区块外不变”从管理区块建立后的第二次切换开始成立。首次结构化迁移只承诺保留语义、未知字段、旧 provider、注释和换行风格。

### 4. 使用完整的备份与原子替换协议

推荐顺序：

1. 读取原始字节并解析。
2. 生成最终文本并再次解析。
3. 创建 `.gpteasy-backups/config-<timestamp>.toml`，写入原始字节并 `sync_all`。
4. 按可排序时间戳文件名裁剪旧备份，只保留最近五份。
5. 在目标同目录用 `create_new` 创建临时文件。
6. 写入全部候选字节并 `sync_all`，继承目标权限；Unix 新文件初始权限为 `0600`。
7. 替换前再次读取目标，与最初字节比较；不同则删除临时文件并停止。
8. Windows 对已有文件调用 `ReplaceFileW(..., flags = 0)`。
9. macOS/Unix 在同一文件系统 `rename`，随后同步父目录。
10. 恢复备份也走同目录临时文件和原子替换。

并发保护不能省略：

```rust
if fs::read(path)? != original {
    let _ = fs::remove_file(&temp);
    return Err("configuration changed concurrently".into());
}
atomic_replace(path, &temp)?;
```

Windows 实现必须传 `flags = 0`：

```rust
let result = unsafe {
    ReplaceFileW(
        target_wide.as_ptr(),
        replacement_wide.as_ptr(),
        std::ptr::null(),
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    )
};
```

Microsoft 当前文档把 `REPLACEFILE_WRITE_THROUGH` 标记为不支持。临时文件必须自行 `sync_all`，不能把该标志描述成额外持久性保证。

### 5. 把供应商切换纳入跨资源 Saga

安全写文件不等于数据库状态一致。正式切换应由 `switch-consistency-reconciliation.md` 的 Saga 编排：

1. 在 SQLite 持久化旧/新配置哈希和 `prepared` 意图。
2. 调用本文件协议替换配置。
3. 提交环境当前供应商。
4. 在独立阶段处理桌面/CLI 重启。
5. 崩溃恢复时根据当前文件哈希前滚、回滚或进入外部配置。

### 6. 只记录哈希和状态

诊断可保存目标路径、原始/结果 SHA-256、备份路径、备份数量、替换阶段和错误类别。不要记录配置正文、管理区块或备份内容，因为它们包含明文 API Key。

## What to Avoid

- **不要使用普通 `toml` 序列化重建整份文件。** 会丢失注释、排序和用户格式。
- **不要把 003a 与 003b 简单串联。** 必须在一个结构化事务中移除旧受管节点并建立新区块。
- **不要让显式 `[model_providers]` 与 dotted keys 同时重复定义父表。**
- **不要猜测迁移 `model_providers` 中的直属未知值。**
- **不要把带表头的管理区块插到任意位置。**
- **不要在损坏 TOML、歧义标记或重复 provider ID 上自动修复。**
- **不要先删除 Windows 目标文件再重命名。**
- **不要依赖 `REPLACEFILE_WRITE_THROUGH`。**
- **不要认为原子替换自动解决并发覆盖。**
- **不要统一改写换行。**
- **不要把备份当成无敏感信息的普通日志。**

## Constraints

- 003a 已验证结构化写入、故障、并发、备份和恢复；003b 已验证管理区块替换和歧义停止。
- 006 在 Windows 上以 13 个场景验证了单事务首次接管、implicit 父表处理、后续区块外字节不变、故障、并发、五份备份和恢复。
- 首次迁移会重新渲染被移除受管节点附近的局部格式；精确字节保留从管理区块建立后的后续切换开始。
- `[model_providers]` 中的直属非 table 值不会自动迁移，必须作为外部配置交给用户处理。
- macOS Intel/Apple Silicon 目标编译通过，但真实 APFS 上的替换、崩溃和断电行为尚未执行。
- 安全保证覆盖解析失败、进程级故障和并发修改，不等同于真实断电一致性认证。
- 完全相同的标记若出现在 TOML 多行字符串的独占行，扫描器会安全拒绝而不是冒险写入。

## Origin

Synthesized from spikes: 003a, 003b, 006
Source files available in: `sources/003-a-toml-structural-edit/`, `sources/003-b-managed-block-edit/`, `sources/006-first-takeover-managed-block-transaction/`
