import { expect, test } from "@playwright/test";
import fs from "node:fs";
const probe = fs.readFileSync("src-tauri/src/live-dom-probe.js", "utf8");
test("local capture does not scan private text or prepare an upload mask", async ({
  page,
}) => {
  await page.setContent(
    '<div class="teams-container"><aside class="conversation-sidebar">PRIVATE-NAME</aside><main class="wb-home-page"><p>PRIVATE-MESSAGE</p><input value="PRIVATE-INPUT"></main></div>',
  );
  const geometry = await page.evaluate(probe);
  expect(JSON.stringify(geometry)).not.toContain("PRIVATE-");
  expect(geometry.masks).toBeUndefined();
  expect(probe).not.toContain("createTreeWalker");
  expect(geometry.scene).toBe("home");
});
test("capture consistency tracks changes without returning user content", async ({
  page,
}) => {
  await page.setContent(
    '<div class="teams-container"><aside class="conversation-sidebar">x</aside><main class="wb-home-page"><p>private</p></main></div>',
  );
  const before = await page.evaluate(probe);
  await page.locator("p").evaluate((e) => {
    e.textContent = "CHANGED-PRIVATE";
  });
  const after = await page.evaluate(probe);
  expect(after.mutation_epoch).toBeGreaterThan(before.mutation_epoch);
  expect(JSON.stringify(after)).not.toContain("CHANGED-PRIVATE");
});
test("production IPC surface has no AI commands and CSP forbids external connections", () => {
  const source = fs.readFileSync("src-tauri/src/lib.rs", "utf8");
  const handlers = source.split("tauri::generate_handler![")[1].split("])")[0];
  for (const name of [
    "studio_prepare_ai",
    "studio_send_ai",
    "studio_mask_ai",
    "studio_ai_settings",
    "get_ai_settings",
    "save_ai_settings",
  ])
    expect(handlers).not.toContain(name);
  expect(source.split("#[cfg(test)]\nmod tests")[0]).not.toMatch(
    /CredReadW|CredWriteW|CredEnumerateW/,
  );
  const csp = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"))
    .app.security.csp;
  expect(csp).toContain("default-src 'none'");
  expect(csp.split("connect-src")[1].split(";")[0]).not.toMatch(/https:|\*/);
  expect(fs.existsSync("src-tauri/src/style_advice.rs")).toBe(false);
});
