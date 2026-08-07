---
status: superseded by ADR-0011
---

# 将桌面 Codex 和本机 CLI 视为同一原生环境

本 ADR 记录旧范围曾把 Windows/macOS 桌面宿主与 CLI 建模为“原生 Codex 环境”。ADR-0011 已用更准确的边界取代它：核心受管对象是当前 Windows 用户默认 Codex 环境，桌面 Codex 和 Codex CLI 只是读取该环境的消费者；macOS 和 WSL2 由 ADR-0010 延期。
