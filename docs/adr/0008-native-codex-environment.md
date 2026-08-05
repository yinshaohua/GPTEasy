# 将桌面 Codex 和本机 CLI 视为同一原生环境

Windows/macOS 当前用户的统一 ChatGPT 桌面应用 Codex 功能与本机原生 Codex CLI 可能共享用户级配置，因此 GPTEasy 将它们建模为同一个原生 Codex 环境。托盘切换会影响之后启动的两类进程，重启检测也覆盖两者；GPTEasy 不承诺只修改桌面应用，也不会在桌面应用或 CLI 外部重写配置后持续争夺配置文件，而是按外部配置规则展示实际状态。WSL2 环境继续独立切换。
