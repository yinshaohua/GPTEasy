import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

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
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
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

    expect(await screen.findByRole("heading", { name: "供应商管理" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "添加供应商" }));
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
    expect(screen.queryByRole("heading", { name: "供应商目录" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("Example Provider", { selector: ".provider-row-name" })).toBeInTheDocument();
    const saveCall = invoke.mock.calls.find(([command]) => command === "save_verified_provider");
    expect(saveCall?.[1]).toEqual({ validationId: "validation-1", name: "  Example Provider  " });
    expect(JSON.stringify(saveCall?.[1])).not.toContain("secret-provider-key");
  }, 10_000);

  it("按后端进度依次展示 Responses 与工具闭环", async () => {
    let finishValidation: (value: object) => void = () => undefined;
    const validation = new Promise<object>((resolve) => {
      finishValidation = resolve;
    });
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "discover_provider_models") {
        return Promise.resolve({
          normalizedBaseUrl: "https://provider.example/v1",
          models: ["model-a"],
        });
      }
      if (command === "validate_provider") return validation;
      return Promise.resolve(undefined);
    });
    render(<App />);

    await screen.findByRole("heading", { name: "供应商管理" });
    fireEvent.click(screen.getByRole("button", { name: "添加供应商" }));
    await screen.findByLabelText("服务地址");
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://provider.example/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "test-key" } });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    await screen.findByRole("option", { name: "model-a" });
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-a" } });
    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));

    await waitFor(() => expect(listen).toHaveBeenCalled());
    const validationCall = invoke.mock.calls.find(([command]) => command === "validate_provider");
    const requestId = validationCall?.[1]?.requestId as string;
    const progressListener = listen.mock.calls[0][1] as (event: {
      payload: { requestId: string; stage: string };
    }) => void;
    act(() => {
      progressListener({ payload: { requestId, stage: "responses_stream" } });
    });
    expect(screen.getByText("Responses 流式响应").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
    expect(screen.getByText("工具调用闭环").closest("li")).not.toHaveAttribute("aria-current");

    act(() => {
      progressListener({ payload: { requestId, stage: "tool_round_trip" } });
    });
    expect(screen.getByText("工具调用闭环").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
    act(() => {
      finishValidation({
        validationId: "validation-progress",
        normalizedBaseUrl: "https://provider.example/v1",
        defaultModel: "model-a",
        combinationFingerprint: "b".repeat(64),
        verifiedAtEpochSeconds: 1_786_140_000,
      });
    });
    expect(await screen.findByText("完整验证已通过")).toBeInTheDocument();
  }, 10_000);

  it("离开供应商页会丢弃未保存输入", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    render(<App />);

    await screen.findByRole("heading", { name: "供应商管理" });
    fireEvent.click(screen.getByRole("button", { name: "添加供应商" }));
    const name = await screen.findByLabelText("供应商名称");
    fireEvent.change(name, { target: { value: "Unsaved" } });
    fireEvent.click(screen.getByRole("button", { name: "Codex 环境" }));
    expect(await screen.findByRole("heading", { name: "Codex 环境" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "供应商管理" }));

    expect(await screen.findByRole("heading", { name: "供应商目录" })).toBeInTheDocument();
    expect(screen.queryByLabelText("供应商名称")).not.toBeInTheDocument();
  });

  it("未验证的新供应商不能绕过验证保存", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "discover_provider_models") {
        return Promise.resolve({ normalizedBaseUrl: "https://provider.example/v1", models: ["model-a"] });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "添加供应商" }));
    fireEvent.change(screen.getByLabelText("供应商名称"), { target: { value: "Example" } });
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://provider.example/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    await screen.findByRole("option", { name: "model-a" });
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-a" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(screen.getByRole("dialog", { name: "需要验证供应商" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开始验证" })).toBeEnabled();
    expect(invoke.mock.calls.some(([command]) => command === "save_verified_provider")).toBe(false);
  });

  it("返回详情时保留继续编辑选择，放弃后不持久化候选配置", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "添加供应商" }));
    fireEvent.change(screen.getByLabelText("供应商名称"), { target: { value: "Unsaved" } });
    fireEvent.click(screen.getByRole("button", { name: "返回" }));

    expect(screen.getByRole("dialog", { name: "放弃未保存修改？" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));
    expect(screen.getByLabelText("供应商名称")).toHaveValue("Unsaved");
    fireEvent.click(screen.getByRole("button", { name: "返回" }));
    fireEvent.click(screen.getByRole("button", { name: "放弃修改" }));
    expect(await screen.findByRole("heading", { name: "供应商目录" })).toBeInTheDocument();
    expect(screen.queryByText("Unsaved")).not.toBeInTheDocument();
  });
});

