import { invoke } from "@tauri-apps/api/core";

import { isBrowserPreview } from "./browser-preview";

export type IssueLogLevel = "info" | "warn" | "error";

export interface IssueLogRecord {
  timestampEpochSeconds: number;
  level: IssueLogLevel;
  event: string;
  message: string;
  details: string | null;
}

export interface IssueLogFilter {
  sinceEpochSeconds: number;
  level: IssueLogLevel | null;
  query: string;
}

export type DiagnosticConfigStatus =
  | "missing"
  | "unreadable"
  | "encoding_error"
  | "toml_syntax_error"
  | "valid";

export type DiagnosticConsumerStatus = "running" | "stopped" | "unknown";
export type DiagnosticLoginStatus = "logged_in" | "not_logged_in" | "unavailable";
export type DiagnosticExportFormat = "json" | "markdown";
export type DiagnosticRepairStatus =
  | "succeeded"
  | "not_modified"
  | "rolled_back"
  | "manual_required";

export interface DiagnosticRepairPreview {
  previewId: string;
  source: "current_config" | "gpteasy_backup";
  providerName: string;
  baseUrl: string;
  model: string;
  authentication: "current_api_key";
  changes: Array<
    | "backup_config"
    | "add_custom_provider_definition"
    | "verify_and_rediagnose"
  >;
}

export interface DiagnosticReport {
  schemaVersion: number;
  environment: {
    scope: "current_user";
    codexHome: "~/.codex";
    codexHomeOverrideStatus: "unset" | "matches" | "differs";
    configStatus: DiagnosticConfigStatus;
    activeProvider: string | null;
    declaredProviders: string[];
  };
  authentication: {
    loginStatus: DiagnosticLoginStatus;
    authFileStatus: "missing" | "present" | "unreadable";
    credentialStore: "unknown" | "file" | "keyring" | "auto" | "unsupported";
  };
  consumers: {
    desktop: DiagnosticConsumerStatus;
    cli: DiagnosticConsumerStatus;
  };
  versions: {
    gpteasy: string;
    codexCli: string | null;
  };
  findings: Array<{
    code: string;
    origin: "local" | "remote";
    severity: "error" | "warning" | "info";
    title: string;
    summary: string;
    repairable: boolean;
  }>;
  errors: Array<{
    errorCode: string;
    origin: "local" | "remote";
    occurrences: number;
    lastSeenEpochSeconds: number;
  }>;
  repairPreview: DiagnosticRepairPreview | null;
}

export interface DiagnosticRepairExecution {
  status: DiagnosticRepairStatus;
  messageId: string;
  report: DiagnosticReport;
}

export interface DiagnosticRepairPlanItem {
  id: string;
  findingCode: string;
  title: string;
  description: string;
  action: "repair_custom_provider";
  previewId: string | null;
  requiresConfirmation: boolean;
}

export interface DiagnosticAssistantResult {
  providerId: string;
  providerName: string;
  explanation: string;
  repairPlan: DiagnosticRepairPlanItem[];
}

const browserDiagnosticReport: DiagnosticReport = {
  schemaVersion: 2,
  environment: {
    scope: "current_user",
    codexHome: "~/.codex",
    codexHomeOverrideStatus: "unset",
    configStatus: "valid",
    activeProvider: "custom",
    declaredProviders: [],
  },
  authentication: {
    loginStatus: "logged_in",
    authFileStatus: "present",
    credentialStore: "file",
  },
  consumers: { desktop: "running", cli: "stopped" },
  versions: { gpteasy: "1.2.1", codexCli: "0.147.0" },
  findings: [{
    code: "model_provider_missing_definition",
    origin: "local",
    severity: "error",
    title: "模型供应商定义缺失",
    summary: "config.toml 使用模型供应商“custom”，但没有声明同名 model_providers 配置。",
    repairable: true,
  }],
  errors: [],
  repairPreview: {
    previewId: "browser-preview",
    source: "gpteasy_backup",
    providerName: "Historical Custom",
    baseUrl: "https://provider.example/v1",
    model: "gpt-5",
    authentication: "current_api_key",
    changes: [
      "backup_config",
      "add_custom_provider_definition",
      "verify_and_rediagnose",
    ],
  },
};

