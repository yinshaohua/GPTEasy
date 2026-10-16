# WSL2 注册表辅助子项导致供应商选择失败：修复方案

日期：2026-10-16
状态：待实现。本文件仅定义修复和验收，未实施程序变更。

## 目标与范围

让 WSL2 探测正确忽略 Lxss 下的非发行版配置子项，在包含 `Notifications` 的真实 Windows 环境中正常发现发行版。保留对损坏注册、访问拒绝、枚举失败的错误保护，并补足问题日志证据。

本任务限于注册表发现边界、相关诊断和回归测试。供应商应用、guest 写入协议、生命周期恢复、凭据处理、数据库身份和 Linux 导出行为沿用现有契约。不要求调整两条错误提示的布局或文案。

实现会话先读取根目录 `AGENTS.md`、`CONTEXT.md`、Spike skill，以及 ADR-0027 和 ADR-0031。用户尚未授权发布；实现完成后保持未发布状态。

## 已确认事实

2026-10-16 在用户当前测试主机进行只读诊断：

- 已安装并正在运行 GPTEasy 1.5.0。
- `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss` 下同时存在 `Notifications` 与一个 Ubuntu 的 GUID 子项。
- `Notifications` 只有 `DrvFsPerfDisabled` 配置值，没有 `DistributionName`。
- Ubuntu 注册项包含 `DistributionName=Ubuntu`、`Version=2`、`DefaultUid=1000`，其 BasePath 目录存在。
- `wsl.exe --list --verbose` 显示 Ubuntu / Running / 2；`--list --quiet` 和 `--list --running --quiet` 均返回 Ubuntu。
- 已安装应用的 `%LOCALAPPDATA%\com.gpteasy.desktop\issue-log.jsonl` 多次记录 `event=wsl.list message=wsl.registry_unavailable details=category=probe_failed`，启动恢复也记录同一错误。

已有 Windows 测试二进制实际复现：

```powershell
.\src-tauri\target\debug\deps\gpteasy_lib-4051434b61e28266.exe `
  wsl::tests::real_system_probe_does_not_start_or_stop_distributions `
  --ignored --exact --nocapture
```

结果为 `ProbeFailed / wsl.registry_unavailable`，在 `probe().expect("probe")` 失败，耗时约 0.06 秒。二进制哈希属于诊断时的构建产物，后续验收应从当前代码重建，不能依赖这个文件名。

## 根因与调用链

`src-tauri/src/wsl.rs` 的 `read_registry_distributions()` 枚举 Lxss 所有直接子项，并对每项强制读取字符串 `DistributionName`。辅助项 `Notifications` 不具备此值，触发 `wsl.registry_unavailable`，整个探测提前返回。

调用链：`list_wsl_environments` → `WslApplication::list` → `SystemWslRuntime::probe` → `read_registry_distributions`。

`probe()` 先读注册表，再运行 WSL 命令。因此本次失败发生于注册表阶段，尚未进入 WSL 列表或运行状态读取。

前端 `src/ProviderPage.tsx` 的 `openWslDialog()` 捕获错误后设置 `wslState=error` 和具体错误。`src/messages.ts` 将后端 ID 映射为“无法读取 WSL2 发行版注册信息。”；同一失败状态的通用说明或应用阻止原因显示“无法读取 WSL2 发行版状态。”。两条提示是同一失败的两层呈现。

## 修复决策

### 子项分类

在打开子项并读取发行版字段之前，以子项名称是否为标准带花括号的 GUID 形式进行分类：`{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`，十六进制大小写均接受。复用现有 UUID 解析能力，并显式约束带花括号的标准形式；不接受简写、URN 等宽松 UUID 形式。

- 符合发行版 GUID 形式：进入现有发行版读取流程。
- 非 GUID 子项：作为辅助配置项跳过，涵盖 `Notifications` 及未来新增的非 GUID 配置项；无需打开它们。
- 不把 `Notifications` 写成唯一特殊名称，不用 `DistributionName` 读取失败作为通用跳过条件。
- 注册 ID 保留原始字符串，避免改变数据库 environment_id、revision 或已有状态关联。

本决策基于项目现有“Lxss 注册 GUID 作为发行版身份”的契约。后续若需支持非 GUID 注册身份，应另行定义兼容策略。

### 读取和失败语义

| 输入或失败 | 目标行为 |
|---|---|
| Lxss 不存在 | 返回空列表 |
| Lxss 为空或只有辅助项 | 返回空列表，不运行 WSL 命令 |
| 辅助项与有效 GUID 发行版并存 | 跳过辅助项，返回有效发行版 |
| Lxss 根打开失败，且不是 NotFound | 返回 `wsl.registry_unavailable` |
| 枚举子项失败 | 返回同一错误，并记录枚举阶段 |
| GUID 子项打开失败 | 返回同一错误，并记录子项打开阶段 |
| GUID 子项缺少 DistributionName 或类型错误 | 返回同一错误，并记录名称读取阶段 |
| BasePath 缺失、类型错误或目录不存在 | 保留现有不可用发行版判定 |
| DefaultUid 或 Version 缺失或类型错误 | 保留现有可选字段及后续可管理性判定 |

真实发行版读取失败时继续阻止整个探测成功，避免把不完整清单当成发行版已删除，影响协调和恢复。分布过滤器 `distribution_filter` 继续在成功读取注册清单后生效，不用于隐藏其他 GUID 注册项的损坏。

### 问题日志

当前日志只有错误 ID 和 category，不能区分根路径、枚举、子项打开与字段读取。本次实现须补充最小脱敏证据：

