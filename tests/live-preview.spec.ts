import { test, expect } from "@playwright/test";
test("live CSS updates preserve input, scroll and the renderer document", async ({
  page,
}) => {
  await page.goto("/tests/preview.html?fixture=landscape-airy-light");
  await page
    .getByText("辅助样板与可读性检查（非实机截图）", { exact: true })
    .click();
  await page.getByRole("tab", { name: "对话", exact: true }).click();
  const frame = page.frameLocator("iframe");
  await frame.getByRole("textbox").fill("预览中的未提交内容");
  const documentHandle = await frame.locator("body").elementHandle();
  const source = await page.locator("iframe").getAttribute("srcdoc");
  await page.getByRole("slider", { name: "亮度", exact: true }).fill("12");
  await expect
    .poll(() => frame.locator("#codedrobe-theme-style-workbuddy").textContent())
    .toContain("ui sequence 2");
  await expect(frame.getByRole("textbox")).toHaveText("预览中的未提交内容");
  expect(await page.locator("iframe").getAttribute("srcdoc")).toBe(source);
  expect(await documentHandle!.evaluate((e) => e.isConnected)).toBe(true);
  // Sandbox and CSP still prevent scripts even though the trusted parent can update the style.
  await frame.locator("body").evaluate((e) => {
    const script = e.ownerDocument.createElement("script");
    script.textContent = "window.__forbiddenScriptExecuted = true";
    e.append(script);
  });
  expect(
    await frame
      .locator("body")
      .evaluate((e) =>
        Reflect.get(e.ownerDocument.defaultView!, "__forbiddenScriptExecuted"),
      ),
  ).toBeUndefined();
});
