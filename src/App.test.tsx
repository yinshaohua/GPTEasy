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

    expect(await screen.findByText("验证通过")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "供应商目录" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("Example Provider", { selector: ".provider-row-name" })).toBeInTheDocument();
    const saveCall = invoke.mock.calls.find(([command]) => command === "save_verified_provider");
    expect(saveCall?.[1]).toEqual({ validationId: "validation-1", name: "  Example Provider  " });
    expect(JSON.stringify(saveCall?.[1])).not.toContain("secret-provider-key");
  }, 10_000);

  it("候选地址完整验证后由用户确认，拒绝不会回填或保存", async () => {
    let validationIndex = 0;
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "discover_provider_models") {
        return Promise.resolve({
          requestedBaseUrl: "https://provider.example/api",
          normalizedBaseUrl: "https://provider.example/api/v1",
          models: ["candidate-model"],
        });
      }
      if (command === "validate_provider") {
        validationIndex += 1;
        return Promise.resolve({
          validationId: `candidate-validation-${validationIndex}`,
          requestedBaseUrl: "https://provider.example/api",
          normalizedBaseUrl: "https://provider.example/api/v1",
          defaultModel: "candidate-model",
          combinationFingerprint: "c".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_000,
        });
      }
      if (command === "save_verified_provider") {
        return Promise.resolve({
          id: "candidate-provider",
          name: "Candidate Provider",
          baseUrl: "https://provider.example/api/v1",
          defaultModel: "candidate-model",
          verifiedAtEpochSeconds: 1_786_140_000,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "添加供应商" }));
    fireEvent.change(screen.getByLabelText("供应商名称"), {
      target: { value: "Candidate Provider" },
    });
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://provider.example/api" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));

    expect(await screen.findByRole("option", { name: "candidate-model" })).toBeInTheDocument();
    expect(screen.getByLabelText("服务地址")).toHaveValue("https://provider.example/api");
    fireEvent.change(screen.getByLabelText("默认模型"), {
      target: { value: "candidate-model" },
    });
    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));
    fireEvent.click(await screen.findByRole("button", { name: "完成" }));

    expect(screen.getByRole("dialog", { name: "建议修正服务地址" })).toHaveTextContent(
      "https://provider.example/api",
    );
    expect(screen.getByRole("dialog", { name: "建议修正服务地址" })).toHaveTextContent(
      "https://provider.example/api/v1",
    );
    fireEvent.click(screen.getByRole("button", { name: "保留原地址" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("discard_provider_validation", {
      validationId: "candidate-validation-1",
    }));
    expect(screen.getByLabelText("服务地址")).toHaveValue("https://provider.example/api");
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("dialog", { name: "需要验证供应商" })).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "save_verified_provider")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));

    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));
    fireEvent.click(await screen.findByRole("button", { name: "完成" }));
    fireEvent.click(screen.getByRole("button", { name: "采用建议地址" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "confirm_provider_validation_base_url",
      {
        validationId: "candidate-validation-2",
        baseUrl: "https://provider.example/api/v1",
      },
    ));
    expect(screen.getByLabelText("服务地址")).toHaveValue("https://provider.example/api/v1");
    expect(invoke.mock.calls.some(([command]) => command === "save_verified_provider")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByText("Candidate Provider", { selector: ".provider-row-name" }))
      .toBeInTheDocument();
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
    expect(screen.getByText("Responses API 流式响应").closest("li")).toHaveAttribute(
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
    expect(await screen.findByText("验证通过")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "供应商验证" })).toBeInTheDocument();
  }, 10_000);

  it("从详情返回时明确放弃未保存输入", async () => {
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
    fireEvent.click(screen.getByRole("button", { name: "返回" }));
    fireEvent.click(screen.getByRole("button", { name: "放弃修改" }));

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

describe("逐项供应商验证弹窗", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    cleanup();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("验证期间显示阶段计时和等待状态，且只能明确取消", async () => {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }));
    let rejectValidation: (reason: object) => void = () => undefined;
    const validation = new Promise<object>((_resolve, reject) => {
      rejectValidation = reject;
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
      if (command === "cancel_provider_request") {
        rejectValidation({ category: "cancelled", messageId: "provider.request_cancelled" });
        return Promise.resolve(true);
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "添加供应商" }));
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://provider.example/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    await screen.findByRole("option", { name: "model-a" });
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-a" } });
    vi.useFakeTimers();
    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));

    const dialog = screen.getByRole("dialog", { name: "供应商验证" });
    expect(dialog).toHaveTextContent("模型确认");
    expect(dialog).toHaveTextContent("Responses API 流式响应");
    expect(dialog).toHaveTextContent("工具调用闭环");
    expect(dialog).toHaveTextContent("未开始");
    expect(dialog).toHaveTextContent(/已用 \d+ 秒/);
    expect(screen.queryByText(/%/)).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "取消验证" })).toHaveLength(2);

    expect(dialog.querySelector(".is-spinning")).not.toBeInTheDocument();
    fireEvent.keyDown(dialog, { key: "Escape" });
    fireEvent.click(dialog.parentElement!);
    expect(screen.getByRole("dialog", { name: "供应商验证" })).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(16_000);
    });
    expect(screen.getByText("仍在等待供应商响应")).toBeInTheDocument();
    vi.useRealTimers();

    const cancelButtons = screen.getAllByRole("button", { name: "取消验证" });
    fireEvent.click(cancelButtons[cancelButtons.length - 1]);
    await act(async () => Promise.resolve());
    expect(invoke).toHaveBeenCalledWith(
      "cancel_provider_request",
      expect.objectContaining({ requestId: expect.any(String) }),
    );
    expect(screen.getByRole("dialog", { name: "供应商验证" })).toBeInTheDocument();
    expect(await screen.findByText("请求已取消。")).toBeInTheDocument();
    expect(screen.getByText("失败", { selector: ".validation-step-state" })).toBeInTheDocument();
    expect(screen.getByText("cancelled · provider.request_cancelled")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "返回修改" }));
    expect(screen.queryByRole("dialog", { name: "供应商验证" })).not.toBeInTheDocument();
  }, 10_000);

  it("目录重新验证在用户完成后更新验证时间并只显示页面内反馈", async () => {
    const notification = vi.fn();
    vi.stubGlobal("Notification", notification);
    const provider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const updated = { ...provider, verifiedAtEpochSeconds: 1_786_140_900 };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "revalidate_provider") {
        return Promise.resolve({ provider: updated, validationReceipt: null });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const originalVerifiedAt = (await screen.findByText(/^验证于 /)).textContent;
    fireEvent.click(await screen.findByRole("button", { name: "验证 Atlas" }));

    const dialog = await screen.findByRole("dialog", { name: "供应商验证" });
    expect(dialog).toHaveTextContent("验证通过");
    expect(screen.queryByText("Atlas 重新验证成功。", { selector: "[role='status']" })).not.toBeInTheDocument();
    vi.useFakeTimers();
    fireEvent.click(screen.getByRole("button", { name: "完成" }));

    expect(screen.queryByRole("dialog", { name: "供应商验证" })).not.toBeInTheDocument();
    expect(screen.getByText("Atlas 重新验证成功。", { selector: "[role='status']" })).toBeInTheDocument();
    expect(screen.getByText(/^验证于 /).textContent).not.toBe(originalVerifiedAt);
    expect(screen.getByText("已验证")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);
    expect(notification).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(5_001));
    expect(screen.queryByText("Atlas 重新验证成功。")).not.toBeInTheDocument();
    vi.useRealTimers();
  });

  it("目录重新验证命中候选后保持原记录并要求明确保存建议地址", async () => {
    const provider = {
      id: "candidate-revalidation-provider",
      name: "Atlas",
      baseUrl: "https://atlas.example/api",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const receipt = {
      validationId: "candidate-revalidation-receipt",
      requestedBaseUrl: provider.baseUrl,
      normalizedBaseUrl: "https://atlas.example/api/v1",
      defaultModel: "model-a",
      combinationFingerprint: "d".repeat(64),
      verifiedAtEpochSeconds: 1_786_140_900,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "revalidate_provider") {
        return Promise.resolve({ provider, validationReceipt: receipt });
      }
      if (command === "save_provider_update") {
        return Promise.resolve({
          ...provider,
          baseUrl: receipt.normalizedBaseUrl,
          verifiedAtEpochSeconds: receipt.verifiedAtEpochSeconds,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const originalVerifiedAt = (await screen.findByText(/^验证于 /)).textContent;
    fireEvent.click(screen.getByRole("button", { name: "验证 Atlas" }));
    fireEvent.click(await screen.findByRole("button", { name: "完成" }));

    expect(screen.getByRole("dialog", { name: "建议修正服务地址" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "采用建议地址" }));
    await waitFor(() => expect(screen.getByLabelText("服务地址")).toHaveValue(
      receipt.normalizedBaseUrl,
    ));
    expect(invoke.mock.calls.some(([command]) => command === "save_provider_update")).toBe(false);
    expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();

    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("save_provider_update", {
      validationId: receipt.validationId,
      providerId: provider.id,
      name: provider.name,
    }));
    expect(originalVerifiedAt).toBeTruthy();
  });

  it("目录重新验证失败保留原验证时间、供应商和当前环境", async () => {
    const provider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: true,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "revalidate_provider") {
        return Promise.reject({
          category: "authentication",
          messageId: "provider.authentication_failed",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const originalVerifiedAt = (await screen.findByText(/^验证于 /)).textContent;
    fireEvent.click(await screen.findByRole("button", { name: "验证 Atlas" }));

    expect(await screen.findByText("API Key 未通过供应商认证。")).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "供应商验证" })).toHaveTextContent("验证失败");
    fireEvent.click(screen.getByRole("button", { name: "完成" }));

    expect(screen.getByText("Atlas 最近验证失败。", { selector: "[role='status']" })).toBeInTheDocument();
    expect(screen.getByText(/^验证于 /).textContent).toBe(originalVerifiedAt);
    expect(screen.getByRole("button", { name: "Atlas 当前使用" })).toBeDisabled();
    expect(screen.getByText("已验证")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);
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
  it("展示并配置未持久化的 DayWay 推荐模板，删除后恢复模板", async () => {
    const saved = {
      id: "c950b528-4b0a-4ba7-a578-00585d9d9d0a",
      name: "DayWay",
      baseUrl: "https://dayway.site/v1",
      defaultModel: "dayway-model",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
      recommendationId: "dayway",
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "open_dayway_website") return Promise.resolve(undefined);
      if (command === "discover_provider_models") {
        return Promise.resolve({
          normalizedBaseUrl: "https://dayway.site/v1",
          models: ["dayway-model"],
        });
      }
      if (command === "validate_provider") {
        return Promise.resolve({
          validationId: "dayway-validation",
          normalizedBaseUrl: "https://dayway.site/v1",
          defaultModel: "dayway-model",
          combinationFingerprint: "a".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_000,
        });
      }
      if (command === "save_dayway_provider") return Promise.resolve(saved);
      if (command === "delete_provider") return Promise.resolve(undefined);
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);

    expect(await screen.findByText("DayWay", {}, { timeout: 5_000 })).toBeInTheDocument();
    expect(screen.getByText("推荐")).toBeInTheDocument();
    expect(screen.getByText("待配置")).toBeInTheDocument();
    expect(screen.getByText("https://dayway.site/v1")).toBeInTheDocument();
    expect(screen.getByText("尚未选择")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "访问 DayWay 官网" }));
    expect(invoke).toHaveBeenCalledWith("open_dayway_website");

    fireEvent.click(screen.getByRole("button", { name: "配置 DayWay" }));
    expect(screen.getByLabelText("供应商名称")).toHaveValue("DayWay");
    expect(screen.getByLabelText("供应商名称")).toBeDisabled();
    expect(screen.getByLabelText("服务地址")).toHaveValue("https://dayway.site/v1");
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    expect(await screen.findByRole("option", { name: "dayway-model" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "dayway-model" } });
    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));
    expect(await screen.findByText("验证通过")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_dayway_provider", {
        validationId: "dayway-validation",
        confirmNameConflict: false,
      });
    });
    expect(screen.getByText("已验证")).toBeInTheDocument();
    expect(screen.getByText("推荐")).toBeInTheDocument();
    expect(screen.queryByText("待配置")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "拖拽排序 DayWay" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "删除 DayWay" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("delete_provider", { providerId: saved.id }));
    expect(screen.getByText("待配置")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "配置 DayWay" })).toBeEnabled();
  }, 10_000);

  it("已保存 DayWay 只在用户采用推荐地址后进入重新验证流程", async () => {
    const saved = {
      id: "c950b528-4b0a-4ba7-a578-00585d9d9d0a",
      name: "DayWay",
      baseUrl: "https://saved.dayway.example/v1",
      defaultModel: "saved-model",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
      recommendationId: "dayway",
      hasRecommendationUpdate: true,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([saved]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    expect(await screen.findByText("推荐地址已更新")).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "修改 DayWay" }));
    expect(screen.getByLabelText("服务地址")).toHaveValue(saved.baseUrl);
    fireEvent.click(screen.getByRole("button", { name: "采用 DayWay 推荐地址" }));
    expect(screen.getByLabelText("服务地址")).toHaveValue("https://dayway.site/v1");
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("dialog", { name: "需要验证供应商" })).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "save_provider_update")).toBe(false);
  });

  it("旧普通 DayWay 名称冲突只有确认后才重试推荐保存", async () => {
    let saveAttempts = 0;
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "discover_provider_models") {
        return Promise.resolve({ normalizedBaseUrl: "https://dayway.site/v1", models: ["model-a"] });
      }
      if (command === "validate_provider") {
        return Promise.resolve({
          validationId: "conflict-validation",
          normalizedBaseUrl: "https://dayway.site/v1",
          defaultModel: "model-a",
          combinationFingerprint: "b".repeat(64),
          verifiedAtEpochSeconds: 1,
        });
      }
      if (command === "save_dayway_provider") {
        saveAttempts += 1;
        if (!args?.confirmNameConflict) {
          return Promise.reject({ category: "invalid_input", messageId: "provider.recommended_name_conflict" });
        }
        return Promise.resolve({
          id: "recommended-id",
          name: "DayWay",
          baseUrl: "https://dayway.site/v1",
          defaultModel: "model-a",
          verifiedAtEpochSeconds: 1,
          isCurrent: false,
          recommendationId: "dayway",
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "配置 DayWay" }));
    fireEvent.change(screen.getByLabelText("API Key"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    await screen.findByRole("option", { name: "model-a" });
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-a" } });
    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));
    await screen.findByText("验证通过");
    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(saveAttempts).toBe(2));
    expect(invoke).toHaveBeenCalledWith("save_dayway_provider", {
      validationId: "conflict-validation",
      confirmNameConflict: true,
    });
  });

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
        return Promise.resolve({
          provider: { ...first, name: "Atlas Renamed", verifiedAtEpochSeconds: 1_786_140_500 },
          validationReceipt: null,
        });
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
    expect(await screen.findByText("验证通过")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "save_provider_update")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "完成" }));
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

  it("环境读取失败时仍展示已保存的供应商目录", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Available Provider",
      baseUrl: "https://available.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.reject(new Error("unavailable"));
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByText("Available Provider")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("无法读取当前用户 Codex 环境");
    expect(screen.queryByText("无法读取供应商目录")).not.toBeInTheDocument();
  });

  it("在供应商管理中展示环境状态与完整底部操作，不再保留旧环境入口", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "external",
          mode: null,
          messageId: "environment.external",
          revision: "merged-page-revision",
          requiresTakeoverConfirmation: true,
          restoreAvailability: "no_backup",
          loginStatus: "logged_in",
          pendingRestart: true,
          consumers: { desktop: "unknown", cli: "running" },
          impacts: [],
          currentProvider: null,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "供应商管理" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "外部配置" })).toBeInTheDocument();
    expect(screen.getByText("待重启")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复上次配置" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "切换到 OpenAI 登录模式" })).toBeEnabled();
    expect(screen.getByRole("button", { name: /选择 WSL2 供应商/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /导出 Linux 脚本/ })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Codex 环境" })).not.toBeInTheDocument();
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
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);

    expect(await screen.findByRole("heading", { name: "外部配置" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "切换到 Applied Provider" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        confirmSwitchRisk: true,
        expectedRevision: "external-revision",
      });
    });
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("将接管外部配置并应用“Applied Provider”"));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("config.toml：model、model_provider、model_providers.<provider-id>"));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("auth.json：auth_mode、OPENAI_API_KEY"));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("无法确认桌面版状态"));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("无法确认 Codex CLI 状态"));
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
      takeoverAvailable: true,
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
    const takeover = await screen.findByRole("button", { name: "切换到 Recovery Provider" });
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

  it("管理冲突无法安全解析时不提供强制覆盖", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Blocked Provider",
      baseUrl: "https://blocked.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "conflict",
          mode: null,
          messageId: "environment.managed_conflict",
          revision: "unsafe-conflict-revision",
          requiresTakeoverConfirmation: true,
          takeoverAvailable: false,
          restoreAvailability: "no_backup",
          loginStatus: "not_logged_in",
          pendingRestart: false,
          consumers: { desktop: "stopped", cli: "stopped" },
          impacts: [],
          currentProvider: null,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByText("无法安全解析当前配置，不能强制覆盖。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "切换到 Blocked Provider" })).toBeDisabled();
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);
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
      restorePreview: {
        artifacts: ["config", "credentials"],
        targetMode: null,
        targetProvider: null,
      },
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
    let finishRestore!: (snapshot: unknown) => void;
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(managed);
      if (command === "restore_last_environment_config") {
        return new Promise((resolve) => {
          finishRestore = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    await screen.findByRole("heading", { name: "已由 GPTEasy 管理" });
    const restore = screen.getByRole("button", { name: "恢复上次配置" });
    expect(restore).toBeEnabled();
    fireEvent.click(restore);

    expect(confirm).toHaveBeenCalledWith("将恢复 config.toml、auth.json，恢复后为外部配置。是否继续？");
    expect(screen.getByText("正在恢复上次配置。")).toBeInTheDocument();

    finishRestore({
      ...managed,
      state: "external",
      messageId: "environment.external",
      revision: "restored-revision",
      currentProvider: null,
    });

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

    await screen.findByRole("heading", { name: "已由 GPTEasy 管理" });
    expect(screen.getByRole("button", { name: "恢复上次配置" })).toBeDisabled();
    expect(screen.getByText("受管工件在最近一次修改后发生变化，恢复已禁用。")).toBeInTheDocument();
  });

  it("展示认证与消费者状态，并从供应商管理确认切换到 OpenAI 登录模式", async () => {
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

    expect(await screen.findByRole("heading", { name: "外部配置" })).toBeInTheDocument();
    expect(screen.getByText("桌面版").nextElementSibling).toHaveTextContent("无法确认");
    expect(screen.getByText("Codex CLI").nextElementSibling).toHaveTextContent("无法确认");
    expect(screen.getByText("待重启").nextElementSibling).toHaveTextContent("是");
    fireEvent.click(screen.getByRole("button", { name: "切换到 OpenAI 登录模式" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("switch_to_openai_login", {
        confirmSwitch: true,
        expectedRevision: "openai-ready-revision",
      });
    });
    expect(await screen.findByRole("heading", { name: "OpenAI 登录模式" })).toBeInTheDocument();
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

    expect(await screen.findByText("OpenAI 登录已在外部失效；当前模式保持不变。")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "切换到 Return Provider" }));
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

  it("全新状态直接展示供应商管理且不会创建 Codex 配置", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "external",
          mode: null,
          messageId: "environment.external",
          revision: "fresh-revision",
          requiresTakeoverConfirmation: true,
          takeoverAvailable: true,
          impacts: [],
          currentProvider: null,
          restoreAvailability: "no_backup",
          restorePreview: null,
          loginStatus: "not_logged_in",
          pendingRestart: false,
          requiresConsumerConfirmation: true,
          consumers: { desktop: "unknown", cli: "unknown" },
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    await screen.findByRole("heading", { name: "供应商管理" });

    expect(await screen.findByRole("heading", { name: "外部配置" })).toBeInTheDocument();
    expect(screen.getByText("尚未建立有效的 GPTEasy 供应商 ID。")).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith("get_startup_snapshot");
    expect(invoke.mock.calls.some(([command]) => command.startsWith("apply_"))).toBe(false);
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

describe("ChatGPT/Codex 桌面版命令", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("桌面版停止时从左栏确认启动，并以重新扫描结果显示运行中", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_desktop_snapshot") {
        return Promise.resolve({
          status: "stopped",
          action: "start",
          messageId: "desktop.ready_to_start",
        });
      }
      if (command === "start_desktop_application") {
        return Promise.resolve({
          status: "running",
          action: "unavailable",
          messageId: "desktop.running",
        });
      }
      return Promise.resolve(undefined);
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);

    const start = await screen.findByRole("button", { name: "启动 ChatGPT/Codex" });
    await waitFor(() => expect(start).toBeEnabled());
    expect(start.closest(".sidebar-command-area")?.nextElementSibling).toHaveTextContent("当前用户");
    fireEvent.click(start);

    expect(confirm).toHaveBeenCalledWith("将启动 OpenAI 官方 ChatGPT/Codex 桌面版。是否继续？");
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("start_desktop_application");
    });
    expect(await screen.findByText("ChatGPT/Codex 正在运行")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /重启 ChatGPT\/Codex/ })).not.toBeInTheDocument();
  });

  it("桌面身份检测不可信时禁用命令并显示稳定原因", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_desktop_snapshot") {
        return Promise.resolve({
          status: "unknown",
          action: "unavailable",
          messageId: "desktop.identity_untrusted",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("button", { name: "启动 ChatGPT/Codex" })).toBeDisabled();
    expect(await screen.findByText("无法可靠确认桌面版身份，启动已禁用。")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "start_desktop_application")).toBe(false);
  });
});
