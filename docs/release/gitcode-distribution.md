# GitCode 分发基线

## 分发边界

GitHub 是源码、Tag、版本、候选构建和中文发布说明的唯一权威来源。公开的 GitCode 分发仓库只允许包含：

- `README.md` 下载与校验说明；
- JSON 正文的正式稳定清单 `latest.md`；
- `smoke/` 下的非正式 API 冒烟清单；
- 不可变 Release 及其 NSIS 安装包、`.sig` 和 SHA-256 信息。

GitCode 仓库不是源码镜像，不运行构建，不单独维护版本或发布说明。正式同步与 `latest.md` 推进属于后续发布切片；本基线只建立可信输入、候选签名门禁和真实 API 冒烟路径。GitCode Raw 对 `.json` 和 `.txt` 路径返回“暂不支持预览”，因此清单使用已验证可匿名读取的 `.md` 扩展名承载 Tauri JSON。

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
5. 匿名读取 GitCode Raw 内容并核对字段。

冒烟命令不包含正式清单路径，因此不能推进正式稳定版本。失败时只报告操作与公开错误，不输出 Token。

## 发布准备

在干净的 `main` 上统一准备 JavaScript、Rust 和 Tauri 版本：

```powershell
npm run release:prepare -- -Version 1.1.0 -Tag v1.1.0
```

命令拒绝非稳定 SemVer、现有版本漂移、非 `main`、脏工作树、已存在 Tag 和 Tag/版本不匹配。修改完成后先审查并提交，再创建 Tag。

构建 Windows 候选前，从安全的交互式环境设置私钥路径和密码：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY_PATH = "$HOME\.tauri\gpteasy-updater.key"
$securePassword = Read-Host -AsSecureString
$credential = New-Object System.Management.Automation.PSCredential('', $securePassword)
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $credential.GetNetworkCredential().Password
npm run candidate:windows
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PATH, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

候选门禁要求公开信任根已配置，Tauri 生成同一 NSIS 安装包及其 `.sig`，并以应用内公钥真实验证签名。候选 manifest 同时绑定安装包和 `.sig` 的路径、大小与 SHA-256。静态清单可用以下命令独立校验：

```powershell
npm run release:manifest -- -ManifestPath <latest.md>
```
