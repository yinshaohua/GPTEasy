import { invoke } from "@tauri-apps/api/core";
import { isBrowserPreview } from "./browser-preview";
export type ModelCatalogRefreshStatus = "refreshed" | "not_running" | "failed";
export type ManagedDaemonStatus = "managed" | "unavailable" | "unsafe" | "unknown";
export interface ModelCatalogRefreshResult {
  operationId: string;
  status: ModelCatalogRefreshStatus;
  daemon: ManagedDaemonStatus;
  before: { pid: number | null; version: string | null; cliVersion?: string | null } | null;
  after: { pid: number | null; version: string | null; cliVersion?: string | null } | null;
  messageId: string;
}
export const failedCatalogResult = (): ModelCatalogRefreshResult => ({
  operationId: "frontend-transport-failure", status: "failed", daemon: "unknown",
  before: null, after: null, messageId: "model_catalog_refresh.transport_failed",
});
export async function refreshModelCatalog(): Promise<ModelCatalogRefreshResult> {
  if (isBrowserPreview()) return { operationId: "preview", status: "not_running", daemon: "unavailable", before: null, after: null, messageId: "model_catalog_refresh.unavailable" };
  try {
    const result = await invoke<ModelCatalogRefreshResult>("refresh_model_catalog");
    if (!result || !["refreshed", "not_running", "failed"].includes(result.status)) return failedCatalogResult();
    return result;
  } catch { return failedCatalogResult(); }
}
export function catalogRefreshMessage(result: ModelCatalogRefreshResult): string {
  if (result.status === "refreshed") return "模型目录已刷新。";
  if (result.status === "not_running") return result.daemon === "unavailable"
    ? "未发现可用的 Codex 服务管理入口，无需刷新模型目录。"
    : "当前没有运行中的 Codex 服务，无需刷新模型目录。";
  const reasons: Record<string, string> = {
    "model_catalog_refresh.daemon_unsafe": "当前 Codex 服务身份或 CODEX_HOME 无法安全确认。",
    "model_catalog_refresh.protocol_incompatible": "Codex 服务管理协议不兼容。",
    "model_catalog_refresh.timeout": "等待 Codex 服务恢复运行超时。",
    "model_catalog_refresh.busy": "其他模型目录刷新尚未结束。",
  };
  return "模型目录刷新失败，请点击“刷新模型目录”重试。" + (reasons[result.messageId] ?? "");
}
