import { expect, test, type Page } from "@playwright/test";

const providers = [
  {
    id: "dayway-provider",
    name: "DayWay",
    baseUrl: "https://dayway.site/v1",
    defaultModel: "dayway-codex-model",
    verifiedAtEpochSeconds: 1_786_140_000,
    isCurrent: true,
    recommendationId: "dayway",
    hasRecommendationUpdate: false,
  },
  {
    id: "long-provider",
    name: "Long Provider Name",
    baseUrl: "https://provider.example/very/long/responses/compatible/api/v1",
    defaultModel: "provider-model-with-a-very-long-version-identifier",
    verifiedAtEpochSeconds: 1_786_140_900,
    isCurrent: false,
    recommendationId: null,
    hasRecommendationUpdate: false,
  },
];

const pendingUpdate = {
  currentVersion: "1.1.1",
  state: "pending",
  availableVersion: "1.2.0",
  notes: "布局测试更新",
  publishedAt: "2026-08-18T00:00:00Z",
  checkedAtEpochSeconds: 1_787_027_200,
  downloadedBytes: 100,
  totalBytes: 100,
  progressPercent: 100,
  failureCategory: null,
  errorMessage: null,
  manualDownloadUrl: "https://github.com/yinshaohua/GPTEasy/releases/latest",
  releaseNotesUrl: "https://gitcode.com/ericyin99/GPTEasy-Releases/releases/tag/v1.2.0",
};

async function openProviderCatalog(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  await page.addInitScript(({ catalog, updateSnapshot }) => {
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
                providerCount: catalog.length,
                hasLastAppliedState: true,
                hasPendingConfigOperation: false,
                pendingRestart: false,
                pendingConfigOperation: null,
              },
            },
            codex: {
              configStatus: "valid",
              configFingerprint: "layout-fixture",
              credentialStore: "file",
              credentialFileStatus: "present",
              loginStatus: "logged_in",
            },
          };
        }
        if (command === "list_providers") return catalog;
        if (command === "get_update_snapshot") return updateSnapshot;
        if (command === "list_wsl_environments") {
          return [{
            environmentId: "{11111111-1111-1111-1111-111111111111}",
            displayName: "Ubuntu 24.04",
            commandName: "Ubuntu-24.04",
            defaultUid: 1000,
            running: false,
            availability: "manageable",
            currentProvider: catalog[1],
            actualProviderId: catalog[1].id,
            configurationState: "current",
            requiresAttention: false,
            pendingRestart: false,
            revision: "layout-wsl-revision",
            messageId: null,
          }];
        }
        if (command === "get_environment_snapshot") {
          return {
            state: "managed",
            mode: "provider",
            messageId: "environment.managed",
            revision: "layout-revision",
            requiresTakeoverConfirmation: false,
            requiresConsumerConfirmation: false,
            takeoverAvailable: true,
            restoreAvailability: "no_backup",
            restorePreview: null,
            loginStatus: "logged_in",
            pendingRestart: false,
            consumers: { desktop: "stopped", cli: "stopped" },
            impacts: [],
            currentProvider: catalog[0],
          };
        }
        if (command === "plugin:event|listen") return callbackId++;
        if (command === "plugin:event|unlisten") return undefined;
        if (command === "apply_wsl_provider") return new Promise(() => undefined);
        if (command === "choose_linux_export_destination") {
          return { path: "C:\\Users\\example\\gpteasy.sh", exists: false };
        }
        if (command === "export_linux_script") {
          return {
            exportId: "33333333-3333-4333-8333-333333333333",
            providerCount: catalog.length,
            suggestedFileName: "gpteasy.sh",
          };
        }
        return undefined;
      },
    };
    Object.assign(window, { __TAURI_INTERNALS__: tauri });
  }, { catalog: providers, updateSnapshot: pendingUpdate });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "供应商目录" })).toBeVisible();
  await expect(page.getByLabel("已验证供应商").getByText("Long Provider Name")).toBeVisible();
  await expect(page.getByRole("button", { name: "更新" })).toBeVisible();
}

