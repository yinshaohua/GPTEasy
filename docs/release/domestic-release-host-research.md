# 国内公开 Release 分发平台调研

调研日期：2026-08-28（Asia/Shanghai）

## 结论

如果目标是让中国大陆用户无需注册、无需登录即可下载 GPTEasy 的 Windows Release，当前已经实测可行的平台包括 **GitCode、Gitee、极狐 GitLab（JiHuLab）、AtomGit 和 GitLink**。但有一个重要反转：**GitCode 当前的公开 Release 附件本身已经可以匿名下载**；在无 Cookie、无 Token 的完整 `GET` 中，本项目 v1.3.0 安装包返回 200、长度和 SHA-256 均与 GitHub 一致。`HEAD` 返回 401，或者网页交互出现登录引导，不能代表 `GET` 下载也需要登录。

因此：

1. 若问题只是“GitCode 附件是否强制登录”，现有判断不成立，不必仅因此迁移。
2. 若仍希望更换国内分发源，**Gitee 是最接近现有 GitCode Release 模型、改造量较小的候选**。两个真实公开附件均在无凭据完整 `GET` 中返回文件；官方 OpenAPI 也提供 Release 附件上传、枚举和下载接口。
3. 若更看重确定性的制品路径和成熟 API，**极狐 GitLab 是与 GitCode 基础设施独立、证据最完整的候选**。建议采用“Generic Package 存二进制 + Release 链接到包文件 + 公开仓库 Raw 存更新清单”的组合，而不是把源码归档当作安装包分发。
4. AtomGit 的 Release 附件也已证实可匿名下载，且单附件上限为 2 GB；但实测最终仍落到 `file-cdn.gitcode.com`，不能视为与 GitCode 独立的故障域。
5. GitLink 已找到可匿名下载的真实 Release 附件，但缺少稳定的官方自动上传合同、文件上限和大文件行为证据；可做迁移 PoC，不宜直接采用。CODING 仍未完成匿名代码 Release 附件的闭环证明。

## 证据分级

- **已证实**：极狐 GitLab、Gitee、AtomGit、GitCode 和 GitLink 都能向未注册、未登录用户返回公开发布附件的实际字节。极狐使用 Generic Package 承载二进制并从 Release 链接，其余平台使用 Release 附件。
- **未证实**：CODING 尚未完成“真实公开二进制附件 + 无凭据 GET + 文件字节/哈希验证”的闭环；GitLink 的匿名下载能力已证实，但自动上传、文件上限和大文件支持仍未证实。不能依据公开仓库、源码归档或文档中的“公开”字样推定成立。
- **已否定的命题**：“GitCode 公开 Release 附件必须登录”已被本项目安装包的匿名完整下载反证；“AtomGit 能提供独立于 GitCode 的附件分发故障域”已被最终下载落到 `file-cdn.gitcode.com` 反证。

## 判定标准

“匿名可下载”必须同时满足：

- 请求不带 `Cookie`、`Authorization`、访问令牌或登录态；
- 测试对象是发布者上传的二进制附件或通用制品，不是 Git 自动生成的源码 `.zip`/`.tar.gz`；
- `GET` 或带 `Range` 的 `GET` 返回文件字节，未跳转登录页；
- 对 GPTEasy 还需具备可自动上传、可形成稳定 HTTPS 地址、可在发布后匿名校验大小与 SHA-256 的路径。

源码仓库公开、匿名 clone、Tag 源码归档可下载，只能证明代码公开，不能证明 Release 安装包可匿名分发。

## 结果矩阵

| 平台 | 公开 Release 二进制匿名下载 | 自动上传 | 对 GPTEasy 的结论 |
|------|------------------------------|----------|--------------------|
| Gitee | **已证实**：两个真实 Release 附件匿名完整 GET 返回 200、文件类型和长度正确 | 官方 multipart Release 附件 API | **最小改造迁移候选** |
| 极狐 GitLab | **已证实**：Generic Package 60,644,521 字节文件匿名 Range 返回 206；Release 页面/API 也匿名 200 | 官方 Package API、Release API，可用 Job Token/Project Token/PAT | **制品分发首选候选** |
| AtomGit | **已证实**：官方 `atomcode` 31,749,264 字节 Windows 附件匿名 Range 返回 206 | v5 Release API 与附件上传接口可自动化 | 可用，但与 GitCode 共用 CDN，独立性较弱 |
| GitCode | **已证实**：GPTEasy v1.3.0 安装包匿名完整 GET 200，哈希匹配 | 现有同步器已经跑通 | 当前无需因“登录墙”迁移；仍有 Raw/WAF 风险 |
| GitLink | **已证实**：真实 Release 附件匿名 GET 返回 200 和文件字节 | 未找到公开、稳定的官方自动上传合同 | 可做 PoC，暂不直接采用 |
| CODING | **未证实**：官方文档支持代码版本上传文件和“公开”制品库，但未证实代码 Release 附件匿名下载 | 代码版本/API 能力不清晰；Generic 制品有命令行路径 | 不作为 Git Release 首选 |

