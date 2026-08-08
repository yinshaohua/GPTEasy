import type {
  CodexConfigStatus,
  CredentialFileStatus,
  CredentialStore,
  DatabaseBlockReason,
  DatabaseStatus,
  LoginStatus,
  StartupBlockReason,
  PendingOperationResolution,
} from "./contracts/startup";

export const databaseStatusMessages: Record<DatabaseStatus, string> = {
  initialized: "本地状态已初始化",
  ready: "本地状态正常",
  recovered: "已从最近的有效备份恢复",
  blocked: "本地状态不可用",
};

export const databaseReasonMessages: Record<DatabaseBlockReason, string> = {
  missing_database: "已有安装的数据库缺失，且没有可用备份。GPTEasy 不会创建空库覆盖原状态。",
  corrupt_database: "数据库无法通过完整性校验，且没有可用备份。原文件已保留。",
  future_schema: "数据库由更高版本的 GPTEasy 创建，当前版本不会改写它。",
  migration_failed: "数据库迁移未完成。原数据库和迁移前备份均已保留。",
  backup_failed: "迁移前无法创建并校验一致备份，因此没有继续修改数据库。",
  recovery_failed: "最近的有效数据库备份无法安全恢复，当前版本不会创建空库。",
  io_failure: "当前用户的应用数据无法安全读取或写入。",
};

export const startupBlockMessages: Record<StartupBlockReason, string> = {
  database_unavailable: "数据库状态无法确认。",
  codex_config_invalid: "Codex 用户配置不是有效 TOML。GPTEasy 不会在此状态下继续普通操作。",
  codex_config_unreadable: "Codex 用户配置无法读取。GPTEasy 不会在无法确认磁盘状态时继续。",
  pending_config_operation: "检测到未完成的配置操作。恢复协调完成前不会继续普通操作。",
  managed_config_conflict: "Codex 配置与 GPTEasy 最后应用证据不一致。磁盘配置已保留。",
  unsupported_credential_store: "Codex 使用了当前版本不支持的凭据载体。GPTEasy 不会假定文件载体有效。",
};

export const pendingResolutionMessages: Record<PendingOperationResolution, string> = {
  matches_old_state: "磁盘状态仍是操作前状态，等待 Rust 恢复协调。",
  matches_new_state: "磁盘状态已达到操作目标，等待 Rust 完成提交协调。",
  conflict: "磁盘状态既不匹配操作前状态，也不匹配操作目标。",
  unknown: "无法用现有指纹判断操作收敛方向。",
};

export const codexConfigMessages: Record<CodexConfigStatus, string> = {
  missing: "尚未创建",
  valid: "已检测到有效配置",
  invalid: "配置格式无效",
  unreadable: "无法读取",
};

export const loginStatusMessages: Record<LoginStatus, string> = {
  logged_in: "已检测到登录",
  not_logged_in: "未检测到登录",
  unavailable: "无法确认",
};

export const credentialStoreMessages: Record<CredentialStore, string> = {
  unknown: "未确认",
  file: "文件载体",
  keyring: "系统凭据库",
  auto: "自动选择",
  unsupported: "不兼容的载体",
};

export const credentialFileStatusMessages: Record<CredentialFileStatus, string> = {
  not_applicable: "不适用",
  missing: "auth.json 不存在",
  present: "auth.json 已存在",
  unreadable: "auth.json 无法确认",
};

export const providerFailureMessages: Record<string, string> = {
  "provider.url_invalid": "服务地址格式无效。",
  "provider.url_components_forbidden": "服务地址不能包含用户信息、查询或片段。",
  "provider.remote_https_required": "远程服务地址必须使用 HTTPS。",
  "provider.url_scheme_unsupported": "服务地址只支持 HTTPS，回环地址可使用 HTTP。",
  "provider.redirect_forbidden": "供应商请求发生了不允许的重定向。",
  "provider.authentication_failed": "API Key 未通过供应商认证。",
  "provider.rate_limited": "供应商暂时限制了请求，请稍后重试。",
  "provider.models_request_failed": "无法从供应商获取模型。",
  "provider.models_invalid": "供应商返回的模型列表无法读取。",
  "provider.models_empty": "供应商没有返回可用模型。",
  "provider.default_model_missing": "默认模型不在本次发现的模型列表中。",
  "provider.response_header_timeout": "等待供应商响应超时。",
  "provider.first_event_timeout": "Responses 流没有在限定时间内开始。",
  "provider.stream_idle_timeout": "Responses 流中断时间过长。",
  "provider.overall_timeout": "供应商验证超过整体时限。",
  "provider.transport_failed": "无法连接供应商。",
  "provider.responses_not_sse": "供应商没有返回 Responses SSE 流。",
  "provider.responses_stream_broken": "Responses 流在完成前断开。",
  "provider.responses_stream_incomplete": "Responses 流缺少完成事件。",
  "provider.responses_protocol_invalid": "供应商的 Responses 事件不兼容。",
  "provider.tool_call_missing": "供应商没有完成要求的工具调用。",
  "provider.tool_call_invalid": "供应商返回了错误的工具名或调用 ID。",
  "provider.tool_arguments_invalid": "供应商返回的严格工具参数不匹配。",
  "provider.tool_result_invalid": "供应商没有完成工具结果回传闭环。",
  "provider.request_cancelled": "请求已取消。",
  "provider.name_required": "保存前需要填写供应商名称。",
  "provider.name_duplicate": "已有同名供应商，请更换名称。",
  "provider.not_found": "供应商已不存在，请刷新目录。",
  "provider.current_delete_forbidden": "当前供应商不能删除。",
  "provider.save_and_apply_required": "当前供应商的访问配置必须通过“保存并应用”同时更新 Codex 环境。",
  "provider.clipboard_unavailable": "无法写入系统剪贴板。",
  "provider.verification_expired": "本次验证已失效，请重新验证。",
  "provider.state_unavailable": "本地供应商目录暂时不可用。",
  "provider.preview_network_unavailable": "浏览器预览不能发起真实供应商验证。",
};

export const providerMessages = {
  pageTitle: "供应商",
  pageSubtitle: "管理通过完整验证的供应商",
  newProvider: "新建供应商",
  verifiedProviders: "已验证供应商",
  loadingCatalog: "正在读取供应商目录",
  catalogUnavailable: "无法读取供应商目录。",
  emptyCatalog: "尚无已验证供应商",
  editorSubtitle: "验证成功后再保存",
  detailsSubtitle: "已验证供应商详情",
  currentProvider: "当前供应商",
  providerName: "供应商名称",
  baseUrl: "服务地址",
  apiKey: "API Key",
  savedApiKey: "已保存；留空保持不变",
  showApiKey: "显示 API Key",
  hideApiKey: "隐藏 API Key",
  copyApiKey: "复制 API Key",
  copied: "已复制",
  copyFailed: "复制失败",
  defaultModel: "默认模型",
  chooseModel: "请选择模型",
  discoverModels: "获取模型",
  cancelRequest: "取消请求",
  validateProvider: "验证供应商",
  validateUpdate: "验证更新",
  revalidate: "重新验证",
  deleteProvider: "删除供应商",
  deleteConfirmation: "确定删除这个供应商吗？",
  save: "保存",
  saveAndApply: "保存并应用",
  validationPassed: "完整验证已通过",
  validationTitle: "供应商验证",
  modelsConfirmed: "模型确认",
  responsesStream: "Responses 流式响应",
  toolRoundTrip: "工具调用闭环",
  technicalDetails: "技术详情",
  validationFallback: "验证未完成，请检查输入后重试。",
} as const;
