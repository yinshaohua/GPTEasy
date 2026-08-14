import { describe, expect, it } from "vitest";

import { shouldUseBrowserPreview } from "./browser-preview";

describe("shouldUseBrowserPreview", () => {
  it.each(["development", "browser-preview"])(
    "在 %s 模式且没有 Tauri IPC 时启用预览数据",
    (mode) => {
      expect(shouldUseBrowserPreview(mode, false)).toBe(true);
    },
  );

  it("正式构建没有 Tauri IPC 时仍保持失败关闭", () => {
    expect(shouldUseBrowserPreview("production", false)).toBe(false);
  });

  it("存在 Tauri IPC 时始终读取真实状态", () => {
    expect(shouldUseBrowserPreview("browser-preview", true)).toBe(false);
  });
});
