# Gitee 国内分发验证记录

## 本地受控证据

`npm run test:gitee-sync` 运行本地 HTTP adapter，不访问 Gitee。它验证同步器的协议与失败边界：数值 Release ID、form/multipart 编码、附件匿名下载和完整性校验、清单最后写入、429/5xx 有界重试，以及错误响应中的 Token 脱敏。

`cargo test --test update_release_baseline --no-default-features` 同样只使用本地 HTTP 服务，验证更新清单、Release URL 和附件读取的应用内闭环。这些结果不能作为 Gitee 平台已通过的证据。

## 真实平台证据

真实 Gitee 冒烟必须由维护者在公开分发仓库上通过 `gitee-smoke.yml` 执行，并人工打开无登录浏览器上下文中的 Release 页面。验收记录至少应包括：工作流 URL、匿名 Range 请求的真实状态（`206` 单字节，或平台忽略 Range 时的 `200` 完整响应）、完整下载的大小和 SHA-256、Raw 清单读取结果、Release 的数值 ID 与 tag，以及清理结果（或明确保留原因）。

初始代码实现阶段未持有 `GITEE_TOKEN`，当时没有真实平台证据；本地 adapter 结果仍不得表述为真实 Gitee 冒烟完成。后续真实平台结果单独记录如下。

## 2026-08-28 真实平台冒烟

GitHub Actions 工作流 [33138532094](https://github.com/yinshaohua/GPTEasy/actions/runs/33138532094) 在提交 `10fcd5a867f26d2be2b18563e8a8fd0e5e10016e` 上通过，公开分发仓库为 `ericshaohua/gpteasy-releases`，默认分支为 `main`。自动证据如下：

- `tag`：`smoke-33138532094-1`；数值 `releaseId`：`926420`；
- 匿名 Range 请求：Gitee 忽略 Range 并返回 HTTP 200，响应为完整的 4,194,304 字节附件，完整性校验通过；
- 独立匿名完整下载：4,194,304 字节，SHA-256 `bb9f8df61474d25e71fa00722318cd387396ca1736605e1248821cc0de3d3af8`；
- 匿名附件：`https://gitee.com/ericshaohua/gpteasy-releases/releases/download/smoke-33138532094-1/gpteasy-smoke-33138532094-1.txt`；
- 匿名 Raw 清单：`https://gitee.com/ericshaohua/gpteasy-releases/raw/main/smoke/smoke-33138532094-1.md`；
- 报告明确记录 `formalManifestAdvanced: false`，公开仓库中 `latest.md` 仍为 HTTP 404，未推进正式版本。

维护者随后在无登录浏览器上下文中打开上述 Gitee Release 页面并点击附件，附件可以直接下载，没有图形验证，也没有要求注册或登录；人工匿名点击通过。

维护者使用精确 ID 和 tag 运行清理脚本后，匿名复核确认 `926296`、`926354`、`926420` 三个 Release 均返回 HTTP 404，对应的三个 smoke 清单均返回 Gitee 表示文件不存在的空数组，非正式资源清理完成。

真实冒烟、人工匿名点击、清理和自动门禁全部完成后，GitHub Actions 中的 `GITCODE_TOKEN`、`GITCODE_REPOSITORY` 和 `GITCODE_DEFAULT_BRANCH` 已删除，只保留活动的 Gitee Secret/Variables。GitCode 站点 Token 的维护者撤销结果仍须单独确认；历史 GitCode 仓库、Tag、Release 和附件保持不变。
