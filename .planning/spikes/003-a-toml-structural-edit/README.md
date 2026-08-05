---
spike: 003a
name: toml-structural-edit
type: comparison
validates: "Given 含未知字段、注释和不同换行的 Codex TOML，when 使用结构化 TOML 编辑执行供应商切换，then 能保留非受管配置并安全原子落盘、备份和恢复"
verdict: VALIDATED
related: [001, 003b, 004]
tags: [rust, toml, atomic-write, backup, comparison]
---

# Spike 003a: 结构化 TOML 编辑

## What This Validates

**Given** 含未知字段、注释、旧供应商配置和不同换行格式的 Codex TOML，  
**when** Rust 使用结构化 TOML 编辑更新当前模型和 GPTEasy provider，  
**then** 能保留非受管配置，并通过并发检查、同目录临时文件、原子替换、备份及恢复安全落盘。

## Research

### 已检查的资料

- `toml_edit::DocumentMut`：`https://docs.rs/toml_edit/latest/toml_edit/struct.DocumentMut.html`
- Rust `std::fs::rename`：`https://doc.rust-lang.org/std/fs/fn.rename.html`
- Windows `ReplaceFileW`：`https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew`
- Apple 文件系统编程指南中的安全保存：`https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/TechniquesforReadingandWritingCustomFiles/TechniquesforReadingandWritingCustomFiles.html`

### 方案比较

| 方案 | 优点 | 缺点 | 状态 |
|---|---|---|---|
| `toml` 反序列化后整体重新生成 | 类型模型简单 | 会丢失注释、排序和原始格式，不适合用户配置 | 淘汰 |
| `toml_edit::DocumentMut` 定点更新 | 保留未修改节点的注释和格式；可安全处理已有顶层键和 provider 表 | 被修改键的局部排版可能变化；损坏 TOML 必须拒绝 | **本 Spike 采用** |
| 纯文本管理区块 | 已建立区块后可字节级保留其余文件 | 首次接管已有 `model`/`model_provider` 会产生重复键，需要结构化迁移 | 见 003b |

### 原子写入协议

1. 读取原始字节并解析 TOML；解析失败时不创建临时文件、不修改目标。
2. 只修改顶层 `model`、`model_provider` 和 `[model_providers.gpteasy]`。
3. 在目标同目录创建独占临时文件，完整写入并 `sync_all`。
4. 修改前把原始字节写入 `.gpteasy-backups/` 并同步，随后裁剪为最近五份。
5. 原子替换前再次读取目标并与初始字节比较；发现外部编辑立即停止。
6. Windows 使用 `ReplaceFileW` 替换已有文件；macOS/Unix 使用同文件系统 `rename` 并同步父目录。
7. 任何替换前故障都保留原文件；恢复同样通过临时文件和原子替换执行。

## How to Run

```powershell
.\.planning\spikes\003-a-toml-structural-edit\run.ps1
```

macOS 两种目标的编译检查：

```powershell
cargo check --manifest-path .planning/spikes/003-a-toml-structural-edit/Cargo.toml --target x86_64-apple-darwin
cargo check --manifest-path .planning/spikes/003-a-toml-structural-edit/Cargo.toml --target aarch64-apple-darwin
```

## What to Expect

`.run/summary.json` 应显示 6/6 场景通过：

1. 注释、未知字段、旧 provider 和 CRLF 保留。
2. 损坏 TOML 拒绝修改。
3. 原子替换前注入失败时保留原文件。
4. 检测到并发外部编辑时停止，不覆盖外部内容。
5. 连续七次切换后只保留最近五份备份。
6. 从最新备份原子恢复到切换前字节。

## Observability

- 测试只在 `.run/` 沙盒中修改文件。
- `summary.json` 只记录场景和布尔结果，不输出 Bearer token。
- 真实实现可沿用同一报告结构记录备份路径、原始/结果哈希、原子替换结果和错误类别，但不得记录文件正文。

## Investigation Trail

1. **格式保留**：`toml_edit` 保留了未修改注释、未知顶层字段、项目表和旧 provider 表。
2. **换行问题**：新增 TOML 节点默认使用 LF；实现必须在渲染后恢复输入文件原有的 LF/CRLF 风格，否则 Windows 文件可能出现混合换行。
3. **已有 provider 不应删除**：当前供应商切换只更新稳定的 `model_providers.gpteasy` 表，外部 provider 定义继续保留。
4. **解析失败前置**：损坏 TOML 在备份和临时写入前就被拒绝，避免“修复”用户文件或扩大损坏。
5. **并发覆盖风险**：仅做原子替换仍可能覆盖编辑器在读取后的修改，因此加入替换前原始字节比较。
6. **Windows 替换语义**：使用 `ReplaceFileW` 而不是删除原文件再重命名，避免“删除成功、移动失败”的空窗，并尽量保留原文件元数据。
7. **备份权限**：Unix 新备份以 `0600` 创建；Windows 继承当前用户配置目录 ACL。正式实现还应在启动诊断中检查异常宽松权限。
8. **macOS 证据边界**：x64 与 Apple Silicon Rust 目标均通过 `cargo check`，但本次没有在真实 APFS 上执行崩溃/断电测试。

## Results

### Verdict: VALIDATED ✓

结构化编辑能够承担**首次接管、外部配置读取和安全迁移**。Windows 上 6 个场景全部通过，macOS Intel/Apple Silicon 目标均编译通过。

### 限制

- 结论证明的是进程级故障、解析错误和并发修改安全，不等同于真实断电一致性测试。
- `toml_edit` 不能保证被修改节点的每个空格都字节不变，因此不满足“整个非管理区块绝对字节一致”的更强目标。
- 当前示例把 Key 写入 `experimental_bearer_token`；备份也会包含明文 Key，必须与主配置采用相同的用户级访问保护。

### Comparison verdict

**003a 是首次接管和异常配置处理的胜者。** 推荐正式实现使用结构化编辑完成首次迁移和冲突检查；若产品坚持管理区块，则迁移成功后再进入 003b 的区块替换模式。
