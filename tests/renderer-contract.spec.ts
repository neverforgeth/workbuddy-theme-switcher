import { test, expect } from "@playwright/test";
import fs from "node:fs";
// @ts-expect-error vendored runtime is native ESM JavaScript.
import {
  buildApplyExpression,
  buildRemoveExpression,
} from "../vendor/codedrobe/src/runtime/renderer-payload.mjs";

for (const width of [900, 1200])
  test(`real preview container layout at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 820 });
    await page.goto("/tests/preview.html?fixture=landscape-airy-light&real=1");
    await expect(page.getByAltText("WorkBuddy 当前真实换肤截图")).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.getByRole("button", { name: "100% 查看" }).click();
    expect(
      await page
        .locator(".real-preview-images")
        .evaluate((e) => e.scrollWidth > e.clientWidth),
    ).toBe(true);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `.qa/real-container-${width}.png`,
      fullPage: true,
    });
  });

test("paused renderer animation frames never block preview readiness", async ({
  page,
}) => {
  await page.goto("/tests/preview.html?fixture=light-airy-light");
  await page.evaluate(() => {
    window.requestAnimationFrame = () => 1;
  });
  const started = Date.now();
  expect(
    await page.evaluate(
      fs.readFileSync("src-tauri/src/live-paint-ready.js", "utf8"),
    ),
  ).toBe(false);
  expect(Date.now() - started).toBeLessThan(1000);
});

test("live CSS updates preserve the one node, image URL, input, scroll and recovery", async ({
  page,
}) => {
  await page.goto("/tests/preview.html?fixture=light-airy-light");
  await page.setContent(
    '<div class="teams-container"><aside class="conversation-sidebar">private task</aside><main class="wb-home-page"><input value="unsent private text"><div style="height:1800px">content</div></main></div>',
  );
  const expression = buildApplyExpression({
    adapter: { id: "workbuddy" },
    targetTheme: {
      theme: { id: "trial-test", version: "1.5.0" },
      css: "body{color:rgb(1,2,3)}",
      imageDataUrls: { hero: "data:image/png;base64,iVBORw0KGgo=" },
    },
  });
  await page.evaluate(expression);
  await page.locator("input").focus();
  await page.evaluate(() => scrollTo(0, 100));
  const before = await page.evaluate(() => ({
    image: document.documentElement.style.getPropertyValue(
      "--codedrobe-image-hero",
    ),
    y: scrollY,
  }));
  await page.evaluate(
    "window.__CODEDROBE__.hosts.workbuddy.updateCss('trial-test','body{color:rgb(3,4,5)}')",
  );
  await expect(page.locator("body")).toHaveCSS("color", "rgb(3, 4, 5)");
  await expect(page.locator("input")).toHaveValue("unsent private text");
  expect(await page.locator("#codedrobe-theme-style-workbuddy").count()).toBe(
    1,
  );
  expect(
    await page.evaluate(() => ({
      image: document.documentElement.style.getPropertyValue(
        "--codedrobe-image-hero",
      ),
      y: scrollY,
    })),
  ).toEqual(before);
  // CodeDrobe can restore a removed node using the newest CSS, not the initial trial version.
  await page
    .locator("#codedrobe-theme-style-workbuddy")
    .evaluate((e) => e.remove());
  await expect(page.locator("body")).toHaveCSS("color", "rgb(3, 4, 5)");
  await page.evaluate(buildRemoveExpression({ id: "workbuddy" }));
  expect(await page.locator("#codedrobe-theme-style-workbuddy").count()).toBe(
    0,
  );
});