export function getDiagnosticReport(): Promise<DiagnosticReport> {
  if (isBrowserPreview()) return Promise.resolve(browserDiagnosticReport);
  return invoke<DiagnosticReport>("get_diagnostic_report");
}

export function repairDiagnosticCustomProvider(
  previewId: string,
): Promise<DiagnosticRepairExecution> {
  if (isBrowserPreview()) {
    return Promise.resolve({
      status: "not_modified",
      messageId: "diagnostics.repair_not_modified",
      report: browserDiagnosticReport,
    });
  }
  return invoke<DiagnosticRepairExecution>("repair_diagnostic_custom_provider", { previewId });
}

export function analyzeDiagnosticReport(providerId: string): Promise<DiagnosticAssistantResult> {
  if (isBrowserPreview()) {
    return Promise.resolve({
      providerId,
      providerName: "浏览器预览供应商",
      explanation: "当前诊断显示本地配置存在问题。请确认修复预览后再修改配置。",
      repairPlan: browserDiagnosticReport.repairPreview
        ? [{
            id: "repair-custom-provider",
            findingCode: "model_provider_missing_definition",
            title: "补回 custom 供应商定义",
            description: "依据已验证的本机证据补回兼容 provider 定义。",
            action: "repair_custom_provider",
            previewId: browserDiagnosticReport.repairPreview.previewId,
            requiresConfirmation: true,
          }]
        : [],
    });
  }
  return invoke<DiagnosticAssistantResult>("analyze_diagnostic_report", { providerId });
}

export function chooseDiagnosticExportDestination(
  format: DiagnosticExportFormat,
): Promise<string | null> {
  if (isBrowserPreview()) return Promise.resolve(null);
  return invoke<string | null>("choose_diagnostic_export_destination", { format });
}

export function exportDiagnosticReport(
  format: DiagnosticExportFormat,
  destination: string,
): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("export_diagnostic_report", { format, destination });
}

export function getIssueLogPath(): Promise<string> {
  if (isBrowserPreview()) return Promise.resolve("issue-log.jsonl");
  return invoke<string>("get_issue_log_path");
}

export function listIssueLogs(filter: IssueLogFilter): Promise<IssueLogRecord[]> {
  if (isBrowserPreview()) return Promise.resolve([]);
  return invoke<IssueLogRecord[]>("list_issue_logs", {
    sinceEpochSeconds: filter.sinceEpochSeconds,
    level: filter.level,
    query: filter.query.trim() || null,
  });
}

export function copyIssueLogs(filter: IssueLogFilter): Promise<number> {
  if (isBrowserPreview()) return Promise.resolve(0);
  return invoke<number>("copy_issue_logs", {
    sinceEpochSeconds: filter.sinceEpochSeconds,
    level: filter.level,
    query: filter.query.trim() || null,
  });
}

export function chooseIssueLogExportDestination(): Promise<string | null> {
  if (isBrowserPreview()) return Promise.resolve(null);
  return invoke<string | null>("choose_issue_log_export_destination");
}

export function exportIssueLogs(filter: IssueLogFilter, destination: string): Promise<number> {
  if (isBrowserPreview()) return Promise.resolve(0);
  return invoke<number>("export_issue_logs", {
    sinceEpochSeconds: filter.sinceEpochSeconds,
    level: filter.level,
    query: filter.query.trim() || null,
    destination,
  });
}

export function exportAllIssueLogs(destination: string): Promise<number> {
  if (isBrowserPreview()) return Promise.resolve(0);
  return invoke<number>("export_all_issue_logs", { destination });
}
