# 功能验收可未签名但正式发布必须 Authenticode

开发和功能验收可以使用未签名的 Windows x64 当前用户安装包，签名资源不阻塞供应商、配置、SQLite 或 UI 阶段。面向外部用户的正式发布必须使用有效 Authenticode 签名，并验证当前用户安装、覆盖安装、卸载和数据保留；Windows ARM64、macOS、公证、GitHub attestation 和一次性 CI 账户不构成该发布的前置证据。签名检查只存在于最终打包流程，避免发布基础设施再次主导核心产品开发。
