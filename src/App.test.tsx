import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

describe("启动状态", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
  });

  it("向用户说明全新状态已初始化且不会创建 Codex 配置", async () => {
    invoke.mockResolvedValue({
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
    });

    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "本地状态已初始化" }),
    ).toBeInTheDocument();
    expect(screen.getByText("尚未创建")).toBeInTheDocument();
    expect(screen.getByText("已检测到登录")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("get_startup_snapshot");
  });

  it("数据库来自更高版本时只显示阻断状态", async () => {
    invoke.mockResolvedValue({
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
    });

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("无法安全打开本地状态");
    expect(screen.getByText("数据库由更高版本的 GPTEasy 创建，当前版本不会改写它。")).toBeInTheDocument();
    expect(screen.queryByText("Codex 环境")).not.toBeInTheDocument();
  });
});
