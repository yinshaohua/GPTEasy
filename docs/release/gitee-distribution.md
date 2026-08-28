# Gitee 分发基线

## 分发边界

GitHub 是源码、Tag、版本、候选构建和中文发布说明的唯一权威来源。公开的 Gitee 分发仓库只允许包含：

- `README.md` 下载与校验说明；
- JSON 正文的正式稳定清单 `latest.md`；
- `smoke/` 下的非正式 API 冒烟清单；
- 不可变 Release 及其 NSIS 安装包、`.sig` 和 SHA-256 信息。

Gitee 仓库不是源码镜像，不运行构建，不单独维护版本或发布说明。正式同步在 GitHub Release 发布后自动执行。Gitee Raw 直接传输 JSON 正文，正式稳定清单路径为 `latest.md`，Tauri 解析正文而不依赖 URL 扩展名。

## 分发源自举

v1.3.0 不会自动迁移国内更新源。首个内置 Gitee Raw 信任根的正式版本必须由旧版本用户从 Gitee Releases 手工安装；不存在 GitCode 桥接版本、Contents API 回退或后台源切换。安装该分发源自举版本后，后续版本仍只读取一次固定的 Gitee Raw 清单，并继续在签名验证完成后等待用户确认安装。

## 配置位置

| 配置 | 存放位置 | 何时修改 |
|------|----------|----------|
| Gitee API/Raw 地址、清单路径、平台键、配置名称 | `scripts/gitee-distribution.json` | 分发协议变化时提交代码修改 |
| `GITEE_REPOSITORY` | GitHub Actions variable | 更换公开分发仓库时 |
| `GITEE_DEFAULT_BRANCH` | GitHub Actions variable | 更换分发仓库默认分支时 |
| `GITEE_TOKEN` | GitHub Actions secret | Token 到期、撤销或轮换时 |
| updater 公钥和唯一 Raw 端点 | `src-tauri/tauri.conf.json` | 由设置向导生成并提交 |
| updater 私钥路径和公开设置缓存 | 被忽略的 `.release.env` | 本机路径变化时 |
| updater 私钥 | 仓库外 `~/.tauri/` | 首次建立或人工轮换信任根时 |
| updater 私钥密码 | 密码管理器 | 首次建立或人工轮换信任根时 |

正式同步和冒烟工作流每次都需要 Gitee Token，但会自动从 `GITEE_TOKEN` secret 注入，不要求维护者重复输入，也不从本机 `.env` 读取。

## 首次设置

在 Git Bash 或 WSL2 中，从仓库根目录运行：

```bash
bash scripts/setup-gitee-distribution.sh
```

向导依次完成公开分发仓库、公开 Actions variables、隐藏 Token、`GITEE_TOKEN` Actions secret、仓库外带密码 updater 密钥、离线备份确认和非正式冒烟工作流。它会把公开仓库坐标、私钥路径和公钥记入被 Git 忽略的 `.release.env`，并把公开端点和公钥写入 `src-tauri/tauri.conf.json`。只提交公开配置；不得提交 `.release.env` 或私钥。

私钥建议保存在 `~/.tauri/gpteasy-updater.key`。向导调用 Tauri 自己的隐藏密码输入，不把密码放进参数或日志，并检查生成结果确实是加密私钥。至少把私钥复制到一份离线加密介质，把密码保存在分离的密码管理器或离线记录中，并实际验证两者可读取。

## 冒烟合同

设置向导和维护者都通过 `.github/workflows/gitee-smoke.yml` 触发真实冒烟。工作流从 GitHub secret 自动读取 Token，并使用唯一的 `smoke-<run-id>-<attempt>` 名称：

1. 使用 form-data 与 Token 创建非正式 prerelease Release；
2. 使用数值 Release ID 以 multipart/form-data 上传接近安装包体积的附件；
3. 匿名下载附件并核对 SHA-256；
4. 写入 JSON 正文的 `smoke/<name>.md` 测试清单；
5. 匿名读取 contents 元数据返回的官方 Gitee Raw blob，并核对字段；报告输出精确的 `tag` 和数值 `releaseId`，供维护者按需运行 `scripts/cleanup-gitee-release.sh <releaseId> <tag>` 二次确认清理。

