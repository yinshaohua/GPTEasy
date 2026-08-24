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
