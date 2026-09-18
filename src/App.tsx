import profiles from "../shared/offline-styles.json";
import legacyProfiles from "../shared/offline-styles-v1.json";
import { useCallback, useEffect, useRef, useState } from "react";
import { WorkBuddyThemePreview } from "./WorkBuddyThemePreview";
import { SettingsPanel } from "./SettingsPanel";
import { RealPreviewPanel } from "./RealPreviewPanel";
import { RegionEditor } from "./RegionEditor";
import { useThemeEditor } from "./use-theme-editor";
import {
  errorText,
  fileBase64,
  referenceKey,
  sameTheme,
  studioApi,
  type StudioApi,
} from "./studio-api";
import type {
  AppError,
  StyleSelection,
  LibraryItem,
  RuntimeSnapshot,
  ThemeControls,
  ThemePreview,
  ThemeRef,
  TrialSession,
} from "./types";

type Page = "library" | "editor" | "settings";
type Confirmation = {
  title: string;
  body: string;
  label: string;
  action: () => Promise<unknown>;
};
export default function App({ api = studioApi }: { api?: StudioApi }) {
  const [page, setPage] = useState<Page>("editor");
  const [library, setLibrary] = useState<LibraryItem[]>([]);
  const [snapshot, setSnapshot] = useState<RuntimeSnapshot | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [operationBusy, setBusy] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [selected, setSelected] = useState<LibraryItem | null>(null);
  const [savedPreview, setSavedPreview] = useState<ThemePreview | null>(null);
  const [rename, setRename] = useState<{
    item: LibraryItem;
    name: string;
  } | null>(null);
  const [now, setNow] = useState(Date.now());
  const [job, setJob] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const editor = useThemeEditor(api);
  const { adopt } = editor;
  const notify = useCallback((message: string) => setNotice(message), []);
  const refreshLibrary = useCallback(
    async () => setLibrary(await api.library()),
    [api],
  );
  useEffect(() => {
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void api
      .subscribe((value) => {
        if (active) setSnapshot(value);
      })
      .then((fn) => {
        if (active) unsubscribe = fn;
        else fn();
      })
      .catch((error) => {
        if (active) notify(errorText(error));
      });
    const poll = () =>
      void api
        .runtime()
        .then((value) => {
          if (active) setSnapshot(value);
        })
        .catch((error) => {
          if (active) notify(errorText(error));
        });
    poll();
    void refreshLibrary().catch((error) => notify(errorText(error)));
    void api
      .latestDraft()
      .then((view) => {
        if (active && view) adopt(view);
      })
      .catch((error) => {
        if (active) notify(errorText(error));
      });
    const timer = window.setInterval(poll, 10000);
    return () => {
      active = false;
      unsubscribe?.();
      window.clearInterval(timer);
    };
  }, [api, adopt, notify, refreshLibrary]);
  useEffect(() => {
    if (!snapshot?.trial) return;
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [snapshot?.trial]);
  const trial = snapshot?.trial;
  const draft = editor.draft?.document;
  const busy = operationBusy;
  const locked = !!busy || !!trial || !!snapshot?.error;
  const editLocked =
    !!busy || !!snapshot?.error || (!!trial && trial.phase !== "active");
  const fields = editor.fields;
  const editingProfiles =
    (fields?.style?.version ?? 1) < 2 ? legacyProfiles : profiles;
  const preview: ThemePreview | null = selected
    ? savedPreview
    : draft && fields
      ? {
          name: fields.name,
          css: draft.compiled.css,
          imagePath: editor.draft!.imagePath,
          compiled: draft.compiled,
        }
      : null;
  const canSave =
    !!draft &&
    !selected &&
    editor.valid &&
    !editor.pending &&
    !editor.error &&
    !editLocked;
  const chooseStyle = (style: StyleSelection, reset = false) => {
    const manual = draft?.manualOverrides;
    const adjusted =
      !!editor.pending ||
      !!manual?.global ||
      !!manual?.position ||
      !!Object.keys(manual?.regions ?? {}).length ||
      (!!draft && draft.schemaVersion < 3);
    const action = async () => editor.chooseStyle(style);
    if (adjusted || reset)
      setConfirmation({
        title: reset ? "恢复风格默认值" : "切换离线风格",
        body: "保留图片、名称和背景位置；其余参数恢复所选风格默认值。旧修订不变，只有保存后才产生新修订。",
        label: "采用风格默认值",
        action,
      });
    else void action();
  };
  const requestTrial = () =>
    setConfirmation({
      title: "开始真实预览",
      body: "将暂时改变当前 WorkBuddy 外观并读取当前页面截图，仅留本机。微调会持续同步；10 分钟到期或关闭编辑器会恢复。不会自动切换聊天或打开菜单。",
      label: "开始 10 分钟预览",
      action: () => startTrial(),
    });
  const onSynced = useCallback(
    (session: TrialSession) =>
      setSnapshot((previous) =>
        previous && previous.trial?.id === session.id
          ? { ...previous, trial: session }
          : previous,
      ),
    [],
  );
  const current = library.find((item) =>
    sameTheme(snapshot?.workbuddy.currentThemeId, item.reference),
  );
  const run = async <T,>(
    label: string,
    action: () => Promise<T>,
  ): Promise<T | undefined> => {
    setBusy(label);
    try {
      return await action();
    } catch (error) {
      notify(errorText(error));
      return undefined;
    } finally {
      setBusy(null);
    }
  };
  const importImage = async (file: File | undefined) => {
    if (!file) return;
    await run("正在本地处理图片…", async () => {
      const data = await fileBase64(file);
      const id = crypto.randomUUID();
      setJob(id);
      const view = await api.importImage(file.name, data, id);
      adopt(view);
      setSelected(null);
      setSavedPreview(null);
      setPage("editor");
      notify("草稿已生成；微调只在本地进行，不会新增正式主题。");
    });
    setJob(null);
  };
  const applyReference = async (
    reference: ThemeRef,
    allowRestart = false,
  ): Promise<void> => {
    try {
      const result = await api.apply(reference, allowRestart);
      notify(result.message);
      await refreshLibrary();
    } catch (error) {
      if ((error as AppError)?.code === "RESTART_REQUIRED") {
        setConfirmation({
          title: "需要重启 WorkBuddy",
          body: "请先保存 WorkBuddy 中的工作。确认后正常关闭并以本地调试模式重新启动，再应用主题。",
          label: "重启并应用",
          action: () => applyReference(reference, true),
        });
      } else throw error;
    }
  };
  const save = async (apply = false) => {
    if (!draft) return;
    await run(apply ? "正在保存并应用…" : "正在保存主题…", async () => {
      const reference = await api.saveDraft(draft.draftId);
      if (!trial) {
        const view = await api.openTheme(reference);
        adopt(view);
      } else {
        const view = await api.latestDraft();
        if (view) adopt(view);
      }
      await refreshLibrary();
      notify("主题已保存到主题库。");
      if (apply) await applyReference(reference);
    });
  };
  const startTrial = async (allowRestart = false): Promise<void> => {
    if (!draft) return;
    try {
      const session = await api.startTrial(draft.draftId, allowRestart);
      setNow(Date.now());
      setSnapshot((previous) =>
        previous ? { ...previous, trial: session } : previous,
      );
      setNotice(null);
    } catch (error) {
      if ((error as AppError)?.code === "RESTART_REQUIRED") {
        setConfirmation({
          title: "连接 WorkBuddy 后试用",
          body: "实机预览需要启动或重启 WorkBuddy。请先保存工作；确认后连接并临时应用，10 分钟未保留会恢复。",
          label: "连接并开始试用",
          action: () => startTrial(true),
        });
      } else throw error;
    }
  };
  const finishTrial = async (keep: boolean) => {
    await run(keep ? "正在保存试用效果…" : "正在恢复之前效果…", async () => {
      if (keep) {
        const reference = await api.confirmTrial();
        adopt(await api.openTheme(reference));
        await refreshLibrary();
      } else await api.cancelTrial();
      setSnapshot((previous) =>
        previous ? { ...previous, trial: null } : previous,
      );
      notify(keep ? "试用效果已保存并正式应用。" : "已恢复试用前的效果。");
    });
  };
  const openItem = async (item: LibraryItem) => {
    await run("正在读取主题…", async () => {
      const value = await api.previewTheme(item.reference);
      setSelected(item);
      setSavedPreview(value);
      setPage("editor");
    });
  };
  const editItem = async (item: LibraryItem) => {
    await run("正在打开编辑草稿…", async () => {
      const view = item.editable
        ? await api.openTheme(item.reference)
        : await api.copyLegacy(item.reference);
      adopt(view);
      setSelected(null);
      setSavedPreview(null);
      setPage("editor");
      if (view.warning) notify(view.warning);
    });
  };
  const control = (key: keyof ThemeControls, value: number | string | null) => {
    if (fields)
      editor.change({ controls: { ...fields.controls, [key]: value } });
  };
  return (
    <main className="studio-shell">
      <header className="studio-topbar">
        <div className="brand-lockup">
          <span className="brand-icon">◈</span>
          <div>
            <strong>WorkBuddy 主题工作台</strong>
            <small>图片融合 · 1.6.4 候选版</small>
          </div>
        </div>
        <div className="top-status">
          <span
            className={
              snapshot?.workbuddy.cdpAvailable
                ? "status-pill connected"
                : "status-pill"
            }
          >
            {snapshot?.workbuddy.cdpAvailable ? "● 已连接" : "○ 需连接"}
          </span>
          <span className="current-theme" title={current?.name}>
            {(trial ? "预览 · " + trial.name : current?.name) ||
              (snapshot?.workbuddy.currentThemeId
                ? "未知主题"
                : snapshot?.workbuddy.styleNodeCount === 0
                  ? "WorkBuddy 原版"
                  : "等待检测")}
          </span>
          <label className="keep-toggle">
            <input
              type="checkbox"
              checked={snapshot?.runtime.autoKeepTheme || false}
              disabled={locked || snapshot?.runtime.desiredState !== "theme"}
              onChange={(e) =>
                void run("更新自动保持…", async () => {
                  await api.autoKeep(e.target.checked);
                })
              }
            />
            自动保持
          </label>
        </div>
      </header>
      <nav className="main-nav" aria-label="主导航">
        {(
          [
            ["library", "主题库"],
            ["editor", "创建与编辑"],
            ["settings", "设置与诊断"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            aria-current={page === id ? "page" : undefined}
            disabled={!!busy}
            onClick={() => {
              if (trial && id !== "editor") {
                setConfirmation({
                  title: "离开编辑器并恢复",
                  body: "离开编辑页将结束实时预览并恢复原来的效果，草稿仍会保留。",
                  label: "恢复并离开",
                  action: async () => {
                    await finishTrial(false);
                    if (!(await api.runtime()).trial) setPage(id);
                  },
                });
              } else setPage(id);
            }}
          >
            {label}
          </button>
        ))}
        <button
          className="restore-link"
          disabled={locked || !snapshot?.workbuddy.appFound}
          onClick={() =>
            setConfirmation({
              title: "恢复 WorkBuddy 原版",
              body: "移除当前主题并停止自动保持。已保存的主题和图片不会删除。",
              label: "恢复原版",
              action: () =>
                run("正在恢复原版…", async () => {
                  notify((await api.restore()).message);
                }),
            })
          }
        >
          恢复原版
        </button>
      </nav>
      {snapshot?.error && (
        <div className="notice field-error" role="alert">
          {snapshot.error}
          <button
            onClick={() =>
              setConfirmation({
                title: "恢复原版并隔离异常记录",
                body: "将损坏的恢复记录留在本地备查，移除 WorkBuddy 主题并停止自动保持。不会删除主题库。",
                label: "恢复原版",
                action: () => api.resolveRecovery(true),
              })
            }
          >
            恢复原版并处理异常
          </button>
        </div>
      )}
      {notice && (
        <div className="notice" role="status">
          <span>{notice}</span>
          <button aria-label="关闭提示" onClick={() => setNotice(null)}>
            ×
          </button>
        </div>
      )}
      {busy && (
        <div className="progress-line" role="status">
          <span className="spinner" />
          {busy}
          {job && (
            <button
              onClick={() =>
                void api
                  .cancelJob(job)
                  .then(() =>
                    notify("已请求取消，将在当前处理步骤安全结束后丢弃结果。"),
                  )
                  .catch((e) => notify(errorText(e)))
              }
            >
              取消处理
            </button>
          )}
        </div>
      )}
      {trial && (
        <section className="trial-banner" aria-live="polite">
          <div>
            <strong>
              {trial.phase === "active"
                ? "实机预览中 · " +
                  Math.max(0, Math.ceil((trial.deadlineMs - now) / 1000)) +
                  " 秒"
                : "试用待恢复"}
            </strong>
            <p>
              {trial.phase === "active"
                ? trial.deadlineMs - now < 60000
                  ? "剩余不足 1 分钟，请保留、续期或等待自动恢复。"
                  : "微调持续同步到 WorkBuddy。到期或关闭编辑器会恢复之前效果。"
                : "尚未确认恢复成功；连接恢复后后台会继续处理。"}
              {trial.error && "（" + trial.error + "）"}
            </p>
          </div>
          <div className="button-row">
            <button
              disabled={!!busy || trial.phase !== "active"}
              onClick={() =>
                void run("续期…", async () =>
                  onSynced(await api.renewTrial(trial.id)),
                )
              }
            >
              再加 10 分钟
            </button>
            {trial.error === "TRIAL_EXTERNAL_CHANGE" && (
              <button
                disabled={!!busy}
                onClick={() =>
                  setConfirmation({
                    title: "保留当前外观",
                    body: "结束待恢复状态并停止自动保持，当前 WorkBuddy 外观不变；恢复记录会留存备查。",
                    label: "保留当前外观",
                    action: () => api.resolveRecovery(false),
                  })
                }
              >
                保留外部外观
              </button>
            )}
            <button disabled={!!busy} onClick={() => void finishTrial(false)}>
              恢复之前效果
            </button>
          </div>
        </section>
      )}
      {page === "library" && (
        <section className="library-page">
          <div className="page-heading">
            <div>
              <span className="eyebrow">YOUR COLLECTION</span>
              <h1>主题库</h1>
              <p>找到一种舒服的工作氛围。保存主题与应用主题彼此独立。</p>
            </div>
            <button
              className="primary"
              disabled={locked}
              onClick={() => {
                setSelected(null);
                setPage("editor");
                input.current?.click();
              }}
            >
              ＋ 从图片创建
            </button>
          </div>
          <div className="library-grid">
            {library.map((item) => (
              <article
                className="theme-card"
                key={referenceKey(item.reference)}
              >
                <button
                  className="card-image"
                  disabled={locked}
                  onClick={() => void openItem(item)}
                  aria-label={"预览 " + item.name}
                >
                  <img
                    src={api.fileUrl(item.previewPath)}
                    alt=""
                    loading="lazy"
                  />
                  <span>
                    {item.editable
                      ? "可编辑"
                      : item.isCustom
                        ? "旧版自定义"
                        : "内置主题"}
                  </span>
                  {sameTheme(
                    snapshot?.workbuddy.currentThemeId,
                    item.reference,
                  ) && <b>使用中</b>}
                </button>
                <div className="card-content">
                  <h2>{item.name}</h2>
                  <p>{item.compatibility}</p>
                  {item.palette && (
                    <div className="card-palette">
                      {[
                        item.palette.background,
                        item.palette.sidebar,
                        item.palette.surface,
                        item.palette.accent,
                        item.palette.text,
                      ].map((color, i) => (
                        <i
                          key={i}
                          style={{ background: color }}
                          title={color}
                        />
                      ))}
                    </div>
                  )}
                  <div className="button-row">
                    <button
                      disabled={locked}
                      onClick={() => void openItem(item)}
                    >
                      预览
                    </button>
                    <button
                      className="primary"
                      disabled={locked || !snapshot?.workbuddy.appFound}
                      onClick={() =>
                        void run("正在应用主题…", () =>
                          applyReference(item.reference),
                        )
                      }
                    >
                      应用
                    </button>
                    {item.isCustom && (
                      <details className="card-menu">
                        <summary>更多</summary>
                        <div>
                          <button
                            disabled={locked}
                            onClick={() => void editItem(item)}
                          >
                            {item.editable ? "继续编辑" : "从图片创建副本"}
                          </button>
                          {item.editable && (
                            <button
                              disabled={locked}
                              onClick={() =>
                                setRename({ item, name: item.name })
                              }
                            >
                              重命名
                            </button>
                          )}
                          <button
                            disabled={
                              locked ||
                              sameTheme(
                                snapshot?.workbuddy.currentThemeId,
                                item.reference,
                              ) ||
                              sameTheme(
                                snapshot?.runtime.selectedThemeId,
                                item.reference,
                              )
                            }
                            onClick={() =>
                              setConfirmation({
                                title: "移除主题",
                                body:
                                  "将“" +
                                  item.name +
                                  "”移入应用数据目录的回收区，可手动找回。正在使用或待恢复的主题不能移除。",
                                label: "移入回收区",
                                action: () =>
                                  run("正在移除主题…", async () => {
                                    await api.deleteTheme(item.reference);
                                    await refreshLibrary();
                                    notify(
                                      "主题已移入本地回收区，图片资源保留。",
                                    );
                                  }),
                              })
                            }
                          >
                            移除主题
                          </button>
                        </div>
                      </details>
                    )}
                  </div>
                </div>
              </article>
            ))}
          </div>
        </section>
      )}
      <input
        ref={input}
        type="file"
        accept="image/jpeg,image/png,image/webp"
        hidden
        data-testid="image-input"
        onChange={(e) => {
          void importImage(e.target.files?.[0]);
          e.currentTarget.value = "";
        }}
      />
      {page === "editor" && (
        <section className="editor-page">
          <aside className="editor-controls">
            <details open>
              <summary>图片与参数</summary>
              <div className="control-content">
                <div
                  className="upload-area"
                  onDragOver={(e) => e.preventDefault()}
                  onDrop={(e) => {
                    e.preventDefault();
                    if (!locked) void importImage(e.dataTransfer.files[0]);
                  }}
                >
                  {editor.draft && !selected ? (
                    <img
                      src={api.fileUrl(editor.draft.imagePath)}
                      alt="上传图片缩略图"
                    />
                  ) : (
                    <span className="upload-symbol">＋</span>
                  )}
                  <strong>
                    {selected
                      ? "正在查看已保存主题"
                      : draft?.sourceFilename || "让一张图片，成为你的工作空间"}
                  </strong>
                  <small>JPG / PNG / WebP · 最大 10MB</small>
                  <button
                    disabled={locked}
                    onClick={() => input.current?.click()}
                  >
                    {draft ? "更换图片" : "选择图片"}
                  </button>
                </div>
                {selected ? (
                  <div className="saved-description">
                    <h2>{selected.name}</h2>
                    <p>{selected.compatibility}</p>
                    {selected.isCustom && (
                      <button
                        disabled={locked}
                        onClick={() => void editItem(selected)}
                      >
                        {selected.editable ? "继续编辑" : "从图片创建副本"}
                      </button>
                    )}
                    <button
                      disabled={locked}
                      onClick={() => {
                        setSelected(null);
                        setSavedPreview(null);
                      }}
                    >
                      返回草稿
                    </button>
                  </div>
                ) : fields ? (
                  <fieldset disabled={editLocked}>
                    <label>
                      主题名称
                      <input
                        value={fields.name}
                        maxLength={48}
                        onChange={(e) =>
                          editor.change({ name: e.target.value })
                        }
                      />
                    </label>
                    <label>
                      离线风格
                      <select
                        aria-label="离线风格"
                        value={fields.style?.id ?? ""}
                        onChange={(e) => {
                          const p = editingProfiles.find(
                            (p) => p.id === e.target.value,
                          );
                          if (p)
                            chooseStyle({
                              id: p.id as StyleSelection["id"],
                              version: p.version,
                            });
                        }}
                      >
                        {!fields.style && (
                          <option value="">旧主题 · 保持原有效果</option>
                        )}
                        {editingProfiles.map((p) => (
                          <option key={p.id} value={p.id}>
                            {p.name}
                            {draft?.analysis?.recommended === p.id
                              ? " · 导入推荐"
                              : ""}
                          </option>
                        ))}
                      </select>
                    </label>
                    <p className="footnote">
                      {editingProfiles.find((p) => p.id === fields.style?.id)
                        ?.description ??
                        "旧主题保持原有效果；可创建新版融合副本。"}
                    </p>
                    {(fields.style?.version ?? 1) < 2 && (
                      <button
                        disabled={
                          locked ||
                          editor.pending ||
                          !editor.valid ||
                          !!editor.error
                        }
                        onClick={() =>
                          setConfirmation({
                            title: "创建新版融合副本",
                            body: "保留图片、名称、背景位置和手动强调色；重建分区样式，背景模糊设为 0。原稿、旧修订及 WorkBuddy 当前效果不变。",
                            label: "创建副本",
                            action: async () => {
                              const id = crypto.randomUUID();
                              setJob(id);
                              try {
                                const view = await api.copyFusion(
                                  draft!.draftId,
                                  id,
                                );
                                adopt(view);
                                notify(
                                  view.warning ??
                                    "新版融合副本已创建，尚未应用。",
                                );
                              } finally {
                                setJob(null);
                              }
                            },
                          })
                        }
                      >
                        创建新版融合副本
                      </button>
                    )}
                    {fields.style?.version === 2 && (
                      <p className="footnote">
                        融合 v2 · 默认 0px
                        模糊。面板不透明度为相对调节，各区域实际值见下方可读性检查。
                      </p>
                    )}
                    {(
                      [
                        ["brightness", "亮度", -40, 40, ""],
                        ["blur", "背景模糊", 0, 24, "px"],
                        ["panelOpacity", "面板不透明度", 55, 96, "%"],
                      ] as const
                    ).map(([key, label, min, max, unit]) => (
                      <label className="range-field" key={key}>
                        <span>
                          {label}
                          <span className="range-value" aria-hidden="true">
                            {key === "panelOpacity" &&
                            fields.style?.version === 2
                              ? `${fields.controls[key] >= (editingProfiles.find((p) => p.id === fields.style?.id)?.controls.panelOpacity ?? 82) ? "+" : ""}${fields.controls[key] - (editingProfiles.find((p) => p.id === fields.style?.id)?.controls.panelOpacity ?? 82)}`
                              : fields.controls[key]}
                            {unit}
                          </span>
                        </span>
                        <input
                          aria-label={label}
                          type="range"
                          min={min}
                          max={max}
                          value={fields.controls[key]}
                          onChange={(e) => control(key, Number(e.target.value))}
                        />
                      </label>
                    ))}
                    <div className="color-control">
                      <span>强调色</span>
                      <div className="accent-field">
                        <input
                          type="color"
                          aria-label="选择强调色"
                          value={
                            /^#[0-9a-f]{6}$/i.test(fields.controls.accent || "")
                              ? fields.controls.accent!
                              : draft!.compiled.palette.accent
                          }
                          onChange={(e) => control("accent", e.target.value)}
                        />
                        <input
                          aria-label="强调色"
                          placeholder="自动，例如 #537B69"
                          value={fields.controls.accent || ""}
                          onChange={(e) =>
                            control("accent", e.target.value || null)
                          }
                        />
                        <button
                          aria-label="恢复自动强调色"
                          onClick={() => control("accent", null)}
                        >
                          ↺
                        </button>
                      </div>
                    </div>
                    <p className="footnote">
                      所有处理均在本机完成；分区方案只在对应区域执行可读性修正。
                    </p>
                    {draft?.design?.explanation && (
                      <p className="footnote">{draft.design.explanation}</p>
                    )}
                  </fieldset>
                ) : (
                  <p className="footnote">
                    导入后自动生成本地方案。先预览，再决定是否应用，不会修改正在运行的
                    WorkBuddy。
                  </p>
                )}
                {!editor.valid && fields && (
                  <p className="field-error">
                    请填写主题名称，并使用 #RRGGBB 格式的强调色或留空。
                  </p>
                )}
                {editor.error && (
                  <div className="field-error" role="alert">
                    <p>{editor.error}；当前微调尚未保存。</p>
                    <button
                      disabled={editLocked || !editor.valid}
                      onClick={editor.retry}
                    >
                      重试保存微调
                    </button>
                  </div>
                )}
                {!selected && draft && editor.design && (
                  <RegionEditor
                    key={draft.draftId}
                    design={editor.design}
                    disabled={editLocked}
                    onChange={editor.changeDesign}
                    onReset={
                      fields?.style
                        ? () => chooseStyle(fields.style!, true)
                        : undefined
                    }
                    effective={draft.compiled.effectiveDesign}
                  />
                )}
              </div>
            </details>
          </aside>
          <div className="editor-canvas">
            {draft && !selected ? (
              <>
                <RealPreviewPanel
                  api={api}
                  trial={trial}
                  draft={draft}
                  pending={editor.pending}
                  onSynced={onSynced}
                />
                <details className="auxiliary-preview">
                  <summary>辅助样板与可读性检查（非实机截图）</summary>
                  {preview && (
                    <WorkBuddyThemePreview
                      preview={preview}
                      imageUrl={api.fileUrl(preview.imagePath)}
                      updating={editor.pending}
                    />
                  )}
                </details>
              </>
            ) : preview ? (
              <WorkBuddyThemePreview
                preview={preview}
                imageUrl={api.fileUrl(preview.imagePath)}
                updating={!selected && editor.pending}
              />
            ) : (
              <div className="empty-preview">
                <span>◈</span>
                <h1>先看到效果，再应用主题</h1>
                <p>上传图片后，在这里检查侧栏、对话、输入框与按钮的搭配。</p>
                <div>
                  <span>01 上传图片</span>
                  <span>02 实时微调</span>
                  <span>03 安全试用</span>
                </div>
              </div>
            )}
          </div>
        </section>
      )}
      <SettingsPanel
        api={api}
        snapshot={snapshot}
        hidden={page !== "settings"}
        notify={notify}
      />
      {page === "editor" && (
        <footer className="editor-actionbar">
          <div>
            <strong>
              {selected
                ? "已保存主题"
                : draft
                  ? !editor.pending &&
                    draft.savedSequence === draft.editSequence
                    ? "主题已保存"
                    : "草稿 · 自动暂存"
                  : "等待导入图片"}
            </strong>
            <small>
              {editor.pending && !selected
                ? "正在更新预览与草稿…"
                : selected
                  ? "应用前可检查三个场景"
                  : "微调只暂存草稿，不新增正式主题"}
            </small>
          </div>
          <div className="button-row">
            {selected ? (
              <>
                <button onClick={() => setPage("library")}>返回主题库</button>
                <button
                  className="primary"
                  disabled={locked || !snapshot?.workbuddy.appFound}
                  onClick={() =>
                    void run("正在应用主题…", () =>
                      applyReference(selected.reference),
                    )
                  }
                >
                  应用此主题
                </button>
              </>
            ) : (
              <>
                <button disabled={!canSave} onClick={() => void save(false)}>
                  保存主题
                </button>
                {trial ? (
                  <button
                    className="primary"
                    disabled={
                      !canSave ||
                      trial.phase !== "active" ||
                      trial.syncedSequence !== draft?.editSequence ||
                      now >= trial.deadlineMs
                    }
                    onClick={() => void finishTrial(true)}
                  >
                    保存并保留
                  </button>
                ) : (
                  <button
                    className="primary"
                    disabled={!canSave}
                    onClick={requestTrial}
                  >
                    开始真实预览
                  </button>
                )}
              </>
            )}
          </div>
        </footer>
      )}
      {confirmation && (
        <div className="modal-backdrop">
          <section
            role="dialog"
            aria-modal="true"
            aria-label={confirmation.title}
            className="confirm-dialog"
          >
            <h2>{confirmation.title}</h2>
            <p>{confirmation.body}</p>
            <div className="button-row">
              <button autoFocus onClick={() => setConfirmation(null)}>
                取消
              </button>
              <button
                className="primary"
                onClick={() => {
                  const action = confirmation.action;
                  setConfirmation(null);
                  void run("正在处理…", action);
                }}
              >
                {confirmation.label}
              </button>
            </div>
          </section>
        </div>
      )}
      {rename && (
        <div className="modal-backdrop">
          <section
            role="dialog"
            aria-modal="true"
            aria-label="重命名主题"
            className="confirm-dialog"
          >
            <h2>重命名主题</h2>
            <label>
              新名称
              <input
                autoFocus
                maxLength={48}
                value={rename.name}
                onChange={(e) => setRename({ ...rename, name: e.target.value })}
              />
            </label>
            <p>创建新修订，已应用的旧修订保持不变。</p>
            <div className="button-row">
              <button onClick={() => setRename(null)}>取消</button>
              <button
                className="primary"
                disabled={!rename.name.trim() || !!busy}
                onClick={() =>
                  void run("正在重命名…", async () => {
                    await api.renameTheme(rename.item.reference, rename.name);
                    setRename(null);
                    await refreshLibrary();
                  })
                }
              >
                保存名称
              </button>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}
