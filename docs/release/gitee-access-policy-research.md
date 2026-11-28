# Gitee Raw 与 Release 附件匿名访问政策：官方资料核查

核查日期：2026-11-27（北京时间）。将“近期”界定为 2025-11-27 至 2026-11-27。范围限于 Gitee.com 托管仓库的 Raw 文件读取、Release 上传附件下载，以及 Gitee 对这两类访问的公开说明。本文的“匿名”指请求不携带登录态或访问令牌。

## 结论

在本次能检索到的 Gitee 官方公开资料中，**没有找到上述时段内明确宣布变更 Raw 文件读取或 Release 附件匿名下载策略的公告**。这是检索结果，不是政策未变的证明：不能据此排除未公开、未被索引或尚未同步到文档的调整，也不能据此判定任何具体端点当前的行为。[1–9]

## Raw：官方文档明确列出认证条件

- Gitee 帮助中心《获取文件原始数据》列出两种需要认证的情况：**非公开仓库**的文件原始数据，以及**公开仓库中原始数据大于 10 MB** 的文件。该节给出两种认证方式：登录后用浏览器访问或下载，或使用私人令牌下载。[1]
- 同一页说明，从 `gitee.com` 访问无需认证的 Raw 数据会重定向至 `raw.giteeusercontent.com`，并列出独立域名的访问形式。因此按文档口径，公开仓库不大于 10 MB 的文件不在所列认证情形内；独立域名重定向也不是登录重定向。这是对文档的解读，不是对实际可达性的保证。[1]
- 该页未标明上述条件的生效日期或修订历史，不能用它确定规则何时上线、是否在近期调整。10 MB 是该文档所述的 Raw 认证门槛，不是 Release 附件的规则。[1]

## Release 附件：官方文档说明可下载，未说明是否需登录

- 《创建 Release（发行版）》称可将软件、发行说明和附件打包，“以供他人下载”；《什么是 Release（发行版）？》称打包二进制文件可方便用户下载。这些页面未明确说明公开仓库的 Release 附件可否匿名下载，也未写明下载者必须登录。创建页对协作者和写权限的说明针对创建、编辑和删除，不是下载权限；《更新和删除 Release（发行版）》也未说明下载权限。[2–3, 9]
- **2022-08-18**：Gitee 官方博客宣布优化附件下载地址，列出 `/releases/download/{git_tag}/{attach_file_path}`，并另列源码归档路由。文章说明的是路由变化，没有说明是否需要登录；附件下载与源码归档不能混为一谈。[4]
- **2026-01-23**：Gitee 官方博客发布 Release 插件上线消息，举例称贡献者可直接下载最新版本，但没有说明下载时是否需要登录，也没有宣布变更匿名下载策略。不能把“直接下载”解读为“免登录下载”。[5]

## 检索范围与限制

沿 Gitee 帮助中心的文件操作和发行版目录查阅文档，并在 Gitee 官方博客、帮助中心及 Gitee 站内公开索引中交叉检索 `Raw`、`原始数据`、`raw.giteeusercontent.com`、`Release`、`发行版附件`，与 `匿名`、`未登录`、`登录后`、`认证`、`公告`、`变更` 等词组合。旧帮助中心的年度更新索引只列到 2022 年；新版帮助中心首页的“查看更多产品动态”链接核查时返回 404；企业版更新日志不是社区版公告的完整记录。本次没有找到可用于逐条排除近期变更公告的官方完整归档。已直接核对两篇官方博客原文。[4–8]

仅采用 Gitee 自行发布的材料作为证据；不把托管在 Gitee 上的用户帖子或其他项目的发行说明当成平台公告。官方资料结论不依赖本项目的端点实测；本项目的观测另见文末。发行页是否可浏览、源码归档是否可下载、OpenAPI 是否需认证，均不用于推断 Release 附件的匿名下载规则。

## 官方来源

