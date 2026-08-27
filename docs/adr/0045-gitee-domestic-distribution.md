# Gitee 国内分发与真实冒烟闭环

## 状态

已采纳。本文取代 ADR-0038 与 ADR-0039 中关于 GitCode 作为活动国内更新源的决定；GitCode 历史仓库、Release 和证据保留，不再由当前工作流写入。

## 决策

国内分发仓库固定为公开的 `ericshaohua/gpteasy-releases`，使用 `main` 分支、Gitee Release 和 `latest.json` 语义（因 Gitee Raw 兼容性，传输文件仍使用 `latest.md` 路径）。GitHub 仍是源码、Tag、构建产物和发布说明的唯一权威来源，Gitee 只复制已发布产物，不独立构建或镜像源码。

同步器通过 Gitee 官方 API 按 Tag 查询 Release，并使用数值 Release ID 操作附件。附件通过 multipart/form-data 上传，内容写入使用 form-data；Token 只来自 GitHub Actions Secret `GITEE_TOKEN`，不进入 URL、清单或日志。附件完成匿名 Range/完整 GET、大小和 SHA-256 校验后，才最后写入清单。已存在同名附件内容不一致时立即失败。

客户端只信任一个 Gitee Raw HTTPS 清单端点和现有 updater 公钥。更新失败时手工入口指向对应 Gitee Release；不实现 GitCode 或其它平台回退，也不轮换 updater 密钥。迁移不包含 #53 的 GitCode Contents API 回退。

## 验证

设置向导验证公开仓库、隐藏读取 Token、写入 Actions Secret/Variables 并清理进程环境。冒烟工作流创建唯一 prerelease，上传接近安装包体积的附件，验证匿名 Release 元数据、Range、完整下载、SHA-256 和 Raw 清单；冒烟资源必须由维护者完成无登录页面点击后，再用精确 Release ID 和 Tag 显式清理。
