import { expect, test } from "@playwright/test";

test("会话列表与详情在最小窗口保持可用布局", async ({ page }) => {
  await page.setViewportSize({ width: 680, height: 520 });
  await page.goto("/");
  await page.getByRole("button", { name: "会话管理" }).click();

  await expect(page.getByRole("heading", { name: "会话管理" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "会话" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("searchbox", { name: "搜索标题或预览" })).toBeVisible();
  await expect(page.getByRole("button", { name: /打开会话/ }).first()).toBeVisible();
  const archiveAction = page.getByRole("button", { name: /归档会话：/ }).first();
  await archiveAction.scrollIntoViewIfNeeded();
  await expect(archiveAction).toBeVisible();
  await expect(page.getByRole("button", { name: /永久删除会话：/ }).first()).toBeVisible();

  const documentWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(documentWidth).toBeLessThanOrEqual(680);

  await page.getByRole("button", { name: /打开会话/ }).first().click();
  await expect(page.getByRole("button", { name: "导出 Markdown" })).toBeVisible();
  await expect(page.getByRole("button", { name: "归档会话" })).toBeVisible();
  await expect(page.getByRole("button", { name: "永久删除会话" })).toBeVisible();
  const actionBoxes = await Promise.all([
    page.getByRole("button", { name: "导出 Markdown" }).boundingBox(),
    page.getByRole("button", { name: "归档会话" }).boundingBox(),
    page.getByRole("button", { name: "永久删除会话" }).boundingBox(),
  ]);
  expect(actionBoxes.every((box) => box !== null)).toBe(true);
  expect(Math.max(...actionBoxes.map((box) => box!.y)) - Math.min(...actionBoxes.map((box) => box!.y))).toBeLessThan(4);
  const tool = page.locator("details.session-tool-entry").first();
  await expect(tool).toBeVisible();
  await tool.locator("summary").click();
  await expect(tool).toHaveAttribute("open", "");
  await expect(page.getByText("test result: ok")).toBeVisible();
  await expect(page.getByRole("button", { name: "返回会话列表" })).toBeVisible();

  await page.getByRole("button", { name: "永久删除会话" }).click();
  const dialog = page.getByRole("dialog", { name: "永久删除会话" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/不可撤销/)).toBeVisible();
  await expect(dialog.getByText(/完整范围无法完全列出/)).toBeVisible();
  const dialogBox = await dialog.boundingBox();
  expect(dialogBox).not.toBeNull();
  expect(dialogBox!.x).toBeGreaterThanOrEqual(0);
  expect(dialogBox!.y).toBeGreaterThanOrEqual(0);
  expect(dialogBox!.x + dialogBox!.width).toBeLessThanOrEqual(680);
  expect(dialogBox!.y + dialogBox!.height).toBeLessThanOrEqual(520);
});