1. Gitee 帮助中心：[获取文件原始数据](https://help.gitee.com/repository/file-operate/raw)，见“认证”“独立域名”。
2. Gitee 帮助中心：[创建 Release（发行版）](https://help.gitee.com/repository/release/create)。
3. Gitee 帮助中心：[什么是 Release（发行版）？](https://help.gitee.com/repository/release/what-is-release)。
4. Gitee 官方博客：[小功能有大用处，Gitee 这些新特性你都 get 了吗？](https://blog.gitee.com/2022/08/18/update/)，2022-08-18，见附件下载与源码归档路由小节。
5. Gitee 官方博客：[Gitee Go Release 插件上线：自动发版、上传构建产物一步到位](https://blog.gitee.com/2026/01/23/gitee-go-release-plugin-auto-deploy-artifact/)，2026-01-23，见“应用场景示例”。
6. [Gitee 官方博客](https://blog.gitee.com/)，公开文章检索入口。
7. [Gitee 旧帮助中心](https://gitee.com/help.com)的“更新日志”索引，以及[新版帮助中心](https://help.gitee.com/)的“产品动态”入口。
8. Gitee 帮助中心：[企业版产品更新日志](https://help.gitee.com/enterprise/changelogs/)，仅作为检索范围记录。
9. Gitee 帮助中心：[更新和删除 Release（发行版）](https://help.gitee.com/repository/release/update-and-delete)。

## GPTEasy 本机复测（2026-11-27 至 2026-11-28，北京时间）

- [当前客户端配置](../../src-tauri/tauri.conf.json)（以及 `v1.4.1`、`v1.5.2`、`v1.5.4`、`v1.5.5` 标签）内置同一 [Raw 清单](https://gitee.com/ericshaohua/gpteasy-releases/raw/main/latest.md)。客户端不先读取 Release 网页，也不单独下载 `.sig`；新版本优先调用同仓库公开 Contents API 读取 Base64 清单，失败时回退到原 Raw 地址。它检查清单，发现更高版本才从 `platforms.windows-x86_64.url` 下载，然后用内置公钥验证清单中提供的签名。参见 [更新器](../../src-tauri/src/update.rs)。
- 匿名 GET 清单的第一跳可达：`gitee.com` 返回 `302`，目标主机是 `raw.giteeusercontent.com`，URL 带平台签名参数；同仓库 `README.md` 和另一个公开仓库 Layui 的 Raw 文件也使用此跳转。没有把动态签名参数写入日志或本文。成功时 `latest.md` 返回 `200` 和纯文本形式的 JSON，版本是 `1.5.5`。
- 用同一 Rust/reqwest 更新器源码独立执行手动检查（不启动或安装桌面应用）。从 `v1.5.4` 标签提取的真实更新器执行 5 次直连，3 次得到原提示“暂时无法检查应用更新，请手动重试。”，另 2 次下载并验签通过、进入 `Pending`。当前 `v1.5.5` 更新器用 `1.5.4` 作为起始版本也复现同样比例：3/5 失败均为 `stage=manifest_request error=connection_reset os_error=10054`，未进入安装包下载；2/5 进入 `Pending`。同一更新器显式使用本机代理后的另一组也只成功 2/5；不能把使用代理当成稳定解法。
- 清单中的 [v1.5.5 安装包](https://gitee.com/ericshaohua/gpteasy-releases/releases/download/v1.5.5/GPTEasy_1.5.5_x64-setup.exe) 经站内附件 URL 再跳到 `foruda.gitee.com`。本机匿名直连和代理均有完整 HTTP `200` 下载记录，文件为有效 PE 起始标记，大小 `3,998,104` 字节，SHA-256 为 `2d0e9fe6a57de79ee6c91aa609e6e8eee2dc4ddb11c0ac8c8a063a2a856cc81b`，与 [正式同步记录](https://github.com/yinshaohua/GPTEasy/actions/runs/46025017406) 一致。同步成功不等于本机能读 Raw：当前脚本校验附件后通过 Contents API 写入清单，写入为最后一步，不以客户端网络上 Raw 的可用性作为通过条件（[同步器](../../scripts/sync-gitee-release.mjs)）。
- [匿名 Contents API 对照地址](https://gitee.com/api/v5/repos/ericshaohua/gpteasy-releases/contents/latest.md?ref=main) 本机直连可返回 `200`，解码 `content` 后为 `1.5.5` 清单。新版本客户端已将它作为优先清单读取路径；响应中的 Base64 内容解码后再按原清单格式解析，不信任 API 返回的 `download_url`。原 Raw 地址仍作为回退，且两条路径都保留原 HTTPS、下载地址和 updater 签名边界。

结论：在本机复现的失败发生在 Raw 清单的网络请求阶段；安装包并非普遍不可下载。Contents API 在本机连续探测中稳定返回 200，更新器改为 API 优先后，真实网络探针在 2026 年 11 月 28 日又连续 3 次完成 1.5.5 清单读取、安装包下载和签名验证；此前连续 8 次也全部成功。尚无法确认 Gitee 最近是否改变了匿名访问规则，但不再把 Raw/CDN 的瞬时连接重置作为唯一清单入口。`v1.5.5` 的有界后台重试仍保留，用于 API 和 Raw 两条读取路径的临时网络故障。