describe("验收凭据泄漏门禁", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("[acceptance-leak-gate] 普通截图辅助和确认通知不包含 API Key", async () => {
    const apiKeyCanary =
      import.meta.env.VITE_GPTEASY_ACCEPTANCE_KEY_A ?? `gpteasy-ui-${crypto.randomUUID()}`;
    const provider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "reveal_provider_api_key") {
        return Promise.resolve({ value: apiKeyCanary });
      }
      return Promise.resolve(undefined);
    });
    const notifications: string[] = [];
    vi.spyOn(window, "confirm").mockImplementation((message) => {
      notifications.push(String(message));
      return false;
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "修改 Atlas" }));
    const apiKey = screen.getByLabelText("API Key") as HTMLInputElement;
    expect(apiKey).toHaveAttribute("type", "password");
    expect(apiKey).toHaveValue("");

    const screenshotAssist = `${document.body.textContent}\n${document.documentElement.outerHTML}`;
    fireEvent.click(screen.getByRole("button", { name: "返回" }));
    fireEvent.click(screen.getByRole("button", { name: "删除 Atlas" }));
    expect(notifications).toHaveLength(1);
    expect(screenshotAssist).not.toContain(apiKeyCanary);
    expect(notifications.join("\n")).not.toContain(apiKeyCanary);
    expect(invoke.mock.calls.some(([command]) => command === "reveal_provider_api_key")).toBe(
      false,
    );
  });
});

