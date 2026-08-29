# GPTEasy 下载

本仓库只提供 GPTEasy 的国内更新清单和不可变 Release 下载，不是源码镜像，也不在这里构建程序。

正式稳定版本请从 Releases 下载 Windows x64 当前用户 NSIS 安装包，并使用同一 Release 中的 SHA-256 信息核对完整性。Gitee 上的安装包因自动上传接口限制使用 `.exe.bin` 后缀，手工下载后须删除末尾 `.bin` 再运行；应用内更新会自动按签名验证并以 `.exe` 安装。Tauri updater `.sig` 不能替代 Windows Authenticode；当前安装包可能触发 SmartScreen 提示。

源码、Issue、Tag 和权威发布说明位于 GitHub `yinshaohua/GPTEasy`。Gitee Releases 无法访问时，可从 GitHub Releases 获取同一正式版本作为备用下载入口。`smoke-*` Release 与 `smoke/` 清单只用于 API 冒烟，不是正式版本。
