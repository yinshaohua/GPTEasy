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

```bash
gh workflow run gitee-smoke.yml -f source_tag=v1.4.1
```

`source_tag` 必须是已经发布的稳定 GitHub Release；留空时使用 latest。迁移 API 主机、上传参数或 Runner 环境后，以及正式发布前需要重新执行，不得沿用旧 `.txt` 冒烟作为 PE 上传证据。

1. 使用 form-data 与 Token 创建非正式 prerelease Release；
2. 从指定 GitHub 稳定 Release（留空时取 latest）下载真实 Windows PE 安装包，使用数值 Release ID 以 multipart/form-data 上传；
3. 匿名发送 Range 下载探测；平台返回 `206` 时核对单字节和 `Content-Range`，忽略 Range 返回 `200` 时核对完整大小和 SHA-256；随后再次完整下载并核对 SHA-256；
4. 写入 JSON 正文的 `smoke/<name>.md` 测试清单；
5. 匿名读取 contents 元数据返回的官方 Gitee Raw blob，并核对字段；报告输出精确的 `tag` 和数值 `releaseId`，供维护者按需运行 `scripts/cleanup-gitee-release.sh <releaseId> <tag>` 二次确认清理。

冒烟命令不包含正式清单路径，因此不能推进正式稳定版本。失败时只报告操作与公开错误，不输出 Token。

### 人工匿名验收与冒烟清理

工作流成功后先保留冒烟 Release。维护者必须新建无登录浏览器上下文，直接进入报告中 `tag` 对应的 Gitee Release 页面，点击附件并确认最多出现图形验证、不要求注册或登录。记录工作流 URL、数值 `releaseId`、`tag`、页面点击结果，以及工作流报告中的匿名 Range、完整下载大小、SHA-256 和 Raw 清单结果；未实际观察到的项目不得记录为通过。

人工点击完成后，在 Git Bash 或 WSL2 中从密码管理器隐藏读取 Gitee Token，再按报告中的精确 ID 和 tag 同时删除 `smoke/<tag>.md` 与 prerelease：

```bash
read -rs GITEE_TOKEN && printf '\n'
export GITEE_TOKEN
export GITEE_REPOSITORY=ericshaohua/gpteasy-releases
export GITEE_DEFAULT_BRANCH=main
bash scripts/cleanup-gitee-release.sh <releaseId> <tag>
unset GITEE_TOKEN
```

脚本会先核对数值 Release ID、tag、测试清单类型和 blob SHA，再要求输入完整 tag 二次确认；它先删除测试清单，后删除 Release。若 Release 删除失败，可用同一命令重跑，已经删除的测试清单按可恢复状态处理。不得在人工点击前清理，也不得把 Token 放入命令行参数、`.release.env`、日志或验收记录。

本机没有 Token 时，可在 GitHub Actions 手工运行 `.github/workflows/gitee-cleanup-smoke.yml`，分别输入报告中的数值 Release ID 和精确 tag。工作流仍调用同一清理脚本，逐项核对 ID、tag 和清单身份，只从 `GITEE_TOKEN` Secret 读取凭据。

本地 adapter 与应用内 HTTP 测试只证明协议实现，等体积 `.txt` 也不能替代真实 PE 上传；两类证据的边界和真实验收字段见 `docs/evidence/gitee-domestic-distribution.md`。

## 正式同步

`.github/workflows/gitee-sync.yml` 由 GitHub Release `published` 事件触发，也允许维护者输入同一 Tag 人工重试。工作流只调用 `scripts/sync-gitee-release.mjs` 下载 Release 附件，不运行应用构建。同步器执行以下顺序：

