# GPTEasy 实施与发布陷阱研究

**Domain:** Windows/macOS 托盘型桌面伴侣、原生 Codex 环境与 WSL2 配置管理、独立 Linux shell function
**Researched:** 2026-08-05
**Confidence:** MEDIUM

> 本文把 `CONTEXT.md`、ADR 0001–0008 和 `docs/ui/UI-SPEC.md` 视为锁定基线，只研究如何在这些决定内避免实现和发布失败。

## 严重度与阶段约定

| 等级 | 判定 |
|------|------|
| **CRITICAL** | 可能破坏用户 Codex 配置、丢失 SQLite 状态、泄露供应商凭据，或使升级后无法恢复 |
| **HIGH** | 会让平台行为与产品承诺不一致，或阻止正式发布 |
| **MODERATE** | 通常可局部修复，但会造成错误状态、可访问性缺陷或大量支持成本 |

本文使用以下实施阶段名称映射责任：

1. **阶段 1：数据与配置安全基础** — SQLite、迁移、配置编辑器、备份、恢复、凭据边界。
2. **阶段 2：供应商与供应商验证闭环** — 验证网络路径、错误模型、验证后替换和脱敏测试。
3. **阶段 3：原生 Codex 环境切换与重启协调** — 外部配置、直接配置模式、进程识别、待重启。
4. **阶段 4：托盘与设置界面** — Tauri 生命周期、托盘驻留、i18n、无障碍和反馈。
5. **阶段 5：WSL2 管理** — 发现、默认用户、临时启动、状态恢复和批量切换。
6. **阶段 6：Linux 切换脚本** — Bash 4+、Zsh 5+、GPTEasy 管理区块和独立恢复。
7. **阶段 7：运行维护与更新** — 诊断日志、诊断导出、每日更新检查、更新状态机。
8. **阶段 8：跨平台打包与发布验收** — 签名、公证、安装器、架构矩阵和升级演练。

## Critical Pitfalls

### Pitfall 1：迁移前“复制数据库文件”得到的备份并不一致

**严重度 / 威胁面:** CRITICAL；数据完整性、供应商凭据、发布就绪

**What goes wrong:**

GPTEasy 的 SQLite 数据库包含供应商目录、明文供应商凭据、验证状态和各受管环境的当前供应商。如果启用 WAL 后仍用普通文件复制只备份主数据库，备份可能遗漏尚在 `-wal` 中的已提交页；在连接活动时复制也可能得到无法恢复的一半快照。迁移失败后恢复这个“备份”，可能表现为供应商消失、引用悬空、API Key 回退到旧值或数据库损坏。

**Why it happens:**

- 把 SQLite 误当成普通单文件，而忽略 WAL/SHM 和活动事务。
- 为了“迁移前备份”在数据库连接仍打开时直接调用 `fs::copy`。
- 只测试空数据库到最新版，未保存带 WAL 状态的历史样本。
- 备份本身含明文供应商凭据，却按普通缓存文件设置权限和诊断导出规则。

**How to avoid:**

- 由 Rust 后端的唯一数据库所有者使用 SQLite Online Backup API 或 `VACUUM INTO` 生成一致快照；不要复制活动主文件来冒充备份。
- 备份成功、可打开并通过 `PRAGMA quick_check` 后，才开始迁移。
- 每个迁移使用永久顺序编号；迁移步骤和 schema 版本更新在同一显式写事务内提交。
- 每个连接打开时立即启用并验证 `PRAGMA foreign_keys = ON`；不要在迁移事务内部才切换。
- 遇到高于当前应用支持的 schema 版本时只读拒绝，绝不尝试“兼容写入”或自动清库。
- 数据库及最近三份迁移备份使用与明文凭据相同的文件权限策略，并明确排除在诊断导出之外。

**Warning signs:**

- 备份文件大小异常小，或仅出现 `.db` 而运行目录同时存在 `.db-wal`。
- 迁移日志显示成功，但 `foreign_key_check` 出现结果或当前供应商引用为空。
- CI 只有“全新建库”测试，没有每个已发布 schema 的真实样本。
- 迁移失败后的代码路径出现 `delete database`、`create default database` 或“忽略错误继续启动”。
- 备份文件可以被同机其他普通用户读取，或被诊断打包器收集。

**Prevention and verification strategy:**

- 为每个历史 schema 保存包含供应商、环境关联、Unicode、空值、删除约束和 WAL 写入的 fixture。
- CI 对每个 fixture 执行：备份 → 逐步迁移 → `quick_check`/`foreign_key_check` → 领域不变量校验 → 再次启动幂等校验。
- 在每个迁移语句前后注入失败点，验证事务回滚、备份可恢复且原数据库不被清空。
- 在磁盘满、只读目录、`SQLITE_BUSY`、进程强制退出场景做故障测试。

**Phase to address:** 阶段 1 负责机制和历史 fixture；阶段 8 必须用 N-1/N-2 正式工件做真实升级门禁。

---

### Pitfall 2：单文件原子替换被误当成完整配置事务

**严重度 / 威胁面:** CRITICAL；数据完整性、平台行为、发布就绪

**What goes wrong:**

原生 Codex 环境的一次切换可能同时涉及 Codex 配置、认证资料、GPTEasy SQLite 当前状态、配置备份和待重启状态。即使每个文件都通过临时文件重命名，进程也可能在文件 A 替换后、文件 B 或 SQLite 更新前崩溃，留下“文件使用目标供应商、应用仍显示旧供应商”或相反的分裂状态。Windows 上把 Unix `rename` 语义直接照搬，还可能因目标存在或文件被共享打开而替换失败。

**Why it happens:**

- “原子写入”只被实现为 `write(temp); rename(temp, target)`。
- 未区分 Unix 可覆盖重命名与 Windows 替换已有文件的不同语义。
- 临时文件建在系统临时目录，跨卷移动后不再是原子操作。
- 写完未刷盘，或替换后未处理目录项持久化。
- 状态库先标记成功，文件替换随后失败；或反向执行但没有恢复日志。

**How to avoid:**

- 建立统一的 **配置变更事务状态机**：读取和指纹 → 预确认 → 备份 → 准备全部新内容 → 逐目标提交 → 复读验证 → 更新 SQLite 领域状态 → 判定待重启 → 完成。
- 在 SQLite 中持久化不含供应商凭据的操作 journal：操作 ID、目标环境、目标供应商 ID、每个工件的旧/新哈希、当前步骤和恢复动作。应用重启时必须先恢复未完成操作。
- 临时文件必须与目标在同一目录；Unix 使用同文件系统替换并刷盘，Windows 使用明确支持覆盖已有目标的替换 API，并处理共享模式、杀毒软件短暂占用和重试上限。
- 为旧文件与新文件分别保存哈希；替换后重新打开、解析并确认只改变受管字段。
- 新建敏感文件使用限制性权限；替换已有文件时保留权限/ACL 和必要的所有权元数据。
- 不声称多文件具有真正原子性；依靠可检测 journal 和幂等恢复实现可恢复事务。

**Warning signs:**

- 代码中“写配置成功”发生在读回解析之前。
- `current_provider_id` 在任何文件写入前更新。
- 直接调用跨平台 `std::fs::rename`，没有 Windows 特化和目标占用测试。
- 临时路径来自 `%TEMP%`、`/tmp`，而不是目标配置目录。
- 强制结束 GPTEasy 后，托盘与实际 Codex 配置显示不同供应商。

**Prevention and verification strategy:**

- 对状态机每个步骤注入崩溃，重新启动后验证只能得到“完整旧状态”或“完整新状态”，不能静默停在混合状态。
- Windows 测试目标文件被另一个带不同共享模式的进程打开、Defender 扫描、只读属性和长路径。
- macOS 测试 APFS 上的权限保留、目录只读、磁盘满和应用被强制退出。
- 每次正式写入后做结构化复读，并将验证失败转入可恢复错误，而不是显示成功 Toast。

