# Linux 离线供应商脚本

从 GPTEasy 导出 Bash 或 Zsh 脚本后，把文件放到目标 Linux 用户可信的位置。文件包含全部已验证供应商的 API Key，须保护权限。导出固定保存完整模型发现集合，使用时无需联网更新目录或安装 Python、Node、jq、GPTEasy。目标环境使用 Bash 4+ 或 Zsh 5+，以及 GNU/Linux 常用文件工具和 timeout。

`source` 只定义函数与常量，不修改配置、不探测 Codex、不访问网络。加载后运行 `gpteasy` 明确选择供应商，或直接执行导出脚本进入相同菜单。`gpteasy current` 查看状态，`gpteasy info` 显示目标，`gpteasy restore` 经确认恢复最近一次切换，`gpteasy unlock` 处理经核验已失效的 shell 锁。`codex-full` 保持原有参数透传行为，不隐式刷新服务。

切换前需安装支持完整目录 schema 的原生 Linux Codex。脚本选择 PATH 中的外部入口，拒绝 Windows 可执行入口，并在私有临时 home 运行限时的 App Server model/list 探针。高版本号或有 daemon 子命令都不能代替目录兼容证据；不兼容时保留旧配置，不自动升级。没有后台服务管理接口的兼容 CLI 仍能切换。

目标由当前操作用户及 `CODEX_HOME` 确定，默认是 `$HOME/.codex`；自定义值必须为 Linux 绝对路径，不含控制字符。模型目录固定保存在该 home 的 `.gpteasy-shell/model-catalogs/<导出 UUID>/<工件 UUID>.json`，不随工作目录或 config.toml 的 symlink 目标改变。目录为私有不可变工件、不含 API Key；脚本拒绝不安全的工件 symlink/hardlink、所有者或权限。已有 home/config 的宽权限仍会提示。

脚本前部的 Tab 分隔供应商表允许单独维护显示名称。默认模型、模型集合、地址或 API Key 变化后须在 GPTEasy 重新验证并重新导出；只改表格不能同步冻结载荷与绑定，明确切换会拒绝。导出时缺失、损坏、空集合或绑定失效的验证快照也会停止，不仅导出默认模型。

一次切换在共享锁内保存完整旧状态，先提交 JSON 目录，再原子替换包含唯一根级 model_catalog_json 的完整最小配置并复读。按 ADR-0048，这会替换旧配置中的其它自定义字段，原内容保存在恢复点，最多保留五个。配置替换前失败保留旧内容；目录提交后配置未提交时可留下完整孤立目录；配置提交后的同步或复读失败保留新现场及恢复点供检查。

restore 可恢复另一份导出的旧供应商，即使它不在当前导出表内。旧受管目录必须仍存在且摘要、身份、权限有效；确认期间恢复点、配置或目录发生变化会停止。原配置没有目录引用或原来没有配置时按原状恢复，不强补目录。外部目录不接管、不覆写、不删除；首版也不自动清理受管模型目录。

成功切换、恢复以及凭据清理警告都会说明：**配置已保存，但 CLI/共享后台服务可能仍使用旧配置。** 按提示的 uid 和 CODEX_HOME，在该用户环境下依照所安装原生 CLI 的管理说明人工刷新。磁盘写入成功不证明运行实例已刷新；脚本不提供未经核验的裸入口、sudo、update 或重启命令。

问题诊断的 stderr 只增加固定 stage/catalog_state 标签，用于区分能力探针、恢复点、目录提交、配置提交与复读；这些日志不携带 API Key、地址、配置正文或模型 ID。实现与本次验证边界见 [#69 归档](archive/issue-69-linux-offline-catalog.md)。
