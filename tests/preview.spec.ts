import { test, expect } from "@playwright/test";
import fs from "node:fs";
const fixtures = JSON.parse(
  fs.readFileSync(".qa/engine-fixtures.json", "utf8"),
);
for (const fixture of fixtures) {
  test("shared CSS and readable scenes: " + fixture.name, async ({ page }) => {
    const external: string[] = [];
    page.on("request", (r) => {
      if (
        /^https?:/.test(r.url()) &&
        !r.url().startsWith("http://127.0.0.1:1432")
      )
        external.push(r.url());
    });
    await page.goto("/tests/preview.html?fixture=" + fixture.name);
    await page
      .getByText("辅助样板与可读性检查（非实机截图）", { exact: true })
      .click();
    const frame = page.frameLocator('iframe[title="WorkBuddy 仿真效果"]');
    await expect(
      frame.getByRole("heading", { name: "今天想做些什么？" }),
    ).toBeVisible();
    expect(
      await frame.locator("#codedrobe-theme-style-workbuddy").textContent(),
    ).toBe(fixture.view.document.compiled.css);
    await expect(frame.getByRole("button", { name: "发送示例" })).toHaveCSS(
      "color",
      "rgb(" +
        fixture.view.document.compiled.palette.onAccent
          .slice(1)
          .match(/../g)
          .map((v: string) => parseInt(v, 16))
          .join(", ") +
        ")",
    );
    await page.screenshot({
      path: ".qa/" + fixture.name + "-home.png",
      fullPage: true,
    });
    await page.getByRole("tab", { name: "对话", exact: true }).click();
    await expect(frame.getByText("检查消息区域的可读性")).toBeVisible();
    await expect(frame.locator("._mainArea_fixture")).toHaveCSS(
      "background-color",
      "rgba(0, 0, 0, 0)",
    );
    await expect(frame.locator(".cb-markdown").first()).toHaveCSS(
      "background-color",
      "rgba(0, 0, 0, 0)",
    );
    await expect(frame.locator(".cb-assistant-message").first()).toHaveCSS(
      "border-radius",
      "18px",
    );
    await page.screenshot({
      path: ".qa/" + fixture.name + "-chat.png",
      fullPage: true,
    });
    await frame.getByRole("textbox").fill("用于验证输入和点击的脱敏示例");
    await expect(frame.getByRole("textbox")).toHaveText(
      "用于验证输入和点击的脱敏示例",
    );
    await page.getByRole("tab", { name: "弹窗与菜单" }).click();
    await expect(frame.getByRole("dialog")).toBeVisible();
    await frame.getByRole("button", { name: "确认", exact: true }).hover();
    await page.screenshot({
      path: ".qa/" + fixture.name + "-dialog.png",
      fullPage: true,
    });
    expect(external).toEqual([]);
    expect(await page.locator("iframe").getAttribute("sandbox")).toBe(
      "allow-same-origin",
    );
  });
}
for (const width of [1200, 900]) {
  test("bounded layout at " + width, async ({ page }) => {
    await page.setViewportSize({ width, height: 660 });
    await page.goto("/tests/preview.html?fixture=landscape-airy-light");
    await expect(
      page.getByRole("button", { name: "开始真实预览" }),
    ).toBeVisible();
    await page
      .getByText("辅助样板与可读性检查（非实机截图）", { exact: true })
      .click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.getByRole("button", { name: "100% 缩放" }).click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      await page
        .locator(".preview-viewport")
        .evaluate((e) => e.scrollWidth > e.clientWidth),
    ).toBe(true);
    await page.getByRole("button", { name: "适应宽度" }).click();
    await page.screenshot({
      path: ".qa/layout-" + width + ".png",
      fullPage: true,
    });
  });
}
