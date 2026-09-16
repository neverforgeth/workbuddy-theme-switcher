import { useEffect, useRef, useState } from "react";
import type { RenderedPreview, ThemeDocument, TrialSession } from "./types";
import { errorText, type StudioApi } from "./studio-api";

export function RealPreviewPanel({
  api,
  trial,
  draft,
  pending,
  onSynced,
}: {
  api: StudioApi;
  trial: TrialSession | null | undefined;
  draft: ThemeDocument;
  pending: boolean;
  onSynced: (s: TrialSession) => void;
}) {
  const [frame, setFrame] = useState<RenderedPreview | null>(null);
  const [comparison, setComparison] = useState<RenderedPreview | null>(null);
  const [full, setFull] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [capturedRefresh, setCapturedRefresh] = useState(-1);
  const [error, setError] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [syncRetry, setSyncRetry] = useState(0);
  const [captureRetry, setCaptureRetry] = useState(0);
  const [synced, setSynced] = useState<{
    sessionId: string;
    draftId: string;
    sequence: number;
    cssHash?: string | null;
    refresh: number;
  } | null>(null);
  const syncFlight = useRef(false),
    captureFlight = useRef(false),
    lastCapture = useRef(0);
  const callback = useRef(onSynced);
  callback.current = onSynced;
  const sessionId = trial?.id;
  const phase = trial?.phase;
  const working = syncing || capturing;
  useEffect(() => {
    setFrame(null);
    setComparison(null);
    setSynced(null);
    setError(null);
    setExportError(null);
    setCapturedRefresh(-1);
  }, [sessionId]);
  useEffect(() => {
    if (!sessionId || phase !== "active" || pending) return;
    let active = true;
    const timer = window.setTimeout(() => {
      if (syncFlight.current) return;
      syncFlight.current = true;
      setSyncing(true);
      setError(null);
      void (async () => {
        const session = await api.syncTrial(sessionId, draft.editSequence);
        if (!active) return;
        if (
          session.id !== sessionId ||
          session.syncedSequence !== draft.editSequence
        )
          throw new Error("实机同步版本已过期，请刷新当前画面。");
        callback.current(session);
        setSynced({
          sessionId,
          draftId: draft.draftId,
          sequence: draft.editSequence,
          cssHash: session.cssHash,
          refresh,
        });
      })()
        .catch((e) => {
          if (active) setError(errorText(e));
        })
        .finally(() => {
          syncFlight.current = false;
          setSyncing(false);
          if (!active) setSyncRetry((v) => v + 1);
        });
    }, 0);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [
    api,
    sessionId,
    phase,
    draft.draftId,
    draft.editSequence,
    pending,
    refresh,
    syncRetry,
  ]);
  useEffect(() => {
    if (
      !sessionId ||
      phase !== "active" ||
      pending ||
      !synced ||
      synced.sessionId !== sessionId ||
      synced.draftId !== draft.draftId ||
      synced.sequence !== draft.editSequence ||
      synced.refresh !== refresh
    )
      return;
    let active = true;
    const timer = window.setTimeout(() => {
      if (captureFlight.current) return;
      captureFlight.current = true;
      setCapturing(true);
      setError(null);
      void (async () => {
        const delay = Math.max(0, 1100 - (Date.now() - lastCapture.current));
        await new Promise((resolve) => window.setTimeout(resolve, delay));
        if (!active) return;
        lastCapture.current = Date.now();
        const next = await api.captureTrial(sessionId);
        if (!active) return;
        if (
          next.trialId !== sessionId ||
          next.draftId !== draft.draftId ||
          next.sequence !== draft.editSequence ||
          (synced.cssHash && next.cssHash !== synced.cssHash)
        )
          throw new Error("截图与当前参数不一致，请刷新当前画面。");
        setFrame(next);
        setCapturedRefresh(refresh);
      })()
        .catch((e) => {
          if (active) setError(errorText(e));
        })
        .finally(() => {
          captureFlight.current = false;
          setCapturing(false);
          if (!active) setCaptureRetry((v) => v + 1);
        });
    }, 0);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [
    api,
    sessionId,
    phase,
    draft.draftId,
    draft.editSequence,
    pending,
    refresh,
    synced,
    captureRetry,
  ]);
  const current =
    frame &&
    frame.trialId === sessionId &&
    frame.draftId === draft.draftId &&
    frame.sequence === draft.editSequence &&
    (!trial?.cssHash || frame.cssHash === trial.cssHash) &&
    capturedRefresh === refresh &&
    !pending &&
    !working &&
    !error &&
    phase === "active";
  const aligned =
    frame &&
    comparison &&
    frame.targetId === comparison.targetId &&
    frame.scene === comparison.scene &&
    frame.width === comparison.width &&
    frame.height === comparison.height;
  return (
    <section
      className="theme-preview real-preview"
      aria-label="WorkBuddy 真实效果"
    >
      <div className="preview-heading">
        <div>
          <span className="eyebrow">真实渲染 · 当前 WorkBuddy 内容区</span>
          <h2>{draft.name}</h2>
        </div>
        <span role="status">
          {!trial
            ? "尚未开始预览"
            : phase !== "active"
              ? "待恢复"
              : error
                ? "画面待刷新"
                : pending || syncing
                  ? "参数同步中"
                  : !current
                    ? "画面待刷新"
                    : "当前真实效果"}
        </span>
      </div>
      <div className="preview-toolbar">
        <button
          disabled={phase !== "active" || working || pending}
          onClick={() => setRefresh((v) => v + 1)}
        >
          刷新当前画面
        </button>
        <button onClick={() => setFull((v) => !v)}>
          {full ? "适应宽度" : "100% 查看"}
        </button>
        <button disabled={!current} onClick={() => setComparison(frame)}>
          设为对比基准
        </button>
        {comparison && (
          <button onClick={() => setComparison(null)}>关闭对比</button>
        )}
        <button
          disabled={!current || exporting}
          onClick={() => {
            if (!frame) return;
            setExporting(true);
            setExportError(null);
            void api
              .exportCapture(frame.id)
              .catch((e) => setExportError(errorText(e)))
              .finally(() => setExporting(false));
          }}
        >
          导出本机截图…
        </button>
      </div>
      {(error || exportError) && (
        <p className="field-error" role="alert">
          {error || exportError}
        </p>
      )}
      {frame ? (
        <>
          <div
            className={
              "real-preview-images " +
              (comparison ? "compare " : "") +
              (full ? "full-size" : "")
            }
          >
            {comparison && (
              <figure>
                <figcaption>
                  对比基准 · 参数 {comparison.sequence}
                  {!aligned && "（不同场景或尺寸，仅并排参考）"}
                </figcaption>
                <img
                  src={comparison.imageDataUrl}
                  alt="此前的 WorkBuddy 实机效果"
                />
              </figure>
            )}
            <figure>
              <figcaption>
                {current ? "当前实机效果" : "旧画面 · 不代表当前参数"}
              </figcaption>
              <img
                style={full ? { minWidth: frame.width } : undefined}
                src={frame.imageDataUrl}
                alt="WorkBuddy 当前真实换肤截图"
              />
            </figure>
          </div>
          <p className="footnote">
            WorkBuddy {frame.workbuddyVersion || "版本未知"} · {frame.width} ×{" "}
            {frame.height} · {frame.scene} · 参数 {frame.sequence} ·{" "}
            {frame.capturedAt} · 截图 {frame.captureMs}ms
          </p>
        </>
      ) : (
        <div className="empty-real-preview">
          <h3>
            {!trial
              ? "上传完成，下一步：开始真实预览"
              : error || phase !== "active"
                ? "尚未取得真实画面"
                : "正在获取真实画面"}
          </h3>
          <p>
            真实画面来自正在运行的
            WorkBuddy，不是仿真样板。开始后，微调会直接改变 WorkBuddy 外观。
          </p>
        </div>
      )}
      <p className="footnote">
        请保持 WorkBuddy
        不最小化；截图失败后可点击“刷新当前画面”重试。可自行打开对话、菜单或弹窗，程序不会抢焦点或切换会话。截图仅在本机会话内保留，不会外发。
      </p>
    </section>
  );
}
