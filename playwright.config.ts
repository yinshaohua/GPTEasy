import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { defineConfig } from "@playwright/test";

const chromePath = process.env.PLAYWRIGHT_CHROME_PATH ?? [
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
].find(existsSync);

export default defineConfig({
  testDir: "./tests/ui",
  outputDir: join(tmpdir(), "gpteasy-playwright-results"),
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:1421",
    browserName: "chromium",
    launchOptions: chromePath ? { executablePath: chromePath } : undefined,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm exec vite -- preview --host 127.0.0.1 --port 1421",
    url: "http://127.0.0.1:1421",
    reuseExistingServer: false,
  },
});
