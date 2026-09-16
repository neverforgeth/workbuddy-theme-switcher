/* global fetch, AbortSignal, WebSocket, URL, setTimeout, clearTimeout, console, process */
// Local, read-only diagnostic: emit stage timings and numeric geometry only, never user text or pixels.
import { readFileSync } from "node:fs";
const list = await (
  await fetch("http://127.0.0.1:19336/json/list", {
    signal: AbortSignal.timeout(2000),
  })
).json();
const target = list.find(
  (t) =>
    t.type === "page" &&
    t.url.includes("app.asar") &&
    t.url.includes("renderer"),
);
if (!target) throw new Error("No WorkBuddy renderer");
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = reject;
});
let seq = 0;
async function call(method, params) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.removeEventListener("message", handler);
      reject(new Error("timeout"));
    }, 3500);
    const handler = (e) => {
      const v = JSON.parse(e.data);
      if (v.id !== id) return;
      clearTimeout(timer);
      socket.removeEventListener("message", handler);
      if (v.error) reject(new Error("protocol " + v.error.code));
      else resolve(v.result);
    };
    socket.addEventListener("message", handler);
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function stage(name, fn) {
  const start = Date.now();
  try {
    const result = await fn();
    console.log(JSON.stringify({ name, ms: Date.now() - start, result }));
  } catch (e) {
    console.log(
      JSON.stringify({ name, ms: Date.now() - start, error: e.message }),
    );
  }
}
await stage("visibility", async () => {
  const r = await call("Runtime.evaluate", {
    expression:
      "({hidden:document.hidden,width:innerWidth,height:innerHeight})",
    returnByValue: true,
  });
  return r.result.value;
});
await stage("two-raf", async () => {
  const r = await call("Runtime.evaluate", {
    expression:
      "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))",
    returnByValue: true,
    awaitPromise: true,
    timeout: 2500,
  });
  return { value: r.result?.value, exception: !!r.exceptionDetails };
});
await stage("geometry", async () => {
  const r = await call("Runtime.evaluate", {
    expression: readFileSync(
      new URL("../src-tauri/src/live-dom-probe.js", import.meta.url),
      "utf8",
    ),
    returnByValue: true,
    timeout: 2500,
  });
  const g = r.result?.value;
  return {
    exception: !!r.exceptionDetails,
    width: g?.width,
    height: g?.height,
    scene: g?.scene,
    regions: g?.regions?.length,
    masks: g?.masks?.length,
    safe: g?.safe,
  };
});
await stage("png", async () => {
  const r = await call("Page.captureScreenshot", {
    format: "png",
    fromSurface: true,
    captureBeyondViewport: false,
  });
  return { encodedBytes: r.data?.length };
});
await stage("png-clip", async () => {
  const v = await call("Runtime.evaluate", {
    expression: "({width:innerWidth,height:innerHeight})",
    returnByValue: true,
  });
  const r = await call("Page.captureScreenshot", {
    format: "png",
    fromSurface: true,
    captureBeyondViewport: false,
    clip: { x: 0, y: 0, ...v.result.value, scale: 1 },
  });
  return { encodedBytes: r.data?.length };
});
await stage("png-widget", async () => {
  const r = await call("Page.captureScreenshot", {
    format: "png",
    fromSurface: false,
    captureBeyondViewport: false,
  });
  return { encodedBytes: r.data?.length };
});
socket.close();
process.exitCode = 0;
