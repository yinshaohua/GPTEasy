# 采用 Tauri 2、Rust 和 TypeScript/React

GPTEasy 采用 Tauri 2 作为 Windows 桌面壳，Rust 是供应商规则、网络验证、SQLite、Codex 配置和进程检测的唯一权威后端，TypeScript/React 只负责界面和页面临时表单状态。前端不得直接访问文件、数据库、供应商网络或进程；Tauri command 按“验证供应商”“保存并应用”“切换供应商”等完整用例设计，不暴露任意 SQL、文件或 Shell 能力。

重建初始化时选择当时稳定且相互兼容的具体版本，并通过普通 Cargo/npm lockfile 固定。旧计划中的 patch 版本、npm 来源人工批准和长期 package allowlist 不构成新实现约束。
