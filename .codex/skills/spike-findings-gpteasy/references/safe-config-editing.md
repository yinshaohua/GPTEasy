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

## How to Build It

### 1. 把配置处理建模为显式状态机

读取原始字节后，先识别以下状态，再决定写入策略：

| 状态 | 处理 |
|------|------|
| TOML 无法解析 | 停止，不创建临时文件，不尝试自动修复 |
| 没有管理区块，但已有 `model`、`model_provider` 或 `model_providers.gpteasy` | 执行首次结构化迁移 |
| 没有管理区块，也没有冲突键 | 可以直接建立管理区块 |
| 恰好一对顺序正确的管理标记 | 只替换区块 |
| 标记缺失、重复或倒置 | 停止并提示恢复备份或人工处理 |

管理区块标记必须是独占整行的精确文本，避免匹配普通注释：

```text
# >>> GPTEasy managed provider >>>
# <<< GPTEasy managed provider <<<
```

### 2. 首次接管使用结构化迁移

使用 `toml_edit::DocumentMut`，不要把整个文件反序列化后重新生成。结构化阶段只处理受管键：

```rust
let mut doc = original_text.parse::<DocumentMut>()?;
doc["model"] = value(provider.model);
doc["model_provider"] = value("gpteasy");

let providers = doc["model_providers"]
    .as_table_mut()
    .ok_or("model_providers is not a table")?;
let managed = providers["gpteasy"]
    .as_table_mut()
    .ok_or("gpteasy provider is not a table")?;
managed["name"] = value(provider.name);
managed["base_url"] = value(provider.base_url);
managed["wire_api"] = value("responses");
managed["supports_websockets"] = value(false);
managed["experimental_bearer_token"] = value(provider.bearer_token);
```

正式实现需要把 003a 与 003b 的结果组合成一个首次迁移事务：

1. 用 `DocumentMut` 验证并定位旧的受管顶层键和 provider 表。
2. 从结构化文档中移除将由管理区块接管的键，保留其他 provider 和未知字段。
3. 按原换行风格渲染剩余文档。
4. 在根上下文插入 dotted-key 管理区块。
5. 重新解析最终 TOML，确认没有重复键和作用域错误。
6. 仅在最终文本有效后进入备份和原子写入。

这一步不能简单地先写 003a 的表，再调用 003b；003b 会正确地把已有受管键视为冲突并拒绝插入。正式组合实现必须在同一个结构化迁移中消除冲突。

### 3. 后续切换只替换 dotted-key 区块

管理区块不能使用 `[model_providers.gpteasy]` 表头，因为 TOML 表头会改变其后裸键的作用域。使用根级 dotted keys：

```toml
# >>> GPTEasy managed provider >>>
model = "provider-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "Provider"
model_providers.gpteasy.base_url = "https://provider.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.supports_websockets = false
model_providers.gpteasy.experimental_bearer_token = "API Key"
# <<< GPTEasy managed provider <<<
```

区块替换算法应：

1. 按完整行扫描开始和结束标记。
2. 无标记时先做结构化冲突检查。
3. 有且仅有一对正确标记时替换 `[start, end]` 整段。
4. 替换后重新用 `DocumentMut` 解析。
5. 保留区块之外的原始字节和原始 LF/CRLF 风格。

已验证的核心判定：

```rust
match (starts.as_slice(), ends.as_slice()) {
    ([], []) => {
        let doc = original.parse::<DocumentMut>()?;
        if has_managed_keys(&doc) {
            return Err("existing managed keys require structural migration".into());
        }
        Ok(format!("{block}{original}"))
    }
    ([(start, _)], [(_, end)]) if start < end => {
        Ok(format!("{}{}{}", &original[..*start], block, &original[*end..]))
    }
    _ => Err("managed block markers are missing, duplicated, or reversed".into()),
}
```

### 4. 使用完整的备份与原子替换协议

推荐顺序：

1. 读取原始字节并解析。
2. 生成最终文本并再次解析。
3. 创建 `.gpteasy-backups/config-<timestamp>.toml`，写入原始字节并 `sync_all`。
4. 按文件名排序裁剪旧备份，只保留最近五份。
5. 在目标同目录使用 `create_new` 创建临时文件。
6. 写入全部字节并 `sync_all`，继承目标权限；Unix 新文件初始权限设为 `0600`。
7. 替换前再次读取目标，与最初字节比较；不同则停止，避免覆盖并发编辑。
8. Windows 对已有文件调用 `ReplaceFileW(..., REPLACEFILE_WRITE_THROUGH, ...)`。
9. macOS/Unix 在同一文件系统 `rename`，随后同步父目录。
10. 恢复备份也必须走同样的临时文件和原子替换流程。

并发保护不能省略：

```rust
if fs::read(path)? != original {
    let _ = fs::remove_file(&temp);
    return Err("configuration changed concurrently".into());
}
atomic_replace(path, &temp)?;
```

### 5. 记录哈希和状态，不记录正文

诊断可以保存目标路径、原始/结果哈希、备份路径、备份数量、原子替换结果和错误类别。不要记录配置正文或管理区块，因为其中包含明文 API Key。

## What to Avoid

- **不要使用普通 `toml` 序列化重建整份文件。** 会丢失注释、排序和用户格式。
- **不要把带表头的管理区块插到任意位置。** 它会改变后续键的 TOML 作用域。
- **不要在首次接管时直接插入 dotted keys。** 已有顶层键会形成重复定义。
- **不要在损坏 TOML 或歧义标记上猜测性修复。** 安全策略是停止修改。
- **不要先删除 Windows 目标文件再重命名。** 删除成功、移动失败会制造空窗。
- **不要认为原子替换自动解决并发覆盖。** 必须在替换前比较原始字节。
- **不要统一改写换行。** 输入为 CRLF 时，新增和替换内容也必须使用 CRLF。
- **不要把备份当成无敏感信息的普通日志。** 备份同样包含明文凭据。

## Constraints

- 003a 已在 Windows 通过 6 个结构化写入、故障、并发、备份和恢复场景；003b 已在 Windows 通过 11 个管理区块场景。
- “首次结构化迁移后立即建立管理区块”的组合事务尚未由一个端到端 Spike 实现；正式开发应优先为该组合补测试。
- 结构化迁移只能近似保留被修改节点附近的格式；区块外字节完全不变只在管理区块已经建立后成立。
- macOS Intel/Apple Silicon 目标编译通过，但真实 APFS 上的替换、崩溃和断电行为尚未执行。
- 安全保证覆盖解析失败、进程级故障和并发修改，不等同于完成了真实断电一致性认证。
- 完全相同的标记若作为 TOML 多行字符串中的独占行出现，扫描器会安全拒绝而不是冒险写入。

## Origin

Synthesized from spikes: 003a, 003b
Source files available in: `sources/003-a-toml-structural-edit/`, `sources/003-b-managed-block-edit/`
