import { test, expect } from "@playwright/test";
import fs from "node:fs";
import { previewDocument, type PreviewScene } from "../src/preview-fixture";
import type { DraftView } from "../src/types";
const fixtures: Array<{ name: string; view: DraftView }> = JSON.parse(
  fs.readFileSync(".qa/engine-fixtures.json", "utf8"),
);
for (const fixture of fixtures) {
  test(
    "fusion component ownership and full scenes: " + fixture.name,
    async ({ page }) => {
      const doc = fixture.view.document;
      expect(doc.style?.version).toBe(2);
    expect(doc.controls.blur).toBe(0);
    expect(doc.compiled.checks.filter(c => c.status === "warning")).toEqual([]);
    expect(doc.compiled.effectiveDesign!.regions.main.opacity).toBeLessThan(95);
      const requests: string[] = [];
      page.on("request", (r) => {
        if (/^https?:/.test(r.url())) requests.push(r.url());
      });
      await page.setViewportSize({ width: 1024, height: 760 });
      for (const scene of ["home", "chat", "dialog"] as PreviewScene[]) {
        await page.setContent(
          previewDocument(doc.compiled.css, fixture.view.imagePath, scene),
        );
        await expect(page.locator(".workbuddy-topbar")).toHaveCSS(
          "background-color",
          "rgba(0, 0, 0, 0)",
        );
        await expect(page.locator(".conversation-agent-card")).not.toHaveCSS(
          "background-color",
          "rgba(0, 0, 0, 0)",
        );
        if (scene !== "dialog") {
          await expect(page.locator("._mainArea_fixture")).toHaveCSS(
            "background-color",
            "rgba(0, 0, 0, 0)",
          );
          const actualSend = page.locator("span._icon_fixture._large_fixture");
          const rgb = (color: string) =>
            "rgb(" +
            color
              .slice(1)
              .match(/../g)!
              .map((v) => parseInt(v, 16))
              .join(", ") +
            ")";
          await expect(actualSend).toHaveCSS(
            "color",
            rgb(doc.compiled.palette.onAccent),
          );
          await expect(actualSend).toHaveCSS(
            "background-color",
            rgb(doc.compiled.palette.accent),
          );
          await page.getByRole("textbox").fill("脱敏输入 · 不读取聊天");
          await expect(page.getByRole("textbox")).toHaveCSS(
            "outline-style",
            "none",
          );
          await actualSend.hover();
          await actualSend.focus();
          await expect(actualSend).toHaveCSS("outline-style", "solid");
          await actualSend.evaluate((e) =>
            e.setAttribute("aria-disabled", "true"),
          );
          await expect(actualSend).toHaveCSS("opacity", "1");
        }
        if (scene === "chat") {
          await expect(page.locator(".cb-markdown").first()).toHaveCSS(
            "background-color",
            "rgba(0, 0, 0, 0)",
          );
          await expect(page.locator("pre code")).toHaveCSS(
            "background-color",
            "rgba(0, 0, 0, 0)",
          );
          await expect(page.locator(".cb-assistant-message").first()).toHaveCSS(
            "border-radius",
            "18px",
          );
          // Independently composite the browser's actual ancestor styles, not the compiler report.
          const ratios = await page.evaluate((samples) => {
            const parse = (s: string) => s.match(/[\d.]+/g)!.map(Number);
            const mix = (under: number[], over: number[], alpha: number) =>
              under.map((v, i) =>
                Math.round(v * (1 - alpha) + over[i] * alpha),
              );
            const luminance = (rgb: number[]) =>
              rgb
                .slice(0, 3)
                .map((v) => {
                  v /= 255;
                  return v <= 0.04045
                    ? v / 12.92
                    : ((v + 0.055) / 1.055) ** 2.4;
                })
                .reduce((a, v, i) => a + v * [0.2126, 0.7152, 0.0722][i], 0);
            const contrast = (a: number[], b: number[]) => {
              const x = luminance(a),
                y = luminance(b);
              return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
            };
            const vars = getComputedStyle(document.documentElement);
            const factor = Number(vars.getPropertyValue("--fusion-brightness"));
            const veil = parse(vars.getPropertyValue("--fusion-veil"));
            const wallpaper = samples.map((hex) =>
              mix(
                hex
                  .slice(1)
                  .match(/../g)!
                  .map((v) =>
                    Math.max(
                      0,
                      Math.min(255, Math.round(parseInt(v, 16) * factor)),
                    ),
                  ),
                veil,
                veil[3] ?? 1,
              ),
            );
            return [
              ...document.querySelectorAll(
                ".cb-markdown p,.cb-markdown a,.cb-markdown code,.cb-markdown th,.cb-markdown td,.cb-markdown small",
              ),
            ].map((node) => {
              const chain: Element[] = [];
              for (
                let e: Element | null = node;
                e && !e.classList.contains("teams-container");
                e = e.parentElement
              )
                chain.unshift(e);
              const fg = parse(getComputedStyle(node).color);
              const worst = Math.min(
                ...wallpaper.map((bg) => {
                  for (const e of chain) {
                    const paint = parse(getComputedStyle(e).backgroundColor);
                    bg = mix(bg, paint, paint[3] ?? 1);
                  }
                  return contrast(fg, bg);
                }),
              );
              return { tag: node.tagName, ratio: worst };
            });
          }, doc.analysis!.samples);
          for (const result of ratios)
            expect(result.ratio, result.tag).toBeGreaterThanOrEqual(4.5);
        }
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
        await page.screenshot({
          path: `.qa/fusion-${fixture.name}-${scene}.png`,
          fullPage: true,
        });
      }
      expect(requests).toEqual([]);
    },
  );
}
test("Ice Blue reference and fusion share message hierarchy on the same sanitized scene", async ({
  page,
}) => {
  const fixture = fixtures.find((f) => f.name === "landscape-airy-light")!;
  const reference = JSON.parse(
    fs.readFileSync("themes/ice-blue/theme.codedrobe-theme", "utf8"),
  ).targets.workbuddy.css;
  await page.setViewportSize({ width: 1024, height: 760 });
  for (const [name, css] of [
    ["reference", reference],
    ["v2", fixture.view.document.compiled.css],
  ]) {
    await page.setContent(previewDocument(css, fixture.view.imagePath, "chat"));
    await expect(page.locator(".cb-assistant-message").first()).toHaveCSS(
      "border-radius",
      "18px",
    );
    await expect(page.locator(".cb-markdown").first()).toHaveCSS(
      "background-color",
      "rgba(0, 0, 0, 0)",
    );
    await page.screenshot({
      path: `.qa/fusion-comparison-${name}.png`,
      fullPage: true,
    });
  }
});