test("默认窗口横向展示目录行且底部操作可见", async ({ page }, testInfo) => {
  await openProviderCatalog(page, 1120, 620);

  const measurements = await page.locator(".provider-list-row").evaluateAll((rows) =>
    rows.map((row) => {
      const summary = row.querySelector(".provider-row-summary")!.getBoundingClientRect();
      const actions = row.querySelector(".provider-row-actions")!.getBoundingClientRect();
      const buttons = [...row.querySelectorAll<HTMLButtonElement>(".provider-row-actions button")];
      const titleItems = [...row.querySelectorAll<HTMLElement>(".provider-row-title > *")];
      return {
        summary: { top: summary.top, bottom: summary.bottom, width: summary.width },
        actions: { top: actions.top, bottom: actions.bottom },
        buttonHeights: buttons.map((button) => button.getBoundingClientRect().height),
        titleTops: titleItems.map((item) => item.getBoundingClientRect().top),
      };
    }),
  );

  for (const row of measurements) {
    expect(row.actions.top).toBeLessThan(row.summary.bottom);
    expect(row.actions.bottom).toBeGreaterThan(row.summary.top);
    expect(row.summary.width).toBeGreaterThan(400);
    expect(row.buttonHeights.every((height) => height >= 29 && height <= 31)).toBe(true);
    expect(new Set(row.titleTops.map(Math.round)).size).toBe(1);
  }

  const environmentButtons = page.locator(".environment-tools button");
  await expect(environmentButtons).toHaveCount(2);
  for (const button of await environmentButtons.all()) {
    const box = await button.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.height).toBeGreaterThanOrEqual(29);
    expect(box!.height).toBeLessThanOrEqual(31);
    expect(box!.y + box!.height).toBeLessThanOrEqual(620);
  }

  const wslProvider = page.getByLabel("WSL2 当前供应商");
  await expect(wslProvider).toContainText("Long Provider Name");
  await expect(wslProvider).toContainText("https://provider.example/very/long/responses/compatible/api/v1");
  await expect(wslProvider).toContainText("provider-model-with-a-very-long-version-identifier");

  const providerName = page.locator(".sidebar-provider-name");
  const updateIndicator = page.getByRole("button", { name: "更新" });
  const providerBeforeHover = await providerName.boundingBox();
  const indicatorBeforeHover = await updateIndicator.boundingBox();
  const updateIconBeforeHover = await updateIndicator.locator("svg").boundingBox();
  expect(indicatorBeforeHover).not.toBeNull();
  expect(updateIconBeforeHover).not.toBeNull();
  expect(indicatorBeforeHover!.width).toBe(32);
  expect(indicatorBeforeHover!.height).toBe(32);
  expect(updateIconBeforeHover!.x + updateIconBeforeHover!.width / 2)
    .toBeCloseTo(indicatorBeforeHover!.x + indicatorBeforeHover!.width / 2, 1);
  await page.screenshot({ path: testInfo.outputPath("update-ready-collapsed-1120x620.png"), fullPage: true });
  await updateIndicator.hover();
  await expect(updateIndicator.locator(".sidebar-update-label")).toHaveCSS("opacity", "1");
  const providerAfterHover = await providerName.boundingBox();
  const indicatorAfterHover = await updateIndicator.boundingBox();
  expect(providerBeforeHover).not.toBeNull();
  expect(indicatorBeforeHover).not.toBeNull();
  expect(providerAfterHover).not.toBeNull();
  expect(indicatorAfterHover).not.toBeNull();
  expect(providerAfterHover!.x).toBeCloseTo(providerBeforeHover!.x, 1);
  expect(providerAfterHover!.width).toBeCloseTo(providerBeforeHover!.width, 1);
  expect(indicatorAfterHover!.width).toBeGreaterThan(indicatorBeforeHover!.width);

  await page.screenshot({ path: testInfo.outputPath("update-ready-hover-1120x620.png"), fullPage: true });

  await page.screenshot({ path: testInfo.outputPath("provider-layout-1120x620.png"), fullPage: true });
});

test("最小窗口无横向溢出且所有操作可滚动到达", async ({ page }, testInfo) => {
  await openProviderCatalog(page, 680, 520);

  const layout = await page.evaluate(() => {
    const intersects = (a: DOMRect, b: DOMRect) =>
      a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    const actionGroups = [...document.querySelectorAll<HTMLElement>(".provider-row-actions")];
    const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((button) => button.offsetParent !== null);
    const overlaps: string[] = [];
    for (const group of actionGroups) {
      const children = [...group.querySelectorAll<HTMLElement>("button")];
      for (let i = 0; i < children.length; i += 1) {
        for (let j = i + 1; j < children.length; j += 1) {
          if (intersects(children[i].getBoundingClientRect(), children[j].getBoundingClientRect())) {
            overlaps.push(`${children[i].textContent}/${children[j].textContent}`);
          }
        }
      }
    }
    return {
      scrollWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
      overlaps,
      clippedButtons: buttons.filter((button) => {
        const box = button.getBoundingClientRect();
        return box.left < 0 || box.right > window.innerWidth;
      }).map((button) => button.textContent),
    };
  });

  expect(layout.scrollWidth).toBeLessThanOrEqual(layout.viewportWidth);
  expect(layout.overlaps).toEqual([]);
  expect(layout.clippedButtons).toEqual([]);

  await page.getByRole("region", { name: "Codex 环境操作" }).scrollIntoViewIfNeeded();
  await expect(page.getByRole("button", { name: "恢复上次配置" })).toHaveCount(0);
  await expect(page.getByText("其他环境供应商操作")).toHaveCount(0);
  await expect(page.getByText("当前 Windows Codex 环境操作")).toHaveCount(0);
  await page.getByRole("button", { name: "设置" }).click();
  await expect(page.getByRole("menuitem", { name: "返回 OpenAI 登录模式" })).toBeVisible();
  await expect(page.getByText("当前用户")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("provider-layout-680x520.png"), fullPage: true });
});

