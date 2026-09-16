import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RenderedPreview, ThemeDocument, TrialSession } from "./types";
import { RealPreviewPanel } from "./RealPreviewPanel";
import { studioApi } from "./studio-api";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
describe("consent and real rendering", () => {
  it("stops the initial loading message after a failed capture and offers manual retry", async () => {
    const session = {
      id: "trial",
      phase: "active",
      syncedSequence: 0,
    } as TrialSession;
    const draft = {
      draftId: "draft",
      name: "x",
      editSequence: 0,
    } as ThemeDocument;
    const api = {
      ...studioApi,
      syncTrial: vi.fn(async () => session),
      captureTrial: vi
        .fn()
        .mockRejectedValue(new Error("WORKBUDDY_NOT_PAINTING")),
    };
    render(
      <RealPreviewPanel
        api={api}
        trial={session}
        draft={draft}
        pending={false}
        onSynced={() => undefined}
      />,
    );
    await screen.findByRole("alert");
    expect(screen.queryByText("正在获取真实画面")).toBeNull();
    expect(screen.getByText("尚未取得真实画面")).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "刷新当前画面",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
    expect(api.captureTrial).toHaveBeenCalledTimes(1);
  });
  it("marks a requested or failed refresh as stale and blocks exporting the old frame", async () => {
    let now = Date.now();
    vi.spyOn(Date, "now").mockImplementation(() => now);
    const session: TrialSession = {
      id: "trial",
      name: "x",
      phase: "active",
      deadlineMs: now + 600000,
      error: null,
      syncedSequence: 0,
      cssHash: "hash",
    };
    const document = {
      draftId: "draft",
      name: "x",
      editSequence: 0,
    } as ThemeDocument;
    const frame: RenderedPreview = {
      id: "first",
      trialId: "trial",
      draftId: "draft",
      sequence: 0,
      cssHash: "hash",
      targetId: "renderer",
      width: 100,
      height: 100,
      scene: "home",
      capturedAt: "now",
      workbuddyVersion: "5.2.6",
      imageDataUrl: "data:image/png;base64,first",
      captureMs: 1,
    };
    const api = {
      ...studioApi,
      syncTrial: vi.fn(async () => session),
      captureTrial: vi
        .fn()
        .mockResolvedValueOnce(frame)
        .mockRejectedValueOnce(new Error("CAPTURE_TIMEOUT"))
        .mockResolvedValueOnce({ ...frame, id: "fresh" }),
      exportCapture: vi.fn(),
    };
    render(
      <RealPreviewPanel
        api={api}
        trial={session}
        draft={document}
        pending={false}
        onSynced={() => undefined}
      />,
    );
    await screen.findByText("当前实机效果");
    now += 1200;
    fireEvent.click(screen.getByRole("button", { name: "刷新当前画面" }));
    expect(screen.queryByText("当前实机效果")).toBeNull();
    await screen.findByText("CAPTURE_TIMEOUT");
    expect(screen.getByText("旧画面 · 不代表当前参数")).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "导出本机截图…",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(
      (
        screen.getByRole("button", {
          name: "设为对比基准",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    now += 1200;
    fireEvent.click(screen.getByRole("button", { name: "刷新当前画面" }));
    await screen.findByText("当前实机效果");
    expect(
      (
        screen.getByRole("button", {
          name: "导出本机截图…",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
    expect(api.exportCapture).not.toHaveBeenCalled();
  });
  it("syncs newer parameters without waiting for an older slow screenshot", async () => {
    let now = Date.now();
    vi.spyOn(Date, "now").mockImplementation(() => now);
    const session: TrialSession = {
      id: "trial",
      name: "x",
      phase: "active",
      deadlineMs: Date.now() + 600000,
      error: null,
      syncedSequence: 0,
    };
    const document = {
      draftId: "draft",
      name: "x",
      editSequence: 0,
    } as ThemeDocument;
    const oldFrame: RenderedPreview = {
      id: "old",
      trialId: "trial",
      draftId: "draft",
      sequence: 0,
      cssHash: "hash",
      targetId: "renderer",
      width: 100,
      height: 100,
      scene: "home",
      capturedAt: "now",
      workbuddyVersion: "5.2.6",
      imageDataUrl: "data:image/png;base64,old",
      captureMs: 1,
    };
    let complete: (frame: RenderedPreview) => void = () => undefined;
    const api = {
      ...studioApi,
      syncTrial: vi.fn(async (_id: string, sequence: number) => ({
        ...session,
        syncedSequence: sequence,
      })),
      captureTrial: vi
        .fn()
        .mockImplementationOnce(
          () =>
            new Promise<RenderedPreview>((resolve) => {
              complete = resolve;
            }),
        )
        .mockResolvedValueOnce({
          ...oldFrame,
          id: "new",
          sequence: 1,
          imageDataUrl: "data:image/png;base64,new",
        }),
    };
    const ui = render(
      <RealPreviewPanel
        api={api}
        trial={session}
        draft={document}
        pending={false}
        onSynced={() => undefined}
      />,
    );
    await waitFor(() => expect(api.captureTrial).toHaveBeenCalledTimes(1));
    ui.rerender(
      <RealPreviewPanel
        api={api}
        trial={session}
        draft={{ ...document, editSequence: 1 }}
        pending={false}
        onSynced={() => undefined}
      />,
    );
    await waitFor(() => expect(api.syncTrial).toHaveBeenCalledWith("trial", 1));
    expect(api.captureTrial).toHaveBeenCalledTimes(1);
    now += 1200;
    await act(async () => complete(oldFrame));
    await screen.findByText("当前实机效果");
    expect(
      (screen.getByAltText("WorkBuddy 当前真实换肤截图") as HTMLImageElement)
        .src,
    ).toBe("data:image/png;base64,new");
    expect(api.captureTrial).toHaveBeenCalledTimes(2);
  });
  it("never presents a late screenshot as the current parameter version", async () => {
    const session: TrialSession = {
      id: "trial",
      name: "x",
      phase: "active",
      deadlineMs: Date.now() + 600000,
      error: null,
      syncedSequence: 0,
    };
    const document = {
      draftId: "draft",
      name: "x",
      editSequence: 0,
    } as ThemeDocument;
    let complete: (f: RenderedPreview) => void = () => undefined;
    const api = {
      ...studioApi,
      syncTrial: vi.fn(async () => session),
      captureTrial: vi.fn(
        () =>
          new Promise<RenderedPreview>((resolve) => {
            complete = resolve;
          }),
      ),
    };
    const ui = render(
      <RealPreviewPanel
        api={api}
        trial={session}
        draft={document}
        pending={false}
        onSynced={() => undefined}
      />,
    );
    await waitFor(() => expect(api.captureTrial).toHaveBeenCalledTimes(1));
    ui.rerender(
      <RealPreviewPanel
        api={api}
        trial={session}
        draft={{ ...document, editSequence: 1 }}
        pending={true}
        onSynced={() => undefined}
      />,
    );
    await act(async () =>
      complete({
        id: "old",
        trialId: "trial",
        draftId: "draft",
        sequence: 0,
        cssHash: "old",
        targetId: "renderer",
        width: 100,
        height: 100,
        scene: "home",
        capturedAt: "now",
        workbuddyVersion: "5.2.6",
        imageDataUrl: "data:image/png;base64,old",
        captureMs: 1,
      }),
    );
    expect(screen.queryByAltText("WorkBuddy 当前真实换肤截图")).toBeNull();
  });
});
