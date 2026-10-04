# Issue #74：Windows 共享后台服务检测证据

## 交付范围

2026-10-04 在 src-tauri/src/shared_daemon.rs 增加独立的 Codex 共享后台服务只读检测模块，并在 src-tauri/src/lib.rs 导出。它不复用 consumer::ConsumerRole，也不写入状态、不提供 Tauri command、不启动、终止、重启、升级或安装服务。

检测器先枚举并去重所有候选入口，再只接受已核验的绝对 native 入口：

- 桌面附带入口被识别为 desktop_bundled 并排除；
- npm codex.cmd 只解析到已存在的架构匹配 native binary，不执行 shim 或用户 shell；
- 当前用户默认 ~/.codex/packages/app-server-daemon 下的 release binary 及受控 standalone 入口可作为独立候选；
- 不调用 where.exe，不接受前端传入路径、PID、命令、socket 或替代参数。

固定只读探针使用绝对入口和 app-server daemon status --json，stdin 为空，stderr 不进入结果，超时为未知。探针成功后仍要求管理报告与实际 Windows 进程的 PID、创建时间、规范化可执行路径、当前用户、默认 Codex home、pid backend 和 socket/control 归属全部一致；精确命中 GPTEasy 自有进程、桌面 bundled、交互 CLI、其他 App Server、PID 复用或创建时间不符时不会认领。

公开 SharedDaemonSnapshot 只返回入口来源、状态、能力、版本可见性、环境匹配和脱敏身份类别；不序列化完整路径、命令行、原始 stdout/stderr、地址、凭据、配置正文或模型 ID。缺入口、缺字段、未知 backend、损坏 JSON、超时和身份矛盾统一保持 unknown 或 unsupported，只有明确的只读停止探针才返回 stopped。

## 自动证据

src-tauri/tests/shared_daemon_detection.rs 的运行时 fixture 覆盖：

- 多安装遮蔽及候选继续枚举；
- npm shim、桌面 bundled、交互 CLI、其他 App Server 和 GPTEasy 自有服务排除；
- PID 复用、创建时间不符、其他用户和自定义 home；
- 缺字段、未知 backend、未知/损坏/超时类探针结果；
- 明确停止与未知状态不混淆；
- 对外快照的路径和 socket 归属脱敏。

本地自动测试已执行：

    cargo test --manifest-path src-tauri/Cargo.toml --test shared_daemon_detection
    13 passed; 0 failed

真实 Windows 安装矩阵、真实 Codex 共享 daemon 的 status JSON 合同和真实菜单刷新尚未在本次开发环境中执行，因此不能把自动 fixture 结果宣称为 Windows UAT 或真实菜单验收通过。后续 #75/#76 可在此检测契约之上接入持久化协调和用户确认控制，但必须保持本 Issue 的只读边界。
