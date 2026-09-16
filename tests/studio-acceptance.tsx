// Test-only route: every API call stays in memory. No Tauri/WorkBuddy fallback.
import { createRoot } from "react-dom/client";
import App from "../src/App";
import "../src/styles.css";
import type { StudioApi } from "../src/studio-api";
import type { DraftView, RuntimeSnapshot } from "../src/types";
import fixtures from "../.qa/engine-fixtures.json";

const fixture =
  fixtures.find((entry) => entry.name === "landscape-airy-light") ||
  fixtures[0];
let view = structuredClone(fixture.view) as DraftView;
const calls: Record<string, number> = {};
const runtime: RuntimeSnapshot = {
  workbuddy: {
    appFound: true,
    path: "MOCK ONLY",
    version: "synthetic",
    running: true,
    cdpAvailable: true,
    rendererAvailable: true,
    currentThemeId: null,
    styleNodeCount: 0,
    logsDirectory: "",
    message: "纯 mock 浏览器验收",
  },
  runtime: {
    desiredState: "original",
    selectedThemeId: null,
    autoKeepTheme: false,
    monitorStatus: "Mock only",
    retryCount: 0,
    lastAutoRecoveryAt: null,
    lastErrorCode: null,
    statePath: "",
    loginAutostartEnabled: false,
  },
  trial: null,
  operation: null,
  capturedAt: "synthetic",
  error: null,
};
const syntheticImage =
  "data:image/svg+xml;charset=utf-8," +
  encodeURIComponent(`
<svg xmlns="http://www.w3.org/2000/svg" width="1440" height="900">
<rect width="1440" height="900" fill="#eef3f1"/>
<rect x="0" width="270" height="900" fill="#dce8e2"/>
<rect x="315" y="170" width="1060" height="480" rx="24" fill="#fff"/>
<rect x="355" y="700" width="980" height="120" rx="18" fill="#d6e5dd"/>
<text x="330" y="92" font-family="sans-serif" font-size="34" fill="#204f3f">合成测试帧 · 非 WorkBuddy 实机截图</text>
<text x="355" y="250" font-family="sans-serif" font-size="26" fill="#456256">仅验证整页布局、控件连续操作与预览挂载状态。</text>
<text x="30" y="94" font-family="sans-serif" font-size="26" fill="#204f3f">MOCK API</text>
</svg>`);
const handlers: Partial<StudioApi> = {
  library: async () => [],
  latestDraft: async () => structuredClone(view),
  runtime: async () => structuredClone(runtime),
  subscribe: async () => () => undefined,
  fileUrl: (path) => path,
  updateDraft: async (_id, update) => {
    if (update.sequence <= view.document.editSequence)
      throw new Error("Non-monotonic mock write");
    view = {
      ...view,
      document: {
        ...view.document,
        name: update.name,
        controls: update.controls,
        design: update.design ?? view.document.design,
        style: update.style ?? view.document.style,
        editSequence: update.sequence,
        compiled: {
          ...view.document.compiled,
          css:
            fixture.view.document.compiled.css +
            `\n/* mock sequence ${update.sequence} */`,
        },
      },
    };
    return structuredClone(view);
  },
  startTrial: async () => {
    runtime.trial = {
      id: "synthetic-trial",
      name: "合成测试 / 非实机",
      phase: "active",
      deadlineMs: Date.now() + 600000,
      error: null,
      syncedSequence: view.document.editSequence,
      cssHash: "mock-" + view.document.editSequence,
    };
    return structuredClone(runtime.trial);
  },
  syncTrial: async (id, sequence) => {
    if (
      !runtime.trial ||
      runtime.trial.id !== id ||
      sequence !== view.document.editSequence
    )
      throw new Error("Unexpected mock sync");
    runtime.trial = {
      ...runtime.trial,
      syncedSequence: sequence,
      cssHash: "mock-" + sequence,
    };
    return structuredClone(runtime.trial);
  },
  captureTrial: async (id) => ({
    id: "mock-frame-" + view.document.editSequence,
    trialId: id,
    draftId: view.document.draftId,
    sequence: view.document.editSequence,
    cssHash: "mock-" + view.document.editSequence,
    targetId: "synthetic-renderer",
    width: 1440,
    height: 900,
    scene: "synthetic / 非实机验收",
    capturedAt: "纯浏览器测试",
    workbuddyVersion: "MOCK",
    imageDataUrl: syntheticImage,
    captureMs: 1,
  }),
  cancelTrial: async () => {
    runtime.trial = null;
  },
  discardAi: async () => undefined,
};
const api = new Proxy(handlers, {
  get(target, property) {
    return (...args: unknown[]) => {
      const name = String(property);
      calls[name] = (calls[name] || 0) + 1;
      const handler = Reflect.get(target, property) as
        ((...values: unknown[]) => unknown) | undefined;
      if (!handler)
        throw new Error("Unexpected API call in synthetic acceptance: " + name);
      return handler(...args);
    };
  },
}) as StudioApi;
declare global {
  interface Window {
    __studioAcceptance: {
      calls: Record<string, number>;
      current: () => DraftView;
    };
  }
}
window.__studioAcceptance = { calls, current: () => structuredClone(view) };
createRoot(document.getElementById("root")!).render(<App api={api} />);
