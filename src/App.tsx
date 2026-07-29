import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { AppError, OperationResult, RuntimeStatus, Theme, WorkBuddyStatus } from "./types";

type Notice = { kind: "success" | "error" | "info"; text: string } | null;

const labelForTheme = (themes: Theme[], themeId: string | null) =>
  themes.find((theme) => theme.id === themeId)?.name ?? (themeId ? "未知 CodeDrobe 主题" : "WorkBuddy 原版");

function errorDetails(error: unknown): Required<Pick<AppError, "code" | "message">> {
  if (typeof error === "object" && error !== null) {
    const candidate = error as AppError;
    return { code: candidate.code ?? "OPERATION_FAILED", message: candidate.message ?? "操作未完成。" };
  }
  return { code: "OPERATION_FAILED", message: String(error) };
}

export default function App() {
  const [themes, setThemes] = useState<Theme[]>([]);
  const [status, setStatus] = useState<WorkBuddyStatus | null>(null);
  const [runtime, setRuntime] = useState<RuntimeStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [restartTheme, setRestartTheme] = useState<Theme | null>(null);

  const refresh = useCallback(async () => {
    const [nextThemes, nextStatus, nextRuntime] = await Promise.all([
      invoke<Theme[]>("get_theme_catalog"),
      invoke<WorkBuddyStatus>("detect_workbuddy"),
      invoke<RuntimeStatus>("get_runtime_status"),
    ]);
    setThemes(nextThemes);
    setStatus(nextStatus);
    setRuntime(nextRuntime);
  }, []);

  useEffect(() => {
    void refresh().catch((error) => {
      const detail = errorDetails(error);
      setNotice({ kind: "error", text: `${detail.code}：${detail.message}` });
    });
    const timer = window.setInterval(() => void refresh().catch(() => undefined), 3000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const currentThemeLabel = useMemo(() => labelForTheme(themes, status?.currentThemeId ?? null), [status, themes]);
  const desiredThemeLabel = useMemo(() => {
    if (runtime?.desiredState !== "theme") return "原版";
    return labelForTheme(themes, runtime.selectedThemeId ?? null);
  }, [runtime, themes]);

  const applyTheme = async (theme: Theme, allowRestart = false) => {
    setBusy(theme.id);
    setNotice({ kind: "info", text: allowRestart ? "正在安全重启 WorkBuddy 并应用主题…" : "正在应用并验证主题…" });
    try {
      const result = await invoke<OperationResult>("apply_theme", { themeId: theme.id, allowRestart });
      await refresh();
      setRestartTheme(null);
      setNotice({ kind: "success", text: result.message });
    } catch (error) {
      const detail = errorDetails(error);
      if (detail.code === "RESTART_REQUIRED") {
        setRestartTheme(theme);
        setNotice({ kind: "info", text: "WorkBuddy 正在运行但尚未开启 CDP。确认后会先正常关闭，再以本地调试模式重新启动。" });
      } else {
        setNotice({ kind: "error", text: `${detail.code}：${detail.message}` });
      }
    } finally {
      setBusy(null);
    }
  };

  const restore = async () => {
    setBusy("restore");
    setNotice({ kind: "info", text: "正在恢复 WorkBuddy 原版，并暂停自动保持…" });
    try {
      const result = await invoke<OperationResult>("restore_theme");
      await refresh();
      setNotice({ kind: "success", text: result.message });
    } catch (error) {
      const detail = errorDetails(error);
      await refresh().catch(() => undefined);
      setNotice({ kind: "error", text: `${detail.code}：${detail.message}` });
    } finally {
      setBusy(null);
    }
  };

  const toggleAutoKeep = async (enabled: boolean) => {
    setBusy("auto");
    try {
      const next = await invoke<RuntimeStatus>("set_auto_keep_theme", { enabled });
      setRuntime(next);
      await refresh();
      setNotice({ kind: "success", text: enabled ? "自动保持主题已开启。" : "自动保持主题已暂停。" });
    } catch (error) {
      const detail = errorDetails(error);
      setNotice({ kind: "error", text: `${detail.code}：${detail.message}` });
    } finally {
      setBusy(null);
    }
  };

  const chooseWorkBuddy = async () => {
    const filename = await open({ multiple: false, directory: false, filters: [{ name: "WorkBuddy", extensions: ["exe"] }] });
    if (!filename || Array.isArray(filename)) return;
    setBusy("path");
    try {
      await invoke("set_workbuddy_path", { path: filename });
      await refresh();
      setNotice({ kind: "success", text: "已保存 WorkBuddy 路径。" });
    } catch (error) {
      const detail = errorDetails(error);
      setNotice({ kind: "error", text: `${detail.code}：${detail.message}` });
    } finally {
      setBusy(null);
    }
  };

  const openLogs = async () => {
    try {
      await invoke("open_logs_directory");
    } catch (error) {
      const detail = errorDetails(error);
      setNotice({ kind: "error", text: `${detail.code}：${detail.message}` });
    }
  };

  return (
    <main className="app-shell">
      <header className="hero">
        <div>
          <p className="eyebrow">LOCAL · CODEDROBE</p>
          <h1>WorkBuddy 主题切换器</h1>
          <p className="hero-copy">只应用已验证的固定主题；不会修改 WorkBuddy.exe 或 app.asar。</p>
        </div>
        <button className="subtle-button" onClick={() => void refresh()} disabled={busy !== null}>
          {busy === null ? "重新检测 WorkBuddy" : "操作进行中"}
        </button>
      </header>

      {notice && <div className={`notice notice-${notice.kind}`}>{notice.text}</div>}

      <section className="status-grid" aria-label="WorkBuddy 状态">
        <StatusItem label="WorkBuddy 路径" value={status?.path ?? "尚未找到"} compact />
        <StatusItem label="版本" value={status?.version ?? "未检测"} />
        <StatusItem label="运行状态" value={status?.running ? "正在运行" : "未运行"} tone={status?.running ? "good" : "neutral"} />
        <StatusItem label="CDP" value={status?.cdpAvailable ? "已连接" : "未连接"} tone={status?.cdpAvailable ? "good" : "neutral"} />
        <StatusItem label="当前实际主题" value={currentThemeLabel} tone={status?.currentThemeId ? "good" : "neutral"} />
        <StatusItem label="主题节点" value={status?.styleNodeCount == null ? "未检测" : String(status.styleNodeCount)} tone={status?.styleNodeCount === 1 || status?.styleNodeCount === 0 ? "good" : "warning"} />
      </section>

      <section className="runtime-panel" aria-label="自动保持主题">
        <div className="runtime-heading">
          <div>
            <p className="eyebrow">自动保持</p>
            <h2>让已选主题在 WorkBuddy 重启后自动恢复</h2>
            <p>开机时切换器会静默运行；只在主题节点丢失、renderer 变化或主题不匹配时重新应用。</p>
          </div>
          <label className="switch-control">
            <input type="checkbox" checked={runtime?.autoKeepTheme ?? false} onChange={(event) => void toggleAutoKeep(event.target.checked)} disabled={busy !== null || runtime?.desiredState !== "theme"} />
            <span aria-hidden="true" />
            自动保持主题
          </label>
        </div>
        <div className="runtime-grid">
          <StatusItem label="期望主题" value={desiredThemeLabel} tone={runtime?.desiredState === "theme" ? "good" : "neutral"} />
          <StatusItem label="后台状态" value={runtime?.monitorStatus ?? "正在启动监控"} tone={runtime?.lastErrorCode ? "warning" : "good"} />
          <StatusItem label="自动重试" value={`${runtime?.retryCount ?? 0} / 3`} tone={runtime?.retryCount ? "warning" : "neutral"} />
          <StatusItem label="最近自动恢复" value={runtime?.lastAutoRecoveryAt ? new Date(runtime.lastAutoRecoveryAt).toLocaleString() : "暂无"} />
          <StatusItem label="开机静默启动" value={runtime?.loginAutostartEnabled ? "已开启" : "未开启"} tone={runtime?.loginAutostartEnabled ? "good" : "neutral"} />
        </div>
        <div className="runtime-actions">
          <button className="secondary-button" onClick={() => void toggleAutoKeep(false)} disabled={busy !== null || !runtime?.autoKeepTheme}>暂停自动保持</button>
          <button className="secondary-button" onClick={() => void refresh()} disabled={busy !== null}>立即重新检查</button>
        </div>
      </section>

      {!status?.appFound && (
        <section className="path-panel">
          <div><h2>未找到 WorkBuddy</h2><p>请选择 WorkBuddy.exe。应用会检查 resources/app.asar 后再保存此路径。</p></div>
          <button className="secondary-button" onClick={() => void chooseWorkBuddy()} disabled={busy !== null}>选择 WorkBuddy.exe</button>
        </section>
      )}

      <section className="theme-grid" aria-label="主题列表">
        {themes.map((theme) => {
          const selected = status?.currentThemeId === theme.id;
          return (
            <article className={`theme-card${selected ? " theme-card-current" : ""}`} key={theme.id}>
              <div className="preview-wrap">
                <img src={convertFileSrc(theme.previewPath)} alt={`${theme.name} 预览`} />
                {selected && <span className="current-badge">当前使用</span>}
              </div>
              <div className="theme-content">
                <div><h2>{theme.name}</h2><p>{theme.description}</p></div>
                <dl>
                  <div><dt>已验证 WorkBuddy</dt><dd>{theme.verifiedWorkBuddyVersion}</dd></div>
                  <div><dt>主题版本</dt><dd>{theme.themeVersion}</dd></div>
                </dl>
                <button className="primary-button" onClick={() => void applyTheme(theme)} disabled={busy !== null || !status?.appFound}>
                  {busy === theme.id ? "正在应用…" : selected ? "重新应用主题" : "应用主题"}
                </button>
              </div>
            </article>
          );
        })}
      </section>

      <footer className="footer-actions">
        <div><h2>恢复与日志</h2><p>{status?.message ?? "正在检测 WorkBuddy…"}</p></div>
        <div className="footer-buttons">
          <button className="danger-button" onClick={() => void restore()} disabled={busy !== null || !status?.appFound}>{busy === "restore" ? "正在恢复…" : "恢复 WorkBuddy 原版"}</button>
          <button className="secondary-button" onClick={() => void chooseWorkBuddy()} disabled={busy !== null}>更换 WorkBuddy 路径</button>
          <button className="secondary-button" onClick={() => void openLogs()} disabled={busy !== null}>打开日志目录</button>
        </div>
      </footer>

      {restartTheme && (
        <div className="modal-backdrop" role="presentation">
          <section className="confirm-dialog" role="dialog" aria-modal="true" aria-labelledby="restart-title">
            <h2 id="restart-title">需要重启 WorkBuddy</h2>
            <p>当前 WorkBuddy 未开放 CDP。继续会先正常关闭现有 WorkBuddy，再以仅绑定 127.0.0.1:9336 的方式重新启动并应用“{restartTheme.name}”。请先保存未完成的工作。</p>
            <div className="dialog-actions">
              <button className="secondary-button" onClick={() => setRestartTheme(null)} disabled={busy !== null}>取消</button>
              <button className="primary-button" onClick={() => void applyTheme(restartTheme, true)} disabled={busy !== null}>重启并应用</button>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}

function StatusItem({ label, value, tone = "neutral", compact = false }: { label: string; value: string; tone?: "good" | "warning" | "neutral"; compact?: boolean }) {
  return <div className={`status-item tone-${tone}${compact ? " status-compact" : ""}`}><span>{label}</span><strong title={value}>{value}</strong></div>;
}