冒烟命令不包含正式清单路径，因此不能推进正式稳定版本。失败时只报告操作与公开错误，不输出 Token。

本地 adapter 与应用内 HTTP 测试只证明协议实现，不能替代真实平台冒烟；两类证据的边界和真实验收字段见 `docs/evidence/gitee-domestic-distribution.md`。

## 正式同步

`.github/workflows/gitee-sync.yml` 由 GitHub Release `published` 事件触发，也允许维护者输入同一 Tag 人工重试。工作流只调用 `scripts/sync-gitee-release.mjs` 下载 Release 附件，不运行应用构建。同步器执行以下顺序：

1. 拒绝草稿、预发布和非稳定 SemVer Release；读取当前正式清单以阻止版本降级，首次发布则验证分发仓库 README 的 Raw 可读性。同步器可用公开 Contents 元数据和官方 Raw blob 复核既有清单，但客户端更新端点始终只有一个稳定分支 Raw 地址；
2. 下载 GitHub Release 中的 Windows x64 NSIS 安装包及其 `.sig`，计算大小和 SHA-256，并生成 `SHA256SUMS.txt`；
3. 创建或复用同 Tag Gitee Release，正文直接采用 GitHub 中文发布说明；
4. 已存在的同名附件必须经匿名下载证明大小和 SHA-256 相同，缺失附件才上传；上传遇到网络异常、超时或 `408/425/429/5xx` 时执行三次有限重试，认证、权限和其它确定性错误立即停止；同名内容冲突也立即停止；
5. 所有附件上传后再次匿名下载校验，最后才写入正式清单。

正式清单只包含 `windows-x86_64`，签名字段保存 `.sig` 正文。任何附件、匿名下载或版本门禁失败都不会写入清单，也不会修改或删除 GitHub Release。可控 HTTP adapter 测试通过 `npm run test:gitee-sync` 运行。

## 已知限制与后续优化

迁移前，GitCode 的稳定分支 Raw 地址曾被 WAF 返回 HTTP 403；同一仓库的公开 Contents API、不可变 blob 和 Release 附件仍可正常读取。该历史现象不是 Gitee 已知结论。Gitee 的真实 Raw 可用性必须由本次冒烟在公开仓库上验证；在此之前，不能将本地 adapter 结果或 GitCode 历史记录表述为 Gitee 平台通过。

ADR-0045 已取代 GitCode 的更新源和 #53 的 GitCode Contents API 回退计划；当前客户端只读取 Gitee Raw。

## 发布准备

在干净的 `main` 上统一准备 JavaScript、Rust 和 Tauri 版本：

```powershell
npm run release:prepare -- -Version 1.1.0 -Tag v1.1.0
```

命令拒绝非稳定 SemVer、现有版本漂移、非 `main`、脏工作树、已存在 Tag 和 Tag/版本不匹配。修改完成后先审查并提交，再创建 Tag。

构建 Windows 候选前，确认当前 PowerShell 会话能够读取签名私钥路径和密码。密码可以预先配置在系统环境变量 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 中，构建脚本不会再交互式读取：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY_PATH = Join-Path $env:USERPROFILE ".tauri\gpteasy-updater.key"
if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD)) {
    throw '请先设置 TAURI_SIGNING_PRIVATE_KEY_PASSWORD。'
}
npm run candidate:windows
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PATH
```

如果密码来自系统环境变量，通常不需要在构建后删除它；如只想让当前 PowerShell 会话失效，可执行 `Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，不会修改系统环境变量。

候选门禁要求公开信任根已配置，Tauri 生成同一 NSIS 安装包及其 `.sig`，并以应用内公钥真实验证签名。候选 manifest 同时绑定安装包和 `.sig` 的路径、大小与 SHA-256。静态清单可用以下命令独立校验：

```powershell
npm run release:manifest -- -ManifestPath <latest.md>
```

维护者确认已经完成与本次变更相称的人工测试后，正式发布检查不要求一次性账户 UAT 证据：

```powershell
npm run release:check -- -Mode Release -InstallerPath <setup.exe> -CandidateManifestPath <manifest.json> -ConfirmMaintainerAcceptance
```

发布说明必须如实列出实际执行的人工测试，不得把未运行的交互式 UAT 写成通过。
