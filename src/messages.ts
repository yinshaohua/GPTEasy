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

export const accessibilityMessages = {
  skipToMain: "跳转到主要内容",
  pageNavigation: "页面导航",
  refresh: "重新检查状态",
  refreshing: "正在重新检查状态",
} as const;

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
