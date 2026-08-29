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

## 2026-08-30 v1.4.1 恢复验证

使用同一份 3,884,346 字节 PE 安装包做受控后缀实验：

- [33265385697](https://github.com/yinshaohua/GPTEasy/actions/runs/33265385697) 使用 `.exe`，上传等待 180,002 毫秒后收到 0 字节并以 `curl` 退出码 28 失败；
- [33265862278](https://github.com/yinshaohua/GPTEasy/actions/runs/33265862278) 延长单次预算后仍使用 `.exe`，连接被对端重置并以 `curl` 退出码 35 失败；
- [33265656678](https://github.com/yinshaohua/GPTEasy/actions/runs/33265656678) 只把同一 PE 的附件后缀改为 `.bin` 后通过；`tag` 为 `smoke-33265656678-1`，数值 `releaseId` 为 `970921`。Gitee 忽略 Range 并返回 HTTP 200 完整内容，匿名完整下载大小为 3,884,346 字节，SHA-256 为 `40c6b7f1dee993fa93c3eaaa8dbb1b633e91fd17cacffa3a538e161ec96ff6e2`，测试清单明确记录 `formalManifestAdvanced: false`。

这组实验把失败条件收敛到 Gitee Release API 自动上传通道接收 `.exe` 文件名时的行为；它不推断平台未公开的内部 WAF 或审查实现。正式分发因此保持 GitHub `.exe` 不变，只在 Gitee 自动同步时为同一不可变字节使用 `.exe.bin`。应用更新器不依赖 URL 后缀，仍使用原 `.sig` 验证下载字节并写入临时 `.exe`；手工下载用户须删除末尾 `.bin`。

正式恢复同步 [33266143156](https://github.com/yinshaohua/GPTEasy/actions/runs/33266143156) 首次把 `GPTEasy_1.4.1_x64-setup.exe.bin`、`GPTEasy_1.4.1_x64-setup.exe.sig` 和 `SHA256SUMS.txt` 全部写入 Gitee，并最后把 `latest.md` 推进到 1.4.1。完善 Release 下载提示和 README 同步后，[33266943845](https://github.com/yinshaohua/GPTEasy/actions/runs/33266943845) 对同一 Tag 幂等重跑通过；[33267292642](https://github.com/yinshaohua/GPTEasy/actions/runs/33267292642) 又覆盖了 README 已一致时的不可变 Raw 校验。工作流升级到 `actions/checkout@v5` 后，最终复核 [33267454797](https://github.com/yinshaohua/GPTEasy/actions/runs/33267454797) 通过且不再依赖 Node 20 弃用兼容。重跑同时证明，Gitee 既有附件枚举不提供附件 ID 时仍可使用数值 Release ID 和稳定附件 URL 完成校验。最终公开结果如下：

- 无 Cookie、无 Token 的安装包 GET 返回 HTTP 200，大小 3,884,346 字节，SHA-256 为 `40c6b7f1dee993fa93c3eaaa8dbb1b633e91fd17cacffa3a538e161ec96ff6e2`，与 GitHub Release 一致；
- `latest.md` 匿名 GET 返回 HTTP 200，`version` 为 `1.4.1`，安装包 URL 指向 `.exe.bin`；
- Gitee Release 页面显示三个正式附件和“Gitee 下载说明”；登录态 Chrome 中点击 `.exe.bin` 后直接进入 `foruda.gitee.com` 附件地址，没有出现图形验证或额外登录提示；
- 当前浏览器会话已有 Gitee 登录态，隔离的应用内浏览器不可用，因此本次没有把页面点击记录为“无登录浏览器通过”。当前附件的匿名 HTTP 完整下载已通过；2026-08-28 的独立无登录浏览器点击仍证明同一公开仓库的 Release 附件页面不强制登录。

三个实验 Release 及清单均已按精确 ID/tag 清理：[33266705774](https://github.com/yinshaohua/GPTEasy/actions/runs/33266705774) 清理 `970815` / `smoke-33265385697-1`，[33266857109](https://github.com/yinshaohua/GPTEasy/actions/runs/33266857109) 清理 `970921` / `smoke-33265656678-1`，[33266706218](https://github.com/yinshaohua/GPTEasy/actions/runs/33266706218) 清理 `970997` / `smoke-33265862278-1`。无凭据复核三个 Release API 和三个 `smoke/*.md` Raw 地址均返回 HTTP 404。

收尾过程中的失败重跑也形成了新的回归边界：[33266705083](https://github.com/yinshaohua/GPTEasy/actions/runs/33266705083) 暴露 Gitee Raw 写后缓存，[33267157559](https://github.com/yinshaohua/GPTEasy/actions/runs/33267157559) 进一步证明查询参数不能保证分支 Raw 后端立即推进；同步器现用 Contents API 的 Base64 内容判断是否需要写入，并以提交 SHA 的不可变匿名 Raw 地址完成最终核对。[33266857112](https://github.com/yinshaohua/GPTEasy/actions/runs/33266857112) 暴露既有附件没有附件 ID，现只要求协议实际需要的数值 Release ID。这些边界均已加入本地 HTTP adapter 回归测试。

## 2026-08-30 网页手工上传对照

维护者在 Gitee 网页中创建临时 Release `manual-exe-upload-20260830`，直接选择原始 `GPTEasy_1.4.1_x64-setup.exe` 后上传成功。公开 API 返回数值 `releaseId` `972559` 和稳定 `.exe` 下载 URL；无 Cookie、无 Token 的完整 GET 返回 HTTP 200，媒体类型为 `application/vnd.microsoft.portable-executable`，大小 3,884,346 字节，SHA-256 为 `40c6b7f1dee993fa93c3eaaa8dbb1b633e91fd17cacffa3a538e161ec96ff6e2`。这证明 Gitee 网页上传和公开下载支持 `.exe`，自动失败边界只适用于本次实测的 Release API 上传通道，不能表述为 Gitee 全平台拒绝 `.exe`。

该临时 Release 创建时实际为 `prerelease=false`，曾使 `/releases/latest` 指向实验 Tag。维护者完成验证后删除 Release 和 Tag；匿名复核 Release API、Release 页面和 Tag API 均返回 HTTP 404，`/releases/latest` 已以 HTTP 302 恢复指向 `v1.4.1`。网页手工上传需要维护者逐次操作，不能满足无人值守正式同步；该对照因此促成“网页上传标准 `.exe`、自动同步其它附件并完成最终验证”的两阶段发布协议。

## 2026-08-30 v1.4.1 标准 EXE 最终迁移

维护者在正式 Gitee `v1.4.1`（数值 `releaseId` `968231`）编辑页删除遗留 `.exe.bin` 和旧 `SHA256SUMS.txt`，保留原 `.sig`，并上传 GitHub Release 中的权威 `GPTEasy_1.4.1_x64-setup.exe`。上传后先以无 Cookie、无 Token 的完整 GET 独立验证 PE、大小和 SHA-256，再触发两阶段协议的自动收尾。

GitHub Actions 工作流 [33269670373](https://github.com/yinshaohua/GPTEasy/actions/runs/33269670373) 在提交 `71424822de3f480a077acf572d0672d77902f4e6` 上通过。同步器复用并匿名校验网页上传的 `.exe` 与既有 `.sig`，自动补回引用标准文件名的 `SHA256SUMS.txt`，移除 Release 正文中的旧 `.exe.bin` 下载提示，并最后更新 `latest.md`。最终公开结果如下：

- `GPTEasy_1.4.1_x64-setup.exe` 为 3,884,346 字节，SHA-256 `40c6b7f1dee993fa93c3eaaa8dbb1b633e91fd17cacffa3a538e161ec96ff6e2`，与 GitHub Release 一致；
- `.sig` 为 416 字节，SHA-256 `b960779f71a28ed48b88097387ffd836cfb0a12cf6ba53dd1dcdb1d33b82a0b2`，与 GitHub Release 一致；
- `SHA256SUMS.txt` 只引用标准 `.exe` 和 `.exe.sig` 文件名，不再包含 `.exe.bin`；
- `latest.md` 匿名 GET 返回版本 `1.4.1`，`windows-x86_64.url` 指向标准 `.exe`，并记录相同大小、SHA-256 和 updater 签名；
- `scripts/test-updater-manifest.ps1` 对公开清单通过，仓库内 `verify_updater_signature` 使用应用内置公钥对公开 `.exe` 与 `.sig` 验证返回 `passed`；
- Gitee Release 正文已恢复为 GitHub 原始中文发布说明，Gitee Raw `README.md` 已说明用户可以直接下载和运行 `.exe`。

这组结果验证了正式清单获取、安装包公开下载和 updater 签名链路。客户端仍按既有安全边界在下载验签后等待用户明确确认安装。维护者随后确认基于最终 Gitee 清单的真实客户端自动升级测试正常，补足了此前未执行的交互式升级验证。