**Phase to address:** 阶段 1 负责通用事务引擎；阶段 3、5、6 只能复用该不变量，不得各自实现简化版。

---

### Pitfall 3：读改写覆盖外部配置，或破坏符号链接与配置优先级

**严重度 / 威胁面:** CRITICAL；数据完整性、平台行为

**What goes wrong:**

用户、宿主应用或本机 Codex CLI 可能在 GPTEasy 读取配置后修改同一文件。GPTEasy 若继续用旧快照写回，会删除用户刚增加的 MCP、profiles、features 或其他非受管配置。若配置路径是符号链接，直接原子替换可能把链接本身换成普通文件。即使写入正确，项目级配置、命令行参数或环境变量也可能覆盖用户级配置，使 GPTEasy 错误宣称当前供应商已生效。

**Why it happens:**

- 使用普通 TOML 序列化器重写全文件，丢失注释、顺序和未知字段。
- 读取时不保存内容哈希和文件身份，提交前不比较。
- 假设 `~/.codex/config.toml` 永远是普通文件，且用户级配置永远拥有最高优先级。
- 将“能解析”误判为“可以安全归属到 GPTEasy 已验证供应商”。

**How to avoid:**

- 用保留格式和未知键的编辑模型，只修改明确的供应商相关字段；任何不能无歧义定位的内容转为外部配置。
- 读取时记录规范化路径、文件身份、mtime、大小和内容哈希；替换前重新读取并比较。发生变化就停止，重新分析，不覆盖。
- 明确检测符号链接、硬链接或重解析点。首版不应静默断开链接；无法保证安全时停止并给出可恢复错误。
- 写入前和写入后都解析 Codex 配置，确认非 GPTEasy 管理区域语义等价。
- 把“配置已写入”和“当前进程实际使用该配置”分开；发现更高优先级覆盖或无法确认时显示外部配置/待重启/状态未知，而不是“当前使用”。
- 把当前支持的 Codex 配置契约、字段和优先级做成版本化 adapter，并对未知新字段默认保留。

**Warning signs:**

- 保存一次后用户配置的注释、排序或未知键大量变化。
- E2E 用例只包含 GPTEasy 自己生成的最小 TOML。
- 修改配置期间从外部追加字段，GPTEasy 仍然返回成功。
- 配置文件是 symlink 时，操作后 `symlink_metadata` 变成普通文件。
- Codex 以项目配置启动后，GPTEasy 仍显示用户级目标供应商“当前使用”。

**Prevention and verification strategy:**

- 建立外部配置 corpus：未知表、重复候选、自定义 profiles、注释、CRLF/LF、无末尾换行、Unicode、符号链接和并发写入。
- 使用差分测试验证除受管字段外的 AST/CST 等价。
- 在最终提交前模拟宿主应用改写，必须得到冲突而不是覆盖。
- Codex 配置参考发生变化时运行 adapter 契约测试，并把不认识的结构判为保守失败。

**Phase to address:** 阶段 1 建编辑与冲突检测；阶段 3 完成 Codex 优先级、外部配置和真实宿主应用契约测试。

---

### Pitfall 4：供应商凭据从“非日志路径”间接泄露

**严重度 / 威胁面:** CRITICAL；供应商凭据、发布就绪

**What goes wrong:**

ADR 已允许明文保存、完整显示、写入 WSL2 和 Linux 切换脚本，但这不等于 API Key 可以进入日志、错误、Toast、系统通知、Tauri command 调试输出、SQL 参数日志、HTTP 请求调试、panic、诊断导出或截图辅助。最危险的泄露通常不是显式 `log(api_key)`，而是 `Debug` 派生、整个 DTO/配置结构、错误链或失败请求被自动格式化。

**Why it happens:**

- 采用事后正则替换，而不是在数据进入日志前禁止。
- 同一个 `Provider` 类型同时用于数据库、HTTP、IPC 和日志。
- 记录完整 URL、请求 Header、响应正文或 SQL bound parameters。
- React 错误边界、DevTools、Tauri command tracing 或 panic hook 输出完整对象。
- 诊断导出按目录打包，而不是按允许清单生成。
- 迁移备份、配置备份和导出脚本被误当成“诊断文件”。

**How to avoid:**

- 建立敏感值类型和显式暴露边界：默认 `Debug`/`Display` 只输出 `[REDACTED]`，只有数据库写入、验证请求、受控前端查看和导出脚本生成器可以取得原值。
- 日志采用字段允许清单：供应商 ID、主机名或经批准的摘要、错误代码、阶段；禁止记录完整服务地址、请求/响应正文和认证 Header。
- Rust → React 的错误 DTO 只返回本地化错误代码、脱敏摘要和经过筛选的技术详情；原始错误链只在内存中用于分类。
- 诊断导出重新构造内容，不递归打包应用数据目录；明确排除数据库、数据库备份、Codex 配置备份、Linux 切换脚本和剪贴板内容。
- release 构建关闭不必要的 DevTools/详细 tracing；Tauri capabilities 只允许需要的 commands。
- 保存/复制脚本前的明文警告保持锁定行为，同时保存文件使用限制性权限。

**Warning signs:**

- 任何日志调用接收完整 `Provider`、HTTP request/response 或 command 参数对象。
- API Key 在错误详情、网络错误 URL、SQL trace 或 React Redux/状态快照中可搜索到。
- 诊断 zip 包含 `.db`、`.db-wal`、Codex 配置、备份或 `gpteasy-providers.*`。
- release 包仍可打开 DevTools 并查看带供应商凭据的前端状态。
- 脱敏测试只覆盖 `sk-...`，没有非标准 Key、短 Key、Unicode 或包含正则特殊字符的 canary。

**Prevention and verification strategy:**

- 每次测试生成多个独一无二的 canary 供应商凭据，覆盖验证成功/失败、迁移失败、配置写入失败、WSL2 失败、脚本导出、panic 和更新失败。
- 测试结束扫描日志、通知捕获、诊断 zip、临时目录、错误 DTO 和前端持久化；任何 canary 原文或 URL 编码变体出现即失败。
- code review gate 禁止敏感类型派生可暴露的 `Debug`/`Serialize`，除非有显式安全理由和测试。
- 诊断导出做“从空目录构建”测试，证明不是从应用目录排除若干文件。

**Phase to address:** 阶段 1 定义敏感类型和日志策略；阶段 2/3/5/6/7 为各自新路径添加 canary 回归；阶段 8 扫描 release 构建。

---

### Pitfall 5：进程检测和重启协调把“已写入”误报成“已生效”

**严重度 / 威胁面:** CRITICAL；平台行为、数据完整性

**What goes wrong:**

GPTEasy 将桌面 Codex 和本机 Codex CLI 视为同一个原生 Codex 环境，但它们是不同进程类型。只按进程名检测容易误杀同名程序；只保存 PID 会遇到 PID 重用；访问被拒绝时当作“没有进程”会漏掉待重启。配置写入和进程枚举之间还存在竞态：预确认后可能新启动 Codex，或确认立即重启时旧进程退出、新进程又出现。最终 UI 可能显示“当前使用”，实际仍有进程持有旧配置。

**Why it happens:**

- 进程身份只看 `codex`、`ChatGPT` 等名称，不校验 Windows 可执行路径或 macOS bundle identifier。
- 将一次进程快照当成稳定事实。
- 把桌面宿主应用与 CLI 使用同一种“终止并重新拉起”策略；CLI 的终端、cwd、参数和会话通常无法可靠重建。
- 写文件、更新 SQLite、退出进程和重新启动没有统一状态机。
- 检测 API 权限失败、僵尸进程或快速退出被折叠为“不在运行”。

