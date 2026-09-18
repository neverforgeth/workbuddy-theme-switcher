/* global console, getComputedStyle, fetch */
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";
import { promisify } from "node:util";
import { execFile } from "node:child_process";
import http from "node:http";
import test, { before, after } from "node:test";
import { chromium } from "@playwright/test";

const root = fileURLToPath(new URL("../", import.meta.url));
const bundleRoot = process.env.STUDIO_BUNDLE_ROOT || root;
const vendor = process.env.STUDIO_BUNDLE_ROOT ? bundleRoot : path.join(root, "vendor");
const exec = promisify(execFile);
const catalog = JSON.parse(await fs.readFile(path.join(bundleRoot, "themes.json"), "utf8"));
let context, page, port, scratch;
// Minimal, public structural fixture derived from the signed 5.5.6 application.
// No production chat text, account state, login tokens or private screenshots.
const officialBaseCss = process.env.STUDIO_WORKBUDDY_CSS
  ? await fs.readFile(process.env.STUDIO_WORKBUDDY_CSS, "utf8") : "";
const html = `<!doctype html><title>WorkBuddy 5.5.6 structural fixture</title><style>${officialBaseCss}</style>
<style>html,body {margin:0} * {box-sizing:border-box} .teams-container {display:flex;height:820px}
.conversation-sidebar {width:240px;flex:none} .teams-main-content {flex:1;min-width:0;padding:24px}
.cr-input-container {background:white;border:1px solid #ddd;border-radius:16px;padding:12px}
.cr-input-editor-host {min-height:90px;background:white} [contenteditable] {min-height:64px;outline:none}
.cr-input-toolbar {display:flex;justify-content:end;height:40px;background:white}
.cr-input-toolbar__right {background:white;display:flex}
.cr-send-button {width:32px;height:32px;border:0;border-radius:50%}
.wb-home-header {height:100px} .wb-home-composer {width:100%}
</style><div id="root"><div class="teams-container"><aside class="conversation-sidebar">Fixture sidebar</aside>
<div class="teams-main-content"><main class="wb-home-page"><header class="wb-home-header">Fixture welcome</header>
<div class="wb-home-composer"><div class="wb-home-composer__input-slot">
<div class="cr-theme cr-input-box"><div class="cr-input-box__main"><div class="cr-input-container-wrapper"><div class="cr-input-container">
<div class="cr-input-editor-host"><div role="textbox" contenteditable="true" data-slate-editor="true"></div></div>
<div class="cr-input-toolbar"><div class="cr-input-toolbar__right"><button class="cr-send-button" aria-label="Send fixture"><span class="cr-send-button__icon"><svg width="32" height="32" viewBox="0 0 32 32"><path fill="currentColor" fill-rule="evenodd" d="M16 0A16 16 0 1 0 16 32A16 16 0 1 0 16 0ZM13 12H19V20H13Z" /></svg></span></button></div></div>
</div></div></div><div class="cr-input-footer">Fixture footer</div></div></div></div></main></div></div></div>`;

async function cli(args, timeout = 30000) {
  const start = Date.now();
  const result = await exec(path.join(vendor, "node/node.exe"), [path.join(vendor, "codedrobe/bin/codedrobe.mjs"),
    ...args, "--app", "workbuddy", "--port", String(port), "--json"], {
    cwd: bundleRoot, windowsHide: true, timeout, maxBuffer: 8 * 1024 * 1024,
    // An installed bundle must not accidentally use developer global modules.
    env: { ...process.env, NODE_PATH: "", NODE_OPTIONS: "" },
  });
  console.log(`${args[0]} ${args[2] ? path.basename(args[2]) : ""}: ${Date.now() - start}ms`);
  return JSON.parse(result.stdout);
}
async function apply(record) {
  return cli(["apply", "--theme", path.join(bundleRoot, record.packagePath), "--no-launch"]);
}
async function count() { return page.locator("#codedrobe-theme-style-workbuddy").count(); }

before(async () => {
  await fs.mkdir(path.join(root, ".qa"), { recursive: true });
  scratch = await fs.mkdtemp(path.join(root, ".qa/cdp-5.5.6-"));
  context = await chromium.launchPersistentContext(path.join(scratch, "profile"), {
    channel: "msedge", headless: true, viewport: { width: 1200, height: 820 },
    args: ["--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1"],
  });
  port = Number((await fs.readFile(path.join(scratch, "profile/DevToolsActivePort"), "utf8")).split("\n")[0]);
  assert.ok(port > 0);
  page = context.pages()[0];
  await page.setContent(html);
});
after(async () => { await context?.close(); });

