import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import AppSidebar, { type UpdateSidebarState } from "./AppSidebar";
import { initialUpdateSnapshot, type UpdateSnapshot } from "./contracts/update";

function updateState(
  snapshot: Partial<UpdateSnapshot>,
  overrides: Partial<UpdateSidebarState> = {},
): UpdateSidebarState {
  return {
    snapshot: { ...initialUpdateSnapshot, availableVersion: "1.1.0", ...snapshot },
    installing: false,
    onOpen: vi.fn(),
    ...overrides,
  };
}

describe("侧栏应用更新入口", () => {
  afterEach(cleanup);

  it("下载时展示百分比并从侧栏打开详情", () => {
    const update = updateState({ state: "downloading", progressPercent: 42 });
    render(<AppSidebar currentProviderName="custom" update={update} />);

    const indicator = screen.getByRole("button", { name: "正在下载更新 42%" });
    expect(indicator).toHaveTextContent("42%");
    fireEvent.click(indicator);

    expect(update.onOpen).toHaveBeenCalledOnce();
  });

  it("未知下载进度使用不定进度状态", () => {
    const update = updateState({ state: "downloading", progressPercent: null });
    render(<AppSidebar update={update} />);

    const indicator = screen.getByRole("button", { name: "正在下载更新" });
    expect(indicator.querySelector(".is-spinning")).toBeInTheDocument();
    expect(indicator).toHaveTextContent("下载中");
  });

  it("待安装更新点击后打开确认详情而不直接安装", () => {
    const update = updateState({ state: "pending", progressPercent: 100 });
    render(<AppSidebar update={update} />);

    const indicator = screen.getByRole("button", { name: "更新" });
    expect(indicator.querySelector(".lucide-download")).toBeInTheDocument();
    expect(indicator).toHaveTextContent("更新");
    fireEvent.click(indicator);

    expect(update.onOpen).toHaveBeenCalledOnce();
  });

  it("始终显示当前版本并点击打开更新详情", () => {
    const update = updateState({ state: "idle", availableVersion: null, currentVersion: "1.1.1" });
    render(<AppSidebar update={update} />);

    const version = screen.getByRole("button", { name: "当前版本 v1.1.1" });
    expect(version).toHaveTextContent("当前版本");
    expect(version).toHaveTextContent("v1.1.1");
    fireEvent.click(version);
    expect(update.onOpen).toHaveBeenCalledOnce();
  });

  it("未完成更新和已发现版本的失败状态只打开详情", () => {
    const incomplete = updateState({ state: "incomplete" });
    const { rerender } = render(<AppSidebar update={incomplete} />);

    fireEvent.click(screen.getByRole("button", { name: "重试更新" }));
    expect(incomplete.onOpen).toHaveBeenCalledOnce();

    const failed = updateState({
      state: "failed",
      failureCategory: "download_failed",
      errorMessage: "应用更新下载失败，请重试。",
    });
    rerender(<AppSidebar update={failed} />);
    fireEvent.click(screen.getByRole("button", { name: "更新失败" }));
    expect(failed.onOpen).toHaveBeenCalledOnce();
  });

  it("检查中和没有目标版本的检查失败仍显示固定状态入口", () => {
    const checking = updateState({ state: "checking", availableVersion: null });
    const { rerender } = render(<AppSidebar update={checking} />);
    expect(screen.getByRole("button", { name: "正在检查更新" })).toBeInTheDocument();

    const failed = updateState({ state: "failed", availableVersion: null, failureCategory: "check_failed" });
    rerender(<AppSidebar update={failed} />);
    expect(screen.getByRole("button", { name: "更新失败" })).toBeInTheDocument();
  });

  it("安装交接期间禁用入口并显示工作状态", () => {
    const update = updateState({ state: "pending" }, { installing: true });
    render(<AppSidebar update={update} />);

    const indicator = screen.getByRole("button", { name: "正在启动更新" });
    expect(indicator).toBeDisabled();
    expect(indicator.querySelector(".is-spinning")).toBeInTheDocument();
    expect(indicator).toHaveTextContent("处理中");
  });
});
