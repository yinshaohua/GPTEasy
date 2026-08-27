# GitCode 分发基线

## 分发边界

GitHub 是源码、Tag、版本、候选构建和中文发布说明的唯一权威来源。公开的 GitCode 分发仓库只允许包含：

- `README.md` 下载与校验说明；
- JSON 正文的正式稳定清单 `latest.md`；
- `smoke/` 下的非正式 API 冒烟清单；
- 不可变 Release 及其 NSIS 安装包、`.sig` 和 SHA-256 信息。

GitCode 仓库不是源码镜像，不运行构建，不单独维护版本或发布说明。正式同步在 GitHub Release 发布后自动执行。GitCode Raw 对 `.json` 和 `.txt` 路径返回“暂不支持预览”，因此正式 `latest.json` 的 JSON 正文通过已验证可匿名读取的 `latest.md` 路径传输；Tauri 解析正文，不依赖 URL 扩展名。

## 配置位置

| 配置 | 存放位置 | 何时修改 |
|------|----------|----------|
| GitCode API/Raw 地址、清单路径、平台键、配置名称 | `scripts/gitcode-distribution.json` | 分发协议变化时提交代码修改 |
| `GITCODE_REPOSITORY` | GitHub Actions variable | 更换公开分发仓库时 |
| `GITCODE_DEFAULT_BRANCH` | GitHub Actions variable | 更换分发仓库默认分支时 |
| `GITCODE_TOKEN` | GitHub Actions secret | Token 到期、撤销或轮换时 |
| updater 公钥和唯一 Raw 端点 | `src-tauri/tauri.conf.json` | 由设置向导生成并提交 |
| updater 私钥路径和公开设置缓存 | 被忽略的 `.release.env` | 本机路径变化时 |
| updater 私钥 | 仓库外 `~/.tauri/` | 首次建立或人工轮换信任根时 |
| updater 私钥密码 | 密码管理器 | 首次建立或人工轮换信任根时 |

正式同步和冒烟工作流每次都需要 GitCode Token，但会自动从 `GITCODE_TOKEN` secret 注入，不要求维护者重复输入，也不从本机 `.env` 读取。

## 首次设置

在 Git Bash 或 WSL2 中，从仓库根目录运行：

```bash
bash scripts/setup-gitcode-distribution.sh
```

向导依次完成公开分发仓库、公开 Actions variables、隐藏 Token、`GITCODE_TOKEN` Actions secret、仓库外带密码 updater 密钥、离线备份确认和非正式冒烟工作流。它会把公开仓库坐标、私钥路径和公钥记入被 Git 忽略的 `.release.env`，并把公开端点和公钥写入 `src-tauri/tauri.conf.json`。只提交公开配置；不得提交 `.release.env` 或私钥。

私钥建议保存在 `~/.tauri/gpteasy-updater.key`。向导调用 Tauri 自己的隐藏密码输入，不把密码放进参数或日志，并检查生成结果确实是加密私钥。至少把私钥复制到一份离线加密介质，把密码保存在分离的密码管理器或离线记录中，并实际验证两者可读取。

## 冒烟合同

设置向导和维护者都通过 `.github/workflows/gitcode-smoke.yml` 触发真实冒烟。工作流从 GitHub secret 自动读取 Token，并使用唯一的 `smoke-<run-id>-<attempt>` 名称：

1. 通过 Bearer Token 创建非正式 Release；
2. 获取上传地址并上传小附件；
3. 匿名下载附件并核对 SHA-256；
4. 写入 JSON 正文的 `smoke/<name>.md` 测试清单；
5. 匿名读取 contents 元数据返回的官方 GitCode Raw blob，并核对字段。

冒烟命令不包含正式清单路径，因此不能推进正式稳定版本。失败时只报告操作与公开错误，不输出 Token。

## 正式同步

`.github/workflows/gitcode-sync.yml` 由 GitHub Release `published` 事件触发，也允许维护者输入同一 Tag 人工重试。工作流只调用 `scripts/sync-gitcode-release.mjs` 下载 Release 附件，不运行应用构建。同步器执行以下顺序：

1. 拒绝草稿、预发布和非稳定 SemVer Release；读取当前正式清单以阻止版本降级，首次发布则验证分发仓库 README 的 Raw 可读性。当前清单优先使用 GitCode API 返回的 Base64 正文；没有内嵌正文时再使用固定分支 Raw 地址和 API 返回的不可变 blob 地址，并在遇到 418 等临时风控响应时重试和回退，避免单一下载路径阻塞同步；
2. 下载 GitHub Release 中的 Windows x64 NSIS 安装包及其 `.sig`，计算大小和 SHA-256，并生成 `SHA256SUMS.txt`；
3. 创建或复用同 Tag GitCode Release，正文直接采用 GitHub 中文发布说明；
4. 已存在的同名附件必须经匿名下载证明大小和 SHA-256 相同，缺失附件才上传；上传遇到网络异常、超时或 `408/425/429/5xx` 时执行三次有限重试，认证、权限和其它确定性错误立即停止；同名内容冲突也立即停止；
5. 所有附件上传后再次匿名下载校验，最后才写入正式清单。

正式清单只包含 `windows-x86_64`，签名字段保存 `.sig` 正文。任何附件、匿名下载或版本门禁失败都不会写入清单，也不会修改或删除 GitHub Release。可控 HTTP adapter 测试通过 `npm run test:gitcode-sync` 运行。

## 已知限制与后续优化

v1.3.0 发布后真实匿名复核发现，客户端内置的 GitCode 稳定分支 Raw 地址可能被 WAF 返回 HTTP 403；同一仓库的公开 Contents API、不可变 blob 和 Release 附件仍可正常读取。该问题不是附件缺失，重复上传或重新创建 Release 不能修复。v1.3.0 遇到此问题时，用户需要从 GitHub Releases 或 GitCode Releases 手工下载安装包。

后续优化由 [#53](https://github.com/yinshaohua/GPTEasy/issues/53) 跟踪：客户端先读取现有 Raw 地址，只有网络失败、非成功 HTTP 状态或无法解析为 JSON 的 WAF 正文时，才匿名读取固定仓库、分支和路径的 GitCode Contents API，并严格解码其 Base64 清单正文。两个通道仍各至多请求一次，不引入后台重试，不使用 GitCode Token，也不回退到 GitHub 或第二个分发仓库；清单语义、HTTPS 下载地址、稳定版本和 updater 签名门禁全部保持不变。

这项优化改变了 ADR-0038 的“单一 Raw 端点”和 ADR-0039 的“每次清单检查只发出一次匿名读取请求”的具体传输约束。实施 #53 时必须同步新增或修订 ADR；在此之前，当前 ADR 和 v1.3.0 行为仍是有效事实。

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