1. 拒绝草稿、预发布和非稳定 SemVer Release；读取当前正式清单以阻止版本降级；比较仓库根 README 与 Gitee `README.md`，不一致时更新并通过匿名 Raw 读取核对。同步器可用公开 Contents 元数据和官方 Raw blob 复核既有清单，但客户端更新端点始终只有一个稳定分支 Raw 地址；
2. 下载 GitHub Release 中的 Windows x64 NSIS 安装包及其 `.sig`，计算大小和 SHA-256；Gitee 上的 PE 附件名追加 `.bin`，并按实际附件名生成 `SHA256SUMS.txt`；
3. 创建或复用同 Tag Gitee Release，正文采用 GitHub 中文发布说明，并幂等追加 Gitee `.exe.bin` 手工重命名提示；更新清单中的 `notes` 仍只保存 GitHub 原始发布说明；
4. 已存在的同名附件必须经匿名下载证明大小和 SHA-256 相同，缺失附件才上传；上传使用官方 `https://gitee.com/api/v5`、HTTP/1.1、无 `Expect: 100-continue` 的 multipart 请求，并保留五分钟单次响应预算；响应丢失时先重新枚举附件，确认未落盘才有限重试，认证、权限和其它确定性错误立即停止；同名内容冲突也立即停止；
5. 所有附件上传后再次匿名下载校验，最后才写入正式清单。

正式清单只包含 `windows-x86_64`，签名字段保存 `.sig` 正文。任何附件、匿名下载或版本门禁失败都不会写入清单，也不会修改或删除 GitHub Release。GitHub Release 发布不等于发布流程完成；只有 `gitee-sync.yml` 成功，且公开 Gitee Release 的三个附件与 `latest.md` 均指向该版本后，才能记录国内分发完成。可控 HTTP adapter 测试通过 `npm run test:gitee-sync` 运行。

Gitee 的安装包附件名为 `GPTEasy_<version>_x64-setup.exe.bin`。应用更新下载后按原 updater 签名验证完整 PE 字节，并固定写入临时 `.exe`，不依赖 URL 扩展名。手工下载用户须先去掉末尾 `.bin`，再按 `SHA256SUMS.txt` 校验并运行；GitHub 备用下载仍保留标准 `.exe` 文件名。

### 失败恢复

- 首次设置可重复运行；向导复用仓库外 updater 密钥，重新校验公开仓库并覆盖同名 Actions Secret/Variables。不得因设置重跑而生成或轮换 updater 密钥。
- 冒烟失败时保留已创建资源。tag 固定为 `smoke-<GitHub run-id>-<attempt>`；从失败日志或 Gitee Release API 按该 tag 取得数值 ID，排障并完成必要证据后运行精确清理命令。不要自动删除无法核实身份的资源。
- 正式同步失败时旧 `latest.md` 保持不变。修复临时平台故障后，对同一 GitHub Tag 手工触发 `gitee-sync.yml`；同一 Tag 受工作流并发锁保护，同步器只补齐缺失且一致的附件，同名内容冲突不得覆盖。重跑成功前不得把本次国内分发记录为完成。
- Token 泄露、到期或权限错误时，先在 Gitee 创建替代 Token并覆盖 GitHub Secret，验证冒烟成功后撤销旧 Token。Token 轮换不改变公开仓库坐标、清单端点或 updater 密钥。

## 已知限制与后续优化

迁移前，GitCode 的稳定分支 Raw 地址曾被 WAF 返回 HTTP 403；同一仓库的公开 Contents API、不可变 blob 和 Release 附件仍可正常读取。该历史现象不是 Gitee 已知结论。Gitee 的真实 Raw 可用性必须由本次冒烟在公开仓库上验证；在此之前，不能将本地 adapter 结果或 GitCode 历史记录表述为 Gitee 平台通过。

ADR-0045 已取代 GitCode 的更新源和 #53 的 GitCode Contents API 回退计划；当前客户端只读取 Gitee Raw。

## 迁移收口

只有真实 Gitee 冒烟、无登录页面点击、自动门禁和证据记录全部完成后，才删除不再使用的 GitCode Actions 配置：

```bash
gh secret delete GITCODE_TOKEN --repo yinshaohua/GPTEasy
gh variable delete GITCODE_REPOSITORY --repo yinshaohua/GPTEasy
gh variable delete GITCODE_DEFAULT_BRANCH --repo yinshaohua/GPTEasy
```

随后由维护者登录 GitCode 设置页撤销 GitCode Token，并复核 GitHub Actions 中只保留活动 Gitee 分发配置。凭据删除后，在 #53 评论 Gitee 替代证据，移除 `ready-for-agent`、添加 `wontfix`，再以 not planned 关闭。

迁移收口不删除 GitCode 公开仓库、Tag、历史 Release 或附件，也不修改历史发布归档。它不授权打 Tag、创建正式 GitHub/Gitee Release、执行 Windows 人工测试或记录发布授权。

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
