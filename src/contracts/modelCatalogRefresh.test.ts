import { beforeEach, expect, it, vi } from "vitest";
import { catalogRefreshMessage, providerCatalogRefreshMessage, refreshModelCatalog } from "./modelCatalogRefresh";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./browser-preview", () => ({ isBrowserPreview: () => false }));
beforeEach(() => invoke.mockReset());
it.each([null, undefined, { status: "future" }])("无效后端响应不会伪造无需刷新：%j", async (value) => {
  invoke.mockResolvedValue(value);
  expect((await refreshModelCatalog()).status).toBe("failed");
  expect(invoke).toHaveBeenCalledWith("refresh_model_catalog");
});

it.each(["restart_timeout", "timeout"])("%s 提示刷新结果未确认，不要求立即重试", (reason) => {
  const message = catalogRefreshMessage({
    operationId: "op", status: "failed", daemon: "unknown", before: null, after: null,
    messageId: `model_catalog_refresh.${reason}`,
  });
  expect(message).toContain("尚未确认");
  expect(message).not.toContain("请点击");
});

it("管理命令执行失败有独立文案", () => {
  const message = catalogRefreshMessage({
    operationId: "op", status: "failed", daemon: "unknown", before: null, after: null,
    messageId: "model_catalog_refresh.command_failed",
  });
  expect(message).toContain("Codex 服务管理命令执行失败");
  expect(message).not.toContain("协议不兼容");
});

it.each([
  ["not_started", "无法连接 Codex 后台服务。未执行模型目录刷新。"],
  ["completed", "Codex 后台服务重启命令已完成，模型目录刷新结果尚未确认。无法连接 Codex 后台服务。"],
] as const)("连接失败时显示重启进度 %s", (restartState, expected) => {
  const result = {
    operationId: "op", status: "failed", daemon: "unknown", before: null, after: null,
    messageId: "model_catalog_refresh.connection_failed", restartState,
  } as const;
  expect(catalogRefreshMessage(result)).toBe(expected);
  expect(providerCatalogRefreshMessage(result)).toBe("供应商配置已保存。" + expected);
});
