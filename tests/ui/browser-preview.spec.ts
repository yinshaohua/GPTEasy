import { expect, test } from "@playwright/test";

test("浏览器预览无需 Tauri IPC 也能打开供应商目录", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "供应商目录" })).toBeVisible();
  await expect(page.getByText("无法读取启动状态")).toHaveCount(0);
  await expect(page.getByText("Rust 后端暂时无法返回可信状态。")).toHaveCount(0);
});
