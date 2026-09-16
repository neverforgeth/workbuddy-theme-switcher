import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { previewDocument } from "./preview-fixture";
import type {
  DesignAdvice,
  DraftUpdate,
  DraftView,
  RuntimeSnapshot,
} from "./types";
import { studioApi, type StudioApi } from "./studio-api";

afterEach(cleanup);
it("creates an explicit independent fusion copy without applying or saving a theme", async () => {
  const { api } = harness();
  const copied = draft();
  copied.document.draftId = "c".repeat(32);
  copied.document.style = { id: "airy-light", version: 2 };
  copied.document.controls.blur = 0;
  api.copyFusion = vi.fn(async () => copied);
  render(<App api={api} />);
  await screen.findByRole("button", { name: "创建新版融合副本" });
  expect(api.copyFusion).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "创建新版融合副本" }));
  expect(api.copyFusion).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "创建副本" }));
  await waitFor(() =>
    expect(api.copyFusion).toHaveBeenCalledWith(
      "a".repeat(32),
      expect.any(String),
    ),
  );
  await waitFor(() =>
    expect(
      screen.queryByRole("button", { name: "创建新版融合副本" }),
    ).toBeNull(),
  );
  expect(screen.getByRole("slider", { name: "背景模糊" })).toHaveProperty(
    "value",
    "0",
  );
  expect(api.saveDraft).not.toHaveBeenCalled();
  expect(api.startTrial).not.toHaveBeenCalled();
});
const image = "data:image/png;base64,iVBORw0KGgo=";
function draft(): DraftView {
  return {
    imagePath: image,
    warning: null,
    document: {
      schemaVersion: 1,
      draftId: "a".repeat(32),
      themeId: null,
      revision: null,
      name: "山间清风",
      assetId: "b".repeat(32),
      sourceFilename: "mountain.png",
      width: 100,
      height: 100,
      background: "#AACCBB",
      extractedAccent: "#336655",
      controls: { brightness: 0, blur: 8, panelOpacity: 82, accent: null },
      advice: {},
      compiled: {
        css: ".cb-assistant-message{color:#123456}",
        palette: {
          background: "#AACCBB",
          surface: "#F1F5ED",
          surfaceStrong: "#FAFCF7",
          sidebar: "#DDE5D5",
          accent: "#336655",
          accentStrong: "#234D32",
          onAccent: "#FFFFFF",
          border: "#667755",
          text: "#123456",
          textMuted: "#456345",
          dark: false,
        },
        checks: [{ name: "正文", ratio: 7, required: 4.5, status: "pass" }],
        templateVersion: "workbuddy-5.2.6-v1",
        effectiveOpacity: 82,
        compileMs: 1,
      },
      editSequence: 0,
      savedSequence: null,
    },
  };
}
function snapshot(): RuntimeSnapshot {
  return {
    workbuddy: {
      appFound: true,
      path: "C:/WorkBuddy.exe",
      version: "5.2.6.0",
      running: true,
      cdpAvailable: true,
      rendererAvailable: true,
      currentThemeId: null,
      styleNodeCount: 0,
      logsDirectory: "",
      message: "已连接",
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
    capturedAt: "now",
    error: null,
  };
}
function harness() {
  let push: (s: RuntimeSnapshot) => void = () => undefined;
  const api: StudioApi = {
    ...studioApi,
    library: vi.fn(async () => []),
    latestDraft: vi.fn(async () => draft()),
    runtime: vi.fn(async () => snapshot()),
    subscribe: vi.fn(async (callback) => {
      push = callback;
      return () => undefined;
    }),
    fileUrl: (path) => path,
    updateDraft: vi.fn(async (_id: string, update: DraftUpdate) => {
      const view = draft();
      view.document = {
        ...view.document,
        name: update.name,
        controls: update.controls,
        design: update.design ?? view.document.design,
        style: update.style ?? view.document.style,
        editSequence: update.sequence,
        compiled: {
          ...view.document.compiled,
          css: "/* brightness " + update.controls.brightness + " */",
        },
      };
      return view;
    }),
    saveDraft: vi.fn(),
    importImage: vi.fn(),
    syncTrial: vi.fn(async (id, sequence) => ({
      id,
      name: "山间清风",
      phase: "active" as const,
      deadlineMs: Date.now() + 600000,
      error: null,
      syncedSequence: sequence,
      cssHash: "hash",
    })),
    captureTrial: vi.fn(async (id) => ({
      id: "frame",
      trialId: id,
      draftId: "a".repeat(32),
      sequence: 0,
      cssHash: "hash",
      targetId: "renderer",
      width: 1024,
      height: 690,
      scene: "home",
      capturedAt: "now",
      workbuddyVersion: "5.2.6.0",
      imageDataUrl: image,
      captureMs: 10,
    })),
    cancelTrial: vi.fn(async () => undefined),
    startTrial: vi.fn(async () => ({
      id: "trial",
      name: "山间清风",
      phase: "active" as const,
      deadlineMs: Date.now() + 600000,
      error: null,
      syncedSequence: 0,
    })),
  };
  return { api, push: (value: RuntimeSnapshot) => push(value) };
}
function regionalHarness() {
  const { api, push } = harness();
  const style = {
    background: "#aabbcc",
    text: "#112233",
    muted: "#223344",
    border: "#8899aa",
    hover: "#99aabb",
    selected: "#ccddee",
    focusRing: "#336699",
    opacity: 80,
    shadow: "none" as const,
  };
  let stored = draft();
  stored.document.design = {
    regions: Object.fromEntries(
      [
        "sidebar",
        "main",
        "composer",
        "userMessage",
        "assistantMessage",
        "primaryButton",
        "secondaryButton",
        "menu",
        "dialog",
      ].map((key) => [key, { ...style }]),
    ),
    backgroundX: 50,
    backgroundY: 50,
    veil: 8,
    explanation: "",
  } as DesignAdvice;
  const copy = () => structuredClone(stored);
  api.latestDraft = vi.fn(async () => copy());
  api.updateDraft = vi.fn(async (_id, update) => {
    stored = {
      ...stored,
      document: {
        ...stored.document,
        name: update.name,
        controls: update.controls,
        design: update.design ?? stored.document.design,
        style: update.style ?? stored.document.style,
        editSequence: update.sequence,
      },
    };
    return copy();
  });
  return { api, push, stored: copy };
}
describe("studio workflows", () => {
  it("confirms legacy profile adoption and preserves input during a slow style reset", async () => {
    const { api, stored } = regionalHarness();
    const update = api.updateDraft;
    let release: () => void = () => undefined;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    api.updateDraft = vi.fn(update).mockImplementationOnce(async (...args) => {
      await gate;
      return update(...args);
    });
    render(<App api={api} />);
    await screen.findByLabelText("离线风格");
    fireEvent.change(screen.getByLabelText("离线风格"), {
      target: { value: "warm-paper" },
    });
    expect(api.updateDraft).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "采用风格默认值" }));
    await waitFor(() => expect(api.updateDraft).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.updateDraft).mock.calls[0][1]).toMatchObject({
      style: { id: "warm-paper", version: 1 },
      resetStyle: true,
      controls: { brightness: -3, blur: 6 },
    });
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 19 } });
    fireEvent.change(screen.getByLabelText("输入框区域底色"), {
      target: { value: "#334466" },
    });
    await act(async () => release());
    await waitFor(() => expect(stored().document.controls.brightness).toBe(19));
    expect(stored().document.design?.regions.composer.background).toBe(
      "#334466",
    );
    expect(vi.mocked(api.updateDraft).mock.calls[1][1].resetStyle).toBe(false);
    expect(api.importImage).not.toHaveBeenCalled();
    expect(api.saveDraft).not.toHaveBeenCalled();
  });
  it("commits simultaneous global and regional changes as one editing intent", async () => {
    const { api } = regionalHarness();
    render(<App api={api} />);
    await screen.findByLabelText("输入框区域底色");
    fireEvent.change(screen.getByLabelText("输入框区域底色"), {
      target: { value: "#667788" },
    });
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 14 } });
    await waitFor(() => expect(api.updateDraft).toHaveBeenCalledTimes(1));
    const update = vi.mocked(api.updateDraft).mock
      .calls[0][1] as DraftUpdate & { design: DesignAdvice };
    expect(update.design?.regions.composer.background).toBe("#667788");
    expect(update.controls.brightness).toBe(14);
  });
  it("preserves a regional change followed immediately by a global change", async () => {
    const { api, stored } = regionalHarness();
    render(<App api={api} />);
    await screen.findByLabelText("输入框区域底色");
    fireEvent.change(screen.getByLabelText("输入框区域底色"), {
      target: { value: "#778899" },
    });
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 10 } });
    await waitFor(() => {
      expect(stored().document.controls.brightness).toBe(10);
      expect(stored().document.design?.regions.composer.background).toBe(
        "#778899",
      );
      expect(
        (screen.getByRole("button", { name: "保存主题" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false);
    });
    expect(
      (screen.getByLabelText("输入框区域底色") as HTMLInputElement).value,
    ).toBe("#778899");
    expect(api.saveDraft).not.toHaveBeenCalled();
  });
  it("keeps all controls responsive and preserves edits made during a regional write", async () => {
    const { api, stored } = regionalHarness();
    const update = api.updateDraft;
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    api.updateDraft = vi.fn(update).mockImplementationOnce(async (...args) => {
      await gate;
      return update(...args);
    });
    render(<App api={api} />);
    await screen.findByLabelText("输入框区域底色");
    fireEvent.change(screen.getByLabelText("输入框区域底色"), {
      target: { value: "#778899" },
    });
    await waitFor(() => expect(api.updateDraft).toHaveBeenCalledTimes(1));
    for (const label of ["亮度", "区域不透明度", "输入框正文颜色"]) {
      expect(screen.getByLabelText(label).closest("fieldset")?.disabled).toBe(
        false,
      );
    }
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 18 } });
    fireEvent.change(screen.getByLabelText("区域不透明度"), {
      target: { value: 55 },
    });
    fireEvent.change(screen.getByLabelText("输入框正文颜色"), {
      target: { value: "#334455" },
    });
    expect(
      (screen.getByRole("button", { name: "保存主题" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    await act(async () => release());
    expect(
      (screen.getByLabelText("区域不透明度") as HTMLInputElement).value,
    ).toBe("55");
    await waitFor(() => {
      expect(stored().document.controls.brightness).toBe(18);
      expect(stored().document.design?.regions.composer).toMatchObject({
        background: "#778899",
        opacity: 55,
        text: "#334455",
      });
      expect(
        (screen.getByRole("button", { name: "保存主题" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false);
    });
  });
  it("retains failed regional edits, blocks saving old effects, and retries only on request", async () => {
    const { api, stored } = regionalHarness();
    api.updateDraft = vi
      .fn(api.updateDraft)
      .mockRejectedValueOnce(new Error("DISK_FULL"));
    render(<App api={api} />);
    await screen.findByLabelText("输入框区域底色");
    fireEvent.change(screen.getByLabelText("输入框区域底色"), {
      target: { value: "#223355" },
    });
    await screen.findByRole("alert");
    expect(
      (screen.getByLabelText("输入框区域底色") as HTMLInputElement).value,
    ).toBe("#223355");
    expect(stored().document.design?.regions.composer.background).toBe(
      "#aabbcc",
    );
    expect(
      (screen.getByRole("button", { name: "保存主题" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "开始真实预览",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 220));
    });
    expect(api.updateDraft).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "重试保存微调" }));
    await waitFor(() => {
      expect(stored().document.design?.regions.composer.background).toBe(
        "#223355",
      );
      expect(
        (screen.getByRole("button", { name: "保存主题" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false);
    });
    expect(api.saveDraft).not.toHaveBeenCalled();
  });
  it("keeps preview mounted while changing every control without saving or reimporting", async () => {
    const { api } = harness();
    render(<App api={api} />);
    await screen.findByTitle("WorkBuddy 仿真效果");
    for (const [label, value] of [
      ["亮度", "15"],
      ["背景模糊", "12"],
      ["面板不透明度", "90"],
      ["强调色", "#888800"],
    ]) {
      fireEvent.change(screen.getByLabelText(label), { target: { value } });
      expect(screen.getByTitle("WorkBuddy 仿真效果")).toBeTruthy();
      await waitFor(() =>
        expect(
          (
            screen.getByRole("button", {
              name: "保存主题",
            }) as HTMLButtonElement
          ).disabled,
        ).toBe(false),
      );
    }
    expect(api.updateDraft).toHaveBeenCalledTimes(4);
    expect(api.saveDraft).not.toHaveBeenCalled();
    expect(api.importImage).not.toHaveBeenCalled();
  });
  it("serializes global writes without letting an earlier response replace current input", async () => {
    const { api } = harness();
    const responses: Array<{
      update: DraftUpdate;
      resolve: (view: DraftView) => void;
    }> = [];
    api.updateDraft = vi.fn(
      (_id: string, update: DraftUpdate) =>
        new Promise<DraftView>((resolve) =>
          responses.push({ update, resolve }),
        ),
    );
    render(<App api={api} />);
    const frame = (await screen.findByTitle(
      "WorkBuddy 仿真效果",
    )) as HTMLIFrameElement;
    // JSDOM does not parse srcdoc, so install the fixture's style node for this DOM unit test.
    const style = frame.contentDocument!.createElement("style");
    style.id = "codedrobe-theme-style-workbuddy";
    frame.contentDocument!.head.append(style);
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 3 } });
    await waitFor(() => expect(responses.length).toBe(1));
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 8 } });
    expect(responses.length).toBe(1);
    const complete = (i: number) => {
      const view = draft();
      view.document.controls = responses[i].update.controls;
      view.document.editSequence = responses[i].update.sequence;
      view.document.compiled.css =
        "/* " + responses[i].update.controls.brightness + " */";
      responses[i].resolve(view);
    };
    await act(async () => complete(0));
    expect((screen.getByLabelText("亮度") as HTMLInputElement).value).toBe("8");
    await waitFor(() => expect(responses.length).toBe(2));
    expect(responses[1].update.sequence).toBe(responses[0].update.sequence + 1);
    await act(async () => complete(1));
    expect(style.textContent).toContain("/* 8 */");
  });
  it("trial starts explicitly and shows a countdown with no implicit permanent save", async () => {
    const { api } = harness();
    render(<App api={api} />);
    await screen.findByTitle("WorkBuddy 仿真效果");
    fireEvent.click(screen.getByRole("button", { name: "开始真实预览" }));
    expect(api.startTrial).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "开始 10 分钟预览" }));
    await screen.findByRole("button", { name: "保存并保留" });
    expect(api.startTrial).toHaveBeenCalledTimes(1);
    expect(api.saveDraft).not.toHaveBeenCalled();
  });
  it("edits during a live trial update the host without saving or reimporting", async () => {
    const { api } = harness();
    render(<App api={api} />);
    await screen.findByRole("button", { name: "开始真实预览" });
    fireEvent.click(screen.getByRole("button", { name: "开始真实预览" }));
    fireEvent.click(screen.getByRole("button", { name: "开始 10 分钟预览" }));
    await screen.findByRole("button", { name: "保存并保留" });
    expect((screen.getByLabelText("亮度") as HTMLInputElement).disabled).toBe(
      false,
    );
    fireEvent.change(screen.getByLabelText("亮度"), { target: { value: 18 } });
    await waitFor(() => expect(api.syncTrial).toHaveBeenCalledWith("trial", 1));
    expect(api.saveDraft).not.toHaveBeenCalled();
  });
});
describe("preview isolation and stylesheet contract", () => {
  it("embeds the actual compiled CSS with one theme style node and blocks script execution", () => {
    const css = ".cb-assistant-message{color:#123456}";
    const doc = previewDocument(css, image, "chat");
    expect(doc).toContain(css);
    expect(doc.match(/id="codedrobe-theme-style-workbuddy"/g)).toHaveLength(1);
    expect(doc).toContain("script-src 'none'");
    expect(doc).not.toContain("<script");
    expect(doc).not.toContain("对比度检查：通过");
  });
  it("rejects remote background URLs and escapes style termination", () => {
    const doc = previewDocument(
      "</style><script>alert(1)</script>",
      "https://evil.example/wallpaper.png",
      "home",
    );
    expect(doc).not.toContain("https://evil.example");
    expect(doc).not.toContain("</style><script>");
  });
});
