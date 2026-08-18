import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

function openSettingsMenu() {
  fireEvent.click(screen.getByRole("button", { name: "设置" }));
  return screen.getByRole("menu");
}

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
    const savedProvider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Example Provider",
      baseUrl: "https://provider.example/api/v1",
      defaultModel: "model-b",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const environment = {
      state: "external",
      mode: null,
      messageId: "environment.external",
      revision: "first-provider-revision",
      requiresTakeoverConfirmation: true,
      takeoverAvailable: true,
      impacts: [],
      currentProvider: null,
      restoreAvailability: "no_backup",
      restorePreview: null,
      loginStatus: "logged_in",
      pendingRestart: false,
      requiresConsumerConfirmation: false,
      consumers: { desktop: "stopped", cli: "stopped" },
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "get_environment_snapshot") return Promise.resolve(environment);
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
        return Promise.resolve(savedProvider);
      }
      if (command === "apply_environment_provider") {
        return Promise.resolve({
          ...environment,
          state: "managed",
          mode: "provider",
          revision: "first-provider-applied",
          requiresTakeoverConfirmation: false,
          currentProvider: { ...savedProvider, isCurrent: true },
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
    const applyDialog = screen.getByRole("dialog", { name: "应用第一个供应商？" });
    expect(applyDialog).toHaveTextContent("是否把“Example Provider”应用到当前 Codex 中？");
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);

    fireEvent.click(within(applyDialog).getByRole("button", { name: "应用" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
      providerId: savedProvider.id,
      expectedRevision: environment.revision,
    }));
  }, 10_000);

  it("候选地址手动验证不保存，从保存发起验证则在采用后自动保存", async () => {
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
    expect(screen.getByRole("dialog", { name: "需要验证供应商" })).toHaveTextContent(
      "验证会实际调用所选模型并产生少量费用，请确保供应商账户有可用余额。",
    );
    expect(invoke.mock.calls.some(([command]) => command === "save_verified_provider")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));

    fireEvent.click(screen.getByRole("button", { name: "验证供应商" }));
    fireEvent.click(await screen.findByRole("button", { name: "完成" }));
    expect(screen.getByRole("dialog", { name: "建议修正服务地址" }))
      .toHaveTextContent("采用后仍需明确保存");
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

    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://provider.example/api" },
    });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    expect(await screen.findByRole("option", { name: "candidate-model" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("默认模型"), {
      target: { value: "candidate-model" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    fireEvent.click(screen.getByRole("button", { name: "开始验证" }));
    fireEvent.click(await screen.findByRole("button", { name: "完成" }));
    expect(screen.getByRole("dialog", { name: "建议修正服务地址" }))
      .toHaveTextContent("采用后将继续保存");
    fireEvent.click(screen.getByRole("button", { name: "采用建议地址" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "confirm_provider_validation_base_url",
      {
        validationId: "candidate-validation-3",
        baseUrl: "https://provider.example/api/v1",
      },
    ));
    expect(await screen.findByText("Candidate Provider", { selector: ".provider-row-name" }))
      .toBeInTheDocument();
    expect(invoke.mock.calls.filter(([command]) => command === "save_verified_provider"))
      .toHaveLength(1);
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
    const progressSubscription = listen.mock.calls.find(
      ([eventName]) => eventName === "provider-validation-progress",
    );
    const progressListener = progressSubscription?.[1] as (event: {
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

describe("应用更新", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("从设置菜单打开更新窗口，并在失败时提供重试入口", async () => {
    const readyUpdate = {
      currentVersion: "1.0.1",
      state: "pending",
      availableVersion: "1.1.0",
      notes: "修复稳定性问题\n\n第二段完整说明不在摘要中显示",
      publishedAt: "2026-08-18T00:00:00Z",
      checkedAtEpochSeconds: 1_787_027_200,
      downloadedBytes: 100,
      totalBytes: 100,
      progressPercent: 100,
      failureCategory: null,
      errorMessage: null,
      manualDownloadUrl: "https://github.com/yinshaohua/GPTEasy/releases/latest",
      releaseNotesUrl: "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1.0",
    };
    const failedUpdate = {
      ...readyUpdate,
      state: "failed",
      availableVersion: null,
      progressPercent: null,
      errorMessage: "更新签名验证失败",
      failureCategory: "signature_invalid",
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "get_update_snapshot") return Promise.resolve(readyUpdate);
      if (command === "check_for_updates") return Promise.resolve(failedUpdate);
      if (command === "list_providers" || command === "list_wsl_environments") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("button", { name: "点击重启升级" })).toBeInTheDocument();
    fireEvent.click(within(openSettingsMenu()).getByRole("menuitem", { name: "检查更新..." }));
    const dialog = screen.getByRole("dialog", { name: "GPTEasy 更新" });
    expect(dialog).toHaveTextContent("更新已下载并通过签名验证");
    expect(dialog).toHaveTextContent("修复稳定性问题");
    expect(dialog).not.toHaveTextContent("第二段完整说明不在摘要中显示");
    expect(within(dialog).getByRole("link", { name: "查看 GitCode 完整发布说明" }))
      .toHaveAttribute("href", readyUpdate.releaseNotesUrl);
    expect(within(dialog).getByRole("button", { name: "稍后" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "重启并更新" })).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("更新签名验证失败")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /更新检查失败/ })).not.toBeInTheDocument();
    expect(within(screen.getByRole("dialog", { name: "GPTEasy 更新" })).getByRole("button", { name: "重试" })).toBeInTheDocument();
  });

  it("稍后不会启动安装，繁忙拒绝保持待安装状态", async () => {
    const readyUpdate = {
      currentVersion: "1.0.1",
      state: "pending",
      availableVersion: "1.1.0",
      notes: "修复稳定性问题",
      publishedAt: "2026-08-18T00:00:00Z",
      checkedAtEpochSeconds: 1_787_027_200,
      downloadedBytes: 100,
      totalBytes: 100,
      progressPercent: 100,
      failureCategory: null,
      errorMessage: null,
      manualDownloadUrl: "https://github.com/yinshaohua/GPTEasy/releases/latest",
      releaseNotesUrl: "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.1.0",
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "get_update_snapshot") return Promise.resolve(readyUpdate);
      if (command === "install_update") {
        return Promise.reject({ category: "busy", messageId: "update.busy" });
      }
      if (command === "list_providers" || command === "list_wsl_environments") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    await screen.findByRole("button", { name: "点击重启升级" });
    fireEvent.click(within(openSettingsMenu()).getByRole("menuitem", { name: "检查更新..." }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "GPTEasy 更新" }))
      .getByRole("button", { name: "稍后" }));
    expect(invoke.mock.calls.some(([command]) => command === "install_update")).toBe(false);
    expect(screen.queryByRole("dialog", { name: "GPTEasy 更新" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "点击重启升级" })).toBeInTheDocument();

    fireEvent.click(within(openSettingsMenu()).getByRole("menuitem", { name: "检查更新..." }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "GPTEasy 更新" }))
      .getByRole("button", { name: "重启并更新" }));
    expect(await screen.findByText("当前有操作正在进行，请先完成或取消后再安装更新。"))
      .toBeInTheDocument();
    expect(screen.getByRole("button", { name: "点击重启升级" })).toBeInTheDocument();
  });

  it("没有可用更新时明确显示已是最新版本", async () => {
    const idleUpdate = {
      currentVersion: "1.1.1",
      state: "idle",
      availableVersion: null,
      notes: null,
      publishedAt: null,
      checkedAtEpochSeconds: null,
      downloadedBytes: 0,
      totalBytes: null,
      progressPercent: null,
      failureCategory: null,
      errorMessage: null,
      manualDownloadUrl: "https://github.com/yinshaohua/GPTEasy/releases/latest",
      releaseNotesUrl: null,
    };
    const currentUpdate = {
      ...idleUpdate,
      state: "up_to_date",
      checkedAtEpochSeconds: 1_787_027_200,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "get_update_snapshot") return Promise.resolve(idleUpdate);
      if (command === "check_for_updates") return Promise.resolve(currentUpdate);
      if (command === "list_providers" || command === "list_wsl_environments") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    await screen.findByRole("heading", { name: "供应商管理" });
    fireEvent.click(within(openSettingsMenu()).getByRole("menuitem", { name: "检查更新..." }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "GPTEasy 更新" }))
      .getByRole("button", { name: "检查更新" }));

    expect(await screen.findByText("已是最新版本")).toBeInTheDocument();
  });

  it("启动确认未完成更新后提供重新下载和手工入口", async () => {
    const incompleteUpdate = {
      currentVersion: "1.0.1",
      state: "incomplete",
      availableVersion: "1.1.0",
      notes: null,
      publishedAt: null,
      checkedAtEpochSeconds: 1_787_027_200,
      downloadedBytes: 0,
      totalBytes: null,
      progressPercent: null,
      failureCategory: null,
      errorMessage: null,
      manualDownloadUrl: "https://github.com/yinshaohua/GPTEasy/releases/latest",
      releaseNotesUrl: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "get_update_snapshot") return Promise.resolve(incompleteUpdate);
      if (command === "list_providers" || command === "list_wsl_environments") return Promise.resolve([]);
      return Promise.resolve(incompleteUpdate);
    });

    render(<App />);
    await screen.findByRole("button", { name: "点击重启升级" });
    fireEvent.click(within(openSettingsMenu()).getByRole("menuitem", { name: "检查更新..." }));
    const dialog = screen.getByRole("dialog", { name: "GPTEasy 更新" });
    expect(dialog).toHaveTextContent("上次确认的更新尚未完成");
    expect(within(dialog).getByRole("button", { name: "检查更新" })).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "GitHub 手工下载" })).toBeInTheDocument();
    expect(within(dialog).queryByRole("button", { name: "重启并更新" })).not.toBeInTheDocument();
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
    expect(dialog).toHaveTextContent(
      "验证会实际调用所选模型并产生少量费用，请确保供应商账户有可用余额。",
    );
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

  it("已保存供应商详情可以直接重新验证并复用已保存凭据", async () => {
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
      if (command === "revalidate_provider") {
        return Promise.resolve({
          provider: { ...provider, verifiedAtEpochSeconds: 1_786_140_900 },
          validationReceipt: null,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "修改 Atlas" }));

    const revalidate = screen.getByRole("button", { name: "重新验证" });
    expect(revalidate).toBeEnabled();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    fireEvent.click(revalidate);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("revalidate_provider", {
        requestId: expect.any(String),
        providerId: provider.id,
      });
    });
    expect(await screen.findByText("验证通过")).toBeInTheDocument();
  });

  it("目录重新验证在用户完成后更新验证证据并只显示页面内反馈", async () => {
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
    fireEvent.click(await screen.findByRole("button", { name: "验证 Atlas" }));

    const dialog = await screen.findByRole("dialog", { name: "供应商验证" });
    expect(dialog).toHaveTextContent("验证通过");
    expect(screen.queryByText("Atlas 重新验证成功。", { selector: "[role='status']" })).not.toBeInTheDocument();
    vi.useFakeTimers();
    fireEvent.click(screen.getByRole("button", { name: "完成" }));

    expect(screen.queryByRole("dialog", { name: "供应商验证" })).not.toBeInTheDocument();
    expect(screen.getByText("Atlas 重新验证成功。", { selector: "[role='status']" })).toBeInTheDocument();
    expect(screen.queryByText(/^验证于 /)).not.toBeInTheDocument();
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
    fireEvent.click(await screen.findByRole("button", { name: "验证 Atlas" }));
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
    expect(screen.queryByText(/^验证于 /)).not.toBeInTheDocument();
  });

  it("目录重新验证失败保留原验证证据、供应商和当前环境", async () => {
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
    fireEvent.click(await screen.findByRole("button", { name: "验证 Atlas" }));

    expect(await screen.findByText(
      "API Key 未通过供应商认证。请确认 API Key 有效且账户有可用余额；验证会实际调用模型并产生少量费用。",
    )).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "供应商验证" })).toHaveTextContent("验证失败");
    fireEvent.click(screen.getByRole("button", { name: "完成" }));

    expect(screen.getByText("Atlas 最近验证失败。", { selector: "[role='status']" })).toBeInTheDocument();
    expect(screen.queryByText(/^验证于 /)).not.toBeInTheDocument();
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
      if (command === "revalidate_provider") {
        return Promise.reject({
          category: "authentication",
          messageId: "provider.authentication_failed",
          details: apiKeyCanary,
        });
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
    fireEvent.click(screen.getByRole("button", { name: "验证 Atlas" }));
    const validationDialog = await screen.findByRole("dialog", { name: "供应商验证" });
    const errorDetails = within(validationDialog).getByText("技术详情").parentElement?.textContent ?? "";
    expect(errorDetails).not.toContain(apiKeyCanary);
    fireEvent.click(within(validationDialog).getByRole("button", { name: "完成" }));
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
    expect(screen.getByRole("img", { name: "DayWay 固定在首位" })).toBeInTheDocument();
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
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("delete_provider", {
      providerId: saved.id,
      authorizeStoppedWsl: false,
    }));
    expect(screen.getByText("待配置")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "配置 DayWay" })).toBeEnabled();
  }, 10_000);

  it("删除核验发现新的 Stopped 发行版时再次取得显式授权", async () => {
    const provider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Atlas",
      baseUrl: "https://atlas.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "list_wsl_environments") return Promise.resolve([]);
      if (command === "delete_provider") {
        return args?.authorizeStoppedWsl
          ? Promise.resolve({
              lifecycleResults: [{
                environmentId: "{11111111-1111-1111-1111-111111111111}",
                displayName: "Ubuntu",
                outcome: "stopped_naturally",
              }],
            })
          : Promise.reject({
              category: "wsl_verification",
              messageId: "wsl.delete_start_authorization_required",
            });
      }
      return Promise.resolve(undefined);
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "删除 Atlas" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("delete_provider", {
      providerId: provider.id,
      authorizeStoppedWsl: true,
    }));
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("临时启动已停止的 WSL2"));
    expect(await screen.findByText(/临时启动的发行版已自然停止/)).toBeInTheDocument();
  });

  it("目录删除失败时仍展示已完成的 WSL2 生命周期结果", async () => {
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
      if (command === "list_wsl_environments") return Promise.resolve([]);
      if (command === "delete_provider") {
        return Promise.reject({
          category: "provider",
          messageId: "provider.current_delete_forbidden",
          lifecycleResults: [{
            environmentId: "{11111111-1111-1111-1111-111111111111}",
            displayName: "Ubuntu",
            outcome: "unchanged_running",
          }],
        });
      }
      return Promise.resolve(undefined);
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "删除 Atlas" }));

    expect(await screen.findByText(/当前供应商不能删除。/)).toBeInTheDocument();
    expect(screen.getByText(/原本为 Running/)).toBeInTheDocument();
  });

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

  it("紧凑目录只显示高价值信息并保留环境操作的可访问说明", async () => {
    const provider = {
      id: "compact-layout-provider",
      name: "Long Provider Name",
      baseUrl: "https://provider.example/very/long/responses/compatible/api/v1",
      defaultModel: "provider-model-with-a-very-long-version-identifier",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "external",
          mode: null,
          messageId: "environment.external",
          revision: "compact-layout-revision",
          requiresTakeoverConfirmation: true,
          restoreAvailability: "no_backup",
          loginStatus: "logged_in",
          pendingRestart: false,
          consumers: { desktop: "stopped", cli: "stopped" },
          impacts: [],
          currentProvider: null,
        });
      }
      if (command === "list_wsl_environments") {
        return Promise.resolve([{
          environmentId: "{11111111-1111-4111-8111-111111111111}",
          displayName: "Ubuntu",
          commandName: "Ubuntu",
          defaultUid: 1000,
          running: true,
          availability: "manageable",
          currentProvider: provider,
          actualProviderId: provider.id,
          configurationState: "current",
          requiresAttention: false,
          pendingRestart: false,
          revision: "compact-wsl-revision",
          messageId: null,
        }]);
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    const catalogHeading = await screen.findByRole("heading", { name: "供应商目录" });
    const providerList = await screen.findByLabelText("已验证供应商");
    expect(await within(providerList).findByText(provider.name)).toBeInTheDocument();
    expect(catalogHeading.parentElement).toHaveTextContent("1 个已验证供应商");
    expect(screen.queryByText("管理、验证和切换 Codex 使用的供应商")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "外部配置" })).not.toBeInTheDocument();
    expect(screen.queryByText(/^验证于 /)).not.toBeInTheDocument();
    expect(within(providerList).getByTitle(provider.baseUrl)).toHaveTextContent(provider.baseUrl);
    expect(within(providerList).getByTitle(provider.defaultModel)).toHaveTextContent(provider.defaultModel);

    const wslProvider = await screen.findByLabelText("WSL2 当前供应商");
    expect(wslProvider).toHaveTextContent(provider.name);
    expect(wslProvider).toHaveTextContent(provider.baseUrl);
    expect(wslProvider).toHaveTextContent(provider.defaultModel);
    expect(screen.queryByRole("button", { name: "恢复上次配置" })).not.toBeInTheDocument();
    expect(screen.queryByText("其他环境供应商操作")).not.toBeInTheDocument();
    expect(screen.queryByText("当前 Windows Codex 环境操作")).not.toBeInTheDocument();
    const openAi = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    expect(openAi).toBeEnabled();
    expect(openAi).toHaveAttribute("title", "使用 Codex 已有的 OpenAI 登录。");
    expect(screen.queryByText("当前用户")).not.toBeInTheDocument();
  });

  it("设置页切换非当前供应商统一确认且契约不包含重启决策", async () => {
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
        return Promise.resolve({
          ...environment,
          currentProvider: target,
          revision: "revision-2",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "供应商目录" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "会话管理" })).toBeEnabled();
    expect(await screen.findAllByText("已验证")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Current Provider 当前使用" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Atlas" })).not.toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "应用 Atlas" }));
    const dialog = screen.getByRole("dialog", { name: "确认配置切换" });
    expect(dialog).toHaveTextContent("Atlas");
    expect(dialog).toHaveTextContent("运行中的 ChatGPT/Codex 桌面版或 Codex CLI 可能继续使用旧配置");
    expect(dialog).not.toHaveTextContent("config.toml");
    expect(dialog).not.toHaveTextContent("auth.json");
    expect(dialog.getElementsByTagName("button")).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "切换" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: target.id,
        expectedRevision: "revision-1",
      });
    });
    expect(screen.getByRole("button", { name: "Atlas 当前使用" })).toBeDisabled();
  });

  it("托盘供应商选择打开同一个简短确认且不会直接写配置", async () => {
    const provider = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Tray Provider",
      baseUrl: "https://tray.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const environment = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "tray-revision",
      requiresTakeoverConfirmation: false,
      requiresConsumerConfirmation: true,
      impacts: [],
      currentProvider: null,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "running", cli: "stopped" },
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(environment);
      return Promise.resolve(undefined);
    });

    render(<App />);
    await screen.findByRole("heading", { name: "供应商目录" });
    await waitFor(() => {
      expect(listen.mock.calls.some(([eventName]) => eventName === "provider-switch-requested"))
        .toBe(true);
    });
    const subscription = listen.mock.calls.find(
      ([eventName]) => eventName === "provider-switch-requested",
    );
    const trayListener = subscription?.[1] as (event: { payload: string }) => void;

    await act(async () => {
      trayListener({ payload: provider.id });
    });

    const dialog = await screen.findByRole("dialog", { name: "确认配置切换" });
    expect(dialog).toHaveTextContent("Tray Provider");
    expect(dialog).toHaveTextContent("运行中的 ChatGPT/Codex 桌面版或 Codex CLI 可能继续使用旧配置");
    expect(dialog).not.toHaveTextContent("重启");
    expect(dialog.getElementsByTagName("button")).toHaveLength(2);
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider"))
      .toBe(false);
  });

  it("供应商切换只让发起控件忙碌且失败后先刷新环境实际状态", async () => {
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
      name: "Concurrent Provider",
      baseUrl: "https://concurrent.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const before = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "concurrent-before",
      requiresTakeoverConfirmation: false,
      takeoverAvailable: true,
      requiresConsumerConfirmation: false,
      impacts: [],
      currentProvider: current,
      restoreAvailability: "no_backup",
      restorePreview: null,
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "stopped", cli: "stopped" },
    };
    let environmentReads = 0;
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([current, target]);
      if (command === "get_environment_snapshot") {
        environmentReads += 1;
        return Promise.resolve(environmentReads === 1 ? before : {
          ...before,
          revision: "concurrent-after",
          currentProvider: target,
        });
      }
      if (command === "apply_environment_provider") {
        return Promise.reject({
          category: "concurrent_modification",
          messageId: "environment.concurrent_modification",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "应用 Concurrent Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    const switchingButton = screen.getByRole("button", { name: "应用 Concurrent Provider" });
    const openAiButton = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    expect(switchingButton.querySelector(".is-spinning")).toBeInTheDocument();
    expect(openAiButton.querySelector(".is-spinning")).not.toBeInTheDocument();
    await waitFor(() => expect(environmentReads).toBe(2));
    expect(await screen.findByRole("button", { name: "Concurrent Provider 当前使用" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("已重新读取环境实际状态");
  });

  it("配置变更失败且环境重读失败时清除过期当前状态", async () => {
    const current = {
      id: "76149f67-0d76-4d41-b606-77ba244bffec",
      name: "Stale Provider",
      baseUrl: "https://stale.example/v1",
      defaultModel: "model-stale",
      verifiedAtEpochSeconds: 1_786_140_100,
      isCurrent: true,
    };
    const target = {
      id: "68bf9ee2-3ba5-4517-b47e-12a11e038de4",
      name: "Target Provider",
      baseUrl: "https://target.example/v1",
      defaultModel: "model-target",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const environment = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "stale-revision",
      requiresTakeoverConfirmation: false,
      takeoverAvailable: true,
      requiresConsumerConfirmation: false,
      impacts: [],
      currentProvider: current,
      restoreAvailability: "no_backup",
      restorePreview: null,
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "stopped", cli: "stopped" },
    };
    let environmentReads = 0;
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([current, target]);
      if (command === "get_environment_snapshot") {
        environmentReads += 1;
        return environmentReads === 1
          ? Promise.resolve(environment)
          : Promise.reject(new Error("environment unavailable"));
      }
      if (command === "apply_environment_provider") {
        return Promise.reject({
          category: "concurrent_modification",
          messageId: "environment.concurrent_modification",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "应用 Target Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    expect(await screen.findByText("无法读取当前用户 Codex 环境。")).toBeInTheDocument();
    expect(screen.getByText(/页面已清除过期状态/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stale Provider 当前使用" }))
      .not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Target Provider 当前使用" }))
      .not.toBeInTheDocument();
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
      expect(invoke).toHaveBeenCalledWith("delete_provider", {
        providerId: first.id,
        authorizeStoppedWsl: false,
      });
    });
    expect(screen.queryByRole("button", { name: "修改 Atlas Renamed" })).not.toBeInTheDocument();

    expect(screen.getAllByText("当前使用")).not.toHaveLength(0);
    expect(screen.getByRole("button", { name: "删除 Current Provider" })).toBeDisabled();
  }, 10_000);

  it("非当前供应商从保存发起验证后自动保存且只保存一次", async () => {
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
          requestedBaseUrl: "https://atlas.example/next/v1",
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
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    fireEvent.click(screen.getByRole("button", { name: "开始验证" }));
    expect(await screen.findByText("验证通过")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "save_provider_update")).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_provider_update", {
        validationId: "update-validation",
        providerId: provider.id,
        name: "Atlas",
      });
    });
    expect(invoke.mock.calls.filter(([command]) => command === "save_provider_update"))
      .toHaveLength(1);
    const discoveryCall = invoke.mock.calls.find(
      ([command]) => command === "discover_provider_models_for_update",
    );
    expect(discoveryCall?.[1]?.input).toEqual({
      providerId: provider.id,
      baseUrl: "https://atlas.example/next/v1",
      apiKey: null,
    });
    const validationCall = invoke.mock.calls.find(
      ([command]) => command === "validate_provider_update",
    );
    expect(validationCall?.[1]?.input).toEqual({
      providerId: provider.id,
      baseUrl: "https://atlas.example/next/v1",
      apiKey: null,
      defaultModel: "model-b",
    });
  }, 10_000);

  it("当前供应商保存并应用并发失败后先刷新环境实际状态", async () => {
    const current = {
      id: "76149f67-0d76-4d41-b606-77ba244bffec",
      name: "Current Provider",
      baseUrl: "https://current.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_100,
      isCurrent: true,
    };
    const environment = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "current-update-revision",
      requiresTakeoverConfirmation: false,
      requiresConsumerConfirmation: true,
      impacts: [],
      currentProvider: current,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "running", cli: "stopped" },
    };
    let environmentReads = 0;
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([current]);
      if (command === "get_environment_snapshot") {
        environmentReads += 1;
        return Promise.resolve(environmentReads === 1 ? environment : {
          ...environment,
          revision: "current-update-refreshed",
        });
      }
      if (command === "discover_provider_models_for_update") {
        return Promise.resolve({
          normalizedBaseUrl: "https://current.example/next/v1",
          models: ["model-b"],
        });
      }
      if (command === "validate_provider_update") {
        return Promise.resolve({
          validationId: "current-update-validation",
          requestedBaseUrl: "https://current.example/next/v1",
          normalizedBaseUrl: "https://current.example/next/v1",
          defaultModel: "model-b",
          combinationFingerprint: "d".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_700,
        });
      }
      if (command === "save_and_apply_provider_update") {
        return Promise.reject({
          category: "save_and_apply_failed",
          messageId: "environment.concurrent_modification",
        });
      }
      return Promise.resolve(undefined);
    });
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
    fireEvent.click(screen.getByRole("button", { name: "保存并应用" }));
    fireEvent.click(screen.getByRole("button", { name: "开始验证" }));
    expect(await screen.findByText("验证通过")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "save_and_apply_provider_update"))
      .toBe(false);
    fireEvent.click(await screen.findByRole("button", { name: "完成" }));
    const dialog = await screen.findByRole("dialog", { name: "确认配置切换" });
    expect(dialog).toHaveTextContent("保存并应用“Current Provider”的已验证更新");
    expect(screen.getByRole("button", { name: "取消" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "切换" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_and_apply_provider_update", {
        validationId: "current-update-validation",
        providerId: current.id,
        name: current.name,
      });
    });
    await waitFor(() => expect(environmentReads).toBe(2));
    expect(screen.getByRole("alert")).toHaveTextContent("已重新读取环境实际状态");
    expect(
      invoke.mock.calls.some(([command]) => command === "save_provider_update"),
    ).toBe(false);
  }, 10_000);

  it("当前供应商更新在无旧消费者时仍需简短确认", async () => {
    const current = {
      id: "76149f67-0d76-4d41-b606-77ba244bffec",
      name: "Current Provider",
      baseUrl: "https://current.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_100,
      isCurrent: true,
    };
    const environment = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "quiet-current-update-revision",
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
      if (command === "list_providers") return Promise.resolve([current]);
      if (command === "get_environment_snapshot") return Promise.resolve(environment);
      if (command === "discover_provider_models_for_update") {
        return Promise.resolve({
          normalizedBaseUrl: "https://current.example/quiet/v1",
          models: ["model-b"],
        });
      }
      if (command === "validate_provider_update") {
        return Promise.resolve({
          validationId: "quiet-current-update-validation",
          normalizedBaseUrl: "https://current.example/quiet/v1",
          defaultModel: "model-b",
          combinationFingerprint: "e".repeat(64),
          verifiedAtEpochSeconds: 1_786_140_800,
        });
      }
      if (command === "save_and_apply_provider_update") {
        const provider = {
          ...current,
          baseUrl: "https://current.example/quiet/v1",
          defaultModel: "model-b",
          verifiedAtEpochSeconds: 1_786_140_800,
        };
        return Promise.resolve({
          provider,
          environment: { ...environment, currentProvider: provider },
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "修改 Current Provider" }));
    fireEvent.change(screen.getByLabelText("服务地址"), {
      target: { value: "https://current.example/quiet/v1" },
    });
    fireEvent.click(screen.getByRole("button", { name: "获取模型" }));
    await screen.findByRole("option", { name: "model-b" });
    fireEvent.change(screen.getByLabelText("默认模型"), { target: { value: "model-b" } });
    fireEvent.click(screen.getByRole("button", { name: "验证更新" }));
    fireEvent.click(await screen.findByRole("button", { name: "保存并应用" }));
    expect(screen.getByRole("dialog", { name: "确认配置切换" })).toHaveTextContent(
      "保存并应用“Current Provider”的已验证更新",
    );
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("save_and_apply_provider_update", {
        validationId: "quiet-current-update-validation",
        providerId: current.id,
        name: current.name,
      });
    });
    expect(screen.queryByRole("dialog", { name: "确认配置切换" })).not.toBeInTheDocument();
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

    expect(await screen.findAllByText("Available Provider")).toHaveLength(1);
    expect(screen.getByRole("alert")).toHaveTextContent("无法读取当前用户 Codex 环境");
    expect(screen.queryByText("无法读取供应商目录")).not.toBeInTheDocument();
  });

  it("在供应商管理中保留完整底部操作和环境规则，不再显示状态栏或旧入口", async () => {
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
    const navigation = screen.getByRole("navigation", { name: "主要菜单" });
    const navigationItems = within(navigation).getAllByRole("button");
    expect(navigationItems.map((button) => button.textContent)).toEqual(["供应商管理", "会话管理"]);
    const openAi = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    await waitFor(() => expect(openAi).toBeEnabled());
    expect(screen.queryByRole("heading", { name: "外部配置" })).not.toBeInTheDocument();
    expect(screen.queryByText("待重启")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "恢复上次配置" })).not.toBeInTheDocument();
    expect(screen.queryByText("其他环境供应商操作")).not.toBeInTheDocument();
    expect(screen.queryByText("当前 Windows Codex 环境操作")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /选择 WSL2 供应商/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: /导出 Linux 脚本/ })).toBeDisabled();
    expect(within(screen.getByRole("region", { name: "Codex 环境操作" })).getAllByRole("button")).toHaveLength(2);
    expect(screen.queryByText("当前用户")).not.toBeInTheDocument();
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
          pendingRestart: true,
          consumers: { desktop: "unknown", cli: "unknown" },
        });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "应用 Applied Provider" }));

    const dialog = screen.getByRole("dialog", { name: "确认配置切换" });
    expect(dialog).toHaveTextContent("将切换到“Applied Provider”");
    expect(dialog).toHaveTextContent("运行中的 ChatGPT/Codex 桌面版或 Codex CLI 可能继续使用旧配置");
    expect(dialog).not.toHaveTextContent("config.toml");
    expect(dialog).not.toHaveTextContent("auth.json");
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "应用 Applied Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        expectedRevision: "external-revision",
      });
    });
    expect(await screen.findByRole("button", { name: "Applied Provider 当前使用" })).toBeDisabled();
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
      mode: "provider",
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
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      requiresConsumerConfirmation: true,
      consumers: { desktop: "running", cli: "running" },
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
          pendingRestart: true,
          consumers: { desktop: "running", cli: "running" },
        });
      }
      return Promise.resolve(undefined);
    });
    render(<App />);
    const takeover = await screen.findByRole("button", { name: "应用 Recovery Provider" });
    expect(takeover).toBeEnabled();
    fireEvent.click(takeover);
    const dialog = screen.getByRole("dialog", { name: "确认配置切换" });
    expect(dialog).toHaveTextContent("Recovery Provider");
    expect(dialog).not.toHaveTextContent("重启");
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        expectedRevision: "conflict-revision",
      });
    });
    expect(await screen.findByText(/运行中的 Codex 消费者可能继续使用旧配置/)).toBeInTheDocument();
  });

  it("运行中消费者只产生被动待重启反馈", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Failure Provider",
      baseUrl: "https://failure.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    const managed = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "failure-revision",
      requiresTakeoverConfirmation: false,
      takeoverAvailable: true,
      requiresConsumerConfirmation: true,
      impacts: [],
      currentProvider: null,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "running", cli: "running" },
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(managed);
      if (command === "apply_environment_provider") {
        return Promise.resolve({
          ...managed,
          currentProvider: { ...provider, isCurrent: true },
          pendingRestart: true,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "应用 Failure Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    expect(await screen.findByText(/运行中的 Codex 消费者可能继续使用旧配置/))
      .toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => [
      "start_desktop_application",
      "restart_desktop_application",
      "force_restart_desktop_application",
    ].includes(command))).toBe(false);
  });

  it("配置切换失败后不调用任何桌面控制命令", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Close Failure Provider",
      baseUrl: "https://close-failure.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    const managed = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "close-failure-revision",
      requiresTakeoverConfirmation: false,
      takeoverAvailable: true,
      requiresConsumerConfirmation: true,
      impacts: [],
      currentProvider: null,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "running", cli: "stopped" },
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(managed);
      if (command === "apply_environment_provider") {
        return Promise.reject({
          category: "concurrent_modification",
          messageId: "environment.concurrent_modification",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "应用 Close Failure Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => [
      "start_desktop_application",
      "restart_desktop_application",
      "force_restart_desktop_application",
    ].includes(command))).toBe(false);
  });

  it("配置切换结果不再提供强制完成重启入口", async () => {
    const provider = {
      id: "90f00c5a-59a7-4936-a791-583d90b81b73",
      name: "Force Provider",
      baseUrl: "https://force.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_900,
      isCurrent: false,
    };
    const managed = {
      state: "managed",
      mode: "provider",
      messageId: "environment.managed",
      revision: "before-force-revision",
      requiresTakeoverConfirmation: false,
      takeoverAvailable: true,
      requiresConsumerConfirmation: true,
      impacts: [],
      currentProvider: null,
      restoreAvailability: "no_backup",
      loginStatus: "logged_in",
      pendingRestart: false,
      consumers: { desktop: "running", cli: "stopped" },
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") return Promise.resolve(managed);
      if (command === "apply_environment_provider") {
        return Promise.resolve({
          ...managed,
          revision: "force-plan-revision",
          currentProvider: { ...provider, isCurrent: true },
          pendingRestart: true,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "应用 Force Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
        expectedRevision: "before-force-revision",
      });
    });
    expect(screen.queryByRole("button", { name: "强制关闭并重启" })).not.toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "force_complete_config_restart"))
      .toBe(false);
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
    expect(screen.getByRole("button", { name: "应用 Blocked Provider" })).toBeDisabled();
    expect(invoke.mock.calls.some(([command]) => command === "apply_environment_provider")).toBe(false);
  });

  it("读取认证与消费者状态，并从供应商管理确认切换到 OpenAI 登录模式", async () => {
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
    render(<App />);

    await screen.findByRole("heading", { name: "供应商管理" });
    const openAiButton = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    await waitFor(() => expect(openAiButton).toBeEnabled());
    expect(screen.queryByRole("heading", { name: "外部配置" })).not.toBeInTheDocument();
    fireEvent.click(openAiButton);
    const dialog = screen.getByRole("dialog", { name: "确认配置切换" });
    expect(dialog).toHaveTextContent("切换到 OpenAI 登录模式");
    expect(dialog).not.toHaveTextContent("重启");
    fireEvent.click(screen.getByRole("button", { name: "切换" }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("switch_to_openai_login", {
        expectedRevision: "openai-ready-revision",
      });
    });
    const activeOpenAi = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    expect(activeOpenAi).toBeDisabled();
    expect(activeOpenAi).toHaveAttribute("title", "当前已是 OpenAI 登录模式。");
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

      await screen.findByRole("heading", { name: "供应商管理" });
      const openAiButton = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
      await waitFor(() => expect(openAiButton).toHaveAttribute("title", message));
      expect(openAiButton).toBeDisabled();
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
    render(<App />);

    await screen.findByRole("heading", { name: "供应商管理" });
    const openAiButton = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    await waitFor(() => expect(openAiButton).toHaveAttribute("title",
      "OpenAI 登录已在外部失效；当前模式保持不变。",
    ));
    fireEvent.click(screen.getByRole("button", { name: "应用 Return Provider" }));
    fireEvent.click(screen.getByRole("button", { name: "切换" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("apply_environment_provider", {
        providerId: provider.id,
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

    const openAiButton = within(openSettingsMenu()).getByRole("menuitem", { name: "返回 OpenAI 登录模式" });
    await waitFor(() => expect(openAiButton).toHaveAttribute("title",
      "请先在 Codex 中完成 OpenAI 登录。",
    ));
    expect(screen.queryByRole("heading", { name: "外部配置" })).not.toBeInTheDocument();
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

describe("被动桌面消费者状态", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("设置窗口不提供桌面版启动或重启入口，也不请求桌面控制命令", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "供应商管理" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /启动 ChatGPT|重启 ChatGPT/ })).not.toBeInTheDocument();
    expect(
      invoke.mock.calls.some(([command]) => String(command).includes("desktop_application")),
    ).toBe(false);
  });
});

describe("WSL2 供应商选择", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("明确选择单个发行版和已验证供应商且不把凭据传给前端命令", async () => {
    const provider = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "WSL Provider",
      baseUrl: "https://provider.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const wslEnvironment = {
      environmentId: "{11111111-1111-1111-1111-111111111111}",
      displayName: "Ubuntu",
      commandName: "Ubuntu",
      defaultUid: 1000,
      running: false,
      availability: "manageable",
      currentProvider: null,
      actualProviderId: null,
      configurationState: "unknown",
      requiresAttention: false,
      pendingRestart: false,
      revision: "wsl-revision",
      messageId: null,
    };
    const unavailableEnvironments = [
      {
        ...wslEnvironment,
        environmentId: "{33333333-3333-4333-8333-333333333333}",
        displayName: "docker-desktop",
        commandName: null,
        availability: "infrastructure",
        requiresAttention: true,
      },
      {
        ...wslEnvironment,
        environmentId: "{44444444-4444-4444-8444-444444444444}",
        displayName: "已移除 Ubuntu",
        commandName: null,
        availability: "removed",
        requiresAttention: true,
      },
    ];
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "external",
          mode: null,
          messageId: "environment.external",
          revision: "windows-revision",
          requiresTakeoverConfirmation: true,
          takeoverAvailable: true,
          impacts: [],
          currentProvider: null,
          restoreAvailability: "no_backup",
          restorePreview: null,
          loginStatus: "logged_in",
          pendingRestart: false,
          requiresConsumerConfirmation: false,
          consumers: { desktop: "stopped", cli: "stopped" },
        });
      }
      if (command === "list_wsl_environments") {
        return Promise.resolve([wslEnvironment, ...unavailableEnvironments]);
      }
      if (command === "apply_wsl_provider") {
        return Promise.resolve({
          environment: { ...wslEnvironment, currentProvider: { ...provider, isCurrent: true } },
          pendingRestart: false,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);

    const dialog = await screen.findByRole("dialog", { name: "选择 WSL2 供应商" });
    expect(dialog).toHaveTextContent("最多等待 10 秒自然停止，绝不强制终止");
    expect(dialog).toHaveTextContent("命令式凭据工件；auth.json 保持不变");
    const distribution = screen.getByLabelText("WSL2 发行版") as HTMLSelectElement;
    expect(Array.from(distribution.options, (option) => option.textContent)).toEqual(["Ubuntu · 可管理"]);
    expect(screen.queryByText("请选择发行版")).not.toBeInTheDocument();
    expect(screen.queryByText("docker-desktop")).not.toBeInTheDocument();
    expect(screen.queryByText("已移除 Ubuntu")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "应用到 WSL2" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("apply_wsl_provider", {
      environmentId: wslEnvironment.environmentId,
      providerId: provider.id,
      expectedRevision: wslEnvironment.revision,
      confirm: true,
    }));
    const applyCall = invoke.mock.calls.find(([command]) => command === "apply_wsl_provider");
    expect(JSON.stringify(applyCall?.[1])).not.toContain("API_KEY");
    expect(JSON.stringify(applyCall?.[1])).not.toContain("secret");
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "选择 WSL2 供应商" })).not.toBeInTheDocument();
    });
    expect(screen.getByRole("status")).toHaveTextContent("已将“WSL Provider”应用到 WSL2 发行版“Ubuntu”");
    expect(screen.getByLabelText("WSL2 当前供应商")).toHaveTextContent("WSL Provider");
  });

  it("应用失败后以 WSL2 实际状态恢复当前供应商和下次选择", async () => {
    const dayway = {
      id: "11111111-1111-4111-8111-111111111111",
      name: "DayWay",
      baseUrl: "https://dayway.site/v1",
      defaultModel: "dayway-model",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const ylai = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "ylai",
      baseUrl: "https://ylai.example/v1",
      defaultModel: "ylai-model",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const environment = {
      environmentId: "{33333333-3333-4333-8333-333333333333}",
      displayName: "Ubuntu",
      commandName: "Ubuntu",
      defaultUid: 1000,
      running: true,
      availability: "manageable",
      currentProvider: ylai,
      actualProviderId: ylai.id,
      configurationState: "current",
      requiresAttention: false,
      pendingRestart: false,
      revision: "ylai-revision",
      messageId: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([dayway, ylai]);
      if (command === "list_wsl_environments") return Promise.resolve([environment]);
      if (command === "apply_wsl_provider") {
        return Promise.reject({
          category: "guest_unavailable",
          messageId: "wsl.codex_version_too_old",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(open).toBeEnabled());
    const currentSummary = await screen.findByLabelText("WSL2 当前供应商");
    expect(currentSummary).toHaveTextContent("ylai");
    expect(currentSummary).not.toHaveTextContent("DayWay");

    fireEvent.click(open);
    const dialog = await screen.findByRole("dialog", { name: "选择 WSL2 供应商" });
    const providerSelect = within(dialog).getByLabelText("已验证供应商") as HTMLSelectElement;
    expect(providerSelect).toHaveValue(ylai.id);
    fireEvent.change(providerSelect, { target: { value: dayway.id } });
    fireEvent.click(screen.getByRole("button", { name: "应用到 WSL2" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("版本过低");
    expect(providerSelect).toHaveValue(ylai.id);
    expect(screen.getByRole("dialog", { name: "选择 WSL2 供应商" })).toHaveTextContent(
      "当前供应商：ylai",
    );
    fireEvent.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByLabelText("WSL2 当前供应商")).toHaveTextContent("ylai");

    fireEvent.click(open);
    const reopened = await screen.findByRole("dialog", { name: "选择 WSL2 供应商" });
    expect(within(reopened).getByLabelText("已验证供应商")).toHaveValue(ylai.id);
  }, 10_000);

  it("没有可管理发行版时不列出不可选择项", async () => {
    const ambiguousEnvironment = {
      environmentId: "{11111111-1111-1111-1111-111111111111}",
      displayName: "Ubuntu",
      commandName: null,
      defaultUid: 1000,
      running: false,
      availability: "ambiguous",
      currentProvider: null,
      actualProviderId: null,
      configurationState: "unknown",
      requiresAttention: true,
      pendingRestart: false,
      revision: "ambiguous-revision",
      messageId: "wsl.environment_ambiguous",
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "list_wsl_environments") return Promise.resolve([ambiguousEnvironment]);
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);

    expect(await screen.findByRole("dialog", { name: "选择 WSL2 供应商" })).toHaveTextContent(
      "没有可管理的 WSL2 发行版",
    );
    expect(screen.queryByText("Ubuntu · 无法安全解歧")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("WSL2 发行版")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "应用到 WSL2" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "返回" })).toBeEnabled();
  });

  it("显式授权核验 Stopped 发行版并分别展示配置与生命周期结果", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const provider = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "WSL Provider",
      baseUrl: "https://provider.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const stopped = {
      environmentId: "{11111111-1111-1111-1111-111111111111}",
      displayName: "Ubuntu",
      commandName: "Ubuntu",
      defaultUid: 1000,
      running: false,
      availability: "manageable",
      currentProvider: null,
      actualProviderId: null,
      configurationState: "unknown",
      requiresAttention: false,
      pendingRestart: false,
      revision: "stopped-revision",
      messageId: null,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "list_wsl_environments") return Promise.resolve([stopped]);
      if (command === "refresh_wsl_environment") {
        return Promise.resolve({
          environment: {
            ...stopped,
            running: true,
            currentProvider: provider,
            actualProviderId: provider.id,
            configurationState: "current",
            requiresAttention: true,
            messageId: "wsl.lifecycle_still_running",
          },
          lifecycleOutcome: "still_running",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);
    fireEvent.click(await screen.findByRole("button", { name: "核验实际状态" }));

    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("自然停止"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("refresh_wsl_environment", {
      environmentId: stopped.environmentId,
      expectedRevision: stopped.revision,
      authorizeStart: true,
    }));
    const dialog = screen.getByRole("dialog", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(dialog).toHaveTextContent("已核验 WSL2 发行版“Ubuntu”的实际配置"));
    expect(dialog).toHaveTextContent("仍为 Running");
    expect(dialog).toHaveTextContent("未强制终止用户会话或工作负载");
    expect(screen.queryByRole("button", { name: /强制终止/ })).not.toBeInTheDocument();
  });

  it("展示 Running WSL2 的实际共同管理状态且不接收凭据", async () => {
    const provider = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "WSL Provider",
      baseUrl: "https://provider.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "managed",
          mode: "provider",
          messageId: "environment.managed",
          revision: "revision",
          requiresTakeoverConfirmation: false,
          takeoverAvailable: true,
          impacts: [],
          currentProvider: provider,
          restoreAvailability: "no_backup",
          restorePreview: null,
          loginStatus: "logged_in",
          pendingRestart: false,
          requiresConsumerConfirmation: false,
          consumers: { desktop: "stopped", cli: "stopped" },
        });
      }
      if (command === "list_wsl_environments") {
        const current = {
          environmentId: "{11111111-1111-1111-1111-111111111111}",
          displayName: "Ubuntu",
          commandName: "Ubuntu",
          defaultUid: 1000,
          running: true,
          availability: "manageable",
          currentProvider: provider,
          actualProviderId: provider.id,
          configurationState: "updated",
          requiresAttention: false,
          pendingRestart: true,
          revision: "wsl-revision",
          messageId: null,
        };
        return Promise.resolve([
          current,
          {
            ...current,
            environmentId: "{33333333-3333-4333-8333-333333333333}",
            displayName: "Debian conflict",
            currentProvider: null,
            actualProviderId: null,
            configurationState: "conflict",
            requiresAttention: true,
            messageId: "wsl.managed_conflict",
          },
          {
            ...current,
            environmentId: "{44444444-4444-4444-8444-444444444444}",
            displayName: "Debian busy",
            configurationState: "busy",
            messageId: "wsl.lock_busy",
          },
        ]);
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);

    const dialog = await screen.findByRole("dialog", { name: "选择 WSL2 供应商" });
    expect(dialog).toHaveTextContent("当前，有更新");
    expect(dialog).toHaveTextContent("当前供应商：WSL Provider");
    expect(dialog).toHaveTextContent("命令式凭据工件");
    expect(dialog).toHaveTextContent("auth.json 保持不变");
    expect(dialog.textContent).not.toContain("API Key");

    const distribution = screen.getByLabelText("WSL2 发行版");
    expect(Array.from((distribution as HTMLSelectElement).options, (option) => option.textContent)).toEqual([
      "Ubuntu · 可管理",
      "Debian conflict · 可管理",
      "Debian busy · 可管理",
    ]);
    fireEvent.change(distribution, {
      target: { value: "{33333333-3333-4333-8333-333333333333}" },
    });
    expect(dialog).toHaveTextContent("管理冲突");
    expect(screen.getByRole("button", { name: "应用到 WSL2" })).toBeDisabled();
    fireEvent.change(distribution, {
      target: { value: "{44444444-4444-4444-8444-444444444444}" },
    });
    expect(dialog).toHaveTextContent("正在被其他操作占用");
    expect(screen.getByRole("button", { name: "应用到 WSL2" })).toBeDisabled();
  });

  it("探测失败时仍可打开弹窗重试", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([]);
      if (command === "list_wsl_environments") {
        return Promise.reject({ category: "probe_failed", messageId: "wsl.environment_unavailable" });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "选择 WSL2 供应商" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);

    expect(await screen.findByRole("dialog", { name: "选择 WSL2 供应商" })).toHaveTextContent(
      "暂时无法探测该 WSL2 发行版",
    );
    expect(screen.getByRole("button", { name: "刷新 WSL2" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "应用到 WSL2" })).toBeDisabled();
  });
});