## 匿名探测记录

所有探测均在 2026-08-28 从全新 HTTP 会话执行，不发送 Cookie 或 Token。`Range: bytes=0-0` 用于避免下载大文件；成功的 206 表示服务端确实返回目标文件字节。

| 平台与样本 | 请求结果 | 判读 |
|------------|----------|------|
| GitCode：[GPTEasy v1.3.0 安装包](https://gitcode.com/ericyin99/GPTEasy-Releases/releases/download/v1.3.0/GPTEasy_1.3.0_x64-setup.exe) | 稳定 URL 302 到带短期 `auth_key` 的 `file-cdn.gitcode.com`；Range 最终 206，`Content-Range: bytes 0-0/3757840`；完整 GET 200，3,757,840 字节，SHA-256 `91bf42174224f199242974b8dba5b5781d3b8b1648eeb88afea9e6fd92e90fd4` | **匿名附件下载成立**。应把稳定 Release URL写入清单，不保存临时 CDN URL |
| GitCode：[Release API](https://api.gitcode.com/api/v5/repos/ericyin99/GPTEasy-Releases/releases/v1.3.0) | 匿名 200，返回 `type: attach` 的安装包、签名和校验文件 | API 可匿名发现附件 |
| GitCode：[latest.md Raw](https://raw.gitcode.com/ericyin99/GPTEasy-Releases/raw/main/latest.md) | 本次匿名 200；仓库既有记录显示同一路径曾被 WAF 返回 403 | 可用但并非始终稳定，仍需保留 Contents API/Base64 回退设计 |
| AtomGit：[atomcode v5.0.9 Windows 附件](https://atomgit.com/atomgit_atomcode/atomcode/releases/download/v5.0.9/atomcode-v5.0.9-windows-x64.exe) | 302 到 `file-cdn.gitcode.com`，最终 206，`Content-Range: bytes 0-0/31749264` | **匿名附件下载成立**；同时证明它与 GitCode 共用附件 CDN |
| AtomGit：[官方项目 Release API](https://api.atomgit.com/api/v5/repos/atomgit_atomcode/atomcode/releases/v5.0.9) | 匿名 200，返回 13 个 `type: attach` 附件，包括 `.exe` 和 `latest.json` | API 可匿名发现附件 |
| Gitee：[easyAi v1.6.9 附件](https://gitee.com/dromara/easyAi/releases/download/v1.6.9/easyAi-1.6.9.jar) | 稳定 URL 302 到 `/attach_files/2988302/download/...`，再 302 到带短期参数的 `foruda.gitee.com`；匿名完整 GET 最终 200，`application/jar`，490,300 字节 | **匿名附件下载成立**；清单应保存第一段稳定 Release URL |
| Gitee：[Layui v2.13.5 附件](https://gitee.com/layui/layui/releases/download/v2.13.5/layui-v2.13.5.zip) | 同样经两次 302，匿名完整 GET 最终 200，`application/zip`，401,828 字节 | 第二个独立仓库样本复核结论，未出现登录跳转 |
| 极狐 GitLab：[公开 Release 页面](https://jihulab.com/gitlab-cn/gitlab/-/releases) / [Release API](https://jihulab.com/api/v4/projects/gitlab-cn%2Fgitlab/releases) | 两者匿名 200 | 公开 Release 元数据无需登录 |
| 极狐 GitLab：[Generic Package 文件](https://jihulab.com/api/v4/projects/13953/packages/generic/gitlab-workhorse/437ae4de0e93c2f6a5f8a8ea54913f002cf3f67a/workhorse-437ae4de0e93c2f6a5f8a8ea54913f002cf3f67a.tar.gz) | 匿名 Range 最终 206，`Content-Range: bytes 0-0/60644521`，响应 SHA-256 `4fbe91a58bb3d55d06dce678474708433715a1e39b6f0e77c37304bcb53d01fc` | **匿名二进制分发成立**，且 URL 由项目、包名、版本和文件名确定 |
| 极狐 GitLab：[README Raw](https://jihulab.com/gitlab-cn/gitlab/-/raw/master/README.md) / [Repository Files Raw API](https://jihulab.com/api/v4/projects/13953/repository/files/README.md/raw?ref=master) | 均匿名 200、5,869 字节 | 更新清单可使用 Raw，并可保留 Repository Files API 回退 |
| GitLink：[公开项目 Release 附件](https://gitlink.org.cn/signriver/file-warehouse/releases/download/ste/stellaris_appinfo.json) | 匿名 GET 返回 200、`application/octet-stream`、`Content-Disposition: attachment` 和文件字节，未跳转登录页 | **匿名 Release 附件下载成立**；样本较小，未证明大文件 Range/断点续传能力 |

极狐样本的匿名响应还给出了 500 次的 rate-limit 配额。该值是本次实例响应，不应硬编码为永久平台合同。

### GitCode 的 `HEAD 401` 陷阱

GitCode 对同一附件的 `HEAD` 可返回 401，而无凭据 `GET` 返回文件。这会让浏览器扩展、下载探测器或只发 `HEAD` 的健康检查产生“必须登录”的误判。迁移与发布门禁都应以匿名 `GET` 为准；为了节省带宽，可发 `Range: bytes=0-0`，但最终正式发布仍应完整下载并核对 SHA-256。

## 平台细节

### 极狐 GitLab：推荐

极狐提供的是 GitLab 的完整分层能力：Release 负责版本说明和资产链接，Generic Package Registry 负责实际二进制文件。官方 [Generic Package 文档](https://docs.gitlab.cn/jh/user/packages/generic_packages/) 给出通过 Package API 上传和下载文件的路径；[Releases API](https://docs.gitlab.cn/jh/api/releases/) 可自动创建 Release 并挂接资产链接。

对 GPTEasy 的建议布局：

- Generic Package：`gpteasy/<semver>/GPTEasy_<semver>_x64-setup.exe`、`.sig`、`SHA256SUMS.txt`；
- Release：Tag 和中文说明，资产链接指向上述确定性 Package URL；
- 公开分发仓库：`latest.md`，最后写入；客户端优先读 Raw，失败时读 Repository Files Raw API；
- 发布器：先上传或复核同版本同名文件，匿名完整下载核对大小和 SHA-256，再创建/更新 Release，最后更新清单。

Generic Package URL 稳定且不依赖每次下载生成的签名查询串，适合写入 Tauri updater 清单。平台允许同版本路径再次上传的具体策略可能由实例设置控制，因此 GPTEasy 自己仍须把版本路径视为不可变：遇到同名内容不一致立即失败，不能覆盖。

本次未从极狐官方资料确认单文件最大限制；60.6 MB 样本已覆盖 GPTEasy 当前约 3.8 MB 安装包的数量级，但正式迁移 PoC 仍应上传一个接近预期最大体积的测试文件。

### AtomGit：可用但不是独立故障域

AtomGit 的公开 Release API 和附件下载行为与 GitCode 接近。其官方前端资源 [发行版说明](https://cdn-static.gitcode.com/assets/index-83285e2d.js) 明示“单个附件不能超过 2G”，足够容纳 GPTEasy 安装包。稳定 Release URL 会 302 到临时签名 CDN URL，调用方应保存前者。

缺点是附件最终由 `file-cdn.gitcode.com` 提供。若更换平台的动机是规避 GitCode 的账户策略或 UI，可以采用 AtomGit；若动机是获得独立的网络、WAF、对象存储和运营故障域，AtomGit 不满足。

### GitCode：附件无需登录，Raw 稳定性仍是问题

当前 [GitCode Release API](https://api.gitcode.com/api/v5/repos/ericyin99/GPTEasy-Releases/releases/v1.3.0) 和实际附件均已完成匿名闭环，现有 `scripts/sync-gitcode-release.mjs` 也已具备自动创建、上传、匿名下载和哈希校验。因此“附件必须登录”不是迁移理由。

真实问题是仓库已有文档记录的 Raw/WAF 波动：稳定分支 Raw 曾返回 403，而 Contents API、不可变 blob 和 Release 附件可用。如果保留 GitCode，应继续完成 Contents API/Base64 回退，并把附件健康检查从 `HEAD` 改为 Range GET/完整 GET。

### Gitee：匿名下载与官方 API 均已证实

Gitee 的官方 [OpenAPI 文档](https://gitee.com/api/v5/swagger) 公开以下接口：

- `POST /v5/repos/{owner}/{repo}/releases/{release_id}/attach_files`：multipart 上传附件；
- `GET /v5/repos/{owner}/{repo}/releases/{release_id}/attach_files`：枚举附件；
- `GET /v5/repos/{owner}/{repo}/releases/{release_id}/attach_files/{attach_file_id}/download`：下载附件。

Swagger 对下载接口的 `access_token` 标记为 `required: false`，响应定义为 200；Release 查询接口也可匿名调用。运行时探测又用两个独立公开仓库闭环验证了该设计：easyAi 的 490,300 字节 JAR 与 Layui 的 401,828 字节 ZIP 均通过稳定 Release URL 匿名完整下载，最终返回 200 和正确文件类型，未跳转登录页。

Gitee 的优势是原生 Release 附件模型与现有 GitCode 同步器更接近，迁移改造通常小于极狐的 Package + Release 组合。风险是本次样本都小于 1 MB，且连续匿名 API 探测曾触发 403 rate limit；正式采用前仍须用专门的公开测试仓库完成创建 Release、上传接近真实安装包大小的附件、无凭据 Range/完整 GET、哈希校验、重复发布冲突和限流测试。

### GitLink：匿名下载成立，发布自动化仍缺证据

GitLink 的公开项目 [Release 附件样本](https://gitlink.org.cn/signriver/file-warehouse/releases/download/ste/stellaris_appinfo.json) 在无凭据 GET 中直接返回 200、附件响应头和文件字节，证明它不只支持公开 clone/raw，Release 附件本身也能匿名下载。官方前端资源 [main.js](https://gitlink.org.cn/build/static/js/main.6c0019b9.chunk.js) 还包含 `/{owner}/{project}/releases`、`releases/new` 和 `releases.json`，证明平台有发行版元数据 UI。

但本次没有找到官方 OpenAPI/CLI 的 Release 附件上传合同，若干公开仓库的 `releases.json` 还返回了 SPA HTML，而非可依赖的 JSON API。现有样本也较小，Range 请求返回完整 200，未验证安装包体积下的断点续传、文件上限和限流表现。因此 GitLink 的“匿名下载”已证实，但“可自动、可靠地承担 GPTEasy 发布”仍未证实，正式采用前必须做真实安装包上传与匿名完整校验 PoC。

### CODING：公开制品与代码 Release 是两条不同路径

CODING 官方 [代码版本文档](https://coding.net/help/docs/repo/version/release.html) 说明代码版本支持上传任意格式、单文件小于 100 MB，容量足够；[制品库权限文档](https://coding.net/help/docs/artifacts/permission.html) 说明制品库可设为“公开”，但团队必须先实名认证；Generic 制品也有标准命令行上传路径。

然而，官方资料没有把“公开制品”明确等同于“任意未登录用户可下载代码 Release 附件”，本次也没有取得公开代码版本附件的无凭据下载样本。CODING 当前产品形态更偏团队 DevOps，公开分发需要实名团队和独立制品库配置。它可以作为后续 Generic 制品 PoC 对象，但不应把公开 Git clone 或“公开”权限字样直接视为匿名 Release 下载已经成立。

## 迁移建议

如果决定直接采用新源，不考虑旧版本过渡，可按改造目标选择两条路线：

- **优先减少改造**：先对 Gitee 做自有公开仓库 PoC，复用原生 Release 附件模型；PoC 全部通过后再替换 GitCode 同步目标。
- **优先确定性制品路径与成熟分层 API**：采用极狐 GitLab，按以下顺序实施。

1. 在极狐创建只用于国内分发的公开项目，先以非正式版本上传安装包、签名和校验文件到 Generic Package。
2. 从无凭据环境完整下载，验证文件大小、SHA-256、Range 支持和稳定 URL；再用 Release API 创建版本并添加 Package 链接。
3. 在公开仓库写入测试清单，验证 Raw 与 Repository Files Raw API 两条匿名路径；清单仍保持“附件全部验证后最后写入”。
4. 将现有 GitCode 同步器抽象为极狐实现时，保留现有的稳定 SemVer、版本降级拒绝、同名内容冲突失败、匿名完整校验、日志脱敏和最终清单写入规则。
5. 正式切换需要修订 ADR-0038/0039、分发文档、工作流变量/Secret、更新端点和真实冒烟测试；本次调研不构成发布授权，也未修改实现。

若希望保留“原生 Release 附件”模式而不采用 Generic Package，可把 GitLink 放入第二轮 PoC；在未完成附件自动上传、安装包大小、匿名完整下载和重复发布冲突测试前，不应写入客户端更新清单。

即使最终不迁移，也建议立即把所有“匿名可下载”健康检查统一为 Range GET/完整 GET，不再用 HEAD 单独下结论。
