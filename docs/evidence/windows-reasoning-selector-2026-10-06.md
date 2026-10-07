# Windows 思考深度选项缺失：本机差分诊断

日期：2026-10-06。范围：分析，不修改产品实现或用户配置，不发布。

## 本机事实

- `codex --version`：0.160.1。
- 正在运行的 managed daemon 所用可执行文件：0.160.0。
- 当前 `CODEX_HOME` 显式设置；当前模型 `gpt-6.1-sol`。
- 当前配置引用 `gpteasy-model-catalog.json`，目录包含 6 个模型。
- 当前配置根级 `model_reasoning_effort` 为字符串 `none`。
- 6 个目录条目的 `supported_reasoning_levels` 全部为空；当前条目的
  `default_reasoning_level` 为 null。

未记录凭据、真实服务地址、完整配置、用户任务或原始 stderr。

## 可重复反馈循环

诊断脚本：[`scripts/diagnose-reasoning-selector.py`](../../scripts/diagnose-reasoning-selector.py)。

读取当前配置时只提取模型、目录路径和根级 effort，不读取 auth.json。
复制模型目录到 `src-tauri/target/selector-diagnostics/` 下独立临时 CODEX_HOME，
使用当前主机真实 Codex 可执行文件启动独占 stdio App Server。
临时供应商固定为 `http://127.0.0.1:9/v1`，不携带凭据，只调用
`initialize` / `initialized` / `model/list`，不创建 thread，不发送推理请求。
每轮启动新进程，并在结束时停止自己的子进程和清理临时目录。

```powershell
$cli = 'C:\rd\nodejs\node_global\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe'
python scripts/diagnose-reasoning-selector.py --codex $cli --require-choices
python scripts/diagnose-reasoning-selector.py --codex $cli --matrix --verify-matrix
```

第一条约 1 秒，以退出码 1 产生断言：
`FAIL: current model exposes no selectable reasoning efforts`。
实际协议返回当前模型 `supportedReasoningEfforts: []`。
这不是根据 JSON 字段名猜测，而是调用真实消费者协议的可运行负向证据。

第二条验证离线对照，0.160.1 和 0.160.0 两个可执行文件均通过。

## 结果

两版结果完全一致，表中针对当前模型 `gpt-6.1-sol`：

| 对照 | 返回模型数量 | 返回思考深度列表 |
| --- | ---: | --- |
| 原目录、原根级 effort | 6 | 空 |
| 只改根级 effort 为 high | 6 | 空 |
| 只改目录默认值为 high | 6 | 空 |
| 只给当前条目加入四个合成测试档位 | 6 | low / medium / high / xhigh |
| 删除每个条目的 supported_reasoning_levels 字段 | 11 | low / medium / high / xhigh / max / ultra |
| 完全不指定 model_catalog_json | 11 | low / medium / high / xhigh / max / ultra |

合成档位只验证客户端消费链路，不证明供应商实际支持这些档位。

省略字段实验的模型 ID 集合、当前模型档位与完全不加载目录的结果相同；
因此它不是“只继承思考深度能力、继续加载原供应商目录”的有效方案。
两个实验额外引入了 5 个并非本次供应商目录中的模型：
`codex-auto-review`、`gpt-5.6-luna`、`gpt-6-luna`、
`gpt-daybreak-blue-latest`、`gpt-daybreak-red-latest`。
不能靠省略必需字段让用户偶然看到菜单。

用户 config.toml 和模型目录在每次探测前后均做 SHA-256 复核，未变化。
完整脱敏结果仅保存在 Git 忽略的 target 目录中。

## 定位与判断

`src-tauri/src/provider/model_catalog.rs` 的 `entry()` 不分已知/未知模型，
统一产生 null 默认值和空档位列表。Windows environment.rs、WSL wsl.rs 和
Linux 导出 linux_export.rs 均复用这一 renderer。

这会用“未知能力”覆盖 Codex 本来对精确同名模型已有的元数据；
菜单上游的 model/list 随之返回空的可选思考深度集合。
根级 effort 只影响请求生效值，不能补回被清空的模型目录范围。

全新隔离进程也稳定复现，故该缺陷不需要旧 daemon 缓存或未重启才能发生。
现有 daemon 是否另有缓存问题不在本次探测中判定。

## 已实施修复与剩余验收

1. 已保留供应商发现模型集合，不将 Codex 全部内置模型并入供应商菜单。
2. 已按可信 Codex 元数据精确匹配模型 ID，保留实际可选范围，不按名称伪造档位。
3. 已对未知模型保留未知能力状态，未按 gpt/deepseek 名称猜测。
4. Windows native 与 Linux 导出已接入能力快照渲染；WSL 继续隔离 native 快照。
5. 已补入能力探测和目录渲染测试；最终桌面选择器人工验收仍待执行。
6. 已保留默认生效值、能力来源、脱敏诊断及原子写入恢复保护。

ADR-0051 的能力与生效值分离原则仍成立，但“所有模型均写未知能力”的实施策略
不能满足保留已有 Codex 思考深度范围的目标，应补充实现和验收设计。

## 证据边界

已验证：本机两版真实 App Server 的模型列表协议和离线对照。
未验证：Windows 或 Linux 的交互式 TUI/桌面菜单逐项点击，真实供应商对六档的
服务端支持，DeepSeek 对应范围，以及真实推理请求是否按所选 effort 生效。
未重启现有 Codex/daemon，未修改真实配置，未生成新安装包。

官方配置说明只作为概念依据，具体选择器行为来自上述本机实验：
https://developers.openai.com/codex/config-reference/
