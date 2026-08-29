import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import SessionVisibilityStatus from "./SessionVisibilityStatus";
import * as sessionContract from "./contracts/session";

vi.mock("./contracts/session", async () => {
  const actual = await vi.importActual<typeof import("./contracts/session")>("./contracts/session");
  return {
    ...actual,
    getSessionVisibilityStatus: vi.fn(),
    listenSessionVisibilityStatus: vi.fn(),
  };
});

describe("自动会话可见性状态", () => {
  beforeEach(() => {
    vi.mocked(sessionContract.getSessionVisibilityStatus).mockReset();
    vi.mocked(sessionContract.listenSessionVisibilityStatus).mockReset();
    vi.mocked(sessionContract.listenSessionVisibilityStatus).mockResolvedValue(() => undefined);
  });

  afterEach(() => cleanup());

  it.each([
    ["pending", "模式已切换，会话可见性将在安全时机自动修复"],
    ["running", "正在自动修复会话可见性"],
    ["partial", "已修复 2 个会话，仍有 1 个待重试"],
    ["blocked", "会话可见性状态无法判定，Codex 重新启动已阻止"],
  ])("独立展示 %s 状态", async (status, message) => {
    vi.mocked(sessionContract.getSessionVisibilityStatus).mockResolvedValue({
      targetMode: "provider",
      modelProvider: "provider-id",
      environmentRevision: "revision",
      status: status as "pending" | "running" | "partial" | "blocked",
      succeeded: 2,
      retryable: 1,
      diagnosticStage: "fixture",
      errorCode: "none",
      updatedAtEpochSeconds: 1,
    });

    render(<SessionVisibilityStatus />);

    const notice = await screen.findByLabelText("会话可见性自动修复状态");
    expect(notice).toHaveTextContent(message);
    expect(notice).toHaveAttribute("role", status === "blocked" ? "alert" : "status");
  });

  it("完整成功清除状态条，事件可以更新部分成功状态", async () => {
    let handler: (status: sessionContract.PendingSessionVisibility | null) => void = () => undefined;
    vi.mocked(sessionContract.getSessionVisibilityStatus).mockResolvedValue(null);
    vi.mocked(sessionContract.listenSessionVisibilityStatus).mockImplementation(async (next) => {
      handler = next;
      return () => undefined;
    });

    render(<SessionVisibilityStatus />);
    await waitFor(() => expect(sessionContract.listenSessionVisibilityStatus).toHaveBeenCalled());
    handler({
      targetMode: "openai_login",
      modelProvider: "openai",
      environmentRevision: "revision",
      status: "partial",
      succeeded: 1,
      retryable: 3,
      diagnosticStage: "verify",
      errorCode: "session_visibility.write_failed",
      updatedAtEpochSeconds: 1,
    });
    expect(await screen.findByLabelText("会话可见性自动修复状态")).toHaveTextContent(
      "已修复 1 个会话，仍有 3 个待重试",
    );
    handler(null);
    await waitFor(() => {
      expect(screen.queryByLabelText("会话可见性自动修复状态")).not.toBeInTheDocument();
    });
  });

  it("首次快照不会覆盖订阅后到达的较新状态事件", async () => {
    let handler: (status: sessionContract.PendingSessionVisibility | null) => void = () => undefined;
    let resolveSnapshot: (status: sessionContract.PendingSessionVisibility | null) => void = () => undefined;
    vi.mocked(sessionContract.listenSessionVisibilityStatus).mockImplementation(async (next) => {
      handler = next;
      return () => undefined;
    });
    vi.mocked(sessionContract.getSessionVisibilityStatus).mockImplementation(
      () => new Promise((resolve) => {
        resolveSnapshot = resolve;
      }),
    );

    render(<SessionVisibilityStatus />);
    await waitFor(() => expect(sessionContract.getSessionVisibilityStatus).toHaveBeenCalled());
    await act(async () => {
      handler({
        targetMode: "provider",
        modelProvider: "provider-id",
        environmentRevision: "new-revision",
        status: "partial",
        succeeded: 2,
        retryable: 1,
        diagnosticStage: "verify",
        errorCode: "session_visibility.write_failed",
        updatedAtEpochSeconds: 2,
      });
      resolveSnapshot({
        targetMode: "provider",
        modelProvider: "provider-id",
        environmentRevision: "old-revision",
        status: "pending",
        succeeded: 0,
        retryable: 0,
        diagnosticStage: "mode_switch",
        errorCode: "none",
        updatedAtEpochSeconds: 1,
      });
    });

    expect(await screen.findByLabelText("会话可见性自动修复状态")).toHaveTextContent(
      "已修复 2 个会话，仍有 1 个待重试",
    );
  });

  it("组件卸载后才完成的事件订阅会立即释放", async () => {
    let resolveListen: (dispose: () => void) => void = () => undefined;
    const dispose = vi.fn();
    vi.mocked(sessionContract.getSessionVisibilityStatus).mockResolvedValue(null);
    vi.mocked(sessionContract.listenSessionVisibilityStatus).mockImplementation(
      () => new Promise((resolve) => {
        resolveListen = resolve;
      }),
    );

    const view = render(<SessionVisibilityStatus />);
    view.unmount();
    resolveListen(dispose);

    await waitFor(() => expect(dispose).toHaveBeenCalledOnce());
    expect(sessionContract.getSessionVisibilityStatus).not.toHaveBeenCalled();
  });

  it("事件订阅不可用时仍回退展示持久快照", async () => {
    vi.mocked(sessionContract.listenSessionVisibilityStatus).mockRejectedValue(
      new Error("event unavailable"),
    );
    vi.mocked(sessionContract.getSessionVisibilityStatus).mockResolvedValue({
      targetMode: "openai_login",
      modelProvider: "openai",
      environmentRevision: "revision",
      status: "pending",
      succeeded: 0,
      retryable: 0,
      diagnosticStage: "mode_switch",
      errorCode: "none",
      updatedAtEpochSeconds: 1,
    });

    render(<SessionVisibilityStatus />);

    expect(await screen.findByLabelText("会话可见性自动修复状态")).toHaveTextContent(
      "模式已切换，会话可见性将在安全时机自动修复",
    );
  });
});
