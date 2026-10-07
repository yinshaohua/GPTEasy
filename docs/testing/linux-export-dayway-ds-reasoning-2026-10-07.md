# Linux 导出 DayWay-DS 思考强度缺失回归

- 日期：2026-10-07。
- 状态：源码修复与自动回归完成，未提交、未发布。
- 关联决策：[ADR-0051](../adr/0051-reasoning-effort-and-selector-range.md)。

## 现象与原因

用户已确认 Ubuntu 上通过导出脚本切换供应商后 Codex 可以启动并选择模型。
普通 DayWay 可选择思考强度，DayWay-DS 的 DeepSeek 模型没有思考强度选项。

桌面应用供应商时已对 Codex 报告为 `not_found` 的精确模型应用 DayWay-DS
兼容规则。Linux 导出从数据库读取能力快照后漏用了该规则，把原始
`not_found` 渲染成空的 `supported_reasoning_levels`。模型列表因此正常，
思考强度列表却为空；根级 `model_reasoning_effort = "high"` 不能补出选择器档位。

## 修复契约

共享供应商读取路径在验证模型目录指纹与完整快照后，应用既有版本化兼容规则。
导出模型目录与导出审计读取同一结果；只修改内存中的读取副本，不改写数据库的
原始探测证据。Bash/Zsh 无需调用 Python、Node.js 或在线能力探测。

DayWay-DS 规则 `dayway-ds-deepseek-effort-v1` 只处理精确模型
`deepseek-v4-flash`、`deepseek-v4-pro`，且仅在 Codex 明确报告 `not_found`
时补充 `low / medium / high`，默认值为 `high`。

已知非空、已知空、冲突和探测失败不会被覆盖；缺失、损坏、不完整或指纹不匹配的
快照继续保持未知。普通 DayWay、预览模型以及已有 OpenAI 原生能力保持原样。

## 自动证据

```powershell
$env:GPTEASY_REQUIRE_SHELL_MATRIX='1'
cargo test --manifest-path src-tauri/Cargo.toml --test linux_export -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib provider:: -- --nocapture
git diff --check
```

在 Windows 主机经 WSL Ubuntu、Bash 5.2.21 / Zsh 5.9 执行：

- Linux 导出完整测试：19 passed，0 failed，0 ignored。
- 供应商模块单元测试：33 passed，0 failed，0 ignored。
- `git diff --check` 通过；Git 对既有 `wsl_guest_harness.rs` 提示 CRLF 将转为 LF。

通过一次受控差分验证新增测试能够捕获本缺陷：仅临时禁用读取路径中的 DayWay-DS
兼容规则时，定向测试以 `default_reasoning_level: null` 而非 `high` 失败；恢复规则
后同一测试通过。临时禁用已恢复，最终完整矩阵与供应商单元测试均通过。

新增回归覆盖实际数据库读取 → 导出脚本 → source → 选择供应商 → 写入模型目录：

- `dayway_ds_linux_export_applies_exact_deepseek_compatibility_profile`：两种 shell
  写入的目录均与导出内容一致，两个精确 DeepSeek 模型有三档；普通 DayWay
  不获得 DeepSeek 兼容档位，已有 OpenAI 档位包含 `xhigh`，数据库证据不变。
- `dayway_ds_export_preserves_known_capabilities_and_unknown_evidence`：覆盖已知空、
  已知非空、冲突、探测失败、缺失、损坏、部分快照和指纹不匹配。
- `export_audit_uses_the_same_vendor_fallback_as_the_exported_catalog`：审计和目录
  一致，能力来源为 `vendor_compatibility`，规则 ID 与 `vendor_fallback=enabled`
  可追踪；无快照时为 unknown、disabled。日志不含供应商 ID、名称、模型、地址或 Key。

既有恢复点测试在两次并行完整运行中曾分别于 Bash、Zsh 返回非零，未输出具体断言；
该测试单独运行与其它完整重跑均通过。临时脱敏阶段探针未再复现失败，已移除；
未修改恢复点实现，偶发失败原因尚未确定，不记录为已修复。

## 本机真实 Codex 离线证据与边界

本次诊断在 WSL Ubuntu 的 Codex CLI 0.160.1 上，用临时配置与 Bash/Zsh
导出目录执行离线 `model/list` 检查。两个目录均读出 `low / medium / high`，
配置与目录文件检查前后保持不变。探针使用隔离 CODEX_HOME 和回环离线端点，
没有发送真实供应商请求，也没有把诊断探针集成到公共 shell 测试参数中。

上述证据验证目录被真实 CLI 读取，不代表远端服务器的交互选择器或真实
DayWay-DS Responses 请求已通过。用户服务器上的完整交互 UAT 尚未执行。

## 使用修复

使用包含修复的 GPTEasy 构建重新导出 Linux 脚本，将新脚本复制到 Ubuntu，
重新 source 并通过 `gpteasy` 选择 DayWay-DS，随后重新启动 Codex。
已有旧脚本不会自动更新；仅重新 source 旧脚本不会获得修复。
