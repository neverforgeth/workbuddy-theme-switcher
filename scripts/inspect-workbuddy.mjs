// Read-only selector contract inspection. Does not connect to or inspect user conversations.
import fs from "node:fs";
import process from "node:process";
import { Buffer } from "node:buffer";
import console from "node:console";
const path = process.argv[2];
if (!path) throw new Error("Pass the local WorkBuddy app.asar path");
const fd = fs.openSync(path, "r");
const prefix = Buffer.alloc(16);
fs.readSync(fd, prefix, 0, 16, 0);
const header = Buffer.alloc(prefix.readUInt32LE(12));
fs.readSync(fd, header, 0, header.length, 16);
const tree = JSON.parse(header.toString());
const entries = [];
function walk(files, parent = "") {
  for (const [name, entry] of Object.entries(files)) {
    const full = parent + name;
    if (entry.files) walk(entry.files, full + "/");
    else if (/^renderer\/.*\.(css|js)$/.test(full) && !entry.unpacked)
      entries.push([full, entry]);
  }
}
walk(tree.files);
const needles = [
  "teams-container",
  "conversation-sidebar",
  "teams-content-wrapper",
  "wb-home-page",
  "wb-home-composer__input-slot",
  "cb-assistant-message",
  "_userMessageBubble_",
  "chat-input-send-button",
  "wb-home-composer__send-button",
];
const result = Object.fromEntries(needles.map((n) => [n, []]));
const controls = new Set();
for (const [name, entry] of entries) {
  const buffer = Buffer.alloc(entry.size);
  fs.readSync(
    fd,
    buffer,
    0,
    buffer.length,
    8 + prefix.readUInt32LE(4) + Number(entry.offset),
  );
  const content = buffer.toString();
  if (name.endsWith(".css"))
    for (const match of content.matchAll(
      /\.([a-zA-Z_][a-zA-Z0-9_-]*(?:[Ss]end|[Pp]rimary|[Cc]omposer)[a-zA-Z0-9_-]*)/g,
    ))
      controls.add(match[1]);
  for (const needle of needles)
    if (content.includes(needle)) result[needle].push(name);
}
fs.closeSync(fd);
console.log(
  JSON.stringify(
    { path, selectorPresence: result, controlSelectors: [...controls] },
    null,
    2,
  ),
);
