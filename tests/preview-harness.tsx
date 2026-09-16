// Browser-only acceptance harness, not included in the production entry.
import { createRoot } from "react-dom/client";
import App from "../src/App";
import "../src/styles.css";
import { studioApi, type StudioApi } from "../src/studio-api";
import type { DraftView, RuntimeSnapshot } from "../src/types";
import fixtures from "../.qa/engine-fixtures.json";
const fixture =
  fixtures.find(
    (f) => f.name === new URLSearchParams(location.search).get("fixture"),
  ) || fixtures[0];
let view = fixture.view as DraftView;
const runtime: RuntimeSnapshot = {
  workbuddy: {
    appFound: true,
    path: "D:/workbuddy/WorkBuddy.exe",
    version: "5.2.6.0",
    running: true,
    cdpAvailable: true,
    rendererAvailable: true,
    currentThemeId: null,
    styleNodeCount: 0,
    logsDirectory: "",
    message: "测试会话",
  },
  runtime: {
    desiredState: "original",
    selectedThemeId: null,
    autoKeepTheme: false,
    monitorStatus: "暂停",
    retryCount: 0,
    lastAutoRecoveryAt: null,
    lastErrorCode: null,
    statePath: "",
    loginAutostartEnabled: false,
  },
  trial: null,
  operation: null,
  capturedAt: "fixture",
  error: null,
};
if (new URLSearchParams(location.search).get("real") === "1")
  runtime.trial = {
    id: "layout-only",
    name: "浏览器布局测试",
    phase: "active",
    deadlineMs: Date.now() + 600000,
    error: null,
    syncedSequence: view.document.editSequence,
    cssHash: "fixture",
  };
const api: StudioApi = {
  ...studioApi,
  library: async () => [],
  latestDraft: async () => view,
  runtime: async () => runtime,
  subscribe: async () => () => undefined,
  fileUrl: (path) => path,
  syncTrial: async (_id, sequence) => ({
    ...runtime.trial!,
    syncedSequence: sequence,
  }),
  captureTrial: async () => ({
    id: "layout-frame",
    trialId: "layout-only",
    draftId: view.document.draftId,
    sequence: view.document.editSequence,
    cssHash: "fixture",
    targetId: "layout-only",
    width: 1440,
    height: 900,
    scene: "home",
    capturedAt: "布局测试图，不是实机验收",
    workbuddyVersion: "5.2.6.0",
    imageDataUrl: view.imagePath,
    captureMs: 1,
  }),
  updateDraft: async (_id, update) => {
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
            "\n/* ui sequence " +
            update.sequence +
            " */",
        },
      },
    };
    return view;
  },
};
createRoot(document.getElementById("root")!).render(<App api={api} />);