**How to avoid:**

- Windows 使用完整可执行路径、可信安装位置和启动时间确认身份；macOS 使用 bundle identifier、bundle URL 和进程实例；裸 PID 只在持有有效 handle/实例期间使用。
- 每次切换至少在预确认前、正式提交前和提交后重新枚举；新出现进程必须使结果进入待重启或重新确认。
- 将状态拆成：目标配置、配置写入完成、检测到的旧配置进程、待重启、重启尝试结果。不要用一个布尔值覆盖。
- 对无法访问或无法确认的进程采用保守策略：状态未知或待重启，而不是已生效。
- 桌面宿主应用可以在用户明确选择后按平台方式退出并重新打开；CLI 不应假装恢复原终端会话，必须明确报告需要用户重新启动的实例。
- 立即重启流程必须等待被确认的旧实例退出，再启动可重启实例，并重新枚举；超时后保持待重启。

**Warning signs:**

- 测试只启动一个名为 `codex.exe` 的假进程便认为识别通过。
- 当前供应商在文件写入后立即更新为“已生效”，没有进程复查。
- 访问拒绝和未发现进程走同一分支。
- 立即重启按钮总能返回成功，即使旧 PID 仍存在或新实例未启动。
- CLI 被终止后，GPTEasy 尝试以未知 cwd/参数自动重建。

**Prevention and verification strategy:**

- 用同名不同路径进程、PID 快速重用、访问拒绝、进程在确认框打开期间启动/退出等场景测试。
- 分别覆盖：仅桌面宿主应用、仅 CLI、两者同时、多 CLI、原生 Codex 环境缺失。
- 在写配置和更新状态的每个边界注入延迟，运行竞态测试。
- E2E 成功条件必须同时验证配置内容、SQLite 状态、进程状态和托盘/窗口呈现。

**Phase to address:** 阶段 3 主责；阶段 4 验证重启对话框与托盘待重启反馈；阶段 8 在签名安装后的真实进程身份上复验。

---

### Pitfall 6：WSL2 临时启动在异常路径上未恢复，或恢复时终止了用户工作

**严重度 / 威胁面:** CRITICAL；平台行为、数据完整性、发布就绪

**What goes wrong:**

一个原本停止的 WSL2 环境因用户切换而被临时启动。如果写配置失败、用户取消、GPTEasy 崩溃或更新退出，发行版可能保持运行；反过来，如果 GPTEasy 操作期间用户或其他工具开始使用该发行版，操作结束后无条件 `wsl --terminate` 会中断用户任务。批量切换会放大这一风险。

**Why it happens:**

- 只在成功分支调用停止恢复。
- 使用内存 `finally`/RAII，但没有跨进程崩溃恢复记录。
- 通过操作结束时“当前在运行”就认定该运行状态归 GPTEasy 所有。
- 批量操作共享一个全局恢复标志，未按 WSL2 环境隔离。
- 错误使用 `wsl --shutdown`，连其他发行版一起终止。

**How to avoid:**

- 操作前用 `wsl --list --running --quiet` 快照目标发行版是否运行；只对确认原本停止的环境建立“临时启动租约”。
- 在启动前把租约、操作 ID、原始状态和恢复责任持久化到 SQLite；完成恢复后再清除。
- 每个 WSL2 环境使用独立互斥和状态机，批量操作只是编排多个独立结果。
- 所有正常错误、取消和更新退出路径都执行恢复；应用启动时优先检查遗留租约并进入明确恢复流程。
- 永远使用 `wsl --terminate <Distribution Name>`，不使用 `--shutdown`。
- 终止前重新检查：如果发现非本操作启动的活动或无法判断所有权，宁可报告“未自动恢复停止状态”并让用户决定，也不要静默终止。

**Warning signs:**

- 恢复状态只保存在 Rust 局部变量。
- 任务管理器中 WSL VM 在失败/取消后仍运行，SQLite 却无恢复记录。
- 批量切换一个发行版失败后，其他发行版的停止/运行状态也变化。
- 代码包含无目标参数的 `wsl --shutdown`。
- 测试没有覆盖 GPTEasy 在启动后、写入中、恢复前被强制结束。

**Prevention and verification strategy:**

- 针对每个状态机步骤进行 kill/power-loss 注入，重启 GPTEasy 后验证遗留租约可见且可恢复。
- 在 GPTEasy 临时启动窗口内从另一个终端进入同一发行版，验证不会被无条件终止。
- 批量测试混合原始状态、单项失败、用户取消和进程崩溃，结果必须逐发行版报告。
- 记录脱敏的状态转换和 WSL 命令退出码，不记录配置内容或供应商凭据。

**Phase to address:** 阶段 5 主责；阶段 1 先提供通用可恢复操作 journal；阶段 8 在 Windows x64/ARM64 和真实 WSL2 版本上做破坏性验收。

---

### Pitfall 7：Linux 切换脚本把数据当成 shell 代码

**严重度 / 威胁面:** CRITICAL；供应商凭据、数据完整性、平台行为

**What goes wrong:**

Linux 切换脚本自包含所有已验证供应商及明文凭据。若生成器把供应商名称、API Key、服务地址或默认模型直接拼入 shell 语句，单引号、反斜杠、换行、命令替换或控制字符可破坏脚本，泄露凭据，甚至执行非预期命令。通过 shell 转义后仍需进行独立的 TOML 字符串转义，否则脚本可运行但生成无效配置。

**Why it happens:**

- 误认为 API Key 只含字母数字和连字符。
- 复用 JSON 转义或 TOML 转义作为 shell literal 转义。
- 为方便选择器使用 `eval`、动态变量名或未加引号的数组展开。
- 试图用一个“兼容脚本”同时覆盖 Bash 和 Zsh，忽略数组索引、`read`、参数展开差异。
- 只在 happy path 上测试 DayWay 和简单英文模型名。

**How to avoid:**

- Bash 4+ 与 Zsh 5+ 使用独立模板和独立测试；共享的只能是经过验证的领域数据模型。
- 明确实现两层编码：先编码为目标 TOML 值，再编码为目标 shell 的不可执行 literal；不要使用 `eval`。
- 所有变量展开、路径、菜单输入和 `printf` 参数都加正确引用；禁止把供应商值用作命令名、变量名或格式字符串。
- 对不能安全表示或违反字段契约的控制字符在供应商验证/保存前给出字段错误，不能在导出时悄悄截断。
- 脚本不得回显 API Key，不把完整配置内容写到 stdout/stderr。

**Warning signs:**

- 模板中出现 `eval`、`source` 动态字符串、`echo "$generated_command" | sh`。
- 同一个数组和 `read -p` 片段未经条件分支用于 Bash 与 Zsh。
- 测试数据没有 `'`、`"`、反斜杠、空格、`$()`、反引号、换行和非 ASCII。
- shellcheck 通过就被当作 Zsh 验证完成，或只运行语法检查不执行切换。

**Prevention and verification strategy:**

- 属性测试随机生成供应商字段；导出后分别在 Bash 4/5 与 Zsh 5 容器执行，解析目标 TOML 并比较原始值。
- 固定恶意 corpus 包含 shell 元字符、TOML 特殊字符、Unicode、超长值和重复显示名称。
- 验证选择退出不修改文件，选择任一供应商只改变 GPTEasy 管理区块。
- 对导出文件做 canary 扫描，确认凭据只存在于设计允许的位置，不进入运行输出和备份文件名。

**Phase to address:** 阶段 6 主责；阶段 2 负责字段契约；阶段 8 对最终生成器做跨发行版 release 回归。

---

### Pitfall 8：更新签名链或退出清理失败，使用户卡在旧版本或损坏更新

