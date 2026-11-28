# GPTEasy v1.5.6 变更归档

## 更新检查

- Gitee 更新清单优先通过公开 Contents API 获取并解码 Base64 正文；API 请求失败时回退到原 Raw 地址。安装包仍须通过原 updater 签名验证，API 返回的下载地址不受信任。
- 更新失败的问题日志增加清单来源字段（API、Raw 或自定义端点），便于区分故障发生在哪条读取路径。

## 交付范围

- Windows x64 当前用户安装包使用独立的 updater 签名。Windows SmartScreen 可能显示提示。
- GitHub 提供正式安装包。Gitee 同步同一安装包，并在附件校验完成后推进应用更新清单。
- 人工确认范围：公开 Gitee Contents API 清单地址可访问；未将桌面应用安装/更新交互测试记为通过。
