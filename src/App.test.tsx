import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const readySnapshot = {
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

describe("供应商创建", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
  });

  it("完整验证后仍需用户明确保存，且保存边界不接收 API Key", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "discover_provider_models") {
        return Promise.resolve({
          normalizedBaseUrl: "https://provider.example/api/v1",
          models: ["model-a", "model-b"],
        });
      }
      if (command === "validate_provider") {
        return Promise.resolve({
          validationId: "validation-1",
          normalizedBaseUrl: "https://provider.example/api/v1",
          defaultModel: "model-b",
          combinationFingerprint: "a".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_000,
        });
      }
      if (command === "save_verified_provider") {
        return Promise.resolve({
          id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
          name: "Example Provider",
          baseUrl: "https://provider.example/api/v1",
          defaultModel: "model-b",
          verifiedAtEpochSeconds: 1_786_140_000,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "供应商" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("供应商名称"), {
      target: { value: "  Example Provider  " },
    });
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://provider.example/api/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "secret-provider-key" },
    });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));

    expect(await screen.findByRole("option", { name: "model-b" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-b" } });
    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));

    expect(await screen.findByText("完整验证已通过")).toBeInTheDocument();
    expect(screen.queryByText("Example Provider", { selector: ".provider-list-row strong" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("Example Provider", { selector: ".provider-list-row strong" })).toBeInTheDocument();
    const saveCall = invoke.mock.calls.find(([command]) => command === "save_verified_provider");
    expect(saveCall?.[1]).toEqual({ validationId: "validation-1", name: "  Example Provider  " });
    expect(JSON.stringify(saveCall?.[1])).not.toContain("secret-provider-key");
  }, 10_000);

  it("离开供应商页会丢弃未保存输入", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<App />);

    const name = await screen.findByLabelText("供应商名称");
    fireEvent.change(name, { target: { value: "Unsaved" } });
    fireEvent.click(screen.getByRole("button", { name: "Codex 环境" }));
    expect(await screen.findByRole("heading", { name: "启动状态" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "供应商" }));

    expect(await screen.findByLabelText("供应商名称")).toHaveValue("");
  });
});

describe("启动状态", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
  });

  it("向用户说明全新状态已初始化且不会创建 Codex 配置", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    await screen.findByRole("heading", { name: "供应商" });
    fireEvent.click(screen.getByRole("button", { name: "Codex 环境" }));

    expect(await screen.findByRole("heading", { name: "本地状态已初始化" })).toBeInTheDocument();
    expect(screen.getByText("尚未创建")).toBeInTheDocument();
    expect(screen.getByText("已检测到登录")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("get_startup_snapshot");
  });

  it("数据库来自更高版本时只显示阻断状态", async () => {
    invoke.mockResolvedValue({
      ...readySnapshot,
      mode: "blocked",
      messageId: "startup.database_blocked",
      blockReason: "database_unavailable",
      database: {
        status: "blocked",
        schemaVersion: null,
        reason: "future_schema",
        contents: null,
      },
    });

    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("无法安全打开本地状态");
    expect(screen.getByText("数据库由更高版本的 GPTEasy 创建，当前版本不会改写它。")).toBeInTheDocument();
    expect(screen.queryByText("Codex 环境")).not.toBeInTheDocument();
  });
});
