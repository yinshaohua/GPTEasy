import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import DesktopControl from "./DesktopControl";
import { catalogRefreshMessage, type ModelCatalogRefreshResult, type ModelCatalogRefreshStatus } from "./contracts/modelCatalogRefresh";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const snapshot = { status: "running", action: "restart", messageId: "desktop.running", roots: [{ role: "desktop", pid: 42, startedAtEpochMillis: 123 }] };
const result = (status: ModelCatalogRefreshStatus): ModelCatalogRefreshResult => ({ operationId: "test", status, daemon: "managed", before: null, after: null, messageId: "model_catalog_refresh.test" });
async function confirmRestart() {
  fireEvent.click(await screen.findByRole("button", { name: "重启 Codex" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("桌面进程及其中任务");
  fireEvent.click(screen.getByRole("button", { name: "确认重启" }));
}
describe("模型目录刷新与桌面重启隔离", () => {
  afterEach(cleanup);
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((command: string) => command === "get_desktop_snapshot" ? Promise.resolve(snapshot) : Promise.reject(new Error("unexpected command")));
  });
  it.each(["refreshed", "not_running", "failed"] as const)("手动刷新报告 %s，仍可再次补偿且不重启桌面", async (status) => {
    invoke.mockImplementation((command: string) => Promise.resolve(command === "get_desktop_snapshot" ? snapshot : result(status)));
    render(<DesktopControl />);
    await screen.findByRole("button", { name: "重启 Codex" });
    fireEvent.click(screen.getByRole("button", { name: "刷新模型目录" }));
    expect(await screen.findByText(catalogRefreshMessage(result(status)))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "刷新模型目录" })).toBeEnabled();
    expect(invoke).toHaveBeenCalledWith("refresh_model_catalog");
    expect(invoke.mock.calls.some(([command]) => command === "restart_desktop_application")).toBe(false);
  });
  it("传输失败显示失败而非无需刷新，允许补偿重试", async () => {
    render(<DesktopControl />);
    await screen.findByRole("button", { name: "重启 Codex" });
    fireEvent.click(screen.getByRole("button", { name: "刷新模型目录" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("模型目录刷新失败");
    invoke.mockImplementation((command: string) => Promise.resolve(command === "get_desktop_snapshot" ? snapshot : result("refreshed")));
    fireEvent.click(screen.getByRole("button", { name: "刷新模型目录" }));
    expect(await screen.findByText("模型目录已刷新。")).toBeInTheDocument();
  });
  for (const status of ["refreshed", "not_running", "failed"] as const) {
    it.each([true, false])(`重启分别保留目录 ${status} 与桌面成功=%s`, async (success) => {
      invoke.mockImplementation((command: string) => {
        if (command === "get_desktop_snapshot") return Promise.resolve(snapshot);
        if (command === "restart_desktop_application") return success
          ? Promise.resolve({ ...snapshot, modelCatalogRefresh: result(status) })
          : Promise.reject({ category: "state_unavailable", messageId: "desktop.state_unavailable", modelCatalogRefresh: result(status) });
        throw new Error("must not issue duplicate refresh");
      });
      render(<DesktopControl />); await confirmRestart();
      expect(await screen.findByText(catalogRefreshMessage(result(status)))).toBeInTheDocument();
      if (success) expect(screen.getByText("Codex 已重新启动。")).toBeInTheDocument();
      else expect(screen.getByRole("button", { name: "刷新模型目录" })).toBeEnabled();
      expect(invoke).toHaveBeenCalledWith("restart_desktop_application", { expectedRoots: snapshot.roots });
      expect(invoke.mock.calls.some(([command]) => command === "refresh_model_catalog")).toBe(false);
    });
  }
  it("手动刷新进行中仍可确认重启，旧手动结果不覆盖重启结果", async () => {
    let finish!: (value: ModelCatalogRefreshResult) => void;
    invoke.mockImplementation((command: string) => {
      if (command === "get_desktop_snapshot") return Promise.resolve(snapshot);
      if (command === "refresh_model_catalog") return new Promise<ModelCatalogRefreshResult>((resolve) => { finish = resolve; });
      return Promise.resolve({ ...snapshot, modelCatalogRefresh: result("refreshed") });
    });
    render(<DesktopControl />); await screen.findByRole("button", { name: "重启 Codex" });
    fireEvent.click(screen.getByRole("button", { name: "刷新模型目录" }));
    expect(screen.getByRole("button", { name: "正在刷新模型目录" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "重启 Codex" })).toBeEnabled();
    await confirmRestart(); await screen.findByText("Codex 已重新启动。");
    await act(async () => { finish(result("failed")); });
    expect(screen.getByText("模型目录已刷新。")).toBeInTheDocument();
    expect(screen.queryByText(/模型目录刷新失败/)).not.toBeInTheDocument();
  });
});