- 失败阶段采用有限枚举，例如 `root_open`、`enumerate`、`distribution_open`、`distribution_name_read`。
- 携带数值 Windows 错误码（若有）或有限错误类别；保留对前端的既有 messageId。
- 成功发现阶段可记录扫描子项数、辅助项跳过数、有效注册数，便于识别“无发行版”与“忽略辅助项后成功”。
- 证据仅保存在后端诊断上下文，通过现有问题日志机制输出。日志接入须覆盖列表读取和启动恢复中实际发生的注册表失败，并避免同一调用重复刷屏。
- 不记录原始 OS 错误文本、完整注册表路径、任意子项名、发行版名称、GUID、BasePath、用户名、配置正文或凭据。辅助项只记录数量，无需记录 `Notifications` 字符串。
- 不为本任务更改数据库 schema；不为传递诊断而扩大前端 IPC 载荷。

具体内部类型由实现会话结合 `WslFailure`、`IssueLoggableFailure`、`finish_command` 与启动恢复日志选择最小改动方式。新增诊断字段或内部包装需核对所有构造点及序列化行为。

## 实施顺序与完成条件

1. 核对工作区和当前实现，保留用户已有修改。诊断时 `wsl.rs` 等多个源码文件已被修改；不要回退、覆盖或把它们直接视作本任务成果。完成条件：明确本次补丁范围及现有改动边界。
2. 建立 Windows 注册表读取的隔离测试入口，让测试能提供一个测试根键，生产仍固定读取 Lxss。入口应位于私有实现层，不暴露任意路径的产品 API。完成条件：回归测试实际经过枚举、分类和字段读取，而非仅测试 GUID 解析器。
3. 先新增“Notifications + 有效 GUID 发行版”的回归测试，确认旧逻辑失败，再实现分类和诊断。完成条件：同一回归测试通过，损坏 GUID 注册项仍失败。
4. 补齐下述矩阵和日志测试。完成条件：每项均有可执行验证或明确标注的环境限制，新增日志有脱敏断言。
5. 在本机重建后执行真实只读探测和界面验收，记录结果。完成条件：清单可读取、两条原始读取错误消失，并且探测未改变发行版运行集合。

## 回归测试矩阵

Windows 隔离测试在当前用户专用随机测试注册表根下创建 fixture，退出时只清理该测试根。不得增删真实 Lxss 项，不改变真实 WSL 注册。BasePath fixture 使用临时目录。

| 场景 | 断言 |
|---|---|
| Notifications + 一个有效 GUID 发行版 | 仅返回该发行版；ID 原文保留；辅助项计数为 1 |
| 未知非 GUID 辅助项，包含或不包含 DistributionName | 均跳过，不因有同名字段成为发行版 |
| 不同枚举顺序 | 辅助项位于前后均不影响结果 |
| 根不存在、空根、仅辅助项 | 空清单；不依赖可运行的 wsl.exe |
| 标准 GUID 大小写与格式边界 | 标准带括号形式接受；简写、URN、无括号、损坏字符串跳过 |
| GUID 项缺少或错误类型 DistributionName | 失败；阶段为 distribution_name_read |
| 根打开、枚举、GUID 子项打开失败 | 对应阶段及错误类别正确；不得转为空清单 |
| 可选字段或 BasePath 不可用 | 原有不可用/不支持判定保持一致 |
| 现有重复显示名及陈旧注册用例 | 有效注册不因陈旧 BasePath 重复项变为不可管理 |
| 诊断接入列表和启动恢复 | 日志保留错误 ID，包含允许的阶段证据 |
| 脱敏 canary | fixture 中的名称、GUID、路径和伪凭据均不出现在日志或 IPC 中 |

拒绝访问、枚举失败等难以稳定通过真实 ACL 构造的场景，可在底层读取结果边界注入错误；避免为测试调整真实用户注册表权限。至少主回归场景必须使用真实隔离注册表，不可全部使用手工构造的 RegistryDistro。

保留现有前端错误流程测试；除非实现改变 IPC 或 UI 行为，否则无需为本次后端缺陷扩大 UI 快照范围。

## 验证与交付

从仓库根执行当前代码构建的定向测试，过滤器按最终测试名称补全：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib wsl::tests::
cargo test --manifest-path src-tauri/Cargo.toml --lib `
  wsl::tests::real_system_probe_does_not_start_or_stop_distributions `
  -- --ignored --exact --nocapture
```

第一条不自动执行 ignored 的真实宿主测试。日志测试若位于其他模块，应另行运行对应过滤器。随后运行仓库要求的格式、静态检查及与实际改动有关的测试；不要把整个 Windows WSL 真实 guest 验收作为本次只读修复的默认门槛。

真实宿主验收分两层记录：

- 纯探测：使用上面的真实系统探测测试，在前后只读比较 `wsl --list --running --quiet` 集合。成功发现 Ubuntu；不执行启动、停止或配置写入。
- 产品界面：明确运行的是包含修复的新构建，打开“选择 WSL2 供应商”，确认发行版列表可用、注册表错误和通用读取失败提示消失。应用列表流程可能沿用现有逻辑读取 Running 发行版配置并协调本地状态，不能把它宣称为纯注册表只读测试。若真实配置存在其他问题，记录独立错误，分别判断本缺陷验收。

验收无需应用供应商或改写 Codex 配置。停止态覆盖优先通过隔离测试完成；真实停止态深度验收若需要，应使用获得授权的一次性发行版，不为本任务停止用户 Ubuntu。

交付时列出修改文件、回归测试命令和结果、本机实际运行构建及剩余限制。既有历史日志可以保留；验收以本次调用的新日志为准，不删除旧日志来制造通过结果。

## 本文档交接状态

本次仅制定方案，未运行修复后的测试，也未确认任何修复验收通过。未创建 GitHub Issue、提交或发布。若后续会话需要登记或关联任务，按 `docs/agents/issue-tracker.md` 操作，以该方案作为实现参考。
