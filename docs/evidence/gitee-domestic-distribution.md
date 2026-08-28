# Gitee 国内分发验证记录

## 本地受控证据

`npm run test:gitee-sync` 运行本地 HTTP adapter，不访问 Gitee。它验证同步器的协议与失败边界：数值 Release ID、form/multipart 编码、附件匿名下载和完整性校验、清单最后写入、429/5xx 有界重试，以及错误响应中的 Token 脱敏。

`cargo test --test update_release_baseline --no-default-features` 同样只使用本地 HTTP 服务，验证更新清单、Release URL 和附件读取的应用内闭环。这些结果不能作为 Gitee 平台已通过的证据。

## 真实平台证据

真实 Gitee 冒烟必须由维护者在公开分发仓库上通过 `gitee-smoke.yml` 执行，并人工打开无登录浏览器上下文中的 Release 页面。验收记录至少应包括：工作流 URL、匿名 Range 请求的真实状态（`206` 单字节，或平台忽略 Range 时的 `200` 完整响应）、完整下载的大小和 SHA-256、Raw 清单读取结果、Release 的数值 ID 与 tag，以及清理结果（或明确保留原因）。

本次代码变更未持有 `GITEE_TOKEN`，未创建远程 Release、未写入 `latest.md`，因此尚无真实平台通过记录，也不应将本地 adapter 结果表述为真实 Gitee 冒烟完成。
