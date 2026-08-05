# 采用 Tauri 2、Rust 和 TypeScript/React

GPTEasy 需要同时提供 Windows/macOS 托盘界面、跨平台配置文件处理、宿主应用进程管理、WSL2 集成和 Linux 脚本导出。首版采用 Tauri 2 作为桌面壳，Rust 负责系统与领域能力，TypeScript/React 负责界面，以在原生系统集成、跨平台能力和 UI 开发效率之间取得平衡。