**严重度 / 威胁面:** CRITICAL；发布就绪、数据完整性、平台行为

**What goes wrong:**

Tauri 2 updater 强制验证更新签名。发布了错误平台 URL、错误 `.sig`、错误公钥或在生成签名后再次修改工件，客户端会拒绝更新。更隐蔽的是更新私钥轮换：旧客户端只信任内置公钥，若直接改用新密钥，旧版本无法验证任何后续更新。Windows 安装更新时 Tauri 会自动退出应用；若退出前还有 SQLite 事务、配置写入或 WSL2 临时启动租约，更新会把未完成操作留给新版本。

**Why it happens:**

- 把 Authenticode/Developer ID 签名与 Tauri updater 签名混为一件事。
- CI 按架构上传工件时把 x64、ARM64、Intel、Apple Silicon 的 URL 或签名配错。
- 私钥未纳入发布灾难恢复，或临近发布才临时生成。
- 更新前退出只是 `process::exit`，没有统一 cleanup barrier。
- 只测试“当前开发版检查当前开发版”，没有从已发布旧版升级。

**How to avoid:**

- 明确维护两条独立信任链：操作系统代码签名/公证，以及 Tauri updater 内容签名。
- 私钥只存在于受控 CI secret/离线备份；公钥固定进入客户端。密钥轮换必须先发布由旧密钥签名、同时支持过渡信任的桥接版本；若框架不支持多公钥，必须把“旧客户端无法在线轮换”纳入恢复预案。
- 生成最终、已完成平台签名和公证的更新工件后再生成/上传对应 updater 签名，之后不再修改文件。
- 更新元数据按 Tauri target 精确映射，并在发布前下载回源文件重新验证哈希、平台和签名。
- 建立更新 cleanup barrier：停止接受新配置操作 → 等待或取消进行中事务 → 刷盘 SQLite/日志 → 处理 WSL2 恢复租约 → 设置明确退出标志 → 允许 updater 退出。
- “每天最多一次”使用 SQLite 持久化检查时间和结果；系统时间回拨/快进不能导致每次启动都检查或永久不检查。

**Warning signs:**

- CI 只有一个通用 `latest.json`，但没有逐 target 验证。
- updater 私钥没有恢复副本、负责人或轮换演练。
- Windows 更新日志显示安装器启动前应用被强制结束，但没有 `on_before_exit` 清理记录。
- N-1 正式安装版无法升级，开发构建却可以。
- 发布流程在 `.sig` 生成后还会压缩、重签或重新封装工件。

**Prevention and verification strategy:**

- 每次候选发布在隔离机器安装 N-1/N-2，使用正式更新端点升级，验证 SQLite、托盘、登录启动和当前供应商状态。
- 为错误签名、错误架构、404、超时、磁盘满、用户取消和安装失败建立可重复测试。
- 在更新退出的每个清理步骤注入延迟/失败，保证新版本启动时能恢复或明确提示。
- 发布门禁必须校验 updater 公钥指纹、工件哈希、操作系统签名、公证状态和 Tauri `.sig`。

**Phase to address:** 阶段 7 构建更新状态机；阶段 8 拥有密钥、CI、N-1/N-2 升级和失败恢复发布门禁。

## High-Severity Platform and UX Pitfalls

### Pitfall 9：托盘驻留、窗口关闭和明确退出共用同一条生命周期路径

**严重度 / 威胁面:** HIGH；平台行为、发布就绪

**What goes wrong:**

设置窗口关闭后程序本应托盘驻留，但若直接销毁最后窗口，应用可能退出或无法重新打开；若对所有 `CloseRequested`/`ExitRequested` 一律阻止，又会导致“退出 GPTEasy”、更新安装和系统关机无法结束进程。反复点击“设置…”若每次创建新窗口，还会出现多窗口、重复事件监听、重复托盘菜单或多个状态副本。

**Why it happens:**

- 没有显式的 `Running / HidingWindow / ExplicitQuit / Updating / SystemShutdown` 生命周期状态。
- 把 hide、close、destroy 和 app exit 当作同义词。
- 明确退出仍经过“关闭时隐藏”的拦截器。
- 设置窗口不按固定 label 复用，托盘初始化可能在窗口重建时重复执行。
- macOS 菜单栏 template icon、激活策略和 Dock 行为只按 Windows 验证。

**How to avoid:**

- 设置窗口使用唯一 label；托盘“设置…”执行 find → unminimize → show → focus，不重复创建。
- 用户关闭设置窗口时仅 prevent close 并 hide；托盘“退出 GPTEasy”先设置明确退出标志，再完成清理并调用应用退出。
- updater 退出、系统退出和用户明确退出进入不同事件原因，但共享幂等 cleanup。
- 托盘图标和菜单只在应用 setup 创建一次；语言/供应商状态变化时更新现有菜单，不新增 tray。
- macOS 使用 template icon，并在隐藏窗口后实机验证菜单栏、Dock、重新激活和系统主题切换。

**Warning signs:**

- 关闭窗口后任务管理器中进程消失，或再次点击托盘无法显示窗口。
- 点击退出后进程仍驻留；更新安装器报告应用仍在运行。
- 多次打开设置后出现多个 WebView、重复 Toast 或菜单项触发多次。
- macOS 深色菜单栏图标不可见，或关闭窗口后无法通过托盘恢复焦点。

**Prevention and verification strategy:**

- 建立桌面生命周期自动化矩阵：关闭、最小化、隐藏、重复打开、明确退出、更新退出、系统关机/注销。
- 对窗口数、tray 数、事件 listener 数建立断言。
- Windows 与 macOS 都需跑 100 次 show/hide/close 循环和语言切换，不允许资源或监听器增长。

**Phase to address:** 阶段 4 主责；阶段 7 验证 updater 退出；阶段 8 做签名包实机生命周期验收。

---

### Pitfall 10：WSL2 发现依赖本地化表格、发行版 launcher 或 `/home/<name>`

**严重度 / 威胁面:** HIGH；平台行为、数据完整性

**What goes wrong:**

解析 `wsl --list --verbose` 的英文 `Running/Stopped` 列会在中文 Windows、列宽变化或发行版名称含空格时失败。导入发行版可能没有 `ubuntu.exe config --default-user` 一类 launcher；默认用户也可能是 root，HOME 不一定是 `/home/<name>`。如果 GPTEasy 拼接命令字符串，发行版名称和 Linux 路径还会引入参数错位或注入风险。

**Why it happens:**

- 把人类可读 CLI 表格当成稳定机器接口。
- 只在默认英文 Ubuntu 上测试。
- 通过 Windows 用户名推导 WSL2 默认用户。
- 使用 `cmd /c "wsl -d $name ..."` 字符串拼接，而不是结构化参数。
- 未显式区分 WSL1 和 WSL2。

**How to avoid:**

- 名称与运行集合使用 `wsl --list --quiet` 和 `wsl --list --running --quiet`；版本信息若必须从 verbose 获取，要把解析封装为有 fixture 的 adapter，失败时显示不支持而不是猜测。
- 明确过滤版本 2；WSL1 可展示为不受支持，但不能执行 WSL2 切换。
- 调用目标发行版时不指定用户即可使用其默认用户，再在发行版内查询 `id -un` 和 `$HOME`；不假设 launcher 或路径。
- 使用 `Command::args` 分离每个参数；发行版内脚本通过 stdin 或受控文件传递，不把供应商数据拼入 Windows shell 命令行。
- 解码和规范化 `wsl.exe` 输出时覆盖 CRLF、NUL、Unicode 和异常退出码。

**Warning signs:**