describe("供应商目录生命周期", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("目录仅通过显式操作切换，并同时显示验证与当前状态", async () => {
    const current = {
      id: "76149f67-0d76-4d41-b606-77ba244bffec",
      name: "Current Provider",
      baseUrl: "https://current.example/v1",
      defaultModel: "model-current",
      verifiedAtEpochSeconds: 1_786_140_100,
      isCurrent: true,
    };
    const target = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const environment = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "revision-1",
      requiresTakeoverConfirmation: false,
      requiresConsumerConfirmation: false,
      impacts: [],
      currentProvider: current,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "stopped", cli: "stopped" },
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([target, current]);
      if (command === "get_environment_snapshot") return Promise.resolve(environment);
      if (command === "apply_environment_provider") {
        return Promise.resolve({ ...environment, currentProvider: target, revision: "revision-2" });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "供应商目录" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /会话管理/ })).toBeDisabled();
    expect(await screen.findAllByText("已验证")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Current Provider 当前使用" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Atlas" })).not.toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "切换到 Atlas" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: target.id,
        confirmSwitchRisk: false,
        expectedRevision: "revision-1",
      });
    });
    expect(screen.getByRole("button", { name: "Atlas 当前使用" })).toBeDisabled();
  });

  it("从详情安全查看凭据，并覆盖改名、重验证和删除限制", async () => {
    const first = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const current = {
      id: "76149f67-0d76-4d41-b606-77ba244bffec",
      name: "Current Provider",
      baseUrl: "https://current.example/v1",
      defaultModel: "model-current",
      verifiedAtEpochSeconds: 1_786_140_100,
      isCurrent: true,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([first, current]);
      if (command === "reveal_provider_api_key") {
        return Promise.resolve({ value: "catalog-secret-key" });
      }
      if (command === "copy_provider_api_key") return Promise.resolve(undefined);
      if (command === "rename_provider") {
        return Promise.resolve({ ...first, name: "Atlas Renamed" });
      }
      if (command === "revalidate_provider") {
        return Promise.resolve({ ...first, name: "Atlas Renamed", verifiedAtEpochSeconds: 1_786_140_500 });
      }
      if (command === "delete_provider") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);

    fireEvent.click(
      await screen.findByRole("button", { name: "修改 Atlas" }, { timeout: 5_000 }),
    );
    expect(screen.getByRole("heading", { name: "修改 Atlas" })).toBeInTheDocument();
    const apiKey = screen.getByLabelText("API Key") as HTMLInputElement;
    expect(apiKey).toHaveAttribute("type", "password");
    expect(apiKey).toHaveValue("");

    fireEvent.click(screen.getByRole("button", { name: "显示 API Key" }));
    await waitFor(() => expect(apiKey).toHaveValue("catalog-secret-key"));
    expect(apiKey).toHaveAttribute("type", "text");
    expect(invoke).toHaveBeenCalledWith("reveal_provider_api_key", { providerId: first.id });
    fireEvent.click(screen.getByRole("button", { name: "复制 API Key" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("copy_provider_api_key", { providerId: first.id });
    });

    fireEvent.change(screen.getByLabelText("供应商名称"), {
      target: { value: "  Atlas Renamed  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("rename_provider", {
        providerId: first.id,
        name: "  Atlas Renamed  ",
      });
    });
    expect(invoke.mock.calls.some(([command]) => command === "validate_provider_update")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "验证 Atlas Renamed" }));
    await waitFor(() => {
      expect(invoke.mock.calls.some(([command]) => command === "revalidate_provider")).toBe(true);
    });
    fireEvent.click(screen.getByRole("button", { name: "删除 Atlas Renamed" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("delete_provider", { providerId: first.id });
    });
    expect(screen.queryByRole("button", { name: "修改 Atlas Renamed" })).not.toBeInTheDocument();

    expect(screen.getAllByText("当前使用")).not.toHaveLength(0);
    expect(screen.getByRole("button", { name: "删除 Current Provider" })).toBeDisabled();
  }, 10_000);

  it("非当前供应商关键字段在重新验证前不会更新目录", async () => {
    const provider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "discover_provider_models_for_update") {
        return Promise.resolve({
          normalizedBaseUrl: "https://atlas.example/next/v1",
          models: ["model-b"],
        });
      }
      if (command === "validate_provider_update") {
        return Promise.resolve({
          validationId: "update-validation",
          normalizedBaseUrl: "https://atlas.example/next/v1",
          defaultModel: "model-b",
          combinationFingerprint: "c".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_600,
        });
      }
      if (command === "save_provider_update") {
        return Promise.resolve({
          ...provider,
          baseUrl: "https://atlas.example/next/v1",
          defaultModel: "model-b",
          verifiedAtEpochSeconds: 1_786_140_600,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(
      await screen.findByRole("button", { name: "修改 Atlas" }, { timeout: 5_000 }),
    );
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://atlas.example/next/v1" },
    });
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("dialog", { name: "需要验证供应商" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));
    expect(invoke.mock.calls.some(([command]) => command === "save_provider_update")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    expect(await screen.findByRole("option", { name: "model-b" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-b" } });
    fireEvent.click(screen.getByRole("button", { name: "验证更新" }));
    expect(await screen.findByText("完整验证已通过")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "save_provider_update")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_provider_update", {
        validationId: "update-validation",
        providerId: provider.id,
        name: "Atlas",
      });
    });
    const discoveryCall = invoke.mock.calls.find(
      ([command]) => command === "discover_provider_models_for_update",
    );
    expect(discoveryCall?.[1]?.input).toEqual({
      providerId: provider.id,
      baseUrl: "https://atlas.example/next/v1",
      apiKey: null,
    });
  }, 10_000);

  it("当前供应商关键字段更新调用完整保存并应用用例", async () => {
    const current = {
      id: "76149f67-0d76-4d41-b606-77ba244bffec",
      name: "Current Provider",
      baseUrl: "https://current.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_100,
      isCurrent: true,
    };
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([current]);
      if (command === "discover_provider_models_for_update") {
        return Promise.resolve({
          normalizedBaseUrl: "https://current.example/next/v1",
          models: ["model-b"],
        });
      }
      if (command === "validate_provider_update") {
        return Promise.resolve({
          validationId: "current-update-validation",
          normalizedBaseUrl: "https://current.example/next/v1",
          defaultModel: "model-b",
          combinationFingerprint: "d".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_700,
        });
      }
      if (command === "save_and_apply_provider_update") {
        if (!args?.confirmConsumerRisk) {
          return Promise.reject({
            category: "save_and_apply_failed",
            messageId: "environment.consumer_confirmation_required",
          });
        }
        return Promise.resolve({
          ...current,
          baseUrl: "https://current.example/next/v1",
          defaultModel: "model-b",
          verifiedAtEpochSeconds: 1_786_140_700,
        });
      }
      return Promise.resolve(undefined);
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(
      await screen.findByRole(
        "button",
        { name: "修改 Current Provider" },
        { timeout: 5_000 },
      ),
    );
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://current.example/next/v1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    await screen.findByRole("option", { name: "model-b" });
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-b" } });
    fireEvent.click(screen.getByRole("button", { name: "验证更新" }));

    const saveAndApply = await screen.findByRole("button", { name: "保存并应用" });
    fireEvent.click(saveAndApply);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_and_apply_provider_update", {
        validationId: "current-update-validation",
        providerId: current.id,
        name: current.name,
        confirmConsumerRisk: false,
      });
      expect(invoke).toHaveBeenCalledWith("save_and_apply_provider_update", {
        validationId: "current-update-validation",
        providerId: current.id,
        name: current.name,
        confirmConsumerRisk: true,
      });
    });
    expect(confirm).toHaveBeenCalledOnce();
    expect(
      invoke.mock.calls.some(([command]) => command === "save_provider_update"),
    ).toBe(false);
  }, 10_000);
});