test("bundled CLI: old selector reproduces failure, corrected themes apply/switch/restore with one node", { timeout: 60000 }, async () => {
  const old = JSON.parse(await fs.readFile(path.join(bundleRoot, catalog.at(-1).packagePath), "utf8"));
  old.targets.workbuddy.verification.contexts.find(c => c.name === "home").required.find(r => r.name === "home-composer-panel").any = [".wb-home-composer__input-slot > section"];
  const oldPath = path.join(scratch, "PRIVATE-USER old theme.codedrobe-theme");
  await fs.writeFile(oldPath, JSON.stringify(old));
  await assert.rejects(cli(["probe", "--theme", oldPath, "--timeout-ms", "300"]), error => {
    assert.match(error.stderr, /CODEDROBE_DOM_INCOMPATIBLE/);
    assert.match(error.stderr, /home-composer-panel/);
    assert.doesNotMatch(error.stderr, /PRIVATE-USER/);
    return true;
  });
  assert.equal(await count(), 0);
  const baseline = await page.locator(".cr-input-container").boundingBox();
  for (const record of catalog) {
    const result = await apply(record);
    assert.ok(result.targets.some(t => t.result?.pass));
    assert.equal(await count(), 1);
    const stats = await page.locator(".cr-input-container").evaluate(el => ({
      surface: getComputedStyle(el).backgroundColor,
      inner: getComputedStyle(el.querySelector(".cr-input-editor-host")).backgroundColor,
      button: getComputedStyle(el.querySelector(".cr-send-button")).backgroundColor,
      buttonColor: getComputedStyle(el.querySelector(".cr-send-button")).color,
      toolbar: getComputedStyle(el.querySelector(".cr-input-toolbar__right")).backgroundColor,
    }));
    assert.match(stats.surface, /^rgba?\(/);
    assert.notEqual(stats.surface, "rgb(255, 255, 255)");
    assert.equal(stats.inner, "rgba(0, 0, 0, 0)");
    assert.notEqual(stats.button, "rgba(0, 0, 0, 0)");
    assert.notEqual(stats.buttonColor, stats.button);
    assert.equal(stats.toolbar, "rgba(0, 0, 0, 0)");
    assert.deepEqual(await page.locator(".cr-input-container").boundingBox(), baseline);
    await page.getByRole("textbox").fill("Fixture input remains editable");
    await page.getByRole("button", { name: "Send fixture" }).click();
  }
  const verified = await cli(["verify", "--theme", path.join(bundleRoot, catalog.at(-1).packagePath)]);
  assert.ok(verified.targets.some(t => t.result?.pass));
  await page.screenshot({ path: path.join(scratch, "sky-breeze-structural-fixture.png") });
  await cli(["restore"]);
  assert.equal(await count(), 0);
  // A recreated renderer can use exactly the same installed package.
  await page.setContent(html);
  await apply(catalog.at(-1));
  assert.equal(await count(), 1);
  await cli(["restore"]);
  assert.equal(await count(), 0);
});

test("an unrelated blank WorkBuddy window cannot stall a healthy primary for 30 seconds", { timeout: 40000 }, async () => {
  const secondary = await context.newPage();
  await secondary.setContent("<title>WorkBuddy loading overlay</title><body>Loading fixture</body>");
  try {
    const start = Date.now();
    const result = await apply(catalog.at(-1));
    assert.ok(result.targets.some(t => t.result?.pass));
    assert.ok(result.targets.some(t => t.skipped));
    assert.ok(Date.now() - start < 12000, "secondary window must not consume the host's 30s deadline");
  } finally {
    await secondary.close();
    await cli(["restore"]);
  }
});

test("a vanished secondary CDP target is skipped only while a primary really passes", { timeout: 20000 }, async () => {
  const actualPort = port;
  const proxy = http.createServer(async (_req, res) => {
    const targets = await (await fetch(`http://127.0.0.1:${actualPort}/json/list`)).json();
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify([...targets, { id: "gone", type: "page", title: "WorkBuddy closed overlay", url: "about:blank", webSocketDebuggerUrl: `ws://127.0.0.1:${port}/closed` }]));
  });
  proxy.on("upgrade", (_req, socket) => socket.destroy());
  await new Promise(resolve => proxy.listen(0, "127.0.0.1", resolve));
  port = proxy.address().port;
  try {
    const result = await apply(catalog.at(-1));
    assert.ok(result.targets.some(t => t.result?.pass));
    assert.ok(result.targets.some(t => t.targetId === "gone" && t.skipped));
    const verified = await cli(["verify", "--theme", path.join(bundleRoot, catalog.at(-1).packagePath)]);
    assert.ok(verified.targets.some(t => t.result?.pass));
    assert.ok(verified.targets.some(t => t.targetId === "gone" && t.skipped));
  } finally {
    port = actualPort;
    await new Promise(resolve => proxy.close(resolve));
    await cli(["restore"]);
  }
  // Zero compatible windows must still fail; no theme node may be injected.
  await page.setContent(html.replaceAll("cr-input-container", "unknown-new-component"));
  try {
    await assert.rejects(cli(["probe", "--theme", path.join(bundleRoot, catalog.at(-1).packagePath), "--timeout-ms", "300"]), error => {
      assert.match(error.stderr, /CODEDROBE_DOM_INCOMPATIBLE/);
      return true;
    });
    assert.equal(await count(), 0);
  } finally { await page.setContent(html); }
});
