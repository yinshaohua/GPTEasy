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

async function openProviderCatalog(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  await page.addInitScript(({ catalog }) => {
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
        return undefined;
      },
    };
    Object.assign(window, { __TAURI_INTERNALS__: tauri });
  }, { catalog: providers });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "供应商目录" })).toBeVisible();
  await expect(page.getByText("Long Provider Name")).toBeVisible();
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

  const environmentButtons = page.locator(".environment-command");
  await expect(environmentButtons).toHaveCount(2);
  for (const button of await environmentButtons.all()) {
    const box = await button.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.height).toBeGreaterThanOrEqual(29);
    expect(box!.height).toBeLessThanOrEqual(31);
    expect(box!.y + box!.height).toBeLessThanOrEqual(620);
  }

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
  await expect(page.getByRole("button", { name: "恢复上次配置" })).toBeVisible();
  await expect(page.getByRole("button", { name: "切换到 OpenAI 登录模式" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("provider-layout-680x520.png"), fullPage: true });
});
