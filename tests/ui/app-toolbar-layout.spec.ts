import { expect, test, type Locator, type Page } from "@playwright/test";

type Viewport = { width: number; height: number };

async function expectChildrenNotToOverlap(container: Locator, selector: string) {
  const boxes = await container.locator(selector).evaluateAll((elements) => elements
    .filter((element) => {
      const style = window.getComputedStyle(element);
      return style.visibility !== "hidden" && style.display !== "none";
    })
    .map((element) => {
      const bounds = element.getBoundingClientRect();
      return {
        left: bounds.left,
        top: bounds.top,
        right: bounds.right,
        bottom: bounds.bottom,
      };
    }));

  for (let left = 0; left < boxes.length; left += 1) {
    for (let right = left + 1; right < boxes.length; right += 1) {
      const a = boxes[left];
      const b = boxes[right];
      const overlaps = a.left < b.right - 0.5
        && a.right > b.left + 0.5
        && a.top < b.bottom - 0.5
        && a.bottom > b.top + 0.5;
      expect(overlaps, `元素 ${left} 与 ${right} 不应重叠`).toBe(false);
    }
  }
}

async function expectNoHorizontalOverflow(page: Page, width: number) {
  const documentWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(documentWidth).toBeLessThanOrEqual(width);
}

for (const viewport of [{ width: 680, height: 520 }, { width: 1120, height: 800 }] satisfies Viewport[]) {
  test(`全局与页面工具栏在 ${viewport.width}x${viewport.height} 不重叠`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/");

    const titleBar = page.getByRole("banner", { name: "全局标题栏" });
    await expect(titleBar.getByRole("heading", { name: "供应商管理" })).toBeVisible();
    await expect(titleBar.getByRole("button", { name: "启动 Codex" })).toBeVisible();
    await expect(titleBar.getByRole("button", { name: "帮帮我" })).toBeVisible();
    const visibilityStatus = page.getByRole("status", { name: "会话可见性自动修复状态" });
    await expect(visibilityStatus).toContainText("模式已切换");
    await expectChildrenNotToOverlap(titleBar, ":scope > h1, :scope > .app-header-actions > .desktop-control > button, :scope > .app-header-actions > button");
    const [titleBounds, statusBounds] = await Promise.all([
      titleBar.boundingBox(),
      visibilityStatus.boundingBox(),
    ]);
    expect(titleBounds).not.toBeNull();
    expect(statusBounds).not.toBeNull();
    expect(statusBounds!.y).toBeGreaterThanOrEqual(titleBounds!.y + titleBounds!.height - 0.5);
    await expectNoHorizontalOverflow(page, viewport.width);

    await page.getByRole("button", { name: "会话管理" }).click();
    await expect(titleBar.getByRole("heading", { name: "会话管理" })).toBeVisible();
    const sessionToolbar = page.getByRole("region", { name: "会话列表工具栏" });
    await expect(sessionToolbar.getByRole("tab", { name: "会话" })).toBeVisible();
    await expect(sessionToolbar.getByRole("tab", { name: "已归档" })).toBeVisible();
    await expect(sessionToolbar.getByRole("button", { name: "修复会话" })).toBeVisible();
    await expect(sessionToolbar.getByRole("button", { name: "刷新会话列表" })).toBeVisible();
    const sessionToolbarBounds = await sessionToolbar.boundingBox();
    expect(sessionToolbarBounds).not.toBeNull();
    expect(sessionToolbarBounds!.y).toBeGreaterThanOrEqual(statusBounds!.y + statusBounds!.height - 0.5);
    await expectChildrenNotToOverlap(sessionToolbar, "[role='tab'], .session-list-actions > button");
    await sessionToolbar.getByRole("tab", { name: "已归档" }).focus();
    await page.keyboard.press("Enter");
    await expect(sessionToolbar.getByRole("tab", { name: "已归档" })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("button", { name: /打开会话/ }).first()).toBeVisible();
    await page.getByRole("button", { name: /打开会话/ }).first().click();
    const detailToolbar = page.getByRole("toolbar", { name: "会话详情工具栏" });
    await expect(detailToolbar).toBeVisible();
    await expect(detailToolbar.getByRole("button", { name: "返回会话列表" })).toBeVisible();
    await expectChildrenNotToOverlap(detailToolbar, ":scope > button, :scope > h2, :scope > .session-detail-actions > button");
    await detailToolbar.getByRole("button", { name: "返回会话列表" }).focus();
    await page.keyboard.press("Enter");
    await expect(sessionToolbar).toBeVisible();
    await expectChildrenNotToOverlap(page.getByLabel("会话筛选"), ":scope > label");
    await expectNoHorizontalOverflow(page, viewport.width);

    await page.getByRole("button", { name: "设置", exact: true }).click();
    await page.getByRole("menuitem", { name: "问题日志" }).click();
    await expect(titleBar.getByRole("heading", { name: "问题日志" })).toBeVisible();
    const issueToolbar = page.getByRole("region", { name: "问题日志筛选与操作" });
    await expect(issueToolbar.getByText("时间范围")).toBeVisible();
    await expect(issueToolbar.getByRole("button", { name: "刷新" })).toBeVisible();
    await expect(issueToolbar.getByRole("button", { name: "复制" })).toBeVisible();
    await expect(issueToolbar.getByRole("button", { name: /^导出$/ })).toBeVisible();
    await expectChildrenNotToOverlap(issueToolbar, ":scope > label, :scope > .issue-log-actions > button");
    await expectNoHorizontalOverflow(page, viewport.width);
  });
}
