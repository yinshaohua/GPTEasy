# 允许未签名的 Windows 正式发布包

GPTEasy 长期不把购买 Windows 代码签名证书作为发布前置条件，正式发布允许 Authenticode 状态为 `NotSigned` 的 Windows x64 当前用户安装包。门禁仍拒绝无效签名，继续绑定安装包哈希、候选清单与真实 UAT 证据，并在 GitHub Release 中明确披露未签名状态和可能出现的 Windows SmartScreen 警告；若未来提供有效签名，沿用同一验证与披露流程。