describe("Codex 环境接管", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("先展示替换范围，再由用户确认接管外部配置", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Applied Provider",
      baseUrl: "https://applied.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    const external = {
      state: "external",
      messageId: "environment.external",
      revision: "external-revision",
      requiresTakeoverConfirmation: true,
      impacts: [
        {
          artifact: "config",
          action: "create",
          fields: ["model", "model_provider", "model_providers.<provider-id>"],
        },
        {
          artifact: "credentials",
          action: "create",
          fields: ["auth_mode", "OPENAI_API_KEY"],
        },
      ],
      currentProvider: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(external);
      if (command === "apply_environment_provider") {
        return Promise.resolve({
          ...external,
          state: "managed",
          messageId: "environment.managed",
          requiresTakeoverConfirmation: false,
          currentProvider: { ...provider, isCurrent: true },
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));

    expect(await screen.findByRole("heading", { name: "Codex 环境" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "外部配置" })).toBeInTheDocument();
    expect(screen.getByText("config.toml")).toBeInTheDocument();
    expect(screen.getByText("auth.json")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("要应用的供应商"), {
      target: { value: provider.id },
    });
    fireEvent.click(screen.getByRole("button", { name: "确认接管并应用" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        confirmSwitchRisk: true,
        expectedRevision: "external-revision",
      });
    });
    expect(await screen.findByText("当前供应商：Applied Provider")).toBeInTheDocument();
  });

  it("允许用户确认后重新接管管理冲突", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Recovery Provider",
      baseUrl: "https://recovery.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    const conflict = {
      state: "conflict",
      messageId: "environment.managed_conflict",
      revision: "conflict-revision",
      requiresTakeoverConfirmation: true,
      impacts: [
        {
          artifact: "config",
          action: "update",
          fields: ["model", "model_provider", "model_providers.<provider-id>"],
        },
        {
          artifact: "credentials",
          action: "update",
          fields: ["auth_mode", "OPENAI_API_KEY"],
        },
      ],
      currentProvider: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(conflict);
      if (command === "apply_environment_provider") {
        return Promise.resolve({
          ...conflict,
          state: "managed",
          messageId: "environment.managed",
          requiresTakeoverConfirmation: false,
          currentProvider: { ...provider, isCurrent: true },
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));
    fireEvent.change(await screen.findByLabelText("要应用的供应商"), {
      target: { value: provider.id },
    });
    const takeover = screen.getByRole("button", { name: "确认接管并应用" });
    expect(takeover).toBeEnabled();
    fireEvent.click(takeover);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        confirmSwitchRisk: true,
        expectedRevision: "conflict-revision",
      });
    });
  });

  it("只在最近配置可安全恢复时允许用户确认恢复", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Applied Provider",
      baseUrl: "https://applied.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: true,
    };
    const managed = {
      state: "managed",
      messageId: "environment.managed",
      revision: "managed-revision",
      requiresTakeoverConfirmation: false,
      restoreAvailability: "available",
      impacts: [
        {
          artifact: "config",
          action: "update",
          fields: ["model", "model_provider", "model_providers.<provider-id>"],
        },
        {
          artifact: "credentials",
          action: "update",
          fields: ["auth_mode", "OPENAI_API_KEY"],
        },
      ],
      currentProvider: provider,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(managed);
      if (command === "restore_last_environment_config") {
        return Promise.resolve({
          ...managed,
          state: "external",
          messageId: "environment.external",
          revision: "restored-revision",
          currentProvider: null,
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));
    const restore = await screen.findByRole("button", { name: "恢复上次配置" });
    expect(restore).toBeEnabled();
    fireEvent.click(restore);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("restore_last_environment_config", {
        confirmRestore: true,
        expectedRevision: "managed-revision",
      });
    });
    expect(await screen.findByRole("heading", { name: "外部配置" })).toBeInTheDocument();
  });

  it("受管工件外部变化后禁用恢复并说明原因", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "managed",
          messageId: "environment.managed",
          revision: "changed-revision",
          requiresTakeoverConfirmation: false,
          restoreAvailability: "artifacts_changed",
          impacts: [],
          currentProvider: null,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));

    expect(await screen.findByRole("button", { name: "恢复上次配置" })).toBeDisabled();
    expect(screen.getByText("受管工件在最近一次修改后发生变化，恢复已禁用。")).toBeInTheDocument();
  });

  it("展示认证与消费者状态，并只从环境页确认切换到 OpenAI 登录模式", async () => {
    const external = {
      state: "external",
      mode: null,
      messageId: "environment.external",
      revision: "openai-ready-revision",
      requiresTakeoverConfirmation: true,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: true,
      consumers: {
        desktop: "unknown",
        cli: "unknown",
      },
      impacts: [],
      currentProvider: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_environment_snapshot") return Promise.resolve(external);
      if (command === "switch_to_openai_login") {
        return Promise.resolve({
          ...external,
          state: "managed",
          mode: "openai_login",
          messageId: "environment.openai_login",
          revision: "openai-active-revision",
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));

    expect(await screen.findByText("外部配置", { selector: "dd" })).toBeInTheDocument();
    expect(screen.getByText("桌面 Codex").nextElementSibling).toHaveTextContent("无法确认");
    expect(screen.getByText("Codex CLI").nextElementSibling).toHaveTextContent("无法确认");
    expect(screen.getByText("待重启").nextElementSibling).toHaveTextContent("需要重启消费者");
    fireEvent.click(screen.getByRole("button", { name: "切换到 OpenAI 登录模式" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("switch_to_openai_login", {
        confirmSwitch: true,
        expectedRevision: "openai-ready-revision",
      });
    });
    expect(await screen.findByText("OpenAI 登录模式", { selector: "dd" })).toBeInTheDocument();
  });

  it("登录缺失或不可判断时解释原因且不发起 OpenAI 模式写入", async () => {
    for (const [loginStatus, message] of [
      ["not_logged_in", "请先在 Codex 中完成 OpenAI 登录。"],
      ["unavailable", "无法确认 Codex 登录状态，已阻止切换。"],
    ] as const) {
      cleanup();
      invoke.mockClear();
      invoke.mockImplementation((command: string) => {
        if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
        if (command === "list_providers") return Promise.resolve([]);
        if (command === "get_environment_snapshot") {
          return Promise.resolve({
            state: "external",
            mode: null,
            messageId: "environment.external",
            revision: `blocked-${loginStatus}`,
            requiresTakeoverConfirmation: true,
            restoreAvailability: "no_backup",
            loginStatus,
            pendingRestart: false,
            consumers: { desktop: "unknown", cli: "unknown" },
            impacts: [],
            currentProvider: null,
          });
        }
        return Promise.resolve(undefined);
      });

      render(<App />);
      fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));

      expect(await screen.findByText(message)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "切换到 OpenAI 登录模式" })).toBeDisabled();
      expect(invoke.mock.calls.some(([command]) => command === "switch_to_openai_login")).toBe(
        false,
      );
    }
  });

  it("外部注销后保留 OpenAI 模式，并在确认后返回已验证供应商", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Return Provider",
      baseUrl: "https://return.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    const openai = {
      state: "managed",
      mode: "openai_login",
      messageId: "environment.openai_login_missing",
      revision: "logged-out-openai-revision",
      requiresTakeoverConfirmation: true,
      restoreAvailability: "no_backup",
      loginStatus: "not_logged_in",
      pendingRestart: false,
      consumers: { desktop: "unknown", cli: "unknown" },
      impacts: [],
      currentProvider: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(openai);
      if (command === "apply_environment_provider") {
        return Promise.resolve({
          ...openai,
          mode: "provider",
          messageId: "environment.managed",
          revision: "provider-revision",
          requiresTakeoverConfirmation: false,
          currentProvider: { ...provider, isCurrent: true },
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Codex 环境" }));

    expect(await screen.findByText("OpenAI 登录已在外部失效；当前模式保持不变。")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "确认返回供应商模式" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        confirmSwitchRisk: true,
        expectedRevision: "logged-out-openai-revision",
      });
    });
  });
});

describe("启动状态", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("向用户说明全新状态已初始化且不会创建 Codex 配置", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    await screen.findByRole("heading", { name: "供应商管理" });
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