- 代码搜索出现对 `Running`、`Stopped`、固定列号或 `Ubuntu.exe` 的硬编码。
- HOME 通过 `format!("/home/{user}")` 构造。
- WSL1 发行版也显示可切换选择器。
- 含空格、非 ASCII 的发行版名称无法打开，或命令日志显示整段 shell 字符串。

**Prevention and verification strategy:**

- fixture 覆盖简中/英文 Windows 输出、长名称、空格、Unicode、WSL1/2 混合、导入发行版、默认 root。
- 在真实 Store 与 imported 发行版上测试发现、默认用户、停止/运行状态和配置路径。
- 命令构造单元测试断言参数数组，不接受拼接后的单字符串。

**Phase to address:** 阶段 5 主责；阶段 8 覆盖受支持 Windows 版本与架构。

---

### Pitfall 11：Linux function 能切换，但备份、权限和管理区块不安全

**严重度 / 威胁面:** HIGH；供应商凭据、数据完整性

**What goes wrong:**

脚本语法正确并不代表配置编辑安全。临时文件或备份若受宽松 `umask` 影响，可能向其他用户暴露 API Key；临时文件建在 `/tmp` 后移动，可能跨文件系统而失去原子性；用 `sed -i` 会遇到 GNU/BusyBox 差异。重复、嵌套或缺失结束标记的 GPTEasy 管理区块若被“自动修复”，会覆盖用户配置。仅用秒级时间戳的备份名还会在快速切换时碰撞。

**Why it happens:**

- 把“常见基础命令”误解为各发行版具有相同 GNU 选项。
- 备份轮换通过解析任意 `ls` 输出，未约束文件名。
- 先截断目标文件，再尝试生成内容。
- 区块替换基于贪婪正则，不先计数和验证边界。
- 忘记导出脚本自身、配置备份和临时文件都包含明文供应商凭据。

**How to avoid:**

- function 开始即设置并恢复安全 `umask 077`；目标目录内创建唯一临时文件，写完后再同目录 `mv`。
- 使用 `trap` 在成功、错误和中断时清理临时文件；禁止先截断目标。
- 写入前先扫描边界：0 个区块表示追加，恰好 1 个且边界顺序正确表示替换；重复、嵌套、半个区块一律停止。
- 备份名使用受控前缀、足够精度时间和进程/随机后缀；轮换只匹配 GPTEasy 自己生成的安全文件名，保留最近五份。
- 不依赖 `sed -i`、Python、Node.js 或 shell 专有外部工具；Bash/Zsh 模板分别验证基础命令行为。
- 保存导出脚本时应用限制性权限；复制到剪贴板只能由用户明确触发。

**Warning signs:**

- 代码出现 `sed -i`、`echo ... > "$config"` 作为第一步，或临时文件固定在 `/tmp/gpteasy.tmp`。
- 管理区块替换没有“边界数量必须为 0 或 1”的断言。
- 两次一秒内切换覆盖同一备份。
- 新建备份/脚本权限为 0644，或临时文件残留。

**Prevention and verification strategy:**

- 在 Debian/Ubuntu、Alpine/BusyBox 类环境和至少一个 Zsh 环境执行完整切换与恢复。
- 注入 SIGINT、写满磁盘、目标只读、`mv` 失败、重复区块和损坏区块。
- 每次测试检查目标文件、五份轮换、权限、无残留 temp 和非受管内容哈希。

**Phase to address:** 阶段 6 主责；阶段 8 对最终导出物做发行版矩阵验收。

---

### Pitfall 12：动态语言只更新 React，原生菜单、通知和状态仍使用旧语言

**严重度 / 威胁面:** MODERATE；平台行为、发布就绪

**What goes wrong:**

界面语言切换后 React 文本已变化，但托盘菜单、原生对话框标题、系统通知、后台批量结果或之后导出的 Linux 切换脚本仍沿用启动时语言。若把已翻译字符串存入 SQLite，升级文案后旧状态还会混用语言。日期、验证时间和错误技术详情也容易使用错误 locale。

**Why it happens:**

- i18n 被当成纯前端 concern，Rust 后端和 tray menu 在 setup 时只构建一次。
- 事件队列保存已经渲染的字符串，而不是稳定错误码和参数。
- 没有更新 `document.documentElement.lang`。
- Linux 脚本语言读取全局 UI 状态，而不是导出时明确选择。

**How to avoid:**

- SQLite 和 Rust 事件只存稳定 code/参数，不存最终翻译文案。
- 语言变化时更新 `html lang`，重建或更新现有托盘菜单，并让未来通知按新 locale 渲染。
- 正在进行的操作可保留开始时语言或切换到当前语言，但必须定义一致规则并测试；不要一半一半。
- 日期时间、复数和列表使用 locale-aware formatter；技术标识、模型 ID 和路径不翻译。
- Linux 切换脚本语言以导出页选择为准，切换 Shell 或语言后重新生成。

**Warning signs:**

- `TrayIconBuilder` 菜单文本只在 app setup 出现一次。
- 数据库中保存“验证失败”“待重启”等中文/英文完整句子。
- 切换语言后系统通知、原生重启对话框或托盘仍为旧语言。
- `<html lang>` 永远是固定值。

**Prevention and verification strategy:**

- 在设置窗口隐藏、验证进行中、WSL2 批量进行中和待重启状态下切换语言。
- 对 React、托盘、原生对话框、通知和导出脚本分别做快照/实机检查。
- 缺少翻译 key 时 CI 失败，不允许静默回退成 key 名。

**Phase to address:** 阶段 4 主责；阶段 5/6/7 为各自后台输出补契约；阶段 8 做双语包验收。

---

### Pitfall 13：视觉上满足 UI-SPEC，但异步状态和敏感字段对辅助技术不可用

**严重度 / 威胁面:** HIGH；平台行为、发布就绪、供应商凭据

**What goes wrong:**

供应商验证的三步状态只换颜色/图标而不通过 live region 宣告；错误出现后焦点停在页面底部；重启模态框没有 focus trap/返回；托盘唤回窗口后键盘焦点丢失；200% 缩放下主操作被挤出；高对比度模式看不见状态；减少动态效果未生效。API Key 若只用 CSS 遮罩，屏幕阅读器仍可能读出真实值。

**Why it happens:**

- 只跑浏览器单元测试，没有 NVDA、VoiceOver 和系统缩放实测。
- 自定义列表、菜单和对话框没有完整键盘语义。
- 每个流式验证事件都写 `aria-live`，造成读屏噪声；或完全不宣告。
- API Key 隐藏仅改变视觉样式，没有从可访问树中隐藏。
- React portal 和 Tauri 原生窗口 show/hide 破坏焦点恢复。

**How to avoid:**

- API Key 隐藏态使用语义正确的 password 控件；显示/隐藏按钮具有动态可访问名称，但不把完整 Key 放进 aria-label 或 live region。
- 验证状态只在步骤变化和最终结果时做节流后的 `aria-live` 宣告。
- 错误提交后提供错误摘要并将焦点移到合理位置，同时不在用户输入过程中无条件抢焦点。
- 模态框实现 focus trap、Escape/取消规则和关闭后焦点返回；危险主操作不能成为误按 Enter 的默认动作。
- 使用原生语义控件优先；自定义 listbox/menu 必须实现方向键、Enter、Escape 和 roving focus。
- 真实验证 200% 缩放、窗口最小尺寸、forced colors、高对比度、prefers-reduced-motion。

**Warning signs:**

- axe 自动扫描通过即宣布无障碍完成，没有读屏和键盘脚本。
- 验证完成只能通过绿色勾辨认。
- 隐藏 API Key 仍出现在可访问树或测试快照。
- 打开/关闭重启对话框后焦点落到 body。
- 200% 缩放下必须横向滚动才能保存或取消。

**Prevention and verification strategy:**

