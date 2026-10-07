import { beforeEach, expect, it, vi } from "vitest";
import { refreshModelCatalog } from "./modelCatalogRefresh";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("./browser-preview", () => ({ isBrowserPreview: () => false }));
beforeEach(() => invoke.mockReset());
it.each([null, undefined, { status: "future" }])("无效后端响应不会伪造无需刷新：%j", async (value) => {
  invoke.mockResolvedValue(value);
  expect((await refreshModelCatalog()).status).toBe("failed");
  expect(invoke).toHaveBeenCalledWith("refresh_model_catalog");
});
