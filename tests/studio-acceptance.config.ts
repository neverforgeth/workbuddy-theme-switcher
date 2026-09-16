import { defineConfig } from "@playwright/test";
import base from "../playwright.config";
import { fileURLToPath } from "node:url";

// Keep the local test server out of any inherited machine proxy.
process.env.NO_PROXY = [process.env.NO_PROXY, "127.0.0.1", "localhost"]
  .filter(Boolean)
  .join(",");

export default defineConfig({
  ...base,
  testDir: ".",
  testMatch: "studio-acceptance.spec.ts",
  webServer: {
    command: "node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 1432",
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    port: 1432,
    reuseExistingServer: true,
    timeout: 30000,
  },
});