- 自动化：语义、Tab 顺序、焦点返回、颜色对比、reduced motion 和 200% reflow。
- 人工：Windows + NVDA、高对比度；macOS + VoiceOver、减少动态效果；全部核心流程纯键盘完成。
- 将供应商添加/验证/保存、托盘切换、重启选择、WSL2 行内选择和脚本导出列为无障碍发布门禁。

**Phase to address:** 阶段 4 主责并建立测试基础；阶段 5/6 补新页面；阶段 8 完成双平台人工验收。

---

### Pitfall 14：Windows 安装包偏离“当前用户安装”或架构/WebView2 组合未验证

**严重度 / 威胁面:** HIGH；发布就绪、平台行为

**What goes wrong:**

使用 per-machine MSI/NSIS 会触发 UAC、写入 `Program Files`/HKLM，违背当前用户安装；x64 安装包可能在 ARM64 设备上通过模拟运行，却加载错误架构的组件或更新包。默认 WebView2 bootstrapper 需要网络，企业代理或离线环境会让首次安装失败；跳过 WebView2 检查又可能在异常系统镜像上启动失败。未签名或时间戳错误的首发包还会触发 SmartScreen/信任警告。

**Why it happens:**

- 只在开发机已有 WebView2、管理员账户和 x64 环境测试。
- 发布矩阵名称正确，但 installer/updater 内实际 target 错配。
- 为“安装更标准”改成 perMachine，忽略锁定的当前用户安装。
- 只签 EXE，不签最终安装器，或签名后再次修改工件。
- 假设有效签名自动消除所有 SmartScreen 信誉提示。

**How to avoid:**

- Windows 正式分发固定使用 Tauri NSIS `installMode: currentUser`；CI 检查配置，安装测试断言无管理员权限、元数据在 HKCU、应用数据在当前用户目录。
- x64 与 ARM64 分别原生构建、代码签名、生成 updater 工件，并校验 target 元数据和下载 URL。
- 明确 WebView2 策略：在线 bootstrapper、嵌入 bootstrapper 或 offline installer 只能选定并做对应离线/代理测试；不要使用不推荐的 skip 作为省体积捷径。
- 对最终 EXE、DLL、安装器和 updater 包执行 Authenticode 验证与可信时间戳检查。
- 在干净 Windows 10 22H2+ 普通用户环境验证安装、登录启动、托盘、更新、卸载和应用数据保留。

**Warning signs:**

- 安装出现 UAC 或默认目标是 `Program Files`。
- ARM64 发布工件由 x64 runner 产出但没有实际 ARM64 PE 检查。
- 安装只在联网、已有 WebView2 的开发机成功。
- updater 下载 x64 包到 ARM64 安装。
- 安装器签名有效，但内部主程序或更新包未签/签名时间戳无效。

**Prevention and verification strategy:**

- 使用非管理员 Windows x64/ARM64 VM，从未安装状态执行完整安装与首次启动。
- 模拟无网络、代理失败、WebView2 缺失/过旧、应用运行中更新和磁盘空间不足。
- 发布脚本自动检查 PE 架构、NSIS install mode、签名、时间戳、版本、bundle identifier 和 updater target。

**Phase to address:** 阶段 8 主责；阶段 4 提前验证托盘在目标 WebView2；阶段 7 提前验证运行中更新退出。

---

### Pitfall 15：macOS 签名、公证、架构和安装位置各自通过，但组合后更新失败

**严重度 / 威胁面:** HIGH；发布就绪、平台行为

**What goes wrong:**

Intel 与 Apple Silicon 工件或 universal bundle 中某个二进制架构缺失；嵌套 helper/资源未按最终内容签名；DMG 或 app 未公证/staple；`minimumSystemVersion` 与 Rust 链接目标不一致；用户把 app 放在无写权限位置后 updater 无法替换。登录启动项若指向旧 bundle 路径，更新或移动应用后会失效。开发机直接运行正常，但从网络下载后受 quarantine/Gatekeeper 限制。

**Why it happens:**

- 在签名、公证或 updater 签名之后重新修改 bundle。
- 只验证主 executable，不验证完整 app bundle 和嵌套代码。
- 只在 Apple Silicon 开发机通过 Rosetta 测 Intel，或反之。
- 把普通 DMG 拖拽流程自动等同于“当前用户安装”。
- 登录启动记录硬编码构建路径或旧应用路径。

**How to avoid:**

- 明确选择 Intel/Apple Silicon 独立工件或 universal，并确保 updater target 与发布策略一致；每个最终工件都检查所有 Mach-O slice。
- 使用 Developer ID Application、hardened runtime、最小必要 entitlements、notarization 和 stapling；对最终下载工件执行 `codesign --verify --deep --strict`、`spctl --assess` 和 `stapler validate`。
- 同时设置并验证 Tauri `minimumSystemVersion` 与 Rust/macOS 构建目标为 macOS 14+，确保 universal 两个 slice 一致。
- 登录启动使用受支持的 Tauri autostart 机制，并在应用移动和更新后调用 `is_enabled`/实际登录验证。
- 在普通用户、带 quarantine 的真实下载路径和预期当前用户安装位置验证 updater 写权限；无权限时必须清楚提示，不能循环下载失败。
- 完成 app 签名/公证/staple 后再生成最终 updater 工件与其内容签名，不再变更。

**Warning signs:**

- `codesign` 只检查主二进制，未检查整个 app/DMG。
- Intel 与 Apple Silicon 的 updater URL 或 `.sig` 共享但工件不同。
- macOS 14 Intel/Apple Silicon 任一真实机器未进入发布矩阵。
- 登录启动在更新后打开旧路径或无响应。
- 从 CI artifact 直接运行成功，从浏览器下载后却被 Gatekeeper 拒绝。

**Prevention and verification strategy:**

- 对 Intel 与 Apple Silicon（或 universal）分别执行安装、首次启动、托盘、登录启动、更新、移动 app 后再启动和卸载。
- 发布 CI 验证 Mach-O slice、bundle identifier、最低系统版本、完整签名链、公证票据、staple 和 updater 签名。
- 从真实 HTTPS 发布地址下载候选包，不只测试 CI 本地文件。

**Phase to address:** 阶段 8 主责；阶段 4 提前验证 macOS 菜单栏和窗口生命周期；阶段 7 提前验证 updater/登录启动协同。

## Technical Debt Patterns

| Shortcut | Immediate Benefit | Long-term Cost | When Acceptable |
|----------|-------------------|----------------|-----------------|
| 各环境各写一套配置编辑代码 | 单阶段实现更快 | 原生、WSL2、Linux 的备份/原子性/脱敏不变量漂移 | Never |
| 迁移失败后重建空数据库 | 开发期少处理恢复 | 明文凭据和环境关联永久丢失 | Never |
| 全文件 TOML 反序列化再序列化 | 代码简单 | 删除未知配置、注释和格式，制造外部配置冲突 | Never |
| 日志后处理正则脱敏 | 快速看到“已脱敏” | 新字段、编码形式和错误链持续漏密 | Never |
| 只按进程名识别 Codex | 跨平台 API 少 | 误判、误终止、待重启状态错误 | Never |
| 单一 Bash/Zsh polyglot 模板 | 少维护一个模板 | 转义、数组和交互行为难以证明 | Never |
| 只在开发构建测试 updater | 无需管理旧版本 | 正式签名、target、退出和迁移问题延迟到发布 | Never |
| Windows 使用 WebView2 `skip` 减小安装包 | 包更小 | 干净系统无法启动且安全补丁基线不可控 | Never for release |
| 无真实 NVDA/VoiceOver 测试 | 自动化速度快 | 核心流程对辅助技术不可用 | 仅早期组件开发，阶段 4 完成前必须补齐 |

