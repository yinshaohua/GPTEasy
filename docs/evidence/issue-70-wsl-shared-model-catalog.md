# Issue #70 WSL2 与 Linux 脚本完整模型目录共同管理验收

日期：2026-10-03。关联决策：[ADR-0056](../adr/0056-wsl-shared-offline-model-catalog.md)。验证基线为 main 的 57e495a；#70 提交仅包含本功能改动，用户已有供应商重验证修复及归档、临时文件未纳入。

## 实现与覆盖

- 桌面复用 Linux schema v2、共享模型目录生成器及 common-reasoning-selector-v1 策略。目录由绑定已验证供应商组合的完整快照生成，全部发现模型提供 low/medium/high/xhigh、default high；根级实际 effort 沿用既有映射。
- bundle/helper V3 携带 config、credentials、catalog 和目录绑定元数据；guest 在共享锁内先安装私有不可变目录，再原子替换配置并复读三类工件。SQLite schema v12 保存旧/新三工件摘要，覆盖 prepared、工件提交、state_committed 等恢复窗口；第三种状态保留现场和锁。
- 只读识别 schema v1 为旧格式，明确应用才迁移；协调分别报告供应商关键组合变化、目录快照/策略差异及目录缺失/损坏。名称、来源 ID、物理路径差异不构成供应商变化。
- 桌面限定发行版注册身份、默认 Linux 用户及默认 $HOME/.codex；脚本自定义 CODEX_HOME 独立。Stopped 普通 inventory 不进入 guest；只清该环境待刷新项。显式操作仅观察自然停止，最多十秒，不执行强制 terminate。
- 原生 Linux CLI 的版本检查后执行隔离 App Server model/list 能力探针，拒绝目录/schema 不兼容及 Windows 互操作入口。Running 提示磁盘已保存与实例可能待刷新，不控制用户共享后台服务，也不把重新打开终端当作刷新确认。
- 问题日志仅记录固定阶段/状态与计数，新增回归测试核验目录失败分类及敏感 canary 不泄露。

## 执行环境

Windows 开发机，真实 WSL2 Ubuntu 已处于 Running。shell 黑盒矩阵使用 Ubuntu 的 Bash 5.2.21 和 Zsh 5.9；设置 GPTEASY_REQUIRE_SHELL_MATRIX=1，要求两个 shell 均执行。真实 Running guest harness 使用发行版内的隔离 home 与协议 fixture CLI，不使用用户 Codex 配置，不代表真实 Codex 交互菜单验收。

未启动处于 Stopped 的 docker-desktop，不终止 Ubuntu，不主动控制用户 daemon。

## 自动化结果

| 检查 | 结果与边界 |
| --- | --- |
| npm run check | 通过 |
| npm test | 通过：113 个 Vitest 测试（7 个文件）、29 个 Node 测试 |
| cargo fmt --check | 通过 |
| cargo test --lib wsl:: | 56 通过、1 ignored |
| cargo test --test state_store | 21 通过，包含 v11 → v12 三工件摘要迁移与历史 NULL 字段兼容 |
| 目录问题日志回归 | 通过，核验缺失、损坏、绑定、旧策略、快照变化、恢复冲突及待刷新计数脱敏 |
| 隔离工作树 cargo test --quiet（强制 Bash/Zsh 矩阵） | 两项供应商重验证测试失败；其余已执行测试通过，详见下文基线核验 |
| 隔离工作树套件，排除已复现的两项基线失败 | 分段累计 468 通过、7 ignored、2 排除；doc tests 通过（0 项） |
| 真实 Running guest 双向切换/恢复、目录安全及锁 harness | 通过；新增 CLI 能力缺失、Windows 互操作拒绝测试后单项再次通过（44.89 秒） |
| 真实 Running guest 各持久化 Saga 阶段恢复 harness | 通过；与双向 harness 一起执行为 2 通过（68.41 秒） |

执行 Rust 命令时工作目录为 src-tauri。隔离工作树以 57e495a 创建，仅复制 #70 文件，未复制用户已有的 provider.rs/provider/catalog.rs 修复。

强制 shell 矩阵：

~~~powershell
$env:GPTEASY_REQUIRE_SHELL_MATRIX='1'
$env:GPTEASY_TEST_WSL_DISTRIBUTION='Ubuntu'
cargo test --quiet
~~~

真实 Running guest harness：

~~~powershell
$env:GPTEASY_RUN_WSL_GUEST_HARNESS='1'
$env:GPTEASY_WSL_TEST_DISTRIBUTION='Ubuntu'
cargo test --features wsl-guest-harness --test wsl_guest_harness running_guest -- --ignored --test-threads=1
~~~

最后增加 CLI 拒绝回归后，单独运行过滤器 running_guest_harness 验证：fixture 版本有效但缺少 xhigh 时返回 wsl.catalog_cli_incompatible；MZ Windows 入口返回 wsl.codex_version_required。两种情况均核验 config/catalog 未变化、没有 pending Saga 或共享锁。

## 两项已有基线失败

仅含 #70 的工作树中，两项测试返回 provider.not_found：

- catalog_revalidation_returns_a_candidate_receipt_without_persisting_it
- revalidation_repairs_missing_and_malformed_snapshots_without_loading_them_first

在完全未修改的 57e495a 工作树运行 cargo test --quiet --test provider_workflow revalidation，得到同样的两项失败，另外两项通过（0.88 秒），确认不是 #70 引入。用户已有未提交的供应商重验证改动涉及这条路径；遵守提交范围要求，保留其独立工作，不将完整隔离套件记录为全绿。

为继续验证未执行到的后续套件，采用以下明确排除命令。发布脚本的本地 mock 测试固定向工作树内 src-tauri/target 写 fixture；因隔离构建使用外部 CARGO_TARGET_DIR，首次在此处遇到目录不存在。补建临时 target 目录后，单独运行 cargo test --quiet --test update_release_baseline --test wsl_guest_harness，得到 14 通过（未启用 harness feature 时该文件 0 项）；cargo test --quiet --doc 也通过（0 项）。本地 mock 不发布到外部服务。其余套件分段累计结果见上表：

~~~powershell
cargo test --quiet -- --skip catalog_revalidation_returns_a_candidate_receipt_without_persisting_it --skip revalidation_repairs_missing_and_malformed_snapshots_without_loading_them_first
~~~

## 双向恢复发现的互操作回归

真实 Running harness 曾发现：shell 目录路径校验把 source 当作 UUID，拒绝桌面生成的 desktop-<UUID> 来源，导致无法恢复桌面目录。修复后，来源组件与桌面规则一致，仅允许字母、数字、点、下划线、连字符并拒绝 ..；artifact 仍要求 UUID，权限与 SHA 校验不变。双向切换/恢复 harness 已重跑通过，临时诊断已清除。

## 未执行的验收

- 真实 Stopped 发行版 UAT；Stopped 零 guest 调用、多环境隔离及无 terminate 由自动化 fake runtime 验证。
- Bash 4.4 与原生 GNU/Linux 主机矩阵；本次 Bash/Zsh 运行于 WSL2。
- 真实原生 Linux Codex CLI 的目录/schema、交互模型/四档菜单及运行服务刷新；本机没有可用目标 CLI，harness 使用协议 fixture。
- Windows 一次性账户完整 UAT、正式发布门禁。

本证据不将上述未执行项记为通过。ADR-0054 保持 proposed，主动 daemon 控制及跨平台真实菜单验收继续由父任务跟进。本次保持未发布状态。
