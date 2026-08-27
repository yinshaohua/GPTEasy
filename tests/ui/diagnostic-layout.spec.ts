import { expect, test, type Page } from "@playwright/test";

const provider = {
  id: "layout-provider",
  name: "当前供应商",
  baseUrl: "https://provider.example/v1",
  defaultModel: "gpt-5",
  verifiedAtEpochSeconds: 1_786_140_000,
  isCurrent: true,
  recommendationId: null,
  hasRecommendationUpdate: false,
};

async function openDiagnosticDialog(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  await page.addInitScript(({ currentProvider }) => {
    let callbackId = 1;
    const callbacks = new Map<number, (...args: unknown[]) => void>();
    const tauri = {
      callbacks,
      transformCallback(callback: (...args: unknown[]) => void, once = false) {
        const id = callbackId++;
        callbacks.set(id, (...args: unknown[]) => {
          callback(...args);
          if (once) callbacks.delete(id);
        });
        return id;
      },
      unregisterCallback(id: number) {
        callbacks.delete(id);
      },
      runCallback(id: number, data: unknown) {
        callbacks.get(id)?.(data);
      },
      async invoke(command: string) {
        if (command === "get_startup_snapshot") {
          return {
            mode: "ready",
            messageId: "startup.database_initialized",
            blockReason: null,
            pendingOperationResolution: null,
            database: {
              status: "initialized",
              schemaVersion: 1,
              reason: null,
              contents: {
                providerCount: 1,
                hasLastAppliedState: true,
                hasPendingConfigOperation: false,
                pendingRestart: false,
                pendingConfigOperation: null,
              },
            },
            codex: {
              configStatus: "valid",
              configFingerprint: "diagnostic-layout",
              credentialStore: "file",
              credentialFileStatus: "present",
              loginStatus: "logged_in",
            },
          };
        }
        if (command === "list_providers") return [currentProvider];
        if (command === "list_wsl_environments") return [];
        if (command === "get_update_snapshot") {
          return {
            currentVersion: "1.2.1",
            state: "idle",
            availableVersion: null,
            notes: null,
            publishedAt: null,
            checkedAtEpochSeconds: null,
            downloadedBytes: null,
            totalBytes: null,
            progressPercent: null,
            failureCategory: null,
            errorMessage: null,
            manualDownloadUrl: null,
            releaseNotesUrl: null,
          };
        }
        if (command === "get_environment_snapshot") {
          return {
            state: "managed",
            mode: "provider",
            messageId: "environment.managed",
            revision: "diagnostic-layout",
            requiresTakeoverConfirmation: false,
            requiresConsumerConfirmation: false,
            takeoverAvailable: true,
            restoreAvailability: "no_backup",
            restorePreview: null,
            loginStatus: "logged_in",
            pendingRestart: false,
            consumers: { desktop: "stopped", cli: "stopped" },
            impacts: [],
            currentProvider,
          };
        }
        if (command === "get_desktop_snapshot") {
          return {
            status: "stopped",
            action: "start",
            messageId: "desktop.ready_to_start",
            roots: [],
          };
        }
        if (command === "get_diagnostic_report") {
          return {
            schemaVersion: 2,
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
              summary: "缺少同名供应商配置。",
              repairable: true,
            }],
            errors: [],
            repairPreview: null,
          };
        }
        if (command === "chat_diagnostic_assistant") {
          return {
            providerId: currentProvider.id,
            providerName: currentProvider.name,
            reply: Array.from({ length: 80 }, (_, index) => `诊断说明 ${index + 1}`).join("\n"),
            repairPlan: [],
          };
        }
        if (command === "plugin:event|listen") return callbackId++;
        if (command === "plugin:event|unlisten") return undefined;
        return undefined;
      },
    };
    Object.assign(window, { __TAURI_INTERNALS__: tauri });
  }, { currentProvider: provider });
  await page.goto("/");
  const startButton = page.getByRole("button", { name: "启动 Codex" });
  const helpButton = page.getByRole("button", { name: "帮帮我" });
  await expect(startButton).toBeVisible();
  await expect(helpButton).toBeVisible();
  const buttonStyles = await Promise.all([startButton, helpButton].map((button) => button.evaluate((element) => {
    const style = window.getComputedStyle(element);
    return {
      background: style.backgroundColor,
      border: style.borderColor,
      color: style.color,
      height: element.getBoundingClientRect().height,
    };
  })));
  expect(buttonStyles[1]).toEqual(buttonStyles[0]);
  await helpButton.click();
  const dialog = page.getByRole("dialog", { name: "帮帮我" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "无法将供应商设置到 Codex" }).click();
  await expect(dialog.locator(".diagnostic-chat-bubble.assistant")).toBeVisible();
  return dialog;
}

for (const viewport of [{ width: 1120, height: 900 }, { width: 680, height: 520 }]) {
  test(`帮帮我弹框在 ${viewport.width}x${viewport.height} 仅滚动对话内容`, async ({ page }) => {
    const dialog = await openDiagnosticDialog(page, viewport.width, viewport.height);
    const layout = await dialog.evaluate((element) => {
      const scrollable = [...element.querySelectorAll<HTMLElement>("*")]
        .filter((candidate) => {
          const overflowY = window.getComputedStyle(candidate).overflowY;
          return (overflowY === "auto" || overflowY === "scroll")
            && candidate.scrollHeight > candidate.clientHeight;
        })
        .map((candidate) => candidate.className);
      const bounds = element.getBoundingClientRect();
      const providerBounds = element.querySelector(".diagnostic-toolbar label")!.getBoundingClientRect();
      const exportButtons = [...element.querySelectorAll<HTMLElement>(".diagnostic-export-button")]
        .map((button) => button.getBoundingClientRect());
      return {
        bounds: { left: bounds.left, top: bounds.top, right: bounds.right, bottom: bounds.bottom },
        dialogOverflow: window.getComputedStyle(element).overflowY,
        scrollable,
        providerTop: providerBounds.top,
        exportTops: exportButtons.map((button) => button.top),
        exportHeights: exportButtons.map((button) => button.height),
      };
    });

    expect(layout.bounds.left).toBeGreaterThanOrEqual(0);
    expect(layout.bounds.top).toBeGreaterThanOrEqual(0);
    expect(layout.bounds.right).toBeLessThanOrEqual(viewport.width);
    expect(layout.bounds.bottom).toBeLessThanOrEqual(viewport.height);
    expect(layout.dialogOverflow).toBe("hidden");
    expect(layout.scrollable).toEqual(["diagnostic-chat-scroll"]);
    expect(layout.exportTops.every((top) => Math.abs(top - layout.providerTop) < 2)).toBe(true);
    expect(layout.exportHeights.every((height) => height <= 32)).toBe(true);
    await expect(dialog).not.toContainText("当前用户 Codex 环境");
    await expect(dialog).toContainText("不会直接执行任意命令");
  });
}