## Integration Gotchas

| Integration | Common Mistake | Correct Approach |
|-------------|----------------|------------------|
| Codex 用户级配置 | 假设用户级配置一定最终生效 | 检测配置层级、外部改写和运行中进程，区分已写入与已生效 |
| SQLite + WAL | 活动时复制主文件备份 | 使用 SQLite Online Backup API/VACUUM INTO 并校验快照 |
| Windows 文件替换 | 把 Unix rename 语义直接搬过来 | 同目录 temp + Windows replace API + 占用/重试/复读验证 |
| Tauri tray | 每次打开设置重新创建 tray/window | tray 单例、窗口固定 label、show/focus 复用 |
| Tauri updater | 只依赖 OS 代码签名 | 另行生成并验证 Tauri updater `.sig` 与内置 pubkey |
| WSL2 CLI | 解析本地化 verbose 表格 | 用 quiet/running 名称集合，版本 adapter 有 fixture |
| WSL2 配置写入 | 从 Windows 直接猜 Linux HOME | 在目标发行版默认用户上下文查询 HOME 并在 Linux 内写入 |
| Bash/Zsh | 用 shell escaping 代替 TOML escaping | 两层编码、两个模板、执行后结构化解析回验 |
| i18n | 只翻译 React | Rust 事件用 code，语言变化同步 tray/通知/导出 |
| macOS 登录启动 | 硬编码 app 路径 | 使用受支持 autostart 机制并在更新/移动后实测 |

## Performance and Responsiveness Traps

| Trap | Symptoms | Prevention | When It Breaks |
|------|----------|------------|----------------|
| 在 Tauri 主线程执行迁移、WSL 命令或网络验证 | 窗口白屏、托盘无响应、系统误判卡死 | 后台任务 + 可取消状态机；主线程只更新 UI | 首次迁移、慢 WSL 启动或供应商超时时立即出现 |
| 每个 Responses API 流片段写日志/IPC | 日志膨胀、UI 重渲染、凭据/响应泄露面扩大 | 只上报阶段和聚合进度，不记录完整流内容 | 单次验证即可出现 |
| WSL2 批量切换无限并行 | 多发行版抢占 CPU/磁盘、结果竞态、恢复顺序混乱 | 有界并发或顺序执行；每发行版独立状态机 | 2–3 个发行版即可观察 |
| tray 菜单每次状态变化整体追加监听器 | 切换一次触发多次写入 | 更新现有菜单并集中管理 listener 生命周期 | 多次打开窗口/切换语言后 |
| SQLite 长事务包住网络或进程等待 | `SQLITE_BUSY`、设置保存卡住 | DB 事务只覆盖本地最小原子状态；外部操作用 journal 编排 | 任一慢网络/进程退出即可 |

## Security Mistakes

| Mistake | Risk | Prevention |
|---------|------|------------|
| 数据库备份和 Codex 配置备份权限宽松 | 明文供应商凭据被同机其他用户读取 | 创建时限制权限/ACL，备份轮换继承同等级保护 |
| 服务地址校验只看字符串前缀 | 利用大小写、userinfo、重定向或解析差异绕过安全服务地址 | 使用 URL parser 校验最终 scheme/host；远程仅 HTTPS，HTTP 仅明确回环主机；重定向后再验证 |
| 把 API Key 放进 URL 查询参数 | 进入代理、系统日志和错误消息 | 只使用受控认证 Header/配置位置，日志禁止完整 URL |
| Tauri command 暴露“读取全部数据库/配置” | 前端 XSS 或调试路径一次取得全部凭据 | 最小 command、最小 capability、按操作返回 DTO |
| 临时文件/脚本使用默认权限 | 凭据在窗口期暴露 | `umask 077`、安全创建、成功/失败都清理 |
| 诊断导出打包整个应用目录再排除 | 新文件类型加入后自动泄密 | 从允许清单重新构造诊断包并执行 canary 扫描 |

## UX Pitfalls

| Pitfall | User Impact | Better Approach |
|---------|-------------|-----------------|
| 把待重启显示成切换失败 | 用户重复切换、制造更多备份和冲突 | 明确区分配置已更新与旧进程仍运行 |
| 进程检测失败时显示“没有运行中的 Codex” | 用户误以为立即生效 | 显示状态未知并保守进入待重启 |
| WSL2 批量结果只给总成功/失败 | 用户不知道哪个发行版仍需恢复 | 按 WSL2 环境逐项显示成功、失败、待重启和恢复状态 |
| 配置冲突时自动覆盖 | 用户丢失外部配置且不知原因 | 停止写入，展示外部配置/冲突并提供重新读取 |
| 更新失败后立即反复提示 | 每次启动骚扰且无法恢复 | 记录失败类型和最后检查，提供手动重试和可复制脱敏错误 |
| API Key 显示按钮把 Key 放进可访问名称 | 读屏或自动化日志泄露 | 可访问名称只描述动作，真实值仅存在于受控输入值 |

## “Looks Done But Isn’t” Checklist

- [ ] **SQLite 迁移：** 不只验证新建库；每个历史 fixture、WAL 快照、失败回滚和高版本拒写都通过。
- [ ] **配置原子写：** 不只验证单文件 rename；崩溃点、多工件 journal、Windows 占用、权限和复读均通过。
- [ ] **配置保留：** 含 MCP/profiles/features/注释/未知键/符号链接/并发修改的 corpus 不被静默覆盖。
- [ ] **供应商凭据脱敏：** canary 未出现在日志、错误 DTO、通知、诊断导出、临时文件和 release DevTools。
- [ ] **重启协调：** 同名进程、PID 重用、多 CLI、访问拒绝和确认期间竞态都不会误报已生效。
- [ ] **托盘驻留：** 关闭隐藏、重复打开、明确退出、更新退出和系统退出在 Windows/macOS 都通过。
- [ ] **WSL2 临时启动：** 成功、失败、取消、崩溃和外部并发使用都验证恢复策略。
- [ ] **WSL2 发现：** 简中/英文、WSL1/2 混合、imported distro、默认 root、Unicode 名称均覆盖。
- [ ] **Linux 切换脚本：** Bash 4+/Zsh 5+ 实际执行，恶意字段 corpus、损坏区块、备份轮换、权限和中断恢复均通过。
- [ ] **i18n：** React、tray、原生对话框、系统通知和导出脚本在运行时切换语言后保持一致。
- [ ] **无障碍：** NVDA/VoiceOver、纯键盘、200% 缩放、高对比度和减少动态效果完成真实验收。
- [ ] **更新：** N-1/N-2 正式签名安装版可经正式端点升级，失败时不破坏 SQLite/配置/WSL2 状态。
- [ ] **Windows 发布：** currentUser、x64/ARM64、WebView2、Authenticode/时间戳、无管理员安装与更新均通过。
- [ ] **macOS 发布：** Intel/Apple Silicon 或 universal slice、Developer ID、hardened runtime、公证/staple、当前用户更新权限与登录启动均通过。

## Recovery Strategies

| Pitfall | Recovery Cost | Recovery Steps |
|---------|---------------|----------------|
| 迁移失败或当前库损坏 | HIGH | 停止写入 → 验证迁移前一致备份 → 用户确认恢复 → 保留失败库供脱敏诊断 → 再次执行迁移 |
| 配置多工件写入中断 | MEDIUM/HIGH | 读取操作 journal → 比较旧/新哈希 → 幂等完成或从配置备份回滚 → 复读验证 → 修正 SQLite/待重启 |
| 外部配置并发冲突 | LOW | 不覆盖 → 重新读取 → 重新匹配供应商 ID/外部配置 → 用户再次发起切换 |
| 凭据进入日志/诊断 | HIGH | 停止导出/分发 → 删除本地泄露工件 → 提示用户轮换相关 API Key → 修复源头和 canary 回归 |
| 错误终止或未恢复 WSL2 | MEDIUM/HIGH | 展示遗留租约 → 检查当前活动 → 用户确认后仅终止目标发行版，或标记需手工恢复 |
| updater 密钥/签名错误 | HIGH | 暂停发布 → 修复元数据/工件映射；若旧客户端无法信任新密钥，只能提供由旧密钥签名的桥接更新或人工重装 |
| Windows/macOS 安装器错误 | HIGH | 撤下候选包 → 保持旧版本更新端点 → 发布同身份、同安装范围的修复包 → 从旧正式版重演升级 |

