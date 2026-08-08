import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";
import type { StartupSnapshot } from "./contracts/startup";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const readySnapshot: StartupSnapshot = {
  mode: "ready",
  messageId: "startup.database_initialized",
  blockReason: null,
  pendingOperationResolution: null,
  database: {
    status: "initialized",
    schemaVersion: 1,
    reason: null,
    contents: {
      providerCount: 0,
      hasLastAppliedState: false,
      hasPendingConfigOperation: false,
      pendingRestart: false,
      pendingConfigOperation: null,
    },
  },
  codex: {
    configStatus: "missing",
    configFingerprint: null,
    credentialStore: "unknown",
    credentialFileStatus: "not_applicable",
    loginStatus: "logged_in",
  },
};

const blockedSnapshot: StartupSnapshot = {
  mode: "blocked",
  messageId: "startup.database_blocked",
  blockReason: "database_unavailable",
  pendingOperationResolution: null,
  database: {
    status: "blocked",
    schemaVersion: null,
    reason: "future_schema",
    contents: null,
  },
  codex: {
    configStatus: "valid",
    configFingerprint: "0123456789abcdef",
    credentialStore: "file",
    credentialFileStatus: "missing",
    loginStatus: "not_logged_in",
  },
};

describe("启动状态", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
  });

  it("向用户说明全新状态已初始化且不会创建 Codex 配置", async () => {
    invoke.mockResolvedValue(readySnapshot);

    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "本地状态已初始化" }),
    ).toBeInTheDocument();
    expect(screen.getByText("尚未创建")).toBeInTheDocument();
    expect(screen.getByText("已检测到登录")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("get_startup_snapshot");
  });

  it("数据库来自更高版本时只显示阻断状态", async () => {
    invoke.mockResolvedValue(blockedSnapshot);

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("无法安全打开本地状态");
    expect(screen.getByText("数据库由更高版本的 GPTEasy 创建，当前版本不会改写它。")).toBeInTheDocument();
    expect(screen.queryByText("Codex 环境")).not.toBeInTheDocument();
  });

  it("阻断页重新检查期间禁止重复请求", async () => {
    let resolveRefresh: (snapshot: StartupSnapshot) => void;
    const refreshPromise = new Promise<StartupSnapshot>((resolve) => {
      resolveRefresh = resolve;
    });
    invoke.mockResolvedValueOnce(blockedSnapshot).mockReturnValueOnce(refreshPromise);

    render(<App />);

    const retryButton = await screen.findByRole("button", { name: "重新检查" });
    fireEvent.click(retryButton);

    expect(retryButton).toBeDisabled();
    fireEvent.click(retryButton);
    expect(invoke).toHaveBeenCalledTimes(2);

    resolveRefresh!(blockedSnapshot);

    await waitFor(() => {
      expect(retryButton).toBeEnabled();
    });
  });

  it("提供键盘跳过入口、具名地标和可聚焦导航", async () => {
    invoke.mockResolvedValue(readySnapshot);

    render(<App />);

    await screen.findByRole("heading", { name: "本地状态已初始化" });

    expect(screen.getByRole("link", { name: "跳转到主要内容" })).toHaveAttribute(
      "href",
      "#main-content",
    );
    expect(screen.getByRole("main", { name: "启动状态" })).toHaveAttribute(
      "id",
      "main-content",
    );
    expect(screen.getByRole("link", { name: "本地状态" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("button", { name: "重新检查状态" })).toHaveAttribute(
      "aria-describedby",
      "refresh-status",
    );
  });

  it("刷新期间向辅助技术公开忙碌状态", async () => {
    let resolveRefresh: (snapshot: StartupSnapshot) => void;
    const refreshPromise = new Promise<StartupSnapshot>((resolve) => {
      resolveRefresh = resolve;
    });
    invoke.mockResolvedValueOnce(readySnapshot).mockReturnValueOnce(refreshPromise);

    render(<App />);

    const refreshButton = await screen.findByRole("button", { name: "重新检查状态" });
    fireEvent.click(refreshButton);

    expect(screen.getByRole("main", { name: "启动状态" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByText("正在重新检查状态")).toHaveAttribute("role", "status");
    expect(refreshButton).toBeDisabled();

    resolveRefresh!(readySnapshot);

    await waitFor(() => {
      expect(screen.getByRole("main", { name: "启动状态" })).toHaveAttribute(
        "aria-busy",
        "false",
      );
    });
  });
});
