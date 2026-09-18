import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "tests",
  testMatch: "**/*.spec.ts",
  timeout: 30000,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:1432",
    browserName: "chromium",
    channel: "msedge",
    headless: true,
    viewport: { width: 1200, height: 820 },
  },
  webServer: {
    command: `${JSON.stringify(process.execPath)} node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1432`,
    url: "http://127.0.0.1:1432",
    reuseExistingServer: !process.env.CI,
    timeout: 30000,
  },
});
