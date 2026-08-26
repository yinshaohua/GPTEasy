import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import DiagnosticReportControl from "./DiagnosticReportControl";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

const report = {
  schemaVersion: 1,
  environment: {
    scope: "current_user",
    codexHome: "~/.codex",
    codexHomeOverrideStatus: "unset",
    configStatus: "valid",
    activeProvider: "custom",
    declaredProviders: [],
  },
  authentication: {
    loginStatus: "logged_in",
    authFileStatus: "present",
    credentialStore: "file",
  },
  consumers: { desktop: "running", cli: "stopped" },
  versions: { gpteasy: "1.2.1", codexCli: "0.147.0" },
  findings: [{
    code: "model_provider_missing_definition",
    origin: "local",
    severity: "error",
    title: "模型供应商定义缺失",
    summary: "config.toml 使用模型供应商“custom”，但没有声明同名配置。",
    repairable: false,
  }],
  errors: [],
};

describe("DiagnosticReportControl", () => {
  beforeEach(() => {
    cleanup();
    invoke.mockReset();
  });

  it("shows busy, successful report, and no safe repair state from the top action", async () => {
    const request = deferred<typeof report>();
    invoke.mockImplementation((command: string) => {
      if (command === "get_diagnostic_report") return request.promise;
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    render(<DiagnosticReportControl />);

    fireEvent.click(screen.getByRole("button", { name: "帮我排查" }));

    expect(screen.getByRole("dialog", { name: "本机诊断报告" })).toBeInTheDocument();
    expect(screen.getByText("正在检查当前用户 Codex 环境")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "帮我排查" })).toBeDisabled();

    request.resolve(report);

    expect(await screen.findByText("诊断完成")).toBeInTheDocument();
    expect(screen.getByText("模型供应商定义缺失")).toBeInTheDocument();
    expect(screen.getByText("custom")).toBeInTheDocument();
    expect(screen.getByText("没有可安全自动修复的项目")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: "帮我排查" })).toBeEnabled());
  });

  it("shows a failure state and can retry the diagnosis", async () => {
    const retry = deferred<typeof report>();
    invoke
      .mockRejectedValueOnce({ messageId: "diagnostics.report_failed" })
      .mockImplementation((command: string) => {
        if (command === "get_diagnostic_report") return retry.promise;
        return Promise.reject(new Error(`unexpected command: ${command}`));
      });
    render(<DiagnosticReportControl />);

    fireEvent.click(screen.getByRole("button", { name: "帮我排查" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("诊断失败");
    fireEvent.click(screen.getByRole("button", { name: "重新检查" }));
    retry.resolve(report);

    expect(await screen.findByText("诊断完成")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("exports redacted JSON and Markdown through backend commands", async () => {
    invoke.mockImplementation((command: string, arguments_: { format?: string } = {}) => {
      if (command === "get_diagnostic_report") return Promise.resolve(report);
      if (command === "choose_diagnostic_export_destination") {
        return Promise.resolve(`C:/reports/report.${arguments_.format === "json" ? "json" : "md"}`);
      }
      if (command === "export_diagnostic_report") return Promise.resolve();
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    render(<DiagnosticReportControl />);
    fireEvent.click(screen.getByRole("button", { name: "帮我排查" }));
    await screen.findByText("诊断完成");

    fireEvent.click(screen.getByRole("button", { name: "导出 JSON" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "export_diagnostic_report",
      { format: "json", destination: "C:/reports/report.json" },
    ));
    expect(screen.getByRole("status")).toHaveTextContent("JSON 已导出");

    fireEvent.click(screen.getByRole("button", { name: "导出 Markdown" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "export_diagnostic_report",
      { format: "markdown", destination: "C:/reports/report.md" },
    ));
    expect(screen.getByRole("status")).toHaveTextContent("Markdown 已导出");
  });
});