## Pitfall-to-Phase Mapping

| Pitfall | Prevention Phase | Verification |
|---------|------------------|--------------|
| SQLite 一致备份与迁移 | 阶段 1；阶段 8 放行 | 历史 fixture、WAL、failpoint、N-1/N-2 |
| 多工件可恢复配置事务 | 阶段 1 | 每步骤崩溃注入，重启后只有完整旧/新状态 |
| 外部配置、符号链接和优先级 | 阶段 1 + 阶段 3 | corpus 差分、并发改写、真实 Codex 层级 |
| 供应商凭据脱敏 | 阶段 1 起贯穿全程 | canary 扫描所有新路径和 release 包 |
| 进程识别与重启协调 | 阶段 3 | 路径/bundle ID、多实例、竞态、访问拒绝 |
| Tauri 托盘生命周期 | 阶段 4 | show/hide/quit/update/system-exit 双平台循环 |
| WSL2 发现与默认用户 | 阶段 5 | 本地化、imported、WSL1/2、root/Unicode |
| WSL2 临时启动恢复 | 阶段 5 | kill 注入、遗留租约、外部并发使用 |
| Bash/Zsh 数据编码 | 阶段 6 | 属性测试 + Bash/Zsh 实际解析回验 |
| Linux 配置编辑与备份 | 阶段 6 | 损坏区块、权限、SIGINT、磁盘满、轮换 |
| i18n 与无障碍 | 阶段 4，阶段 5/6 补齐 | 双语 tray/通知/脚本，NVDA/VoiceOver 实测 |
| updater 信任链与清理 | 阶段 7 + 阶段 8 | 正式端点、错误签名、N-1/N-2、退出 failpoint |
| Windows 打包 | 阶段 8 | 非管理员 x64/ARM64 干净 VM |
| macOS 打包 | 阶段 8 | Intel/Apple Silicon 真实下载、签名/公证/更新 |

## Phase-Specific Research Flags

| Phase | Research flag | 原因 |
|-------|---------------|------|
| 阶段 1 | **需要实现前 spike** | Windows 原子覆盖、目录刷盘、ACL 保留、SQLite backup crate 的具体 API 需用所选 crate 验证 |
| 阶段 3 | **需要实现前复核官方 Codex 配置契约** | Codex 配置字段、认证存储和宿主应用可执行身份可能随发布变化 |
| 阶段 5 | **需要真实环境 spike** | WSL2 临时启动期间外部并发使用的安全恢复无法仅靠单元测试证明 |
| 阶段 6 | 标准但需高强度测试 | 两套 shell 模板和属性测试可控，不能用“只做 Bash 再兼容 Zsh”缩减 |
| 阶段 7 | **需要发布基础设施 spike** | updater 公钥策略、私钥恢复、目标元数据和 daily check 时钟语义必须提前冻结 |
| 阶段 8 | **强制实机/签名环境** | Windows ARM64、macOS Intel、Gatekeeper/SmartScreen、当前用户更新权限不能由开发构建替代 |

## Gaps and Open Verification Questions

- 截至 2026-08-05，OpenAI Codex 配置参考仍可能继续演进；阶段 3 必须基于当时锁定的 Codex 版本重新确认字段、认证存储、优先级和宿主应用共享行为。
- macOS “当前用户安装”与用户把 `.app` 拖入 `/Applications` 的现实行为存在实施边界；阶段 8 需明确受支持安装位置和无写权限时的 updater UX，但不能退回管理员/整机安装。
- WSL2 在 GPTEasy 临时启动后被其他程序并发使用时，是否可以可靠证明“运行状态仍归 GPTEasy 所有”需要 spike；无法证明时应保守不终止并报告。
- Windows/macOS 宿主应用最终 executable 路径、bundle identifier 和重启能力需在正式签名版本上确认，不能从开发安装猜测。

## Sources

以下结论以官方文档或一手规范为主，并对关键结论进行跨来源核对；综合置信度为 MEDIUM。

### Tauri 2

- System Tray: https://v2.tauri.app/learn/system-tray/
- Tauri `WindowEvent`: https://docs.rs/tauri/latest/tauri/enum.WindowEvent.html
- Tauri `AppHandle`: https://docs.rs/tauri/latest/tauri/struct.AppHandle.html
- Updater plugin: https://v2.tauri.app/plugin/updater/
- Windows installer / WebView2: https://v2.tauri.app/distribute/windows-installer/
- Windows code signing: https://v2.tauri.app/distribute/sign/windows/
- macOS code signing and notarization: https://v2.tauri.app/distribute/sign/macos/
- Autostart plugin: https://v2.tauri.app/plugin/autostart/
- Tauri configuration reference: https://v2.tauri.app/reference/config/

### SQLite

- SQLite Online Backup API: https://www.sqlite.org/backup.html
- Write-Ahead Logging: https://www.sqlite.org/wal.html
- Transactions: https://www.sqlite.org/lang_transaction.html
- PRAGMA `user_version`, `foreign_keys`, `integrity_check`: https://www.sqlite.org/pragma.html

### Filesystem and process APIs

- Rust `std::fs::rename`: https://doc.rust-lang.org/std/fs/fn.rename.html
- Microsoft `ReplaceFileW`: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew
- Microsoft `MoveFileExW`: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw
- POSIX `rename`: https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html
- Windows process enumeration: https://learn.microsoft.com/en-us/windows/win32/procthread/process-enumeration
- Windows `QueryFullProcessImageNameW`: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-queryfullprocessimagenamew
- Windows process handles and identifiers: https://learn.microsoft.com/en-us/windows/win32/procthread/process-handles-and-identifiers
- Apple `NSWorkspace.runningApplications`: https://developer.apple.com/documentation/appkit/nsworkspace/runningapplications
- Apple `NSRunningApplication`: https://developer.apple.com/documentation/appkit/nsrunningapplication

### Codex, WSL2 and shell

- OpenAI Codex basic configuration: https://developers.openai.com/codex/config-basic
- OpenAI Codex configuration reference: https://developers.openai.com/codex/config-reference
- Microsoft WSL basic commands: https://learn.microsoft.com/en-us/windows/wsl/basic-commands
- Microsoft WSL filesystems: https://learn.microsoft.com/en-us/windows/wsl/filesystems
- Microsoft WSL1/WSL2 comparison: https://learn.microsoft.com/en-us/windows/wsl/compare-versions
- GNU Bash manual: https://www.gnu.org/software/bash/manual/
- Zsh manual: https://zsh.sourceforge.io/Doc/Release/
- POSIX Shell Command Language: https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html

### Security, accessibility and platform release

- Tauri security capabilities: https://v2.tauri.app/security/capabilities/
- OWASP Logging Cheat Sheet: https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html
- WCAG 2.2: https://www.w3.org/TR/WCAG22/
- WAI-ARIA Authoring Practices: https://www.w3.org/WAI/ARIA/apg/
- Microsoft Authenticode/SignTool: https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool
- Apple notarization overview: https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution

---
*Pitfalls research for: GPTEasy*
*Researched: 2026-08-05*
