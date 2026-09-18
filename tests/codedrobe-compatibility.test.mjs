import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, URL } from "node:url";
import test from "node:test";
import { JSDOM } from "jsdom";
import adapter from "../vendor/codedrobe/src/adapters/workbuddy.mjs";
import { buildProbeExpression } from "../vendor/codedrobe/src/runtime/renderer-payload.mjs";
import { safeDiagnostic } from "../vendor/codedrobe/src/diagnostic.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const catalog = JSON.parse(fs.readFileSync(path.join(root, "themes.json"), "utf8"));
// Structure derived from the signed official 5.5.6 bundle, not from a private conversation.
// lib-chat-ui: InputBox -> InputContainerWrapper -> InputContainer -> InputEditor/Toolbar.
const newComposer = `<div class="cr-theme cr-input-box"><div class="cr-input-box__main"><div class="cr-input-container-wrapper"><div class="cr-input-container"><div class="cr-input-editor-host"><div role="textbox" contenteditable="true" data-slate-editor="true"></div></div><div class="cr-input-toolbar"><button class="cr-send-button" disabled></button></div></div></div></div><div class="cr-input-footer"></div></div>`;
const oldComposer = `<section><div class="_mainArea_legacy"><div role="textbox" contenteditable="true"></div></div></section>`;
function probe(verification, composer, scene = "home") {
  const content = scene === "home"
    ? `<main class="wb-home-page"><header class="wb-home-header"></header><div class="wb-home-composer"><div class="wb-home-composer__input-slot">${composer}</div></div></main>`
    : scene === "project"
      ? `<main class="project-detail-view"><div class="project-detail-view__input-area">${composer}</div></main>`
      : `<main class="chat-container">${composer}</main>`;
  const dom = new JSDOM(`<div id="root"><div class="teams-container">${content}</div></div>`, { runScripts: "outside-only" });
  dom.window.Element.prototype.getBoundingClientRect = () => ({ width: 300, height: 90 });
  try {
    return JSON.parse(JSON.stringify(dom.window.eval(buildProbeExpression(adapter, verification))));
  } finally { dom.window.close(); }
}

for (const record of catalog) {
  const bundle = JSON.parse(fs.readFileSync(path.join(root, record.packagePath), "utf8"));
  const verification = bundle.targets.workbuddy.verification;
  for (const [version, composer] of [["5.2.6", oldComposer], ["5.5.6", newComposer]]) {
    for (const scene of ["home", "conversation", "project"]) {
      test(`${record.id}: ${version} ${scene} retains required composer checks`, () => {
        const result = probe(verification, composer, scene);
        assert.equal(result.compatible, true, JSON.stringify(result.missing));
        assert.ok(result.requirements.some(r => r.severity === "required" && r.name.includes("composer")));
      });
    }
  }
  test(`${record.id}: missing composer is still rejected`, () => {
    const result = probe(verification, "<div>Loading</div>");
    assert.equal(result.compatible, false);
    assert.ok(result.missing.some(r => r.name === "home-composer-panel"));
  });
}

test("CLI reports a safe structured error instead of leaking paths or losing codes", () => {
  const result = spawnSync(path.join(root, "vendor/node/node.exe"), [
    path.join(root, "vendor/codedrobe/bin/codedrobe.mjs"), "probe", "--app", "workbuddy",
    "--theme", path.join(root, ".qa/PRIVATE-USER/PRIVATE-CHAT.codedrobe-theme"), "--json",
  ], { cwd: root, encoding: "utf8", timeout: 10000, windowsHide: true });
  assert.equal(result.status, 1);
  const line = result.stderr.split(/\r?\n/).find(s => s.startsWith("[codedrobe-diagnostic] "));
  assert.ok(line, "structured error marker is required");
  const diagnostic = JSON.parse(line.slice("[codedrobe-diagnostic] ".length));
  assert.equal(diagnostic.version, 1);
  assert.equal(diagnostic.code, "CODEDROBE_THEME_READ_FAILED");
  assert.ok(!result.stderr.includes("PRIVATE-"));
  assert.ok(!result.stderr.includes(root));
});

test("diagnostics never expose arbitrary codes, missing selectors or renderer messages", () => {
  const error = { code: "PRIVATE-CODE", message: "PRIVATE-TEXT", fields: { path: "PRIVATE-PATH" },
    missing: [{ name: "PRIVATE-USER", selectors: ["PRIVATE-SELECTOR"] }, { name: "home-composer-panel" }],
    results: [{ title: "PRIVATE-TITLE", url: "PRIVATE-URL", result: { missing: [{ name: "project-chat-composer" }] } }],
  };
  const value = safeDiagnostic(error);
  assert.equal(value.code, "CODEDROBE_COMMAND_FAILED");
  assert.deepEqual(value.checks, ["home-composer-panel", "project-chat-composer"]);
  assert.doesNotMatch(JSON.stringify(value), /PRIVATE/);
});
