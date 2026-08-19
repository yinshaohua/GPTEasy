import { expect, test } from "@playwright/test";

test("完整会话列表不会把侧栏设置区推离视口", async ({ page }) => {
  await page.setViewportSize({ width: 1120, height: 520 });
  await page.goto("/");
  await page.getByRole("button", { name: "会话管理" }).click();

  await expect(page.getByRole("heading", { name: "会话管理" })).toBeVisible();
  await page.locator(".session-main").evaluate((element) => {
    (element as HTMLElement).style.minHeight = "1600px";
  });

  const settings = page.getByRole("button", { name: "设置" });
  await expect(settings).toBeVisible();
  const beforeScroll = await settings.boundingBox();
  expect(beforeScroll).not.toBeNull();
  expect(beforeScroll!.y + beforeScroll!.height).toBeLessThanOrEqual(520);

  await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
  const afterScroll = await settings.boundingBox();
  expect(afterScroll).not.toBeNull();
  expect(afterScroll!.y).toBeGreaterThanOrEqual(0);
  expect(afterScroll!.y + afterScroll!.height).toBeLessThanOrEqual(520);
});

test("会话列表与详情在最小窗口保持可用布局", async ({ page }) => {
  await page.setViewportSize({ width: 680, height: 520 });
  await page.goto("/");
  await page.getByRole("button", { name: "会话管理" }).click();

  await expect(page.getByRole("heading", { name: "会话管理" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "会话" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("searchbox", { name: "搜索标题或预览" })).toBeVisible();
  await expect(page.getByRole("button", { name: /打开会话/ }).first()).toBeVisible();
  const archiveAction = page.getByRole("button", { name: /归档会话：/ }).first();
  const deleteAction = page.getByRole("button", { name: /永久删除会话：/ }).first();
  await expect(archiveAction).toBeVisible();
  await expect(deleteAction).toBeVisible();
  const tableViewport = await page.locator(".session-table-wrap").boundingBox();
  const archiveBox = await archiveAction.boundingBox();
  const deleteBox = await deleteAction.boundingBox();
  expect(tableViewport).not.toBeNull();
  expect(archiveBox).not.toBeNull();
  expect(deleteBox).not.toBeNull();
  expect(archiveBox!.x).toBeGreaterThanOrEqual(tableViewport!.x);
  expect(deleteBox!.x + deleteBox!.width).toBeLessThanOrEqual(tableViewport!.x + tableViewport!.width);

  const documentWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  expect(documentWidth).toBeLessThanOrEqual(680);

  const firstSelection = page.getByRole("checkbox", { name: /选择会话：/ }).first();
  await firstSelection.check();
  const selectionToolbar = page.getByLabel("已选会话操作");
  await expect(selectionToolbar.getByRole("button", { name: "归档所选会话" })).toBeVisible();
  await expect(selectionToolbar.getByRole("button", { name: "永久删除所选会话" })).toBeVisible();
  const selectionToolbarBox = await selectionToolbar.boundingBox();
  expect(selectionToolbarBox).not.toBeNull();
  expect(selectionToolbarBox!.x).toBeGreaterThanOrEqual(0);
  expect(selectionToolbarBox!.x + selectionToolbarBox!.width).toBeLessThanOrEqual(680);

  await selectionToolbar.getByRole("button", { name: "永久删除所选会话" }).click();
  const bulkDeleteDialog = page.getByRole("dialog", { name: "永久删除所选会话" });
  await expect(bulkDeleteDialog).toBeVisible();
  await expect(bulkDeleteDialog.getByRole("button", { name: "永久删除 1 个会话" })).toBeVisible();
  const bulkDeleteDialogBox = await bulkDeleteDialog.boundingBox();
  expect(bulkDeleteDialogBox).not.toBeNull();
  expect(bulkDeleteDialogBox!.x).toBeGreaterThanOrEqual(0);
  expect(bulkDeleteDialogBox!.y).toBeGreaterThanOrEqual(0);
  expect(bulkDeleteDialogBox!.x + bulkDeleteDialogBox!.width).toBeLessThanOrEqual(680);
  expect(bulkDeleteDialogBox!.y + bulkDeleteDialogBox!.height).toBeLessThanOrEqual(520);
  await bulkDeleteDialog.getByRole("button", { name: "取消" }).click();
  await firstSelection.uncheck();

  await archiveAction.click();
  await expect(selectionToolbar).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: /选择会话：/ }).first()).not.toBeChecked();

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
