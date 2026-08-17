import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

import App from "./App";

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

afterEach(() => cleanup());

it("从左侧导航进入真实会话历史列表", async () => {
  listen.mockResolvedValue(() => undefined);
  invoke.mockImplementation((command: string) => {
    if (command === "get_startup_snapshot") {
      return Promise.resolve({
        mode: "ready",
        messageId: "startup.database_initialized",
        blockReason: null,
        pendingOperationResolution: null,
        database: {
          status: "ready",
          schemaVersion: 8,
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
          configStatus: "valid",
          configFingerprint: "fixture",
          credentialStore: "file",
          credentialFileStatus: "present",
          loginStatus: "logged_in",
        },
      });
    }
    if (command === "list_providers") return Promise.resolve([]);
    if (command === "get_environment_snapshot") {
      return Promise.resolve({
        state: "managed",
        mode: "openai_login",
        messageId: "environment.openai_login",
        revision: "openai-login-revision",
        requiresTakeoverConfirmation: false,
        takeoverAvailable: false,
        restoreAvailability: "no_backup",
        loginStatus: "logged_in",
        pendingRestart: false,
        consumers: { desktop: "running", cli: "running" },
        impacts: [],
        currentProvider: null,
      });
    }
    if (command === "list_wsl_environments") return Promise.resolve([]);
    if (command === "enter_session_management") {
      return Promise.resolve({
        status: "available",
        messageId: "session.available",
        codexVersion: "0.147.0",
        mutation: { status: "allowed", messageId: "session.mutations_allowed" },
      });
    }
    if (command === "list_sessions") {
      return Promise.resolve({
        sessions: [{
          id: "thread-1",
          title: "真实历史",
          preview: "来自 App Server 的会话",
          project: "C:\\src\\GPTEasy",
          modelProvider: "history-provider",
          source: "Codex CLI",
          createdAt: 1_786_900_000,
          updatedAt: 1_786_900_300,
        }],
        nextCursor: null,
      });
    }
    return Promise.resolve(undefined);
  });

  render(<App />);
  const initialNavigation = await screen.findByRole("navigation", { name: "主要菜单" });
  const initialOpenAi = within(initialNavigation).getByRole("button", { name: "OpenAI 登录模式" });
  await waitFor(() => expect(initialOpenAi).toBeEnabled());
  expect(initialOpenAi).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(await screen.findByRole("button", { name: "会话管理" }));

  expect(await screen.findByRole("heading", { name: "会话管理" })).toBeInTheDocument();
  expect(await screen.findByRole("button", { name: "打开会话：真实历史" })).toBeInTheDocument();
  const sessionNavigation = screen.getByRole("navigation", { name: "主要菜单" });
  const sessionOpenAi = within(sessionNavigation).getByRole("button", { name: "OpenAI 登录模式" });
  expect(sessionOpenAi).toBeEnabled();
  expect(sessionOpenAi).toHaveAttribute("aria-pressed", "true");
  expect(screen.queryByText(/即将支持/)).not.toBeInTheDocument();
});
