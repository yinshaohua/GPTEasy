# 单一 GitCode 更新信任根与清单最后推进

GPTEasy 的 Windows x64 应用更新只内置一个 GitCode Raw HTTPS 清单端点和一把 Tauri updater 公钥。GitHub 继续作为源码、Tag、版本、构建产物和中文发布说明的唯一权威来源；GitCode 分发仓库只保存下载说明、JSON 正文的 `latest.txt` 和不可变 Release 附件，不镜像源码，也不执行独立构建。GitCode 的稳定分支 Raw 服务对 `.json` 路径返回“暂不支持预览”，因此传输文件使用 `.txt` 扩展名；Tauri updater 解析正文，不依赖 URL 扩展名。

同一 Windows NSIS 安装包在本机 `main` 干净工作树上构建并完成门禁和 UAT 后，才允许复制到 GitHub 与 GitCode。GitCode 发布必须先完成附件上传，再从匿名上下文验证大小、SHA-256、updater 签名材料和 Raw 可读性；正式 `latest.txt` 是最后一个可见写入，之前任一步失败都保留旧清单。

updater 私钥在仓库外带密码生成，并保留至少一份可读取的离线加密备份。应用、源码、日志、诊断和发布附件只能包含公开端点与公钥。GitCode Token 只从 GitHub Actions secret `GITCODE_TOKEN` 注入，公开仓库坐标来自 Actions variable；Token 不进入查询参数、应用配置或附件。

这个决定减少了陈旧多端点选择、同版本二次构建和半成品清单的风险，代价是 GitCode 清单服务故障时客户端不自动回退到 GitHub，以及单公钥丢失或泄露需要新的手工自举版本。本阶段不实现客户端更新界面、公钥轮换或正式发布同步。
