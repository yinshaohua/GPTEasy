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
  restartState?: "unknown" | "not_started" | "started" | "completed";
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
  if (result.messageId === "model_catalog_refresh.restart_timeout") {
    return "等待 Codex 服务重启超时，模型目录刷新结果尚未确认。请稍后查看 Codex 的模型列表。";
  }
  if (result.messageId === "model_catalog_refresh.timeout") {
    return "等待 Codex 服务恢复运行超时，模型目录刷新结果尚未确认。请稍后查看 Codex 的模型列表。";
  }
  if (result.messageId === "model_catalog_refresh.busy") {
    return "其他模型目录刷新尚未结束，本次未执行刷新。";
  }
  const reasons: Record<string, string> = {
    "model_catalog_refresh.daemon_unsafe": "当前 Codex 服务身份或 CODEX_HOME 无法安全确认。",
    "model_catalog_refresh.protocol_incompatible": "Codex 服务管理协议不兼容。",
    "model_catalog_refresh.connection_failed": "无法连接 Codex 后台服务。",
    "model_catalog_refresh.inspect_failed": "查询 Codex 后台服务状态失败。",
    "model_catalog_refresh.inspect_timeout": "查询 Codex 后台服务状态超时。",
    "model_catalog_refresh.command_failed": "Codex 服务管理命令执行失败。",
  };
  const reason = reasons[result.messageId] ?? "";
  if (result.restartState === "not_started") {
    return (reason || "无法确认 Codex 后台服务状态。") + "未执行模型目录刷新。";
  }
  if (result.restartState === "completed") {
    return "Codex 后台服务重启命令已完成，模型目录刷新结果尚未确认。" + reason;
  }
  if (result.restartState === "started") {
    return "已启动 Codex 后台服务重启命令，模型目录刷新结果尚未确认。" + reason;
  }
  return "模型目录刷新结果尚未确认。" + reason;
}

export function providerCatalogRefreshMessage(result: ModelCatalogRefreshResult): string {
  return "供应商配置已保存。" + catalogRefreshMessage(result);
}
