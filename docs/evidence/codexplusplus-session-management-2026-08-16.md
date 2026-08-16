# Codex++ 会话管理调查

## 调查范围

- 日期：2026-08-16
- 目标：确认用户所称“Codex++”的项目身份，调查其会话列表、搜索、读取、继续、删除、归档、跨工作区、并发和生命周期实现，并与 Codex 官方集成入口比较。
- Codex++ 固定版本：[`BigPizzaV3/CodexPlusPlus@1f431ae`](https://github.com/BigPizzaV3/CodexPlusPlus/commit/1f431ae49b57b3055e0e6845ba6156c6b4232b4d)，提交日期 2026-08-13。
- OpenAI Codex 固定版本：[`openai/codex@9ded177`](https://github.com/openai/codex/commit/9ded177ce7c1c0bd2047f902936c177612ab3434)，提交日期 2026-08-16。
- 资料范围：项目官方仓库、源码、发布说明和 OpenAI 官方文档。没有使用第三方评测或转述。
- 本文是实现前证据，不是 ADR，也不代表会话管理的产品范围已经确定。

## 项目身份辨析

本调查所称 Codex++ 是 **`BigPizzaV3/CodexPlusPlus`**。证据是其 README 直接自称 Codex++，定义为 OpenAI Codex / ChatGPT 桌面应用的“外部启动器与管理工具”，明确列出会话管理，并提供独立的启动器和管理工具。[来源：README 的产品定义与入口](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/README.md#L19-L34)

存在另一个独立同名项目 **`b-nnett/codex-plusplus`**。它把自己定义为 Codex Desktop 的 tweak loader，会修改 `app.asar` 以加载本地运行时；这与本需求所讨论的外部管理器、会话扫描和批量删除不是同一产品。[来源：`b-nnett/codex-plusplus@f98e7e9` README](https://github.com/b-nnett/codex-plusplus/blob/f98e7e9d1fa068dde9e0dddfb43b128acb4e2fd7/README.md#L1-L24)

以下“Codex++”均指 `BigPizzaV3/CodexPlusPlus`。

## 核心结论

1. Codex++ **没有建立自己的会话事实库**。它读取 Codex 的 SQLite 元数据和 rollout JSONL，并直接修改或删除这些上游文件；自己的目录只保存设置、日志和删除备份。[来源：数据位置](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/README.md#L219-L225)、[数据库发现](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/codex_sqlite.rs#L25-L36)
2. 它有两层会话体验：管理工具负责分页查看和批量删除；官方 Codex 页面内的注入脚本负责在原生会话行上增加删除、导出、移动和滚动位置恢复。继续会话仍是官方 Codex 的原生导航，不是 Codex++ 后端能力。[来源：管理页](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L5000-L5156)、[Bridge 能力集合](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/routes.rs#L106-L126)
3. 管理页已有多库聚合、按更新时间倒序、线程 ID 去重、分页、当前页多选和逐项删除；**没有标题/正文搜索、详情读取、重命名、置顶、从管理页打开或 resume**。[来源：列表请求只有 offset/limit](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src-tauri/src/commands.rs#L166-L191)、[列表实现](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src-tauri/src/commands.rs#L1747-L1809)、[管理页行操作](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L5075-L5156)
4. 删除不是归档。Codex++ 备份命中的 SQLite 行和 rollout 文件后，删除数据库记录，再删除 rollout 文件；注入 UI 可用备份 token 撤销。管理工具会展示删除结果，但没有同等的撤销操作入口。[来源：线程删除](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L551-L662)、[注入 UI 撤销](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L7551-L7571)
5. Codex++ 对“正在使用的会话”没有权威生命周期协调。管理页只提示用户先关闭对应会话窗口；源码的删除事务只覆盖单个 SQLite 数据库，rollout 删除发生在事务提交之后。因此 Codex 正在写入时仍存在锁竞争或数据库与文件只完成一半的窗口。[来源：UI 警告](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L5054-L5057)、[提交后删除文件](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L609-L660)
6. 当前 OpenAI 官方文档已经把 App Server 定义为深度集成接口，并提供 `thread/list`、`thread/read`、`thread/resume`、`thread/fork`、`thread/name/set`、`thread/metadata/update`、`thread/archive`、`thread/delete`、`thread/unarchive`、运行状态和生命周期通知。GPTEasy 应优先验证并使用这个入口，不应复制 Codex++ 的 CDP/DOM 和直接写库方案。[来源：OpenAI Docs App Server 的定位与 API overview](https://learn.chatgpt.com/docs/app-server#api-overview)

## Codex++ 的实现结构

```text
Codex 官方桌面应用
  ├─ SQLite：线程索引、标题、cwd、provider、归档状态等
  ├─ rollout JSONL：消息与事件正文
  └─ 原生页面：侧栏、归档页、会话导航
          ▲
          │ CDP 注入 + Runtime binding
Codex++ 启动器/本地 Bridge
  ├─ 删除、撤销、Markdown 导出、用量历史、项目移动
  └─ Bridge watchdog 在页面刷新或目标变化后重新注入
          ▲
          │
Codex++ 管理工具
  └─ 直接读取 SQLite，并直接调用本地删除实现
```

Codex++ 通过 `--remote-debugging-port` 启动官方应用，再选择 Codex 页面 target 并安装脚本和 binding；它不修改 `app.asar`。[来源：启动参数](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/launcher.rs#L2253-L2259)、[注入实现](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/launcher.rs#L2498-L2520)、[README 边界](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/README.md#L19-L20)

这种方案有明确兼容成本。源码会读取私有 DOM 属性、React Fiber 和中文“取消归档”按钮来识别线程，项目自己也承认依赖官方页面结构、CDP 和本地数据格式。[来源：会话 ID 提取](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L5084-L5127)、[归档页识别](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L5041-L5077)、[兼容性声明](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/README.md#L306-L308)

## 数据模型与持久化

### Codex++ 的列表投影

`LocalSession` 只有以下字段：

| 字段 | 来源/用途 |
| --- | --- |
| `id` | Codex thread ID |
| `title` | 会话标题 |
| `cwd` | 项目路径/工作目录 |
| `modelProvider` | 创建或同步后的 provider 标记 |
| `archived` | SQLite 归档状态 |
| `updatedAtMs` | 排序与显示 |
| `rolloutPath` | JSONL 文件位置 |
| `dbPath` | 该记录来自哪个候选数据库 |

来源：[`LocalSession` 定义](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L81-L92)。

这是管理投影，不是完整会话领域模型。它没有 fork/parent、session tree、source、ephemeral、Git 信息、运行状态、turn 或 item。相比之下，官方 App Server `Thread` 已经提供这些字段，并明确 `turns` 只在 read/resume/fork 等需要时填充。[来源：官方 `Thread` 模型](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs#L193-L262)

### 数据库发现与兼容

Codex++：

- 优先扫描 `CODEX_SQLITE_HOME`，否则使用 Codex home。
- 枚举 `sqlite/` 下含 `threads`、`automation_runs` 或 `inbox_items` 的 `.db`、`.sqlite`、`.sqlite3` 文件。
- 最后追加旧版 `state_5.sqlite`。
- 当前会话 schema、旧 generic `sessions/messages` schema、`automation_runs` schema分别分支处理。

来源：[`codex_session_db_paths_from_home`](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/codex_sqlite.rs#L25-L36)、[候选库扫描](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/codex_sqlite.rs#L80-L138)、[schema 判别](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L990-L1003)。

这解释了它为何要维护多版本表结构、重复记录和兼容回退，也说明 GPTEasy 若直接采用 SQLite，会继承同样的上游 schema 维护成本。

### 消息正文与导出

Codex++ 从 SQLite 取得 `rollout_path`，再解析 rollout JSONL 中的用户/助手 `message` 事件生成 Markdown。`automation_runs` 没有直接路径时，它会递归扫描 `sessions` 和 `archived_sessions` 并按 thread ID 匹配。[来源：Markdown 导出主流程](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/markdown.rs#L47-L95)、[rollout 发现](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/markdown.rs#L128-L200)

因此 SQLite 不是完整正文存储，不能只复制 `threads` 表实现搜索、详情或导出。

### 删除备份

删除前，Codex++ 把原始 SQLite 行写成 JSON；rollout 文件整体以 Base64 写入同一备份。备份 token 是时间戳加 UUID，保存在 Codex++ 状态目录下的 `backups/`。[来源：备份格式](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/backup.rs#L8-L60)、[rollout 备份](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L1272-L1284)

源码没有备份到期、容量上限或清理策略。撤销会先检查数据库行和目标文件冲突；多库删除返回 token 数组并逐库恢复。[来源：多库 token](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L13-L42)、[撤销预检与恢复](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L874-L929)、[冲突检查](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L1069-L1163)

## 功能逐项调查

| 关注点 | Codex++ 当前实现 | 已确认限制 |
| --- | --- | --- |
| 列表 | 所有候选库各取 `offset + limit + 1` 条，合并后按更新时间倒序、按 ID 去重，再切页；默认 50，最大 100。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src-tauri/src/commands.rs#L1747-L1795) | offset 不是稳定 cursor；并发新增/更新时可能跨页重复或漏项。每个库都随 offset 增大而多取数据。 |
| 搜索 | 管理器无搜索参数、输入框或搜索索引。[来源：请求结构](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src-tauri/src/commands.rs#L166-L191) | 不能按标题、正文、ID、cwd 或 provider 搜索。 |
| 详情 | 管理页只展示标题、ID、cwd、provider、归档状态、更新时间。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L5098-L5127) | 没有消息预览或完整 turn/item 详情。 |
| 恢复/继续 | 没有 `/resume` 或 `/open` bridge 路由；注入按钮挂在官方侧栏行，用户继续会话仍点击官方行。[来源：Bridge 路由全集](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/routes.rs#L227-L281) | 管理工具不能继续会话；“切换对话保留位置”只恢复滚动位置，不是恢复运行态。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L5631-L5702) |
| 删除 | 单删、当前页多选、全选当前页、逐项批量删除；删除 DB 行和 rollout。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L4926-L4998)、[逐项执行](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L1628-L1665) | “全选”仅当前页；批量删除没有跨项目事务，可能部分成功。 |
| 撤销 | 注入 UI 的删除 toast 提供 10 秒内可见的撤销按钮，底层备份本身不随 toast 消失。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L7551-L7571) | 管理工具删除后没有撤销入口；备份长期占用空间。 |
| 归档/取消归档 | 归档动作仍来自官方 UI；Codex++ 识别官方归档页，为归档行补导出等动作，并可按标题回查 ID。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L9023-L9105) | Bridge 没有 archive/unarchive 命令；按标题回查可能命中同名会话。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L264-L282) |
| Markdown 导出 | 从 rollout JSONL 解析用户/助手消息，可处理 active 和 archived 目录。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/markdown.rs#L47-L95) | 输出不是上游正式 API；新事件结构需要继续适配。管理页本身未提供导出按钮，导出在注入 UI 中。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/assets/inject/renderer-inject.js#L8958-L8981) |
| 跨工作区 | 无 cwd 过滤也会列出全部本地会话；行展示 cwd。注入 UI 可把会话移动到 Codex 已知项目。[来源：列表投影](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L162-L203) | “移动”本质是更新 `threads.cwd` 和 rollout 的 `session_meta.cwd`，不是移动代码目录；这依赖私有持久化结构。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-data/src/storage.rs#L285-L366) |
| 并发 | 批量删除在前端顺序执行。每个库使用单独 SQLite 事务。[来源](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/apps/codex-plus-manager/src/App.tsx#L1645-L1654) | 没有跨库事务、每线程 mutation queue 或与 Codex writer 的关停握手；UI 只能提示用户关闭会话。 |
| 启动生命周期 | launcher 启动 helper、启动 Codex、注入 Bridge，并用 watchdog 检测和重新注入；Codex 退出后关闭 helper。[来源：启动流程](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/launcher.rs#L264-L409)、[退出清理](https://github.com/BigPizzaV3/CodexPlusPlus/blob/1f431ae49b57b3055e0e6845ba6156c6b4232b4d/crates/codex-plus-core/src/launcher.rs#L124-L134) | Bridge 生命周期绑定官方桌面页面；页面结构更新会降级或等待重新注入。 |

## 官方 App Server 对比

OpenAI 官方文档把 App Server 定义为供 VS Code 等 rich client 使用的深度集成接口，覆盖认证、会话历史、审批和流式 agent 事件。默认传输是 stdio JSONL；WebSocket 明确仍是 experimental/unsupported。因此本地 Tauri 应优先考虑 stdio 子进程，而不是 WebSocket 或 CDP。[来源：OpenAI Docs App Server 定位与 Protocol](https://learn.chatgpt.com/docs/app-server#protocol)

| 能力 | 官方入口 | 相对 Codex++ 的意义 |
| --- | --- | --- |
| 列表 | `thread/list`：cursor、limit、sort、provider、source、archived、cwd、title `searchTerm`；返回 runtime status。[文档](https://learn.chatgpt.com/docs/app-server#list-threads-with-pagination--filters)、[固定源码](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L1346-L1500) | 不需要扫描多库、去重或维护 offset 合并算法。 |
| 读取 | `thread/read(includeTurns)` 读取而不 resume；`thread/turns/list` 和 `thread/items/list` 提供分页但当前标记 experimental。[文档](https://learn.chatgpt.com/docs/app-server#read-a-stored-thread-without-resuming)、[固定源码](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L1623-L1638) | 可从结构化 turn/item 生成详情与导出，不直接解析 JSONL。 |
| 继续 | `thread/resume(threadId)` 载入已有线程，随后 `turn/start` 追加请求；归档线程必须先 unarchive。[文档](https://learn.chatgpt.com/docs/app-server#resume-a-thread)、[固定源码](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L320-L411)、[归档保护](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server/src/request_processors/thread_processor.rs#L4105-L4168) | 这是“GPTEasy 自己承载会话”的继续能力；它不等于让官方桌面应用跳转到某个会话。 |
| 分支 | `thread/fork` 复制历史并生成新 thread ID。[文档](https://learn.chatgpt.com/docs/app-server#api-overview) | 无需复制 rollout 或修改 SQLite。 |
| 命名 | `thread/name/set`。[固定源码](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/common.rs#L557-L560) | 可做正式重命名，不修改私有列。 |
| 归档 | `thread/archive` 和 `thread/unarchive`，并发送生命周期通知。[文档](https://learn.chatgpt.com/docs/app-server#archive-a-thread)、[固定源码](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/common.rs#L523-L531) | 提供可逆的默认清理路径。 |
| 永久删除 | `thread/delete` 会删除 active/archived 根线程及 spawned descendants，并发送每个删除通知。[文档](https://learn.chatgpt.com/docs/app-server#delete-a-thread)、[固定源码](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server/src/request_processors/thread_delete.rs#L31-L83) | 官方接口没有 Codex++ 式 undo；产品必须把 archive 与 delete 明确分开。 |
| 活跃状态 | `thread/loaded/list`，`Thread.status`，`thread/status/changed`，`thread/unsubscribe` 和 `thread/closed`。[文档](https://learn.chatgpt.com/docs/app-server#api-overview)、[状态类型](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L1577-L1638) | GPTEasy 可在破坏性操作前展示 idle/active/等待审批，而不是仅给静态警告。 |
| 并发协调 | 协议把读列表设为并发，把 thread read/mutation 按 `threadId` 序列化；删除前会卸载活跃线程并等待 shutdown。[方法映射](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/common.rs#L505-L535)、[删除/归档前关停](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server/src/request_processors/thread_processor.rs#L985-L1005) | 比直接写 SQLite 更接近线程 owner，能减少 writer race。 |

### `isPinned` 的版本差异

2026-08-16 获取的在线官方文档写明：

- `thread/list` 支持 `isPinned` 过滤；
- `Thread` 返回 `isPinned`；
- `thread/metadata/update` 可更新 `isPinned`。

来源：[OpenAI Docs 列表过滤](https://learn.chatgpt.com/docs/app-server#list-threads-with-pagination--filters)、[更新 stored metadata](https://learn.chatgpt.com/docs/app-server#update-stored-thread-metadata)。

但是固定的公开源码 `openai/codex@9ded177` 中，`ThreadListParams` 没有 `is_pinned`，`ThreadMetadataUpdateParams` 只有 `git_info`，`Thread` 也没有 `is_pinned`。[列表参数](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L1346-L1405)、[metadata 参数](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L968-L1021)、[Thread 模型](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs#L193-L262)

这不是应被掩盖的细节：在线文档可能领先于该公开提交或对应另一构建。GPTEasy 不应仅凭网页开启置顶功能，应对实际支持的 Codex 版本运行 `codex app-server generate-json-schema` 或 `generate-ts`，保存能力快照并在缺少字段时隐藏功能。官方文档也明确生成结果与执行命令的 Codex 版本完全一致。[来源：OpenAI Docs Message schema](https://learn.chatgpt.com/docs/app-server#message-schema)

### stdio 与 Windows 子进程生命周期

固定源码中的 stdio reader 在 stdin EOF 或读取失败后发送 `ConnectionClosed`；App Server 把 stdio 识别为单客户端模式，并在该连接关闭时退出主循环。因此 GPTEasy 正常关闭时可以先关闭 stdin，等待 App Server 自然退出，而不必直接强制终止。[来源：stdio EOF 处理](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-transport/src/transport/stdio.rs#L43-L80)、[单客户端关闭处理](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server/src/lib.rs)

Windows 的 `CREATE_NO_WINDOW` 会让控制台应用在不创建控制台窗口的情况下运行，适用于后台启动 App Server；stdin、stdout 和 stderr 仍应全部使用管道，避免继承宿主控制台。[来源：Microsoft Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags#CREATE_NO_WINDOW)

Windows Job Object 的 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 会在最后一个 Job handle 关闭时终止关联进程。GPTEasy 应把自有 App Server 进程树放入专用 Job Object，用它覆盖正常退出、崩溃和被终止路径；持久化的 PID 与创建时间只作为防御性异常恢复，不能据此按进程名清理其他 Codex 进程。[来源：Job Object limit flags](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information#members)、[AssignProcessToJobObject](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject)

## 对 GPTEasy 的规划建议

### 建议采用的边界

1. **App Server 是会话事实与 mutation owner。** GPTEasy 不复制会话正文，不直接更新 Codex SQLite，不直接移动或删除 rollout。
2. **Tauri 后端拥有一个长生命周期的 App Server stdio 子进程。** 后端完成 `initialize`/`initialized`、request ID 路由、通知转发、崩溃重启和关闭；React 不直接持有进程或协议连接。
3. **按 Codex 环境隔离连接。** Windows 原生 Codex home 和每个受支持的 WSL/Linux 环境不能混成一个数据库视图；跨工作区是在同一环境内按 `cwd` 聚合和筛选。
4. **GPTEasy 自己的 SQLite 只保存产品状态。** 例如视图偏好、最近过滤器、能力快照和可恢复的 UI 操作状态；不缓存第二份会话事实。
5. **所有版本相关能力先探测。** 启动时记录 Codex 版本和 App Server schema 版本；缺少某个 method/field 时以可解释的 capability 状态降级。

### 建议的最小功能顺序

#### 阶段 0：合同探针

- 对当前最低支持 Codex 版本生成 schema。
- 验证 stdio 初始化、`thread/list`、`thread/read`、`thread/archive`、`thread/unarchive`、`thread/delete`、`thread/resume`。
- 记录稳定字段与 experimental 字段，特别核对 `isPinned`。
- 验证官方桌面应用和 GPTEasy App Server 同时运行时的 Codex home、writer lock 和通知行为。

#### 阶段 1：只读管理

- 以 `thread/list` cursor 实现全工作区列表，默认 `updated_at desc`。
- 显式选择 `sourceKinds`，避免官方默认仅返回一部分交互来源而漏掉 App Server 或 exec 会话。
- 展示 name/preview、thread ID、cwd、provider、source、updatedAt、status、Git 信息和归档状态。
- 先用稳定的 `searchTerm` 做标题搜索；正文全文搜索暂不依赖 experimental `thread/search`。
- 用 `thread/read(includeTurns=true)` 实现详情与 Markdown 导出。

#### 阶段 2：可逆管理

- 归档作为主清理动作，支持单个和批量 archive/unarchive。
- 批量请求保持逐项结果，不伪装成全局事务；失败项可单独重试。
- 订阅 archive/unarchive/status/name 通知，避免操作后全量刷新。
- 若运行状态为 active 或等待审批，明确提示影响并由 App Server 协调关停。

#### 阶段 3：永久删除

- 单独的危险操作入口，不把 delete 叫“清理”或“归档”。
- 明确告知会一并删除 spawned descendants；需要确认输入或第二次确认。
- 官方 `thread/delete` 没有 undo。若产品必须可撤销，应回到“默认归档”而不是私自复制 Codex 内部文件做伪事务。
- 可选提供“删除前导出 Markdown”，但导出不是恢复机制。

#### 阶段 4：继续与跨工作区

- 先定义“继续”的产品语义：
  - 如果只是打开官方桌面应用中的现有会话，需要找到 OpenAI 官方支持的导航/deep-link 合同；本次资料没有找到，Codex++ 也没有提供。
  - 如果 GPTEasy 自己承载对话，则需实现 `thread/resume`、`turn/start`、审批、item/turn 流、interrupt、订阅与输入状态，这远大于“会话管理页”。
- `cwd` 只作为过滤和分组维度。不要复制 Codex++ 直接改 `threads.cwd` 和 rollout `session_meta.cwd` 的“项目移动”。如果以后需要改变线程运行目录，应使用官方稳定方法；当前 `thread/resume.cwd` 是本次已确认的运行时入口，但是否应持久化成“移动”需要另行定义和验证。[来源：`ThreadResumeParams.cwd`](https://github.com/openai/codex/blob/9ded177ce7c1c0bd2047f902936c177612ab3434/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L349-L367)

## 需要在需求访谈中确认的问题

1. “会话管理”的首要任务是找回历史、清理历史、在官方 Codex 中打开，还是在 GPTEasy 内继续聊天？这四种目标的实现规模不同。
2. 首版要跨所有 cwd 浏览，还是默认仅当前项目、再允许切到“全部项目”？
3. 搜索首版只搜标题是否足够？正文搜索若必须首发，需要决定接受 experimental `thread/search`，还是建立受版本控制的本地只读索引。
4. 默认删除语义应否改为归档？是否真的需要永久删除与批量永久删除？
5. 永久删除 spawned descendants 是否符合用户预期？UI 是否需要预览子线程数量？
6. 是否需要展示 CLI、VS Code、App Server、exec、sub-agent 等所有来源，还是只展示用户直接创建的交互会话？
7. archived 和 active 是否在同一列表用筛选器，还是分成两个视图？
8. “跨工作区”是否包含 Windows 与 WSL2 的不同 Codex home？若包含，应视为多环境聚合，而不是一个查询过滤器。
9. 是否接受 GPTEasy 依赖某个最低 Codex CLI 版本？若不接受，需要定义老版本只读降级，而不是继续直接写 SQLite。
10. 置顶是否为首版必需？若是，必须先解决在线文档与公开源码的版本差异。

## 最终判断

Codex++ 值得借鉴的是产品层经验：全工作区视图、分页、多选、逐项批量结果、归档会话导出、运行中会话警告和滚动位置恢复。它的底层实现是官方接口不足时期的兼容层：多版本 SQLite 探测、rollout 解析、CDP/DOM 注入和直接修改本地状态。

对当前 GPTEasy，更稳的路线是把 OpenAI App Server 当作权威会话端口，以官方 thread 生命周期实现列表、读取、恢复、归档和删除；只把 Codex++ 当作交互与边界案例样本，不复制其私有数据写入与页面注入方式。实施前必须用实际支持版本生成 schema，因为当前在线文档和固定公开源码已经出现 `isPinned` 合同差异。
