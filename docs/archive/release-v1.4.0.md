# GPTEasy v1.4.0 变更归档

发布日期：2026-08-28

## 用户可见变更

- 国内应用更新和手工下载入口从 GitCode 切换到 Gitee，更新失败和版本说明均进入公开的 Gitee Releases。
- v1.4.0 是分发源自举版本。v1.3.0 及更早版本不会自动切换更新源，用户必须从 Gitee 或 GitHub Release 手工安装 v1.4.0；安装后后续版本继续使用应用内更新。
- 应用仍只接受 Windows x64 正式稳定版本、HTTPS 附件和现有 updater 签名，下载完成后仍由用户确认安装。

## 发布与分发

- GitHub 继续作为源码、Tag、构建产物和中文发布说明的唯一权威来源；Gitee 分发仓库不镜像源码、不执行构建，也不维护独立版本。
- GitHub Release 发布后自动把同一安装包、updater 签名、SHA-256 校验文件和发布说明同步到 Gitee。
- Gitee 同步器拒绝草稿、预发布、非稳定版本、版本降级和同名内容冲突；所有附件匿名完整校验后才最后推进稳定清单。
- 同步遇到连接异常、限流或瞬时服务错误时执行有限重试；认证、权限、输入和内容冲突立即失败，旧稳定清单保持不变。

## 被取代的需求

| 旧需求 | v1.4.0 决定 |
| --- | --- |
| ADR-0038/0039 使用 GitCode 作为活动国内更新源 | 由 ADR-0045 取代：唯一国内分发源切换为 Gitee，保留历史 GitCode 发布物 |
| #53 为 GitCode Raw 增加 Contents API 回退 | 由 #54 的 Gitee 直接迁移取代，不实现 GitCode 或 Gitee Contents API 回退 |
| 通过桥接版本自动迁移国内更新源 | 不创建桥接版本；v1.4.0 由旧版本用户手工安装完成分发源自举 |

## 安全边界

- updater 公私钥不因托管平台迁移而轮换；私钥保持在仓库外，正式候选继续使用加密私钥签名并以应用内公钥验证。
- Gitee Token 只由 GitHub Actions Secret 注入写操作，不进入源码、URL、日志、清单或公开读取请求。
- 正式清单只保存稳定 Gitee Release URL，不保存带时效参数的 CDN URL；安装包、签名和 SHA-256 均按不可变发布物处理。
- 旧 GitCode Actions Secret/Variables 和站点 Token 已撤销，历史 GitCode 仓库、Tag、Release 和附件未删除。

## 验证记录

- Gitee 真实冒烟上传 4,194,304 字节附件；匿名 Range 请求、独立完整下载、大小和 SHA-256 校验通过，Raw 测试清单可匿名读取。
- 维护者在无登录浏览器中从 Gitee Release 页面直接下载附件，没有图形验证，也没有注册或登录要求；随后按精确 Release ID 和 Tag 清理全部非正式资源。
- Gitee 协议测试覆盖 Release 创建目标分支、数值 ID、form/multipart 编码、附件完整性、瞬时重试、错误脱敏、缺失清单和清单最后推进。
- 正式候选仍须在干净 `main` 上通过前端、Rust、布局、完整自动验收、发布树、发布合同、更新信任根和 updater 签名门禁；未执行的一次性账户交互式 UAT 不记录为通过。
