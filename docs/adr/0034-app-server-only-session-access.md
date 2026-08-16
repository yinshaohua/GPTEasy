---
status: accepted
---

# 会话管理只使用官方 App Server

Codex App Server 是 GPTEasy 获取和修改 Codex 会话的唯一接口。会话管理不直接读取或写入 Codex SQLite，不解析、移动或删除 rollout JSONL，也不复制 Codex++ 的多版本私有 schema 兼容层。

进入会话管理前，后端必须探测实际安装的 Codex 命令、版本和 App Server 协议能力。找不到 App Server、版本低于最低支持范围、初始化失败或缺少首版核心方法时，会话管理显示明确的不可用状态，并引导用户自行安装或升级 Codex；GPTEasy 不自动安装或升级，也不降级到私有文件的只读模式。

可选协议能力按实际 schema 独立降级，不能因为在线文档存在某个字段就假定本机版本支持。GPTEasy 自己的 SQLite 可以保存能力快照和产品界面状态，但不保存第二份会话事实或正文索引来绕过 App Server 不可用状态。

首版搜索只使用稳定 App Server 接口匹配会话标题或预览文本，用户打开详情时再按需读取会话内容。实验性全文搜索不进入首版，GPTEasy 也不建立本地正文索引；因此搜索结果必须明确保持为元数据搜索，不能暗示已经检索完整会话正文。