test("WSL2 供应商弹窗在最小窗口展示单发行版范围和生命周期提示", async ({ page }, testInfo) => {
  await openProviderCatalog(page, 680, 520);
  await page.getByRole("button", { name: "选择 WSL2 供应商" }).click();

  const dialog = page.getByRole("dialog", { name: "选择 WSL2 供应商" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Ubuntu 24.04", { exact: true })).toBeVisible();
  await expect(dialog.getByText(/最多等待 10 秒自然停止，绝不强制终止/)).toBeVisible();
  await expect(dialog.getByText(/命令式凭据工件；auth.json 保持不变/)).toBeVisible();
  await expect(dialog.getByText("请选择发行版")).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "应用到 WSL2" })).toBeEnabled();

  const bounds = await dialog.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(680);
  expect(bounds!.width).toBeGreaterThanOrEqual(600);
  expect(bounds!.height).toBeLessThan(430);
  const paragraphMargins = await dialog.locator(".wsl-environment-detail p").evaluateAll((paragraphs) =>
    paragraphs.map((paragraph) => {
      const style = window.getComputedStyle(paragraph);
      return { top: style.marginTop, bottom: style.marginBottom };
    }),
  );
  expect(paragraphMargins.every(({ top, bottom }) => top === "0px" && bottom === "0px")).toBe(true);

  const actionButtons = dialog.locator(".dialog-actions button");
  const before = await actionButtons.evaluateAll((buttons) => buttons.map((button) => {
    const bounds = button.getBoundingClientRect();
    return { top: bounds.top, height: bounds.height };
  }));
  await dialog.getByRole("button", { name: "应用到 WSL2" }).click();
  await expect(dialog.getByRole("button", { name: "正在应用 WSL2 供应商" })).toBeVisible();
  const after = await actionButtons.evaluateAll((buttons) => buttons.map((button) => {
    const bounds = button.getBoundingClientRect();
    return { top: bounds.top, height: bounds.height };
  }));
  expect(after.map(({ top }) => Math.round(top))).toEqual(before.map(({ top }) => Math.round(top)));
  expect(after.map(({ height }) => Math.round(height))).toEqual(before.map(({ height }) => Math.round(height)));
  await page.screenshot({ path: testInfo.outputPath("wsl-dialog-680x520.png"), fullPage: true });
});

test("Linux 导出首屏说明凭据风险且成功用法逐行紧排", async ({ page }, testInfo) => {
  await openProviderCatalog(page, 680, 520);
  await page.getByRole("button", { name: "导出 Linux 脚本" }).click();

  const exportDialog = page.getByRole("dialog", { name: "导出 Linux 脚本" });
  await expect(exportDialog).toContainText("导出文件包含敏感凭据");
  await expect(exportDialog).toContainText("仅保存到受信任的当前用户位置");
  await expect(page.getByRole("dialog", { name: "导出文件包含敏感凭据" })).toHaveCount(0);
  const sensitiveNote = exportDialog.locator(".linux-export-sensitive-note");
  const sensitiveNoteColors = await sensitiveNote.evaluate((note) => {
    const style = window.getComputedStyle(note);
    const dangerProbe = document.createElement("span");
    dangerProbe.style.color = "var(--danger)";
    document.body.append(dangerProbe);
    const danger = window.getComputedStyle(dangerProbe).color;
    dangerProbe.remove();
    return {
      foreground: style.color,
      border: style.borderLeftColor,
      danger,
    };
  });
  expect(sensitiveNoteColors.foreground).not.toBe(sensitiveNoteColors.danger);
  expect(sensitiveNoteColors.border).not.toBe(sensitiveNoteColors.danger);
  await page.screenshot({ path: testInfo.outputPath("linux-export-first-dialog-680x520.png"), fullPage: true });

  await exportDialog.getByRole("button", { name: "选择保存位置" }).click();
  const successDialog = page.getByRole("dialog", { name: "Bash 脚本已导出" });
  await expect(successDialog).toBeVisible();
  await expect(successDialog).toContainText("建议保护权限");
  await expect(successDialog).toContainText("chmod 600 ./gpteasy.sh");
  const rows = successDialog.locator(".linux-command-instructions > div");
  await expect(rows).toHaveCount(5);
  const rowBounds = await rows.evaluateAll((items) => items.map((item) => {
    const bounds = item.getBoundingClientRect();
    return { top: bounds.top, bottom: bounds.bottom };
  }));
  const gaps = rowBounds.slice(1).map((bounds, index) => bounds.top - rowBounds[index].bottom);
  expect(gaps.every((gap) => Math.abs(gap) < 0.5)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("linux-export-success-680x520.png"), fullPage: true });
});
