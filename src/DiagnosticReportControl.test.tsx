import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import DiagnosticReportControl from "./DiagnosticReportControl";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const report = {
  schemaVersion: 2,
  environment: { scope: "current_user" as const, codexHome: "~/.codex" as const, codexHomeOverrideStatus: "unset" as const, configStatus: "valid" as const, activeProvider: "custom", declaredProviders: [] },
  authentication: { loginStatus: "logged_in" as const, authFileStatus: "present" as const, credentialStore: "file" as const },
  consumers: { desktop: "stopped" as const, cli: "stopped" as const },
  versions: { gpteasy: "1.2.1", codexCli: "0.147.0" },
  findings: [{ code: "model_provider_missing_definition", origin: "local" as const, severity: "error" as const, title: "模型供应商定义缺失", summary: "config.toml 使用模型供应商 custom，但没有声明同名配置。", repairable: true }],
  errors: [],
  repairPreview: { previewId: "preview-1", source: "gpteasy_backup" as const, providerName: "历史供应商", baseUrl: "https://provider.example/v1", model: "gpt-5", authentication: "current_api_key" as const, changes: ["backup_config" as const, "add_custom_provider_definition" as const, "verify_and_rediagnose" as const] },
};
const provider = { id: "provider-1", name: "当前供应商", baseUrl: "https://provider.example/v1", defaultModel: "gpt-5", verifiedAtEpochSeconds: 1, isCurrent: true, recommendationId: null, hasRecommendationUpdate: false };

describe("DiagnosticReportControl", () => {
  beforeEach(() => { cleanup(); invoke.mockReset(); });

  it("matches the Codex action style and reveals diagnostic details on demand", async () => {
    invoke.mockImplementation((command: string) => command === "get_diagnostic_report" ? Promise.resolve(report) : command === "list_providers" ? Promise.resolve([provider]) : Promise.reject(new Error(command)));
    render(<DiagnosticReportControl />);
    const trigger = screen.getByRole("button", { name: "帮帮我" });
    expect(trigger).toHaveClass("secondary-button", "compact");
    expect(trigger).not.toHaveClass("command-button");
    fireEvent.click(trigger);
    const dialog = await screen.findByRole("dialog", { name: "帮帮我" });
    expect(dialog).toBeInTheDocument();
    expect(dialog).toHaveTextContent("AI 将结合脱敏诊断和你输入的问题协助排查");
    expect(dialog).toHaveTextContent("不会直接执行任意命令");
    expect(dialog).not.toHaveTextContent("当前用户 Codex 环境");
    expect(screen.getByText("发现 1 个需要处理的问题")).toBeInTheDocument();
    expect(screen.queryByText("模型供应商定义缺失")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /查看详情/ }));
    expect(screen.getByText("模型供应商定义缺失")).toBeInTheDocument();
    const providerSelect = screen.getByRole("combobox", { name: "对话供应商" });
    const toolbar = providerSelect.closest(".diagnostic-toolbar");
    expect(toolbar).toContainElement(screen.getByRole("button", { name: "复制信息" }));
    expect(toolbar).toContainElement(screen.getByRole("button", { name: "导出信息" }));
    expect(screen.queryByRole("button", { name: /JSON|Markdown/ })).not.toBeInTheDocument();
  });

  it("sends quick prompts through the selected provider and renders an action card", async () => {
    invoke.mockImplementation((command: string, args: { providerId?: string; message?: string } = {}) => {
      if (command === "get_diagnostic_report") return Promise.resolve(report);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "chat_diagnostic_assistant") {
        expect(args.providerId).toBe(provider.id);
        expect(args.message).toBe("无法将供应商设置到 Codex");
        return Promise.resolve({ providerId: provider.id, providerName: provider.name, reply: "可以依据本机证据修复。", repairPlan: [{ id: "repair-custom-provider", findingCode: "model_provider_missing_definition", title: "补回 provider 定义", description: "使用已验证证据。", action: "repair_custom_provider", previewId: "preview-1", requiresConfirmation: true }] });
      }
      if (command === "repair_diagnostic_custom_provider") return Promise.resolve({ status: "succeeded", messageId: "diagnostics.repair_succeeded", report: { ...report, findings: [], repairPreview: null } });
      return Promise.reject(new Error(command));
    });
    render(<DiagnosticReportControl />);
    fireEvent.click(screen.getByRole("button", { name: "帮帮我" }));
    await screen.findByText("发现 1 个需要处理的问题");
    fireEvent.click(screen.getByRole("button", { name: "无法将供应商设置到 Codex" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("chat_diagnostic_assistant", expect.objectContaining({ providerId: provider.id, message: "无法将供应商设置到 Codex" })));
    expect(await screen.findByText("可以依据本机证据修复。")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "查看并确认" }));
    fireEvent.click(screen.getByRole("button", { name: "确认执行" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("repair_diagnostic_custom_provider", { previewId: "preview-1" }));
  });

  it("copies and exports the redacted report and current conversation as one Markdown bundle", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "get_diagnostic_report") return Promise.resolve(report);
      if (command === "list_providers") return Promise.resolve([provider]);
      if (command === "copy_diagnostic_bundle") return Promise.resolve();
      if (command === "choose_diagnostic_export_destination") return Promise.resolve("C:/reports/report.md");
      if (command === "export_diagnostic_bundle") return Promise.resolve();
      return Promise.reject(new Error(command));
    });
    render(<DiagnosticReportControl />);
    fireEvent.click(screen.getByRole("button", { name: "帮帮我" }));
    await screen.findByText("发现 1 个需要处理的问题");
    fireEvent.click(screen.getByRole("button", { name: "复制信息" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("copy_diagnostic_bundle", { conversation: [] }));
    expect(screen.getAllByRole("status").some((element) => element.textContent?.includes("已复制诊断信息"))).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "导出信息" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("choose_diagnostic_export_destination"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("export_diagnostic_bundle", { destination: "C:/reports/report.md", conversation: [] }));
    expect(screen.getAllByRole("status").some((element) => element.textContent?.includes("已导出诊断信息"))).toBe(true);
  });
});
