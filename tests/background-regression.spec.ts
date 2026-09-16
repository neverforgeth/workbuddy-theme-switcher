import { expect, test } from "@playwright/test";
import fs from "node:fs";
// @ts-expect-error vendored runtime is native ESM JavaScript.
import { buildApplyExpression } from "../vendor/codedrobe/src/runtime/renderer-payload.mjs";

test("uploaded image remains visible behind the actual WorkBuddy grid wrapper", async ({
  page,
}) => {
  const fixtures = JSON.parse(
    fs.readFileSync(".qa/engine-fixtures.json", "utf8"),
  );
  const fixture = fixtures.find(
    (f: { name: string }) => f.name === "landscape-airy-light",
  ).view;
  await page.setViewportSize({ width: 900, height: 660 });
  // Sanitized 5.2.6 hierarchy, including the opaque wrapper missed by the old sample.
  await page.setContent(`<style>html,body{margin:0}.teams-container{height:660px}
    ._grid_48kdk_4,._gridView_48kdk_9{position:relative;height:100%}
    ._gridViewItem_48kdk_14{position:absolute;inset:0 0 0 200px;background:#fafafa}
    .main-content{position:relative;height:100%}.wb-home-page{height:500px}
    input{margin-top:200px}</style><div id="root"><div class="teams-container">
    <div class="_grid_48kdk_4"><div class="_gridView_48kdk_9">
    <div class="_gridViewItem_48kdk_14"><div class="teams-content-wrapper">
    <div class="teams-main-content"><main class="main-content main-content--welcome">
    <div class="chat-container chat-container--welcome"><div class="wb-home-page">
    <h1>脱敏测试首页</h1><input aria-label="test-input" value="unsent fixture"></div></div>
    </main></div></div></div></div></div></div></div>`);
  const before = await page.getByRole("textbox").boundingBox();
  await page.evaluate(
    buildApplyExpression({
      adapter: { id: "workbuddy" },
      targetTheme: {
        theme: { id: "background-regression", version: "1.6.0" },
        css: fixture.document.compiled.css,
        imageDataUrls: { hero: fixture.imagePath },
      },
    }),
  );
  await expect(page.locator("._gridViewItem_48kdk_14")).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  // The menu bar's ID must not give the base home-panel rule higher specificity.
  const opacity =
    fixture.document.compiled.effectiveDesign.regions.main.opacity / 100;
  const alpha = await page.locator(".teams-content-wrapper").evaluate((e) => {
    const color = getComputedStyle(e).backgroundColor;
    return color.startsWith("rgba")
      ? Number(color.split(",")[3].replace(")", ""))
      : 1;
  });
  expect(alpha).toBeCloseTo(opacity, 2);
  await expect(page.getByRole("textbox")).toHaveValue("unsent fixture");
  expect(await page.getByRole("textbox").boundingBox()).toEqual(before);
  expect(await page.locator("#codedrobe-theme-style-workbuddy").count()).toBe(
    1,
  );
  // Check rendered pixels, not just a successful style injection or image property.
  const pixel = async () => {
    const png = await page.screenshot({
      clip: { x: 700, y: 100, width: 1, height: 1 },
    });
    return page.evaluate(
      async (bytes) => {
        const bitmap = await createImageBitmap(
          new Blob([new Uint8Array(bytes)], { type: "image/png" }),
        );
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 1;
        const context = canvas.getContext("2d")!;
        context.drawImage(bitmap, 0, 0);
        const value = [...context.getImageData(0, 0, 1, 1).data];
        bitmap.close();
        return value;
      },
      [...png],
    );
  };
  const withImage = await pixel();
  await page.evaluate(() =>
    document.documentElement.style.setProperty(
      "--codedrobe-image-hero",
      "none",
    ),
  );
  expect(await pixel()).not.toEqual(withImage);
});
