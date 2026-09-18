import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  DraftUpdate,
  DraftView,
  LibraryItem,
  OperationResult,
  RuntimeSnapshot,
  ThemePreview,
  ThemeRef,
  TrialSession,
  RenderedPreview,
} from "./types";

export const studioApi = {
  library: () => invoke<LibraryItem[]>("studio_library"),
  latestDraft: () => invoke<DraftView | null>("studio_latest_draft"),
  importImage: (filename: string, data: string, jobId?: string) =>
    invoke<DraftView>("studio_import", {
      filename,
      data,
      jobId: jobId ?? null,
    }),
  updateDraft: (draftId: string, update: DraftUpdate) =>
    invoke<DraftView>("studio_update_draft", { draftId, update }),
  saveDraft: (draftId: string) =>
    invoke<ThemeRef>("studio_save_draft", { draftId }),
  openTheme: (reference: ThemeRef) =>
    invoke<DraftView>("studio_open_theme", { reference }),
  copyLegacy: (reference: ThemeRef) =>
    invoke<DraftView>("studio_copy_legacy", { reference }),
  copyFusion: (draftId: string, jobId: string) =>
    invoke<DraftView>("studio_copy_fusion", { draftId, jobId }),
  renameTheme: (reference: ThemeRef, name: string) =>
    invoke<ThemeRef>("studio_rename_theme", { reference, name }),
  deleteTheme: (reference: ThemeRef) =>
    invoke<void>("studio_delete_theme", { reference }),
  previewTheme: (reference: ThemeRef) =>
    invoke<ThemePreview>("studio_preview_theme", { reference }),
  cancelJob: (jobId: string) => invoke<void>("studio_cancel_job", { jobId }),
  apply: (reference: ThemeRef, allowRestart = false) =>
    invoke<OperationResult>("studio_apply", { reference, allowRestart }),
  restore: () => invoke<OperationResult>("studio_restore"),
  runtime: () => invoke<RuntimeSnapshot>("studio_runtime"),
  exportDiagnostic: () => invoke<string>("studio_export_diagnostic"),
  autoKeep: (enabled: boolean) => invoke("studio_auto_keep", { enabled }),
  setPath: (path: string) => invoke("studio_set_path", { path }),
  startTrial: (draftId: string, allowRestart = false) =>
    invoke<TrialSession>("studio_start_trial", { draftId, allowRestart }),
  syncTrial: (trialId: string, sequence: number) =>
    invoke<TrialSession>("studio_sync_trial", { trialId, sequence }),
  renewTrial: (trialId: string) =>
    invoke<TrialSession>("studio_renew_trial", { trialId }),
  captureTrial: (trialId: string) =>
    invoke<RenderedPreview>("studio_capture_trial", { trialId }),
  exportCapture: (captureId: string) =>
    invoke<boolean>("studio_export_capture", { captureId }),
  cancelTrial: () => invoke<void>("studio_cancel_trial"),
  resolveRecovery: (restoreOriginal: boolean) =>
    invoke<void>("studio_resolve_recovery", { restoreOriginal }),
  confirmTrial: () => invoke<ThemeRef>("studio_confirm_trial"),
  clearLegacyAiCredential: (confirmed: boolean) =>
    invoke<void>("clear_legacy_ai_credentials", { confirmed }),
  openLogs: () => invoke("open_logs_directory"),
  pickWorkBuddy: () =>
    open({
      multiple: false,
      directory: false,
      filters: [{ name: "WorkBuddy", extensions: ["exe"] }],
    }),
  subscribe: (callback: (snapshot: RuntimeSnapshot) => void) =>
    listen<RuntimeSnapshot>("studio-runtime", (event) =>
      callback(event.payload),
    ),
  fileUrl: (path: string) => convertFileSrc(path),
};
export type StudioApi = typeof studioApi;
export function referenceKey(reference: ThemeRef) {
  return reference.revision
    ? `${reference.id}@${reference.revision}`
    : reference.id;
}
export function sameTheme(key: string | null | undefined, reference: ThemeRef) {
  return key?.split("@")[0] === reference.id;
}
export function errorText(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    const code = "code" in error && typeof error.code === "string" && /^[A-Z][A-Z0-9_]{1,63}$/.test(error.code)
      ? `（${error.code}）` : "";
    return String(error.message) + code;
  }
  return typeof error === "string" ? error : "操作未完成，请重试。";
}
export function fileBase64(file: File): Promise<string> {
  if (
    !/\.(png|jpe?g|webp)$/i.test(file.name) ||
    !file.size ||
    file.size > 10 * 1024 * 1024
  )
    return Promise.reject(
      new Error("请选择不超过 10MB 的 JPG、PNG 或 WebP 图片。"),
    );
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("图片读取失败。"));
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1]);
    reader.readAsDataURL(file);
  });
}
