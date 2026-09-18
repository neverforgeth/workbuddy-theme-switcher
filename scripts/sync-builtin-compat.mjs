/* global console */
// Deterministic package rewrite, not a user-data migration. Existing saved custom
// revisions are never read or changed. --check is suitable for CI.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, URL } from "node:url";
import process from "node:process";

const root = fileURLToPath(new URL("../", import.meta.url));
const catalogPath = path.join(root, "themes.json");
const catalog = JSON.parse(fs.readFileSync(catalogPath, "utf8"));
const bridge = fs.readFileSync(path.join(root, "themes/compat/workbuddy-5.5.css"), "utf8").trim();
const marker = "/* WorkBuddy 5.5 composer bridge.";
const versions = { "pink-lijiajia": "1.1.5", "ice-blue": "1.0.5", "oriental-landscape": "1.0.3", "sky-breeze": "1.0.1" };
function update(filename, value) {
  const text = JSON.stringify(value, null, 2) + "\n";
  if (fs.readFileSync(filename, "utf8").replace(/\r\n/g, "\n") === text) return;
  if (process.argv.includes("--check")) throw new Error(`Outdated bundled compatibility: ${path.relative(root, filename)}`);
  fs.writeFileSync(filename, text);
}
for (const record of catalog) {
  const filename = path.join(root, record.packagePath);
  const bundle = JSON.parse(fs.readFileSync(filename, "utf8"));
  const target = bundle.targets.workbuddy;
  for (const context of target.verification.contexts) {
    for (const requirement of context.required ?? []) {
      const selector = {
        "home-composer-panel": ".wb-home-composer__input-slot .cr-input-container",
        "project-chat-composer": ".project-detail-view__input-area .cr-input-container",
      }[requirement.name];
      if (selector && !requirement.any.includes(selector)) requirement.any.push(selector);
    }
  }
  target.css = target.css.split(marker)[0].trimEnd() + "\n\n" + bridge + "\n";
  bundle.theme.version = versions[record.id];
  record.themeVersion = bundle.theme.version;
  record.verifiedWorkBuddyVersion = "按当前页面结构运行时检查";
  update(filename, bundle);
  const presetPath = path.join(root, "themes", record.id, "preset.json");
  const preset = JSON.parse(fs.readFileSync(presetPath, "utf8"));
  preset.version = bundle.theme.version;
  update(presetPath, preset);
}
update(catalogPath, catalog);
console.log("Built-in 5.5 composer bridge synchronized.");
