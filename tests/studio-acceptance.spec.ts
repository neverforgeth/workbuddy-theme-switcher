import { test, expect } from "@playwright/test";

for (const viewport of [
  { width: 900, height: 660 },
  { width: 1200, height: 820 },
]) {
  test(`whole App synthetic acceptance ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    const external: string[] = [];
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (
        /^https?:$/.test(url.protocol) &&
        url.origin !== "http://127.0.0.1:1432"
      ) {
        external.push(url.href);
        await route.abort();
      } else await route.continue();
    });
    await page.goto("/tests/studio-acceptance.html");
    const preview = page.getByRole("region", { name: "WorkBuddy 真实效果" });
    await expect(preview).toBeVisible();
    const originalPreview = await preview.elementHandle();
    const noHorizontalOverflow = () =>
      page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);
    expect(await noHorizontalOverflow()).toBe(true);
    await expect(page.locator(".editor-actionbar")).toBeInViewport();
    const start = page.getByRole("button", {
      name: "开始真实预览",
      exact: true,
    });
    await expect(start).toBeEnabled();
    await start.click();
    await page.getByRole("button", { name: "开始 10 分钟预览" }).click();
    await expect(
      preview.getByText("当前实机效果", { exact: true }),
    ).toBeVisible();
    for (const [name, value] of [
      ["亮度", "12"],
      ["背景模糊", "15"],
      ["面板不透明度", "88"],
    ]) {
      const control = page.getByRole("slider", { name, exact: true });
      await expect(control).toBeEnabled();
      await control.fill(value);
      expect(
        await originalPreview!.evaluate((element) => element.isConnected),
      ).toBe(true);
    }
    await page
      .getByRole("textbox", { name: "强调色", exact: true })
      .fill("#537b69");
    await expect
      .poll(() =>
        page.evaluate(
          () => window.__studioAcceptance.current().document.controls,
        ),
      )
      .toEqual({
        brightness: 12,
        blur: 15,
        panelOpacity: 88,
        accent: "#537b69",
      });
    expect(
      await originalPreview!.evaluate((element) => element.isConnected),
    ).toBe(true);
    await expect(
      preview.getByText("当前实机效果", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "刷新当前画面" })
      .click({ trial: true });
    await page
      .getByRole("button", { name: "保存主题", exact: true })
      .click({ trial: true });
    await expect(page.locator(".editor-actionbar")).toBeInViewport();
    expect(await noHorizontalOverflow()).toBe(true);
    const calls = await page.evaluate(() => window.__studioAcceptance.calls);
    expect(calls.startTrial).toBe(1);
    expect(calls.updateDraft).toBeGreaterThan(0);
    for (const forbidden of [
      "prepareAi",
      "maskAi",
      "sendAi",
      "saveDraft",
      "confirmTrial",
      "apply",
      "openTheme",
    ])
      expect(calls[forbidden] || 0, forbidden).toBe(0);
    expect(external).toEqual([]);
    expect(errors).toEqual([]);
    await page.locator(".editor-controls").evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({
      path: `.qa/studio-app-${viewport.width}x${viewport.height}-synthetic.png`,
    });
  });
}
