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

真实冒烟、人工匿名点击、清理和自动门禁全部完成后，GitHub Actions 中的 `GITCODE_TOKEN`、`GITCODE_REPOSITORY` 和 `GITCODE_DEFAULT_BRANCH` 已删除，只保留活动的 Gitee Secret/Variables。维护者随后在 GitCode 设置页撤销了原 GitHub Actions 使用的旧 Token；历史 GitCode 仓库、Tag、Release 和附件保持不变。

## 2026-08-29 v1.4.1 分发事故

GitHub Release `v1.4.1` 于 2026-08-29 14:32:34 UTC 发布。自动同步 [33257879357](https://github.com/yinshaohua/GPTEasy/actions/runs/33257879357) 和人工重跑 [33258808503](https://github.com/yinshaohua/GPTEasy/actions/runs/33258808503) 都在第一个 `GPTEasy_1.4.1_x64-setup.exe` 附件上传阶段失败：每次请求等待 180 秒后以 `curl` 退出码 28 结束，三次尝试均未收到 HTTP 响应字节。公开复核确认 Gitee `v1.4.1` 只有平台自动生成的源码压缩包，安装包和 `.sig` URL 返回 404，`latest.md` 仍为 1.4.0。

此前真实冒烟只上传 4,194,304 字节 `.txt`，证明了体积、Release API 和匿名下载，却没有覆盖 `.exe` 文件名、PE 内容及其平台审查路径；这是自动门禁未能在发布前发现问题的直接原因。仓库同时使用了可兼容读取的 `api.gitee.com/api/v5`，而当前官方 Swagger 声明的主机和 base path 组合为 `gitee.com/api/v5`。无法从公开日志证明 Gitee 内部为何对 PE 上传保持连接无响应，因此不把某个未公开的 WAF 规则写成既定根因。

修复将 API 根地址收敛到官方声明入口，固定附件传输参数，在响应丢失后重新枚举附件，并把真实稳定版 PE 作为冒烟输入。该记录只证明旧链路失败和本地修复完成；在新的 `gitee-smoke.yml` 真实运行成功、完成无登录下载验收之前，不得宣称 Gitee 的 PE 上传问题已经由平台验证解决。