describe("Linux 脚本导出", () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    invoke.mockReset();
    listen.mockReset();
    listen.mockResolvedValue(() => undefined);
  });

  it("原生文件选择器确认覆盖后直接导出 Bash 快照并逐行展示用法", async () => {
    const provider = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "Linux Provider",
      baseUrl: "https://provider.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const destination = "C:\\Users\\example\\gpteasy.sh";
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "list_wsl_environments") return Promise.resolve([]);
      if (command === "get_environment_snapshot") {
        return Promise.resolve({
          state: "external",
          mode: null,
          messageId: "environment.external",
          revision: "linux-export-revision",
          requiresTakeoverConfirmation: true,
          takeoverAvailable: true,
          impacts: [],
          currentProvider: null,
          restoreAvailability: "no_backup",
          restorePreview: null,
          loginStatus: "logged_in",
          pendingRestart: false,
          requiresConsumerConfirmation: false,
          consumers: { desktop: "stopped", cli: "stopped" },
        });
      }
      if (command === "choose_linux_export_destination") {
        return Promise.resolve({ path: destination, exists: true });
      }
      if (command === "export_linux_script") {
        return Promise.resolve({
          exportId: "33333333-3333-4333-8333-333333333333",
          providerCount: 1,
          suggestedFileName: "gpteasy.sh",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "导出 Linux 脚本" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);

    const shellDialog = screen.getByRole("dialog", { name: "导出 Linux 脚本" });
    expect(within(shellDialog).getByRole("radio", { name: "Bash 4+" })).toBeChecked();
    expect(within(shellDialog).getByRole("radio", { name: "Zsh 5+" })).not.toBeChecked();
    expect(within(shellDialog).getByRole("radio", { name: "Zsh 5+" })).toBeEnabled();
    expect(shellDialog).toHaveTextContent("导出文件包含敏感凭据");
    expect(shellDialog).toHaveTextContent("全部已验证供应商");
    expect(shellDialog).toHaveTextContent("仅保存到受信任的当前用户位置");
    expect(screen.queryByRole("dialog", { name: "导出文件包含敏感凭据" })).not.toBeInTheDocument();
    fireEvent.click(within(shellDialog).getByRole("button", { name: "选择保存位置" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("export_linux_script", {
      shell: "bash",
      destination,
      confirmOverwrite: true,
    }));
    expect(screen.queryByRole("dialog", { name: "覆盖已有导出文件？" })).not.toBeInTheDocument();
    const success = await screen.findByRole("dialog", { name: "Bash 脚本已导出" });
    expect(success).toHaveTextContent("建议保护权限");
    expect(success).toHaveTextContent("chmod 600 ./gpteasy.sh");
    expect(success).toHaveTextContent("bash ./gpteasy.sh");
    expect(success).toHaveTextContent("source ./gpteasy.sh");
    expect(success).toHaveTextContent(".bashrc");
    expect(success).toHaveTextContent("gpteasy current");
    expect(success).toHaveTextContent("gpteasy restore");
    expect(success).toHaveTextContent("gpteasy info");
    expect(success).toHaveTextContent("gpteasy unlock");
    expect(within(success).getByText("gpteasy", { selector: "dt code" }).closest("div")).toHaveTextContent("交互选择供应商");
    expect(within(success).getByText("gpteasy current", { selector: "dt code" }).closest("div")).toHaveTextContent("查看当前供应商");
    expect(within(success).getByText("gpteasy restore", { selector: "dt code" }).closest("div")).toHaveTextContent("恢复最近一个 Linux 恢复点");
    const exportCall = invoke.mock.calls.find(([command]) => command === "export_linux_script");
    expect(JSON.stringify(exportCall)).not.toContain("secret");
    expect(JSON.stringify(exportCall)).not.toContain("apiKey");
  });

  it("取消原生保存位置选择时不调用导出命令", async () => {
    const provider = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "Linux Provider",
      baseUrl: "https://provider.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "list_wsl_environments") return Promise.resolve([]);
      if (command === "choose_linux_export_destination") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "导出 Linux 脚本" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);
    fireEvent.click(screen.getByRole("button", { name: "选择保存位置" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "choose_linux_export_destination",
      { shell: "bash" },
    ));
    expect(invoke.mock.calls.some(([command]) => command === "export_linux_script")).toBe(false);
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "导出 Linux 脚本" })).not.toBeInTheDocument();
    });
  });

  it("选择 Zsh 时只导出 gpteasy.zsh 并展示 Zsh 使用方式", async () => {
    const provider = {
      id: "22222222-2222-4222-8222-222222222222",
      name: "Linux Provider",
      baseUrl: "https://provider.example/v1",
      defaultModel: "model-a",
      verifiedAtEpochSeconds: 1_786_140_000,
      isCurrent: false,
    };
    const destination = "C:\\Users\\example\\gpteasy.zsh";
    invoke.mockImplementation((command: string) => {
      if (command === "get_startup_snapshot") return Promise.resolve(readySnapshot);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "list_wsl_environments") return Promise.resolve([]);
      if (command === "choose_linux_export_destination") {
        return Promise.resolve({ path: destination, exists: false });
      }
      if (command === "export_linux_script") {
        return Promise.resolve({
          exportId: "33333333-3333-4333-8333-333333333333",
          providerCount: 1,
          suggestedFileName: "gpteasy.zsh",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<App />);
    const open = await screen.findByRole("button", { name: "导出 Linux 脚本" });
    await waitFor(() => expect(open).toBeEnabled());
    fireEvent.click(open);

    const shellDialog = screen.getByRole("dialog", { name: "导出 Linux 脚本" });
    fireEvent.click(within(shellDialog).getByRole("radio", { name: "Zsh 5+" }));
    fireEvent.click(within(shellDialog).getByRole("button", { name: "选择保存位置" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "choose_linux_export_destination",
      { shell: "zsh" },
    ));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("export_linux_script", {
      shell: "zsh",
      destination,
      confirmOverwrite: false,
    }));
    const success = await screen.findByRole("dialog", { name: "Zsh 脚本已导出" });
    expect(success).toHaveTextContent("zsh ./gpteasy.zsh");
    expect(success).toHaveTextContent("source ./gpteasy.zsh");
    expect(success).toHaveTextContent(".zshrc");
    expect(success).not.toHaveTextContent("gpteasy.sh");
  });
});
