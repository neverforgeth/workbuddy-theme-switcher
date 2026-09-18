import { useState } from "react";
import { errorText, type StudioApi } from "./studio-api";
import type { RuntimeSnapshot } from "./types";

export function SettingsPanel({
  api,
  snapshot,
  hidden,
  notify,
}: {
  api: StudioApi;
  snapshot: RuntimeSnapshot | null;
  hidden: boolean;
  notify: (message: string) => void;
}) {
  const [clearPrompt, setClearPrompt] = useState(false);
  const [clearing, setClearing] = useState(false);
  const choosePath = async () => {
    try {
      const path = await api.pickWorkBuddy();
      if (path && !Array.isArray(path)) {
        await api.setPath(path);
        notify("WorkBuddy 路径已验证并保存。");
      }
    } catch (error) {
      notify(errorText(error));
    }
  };
  return (
    <section hidden={hidden} className="settings-page">
      <div className="page-heading">
        <div>
          <span className="eyebrow">SETTINGS & DIAGNOSTICS</span>
          <h1>设置与诊断</h1>
          <p>本地运行，仅连接本机 WorkBuddy；不会发送图片或调用 AI。</p>
        </div>
      </div>
      <div className="settings-grid">
        <section className="panel">
          <h2>WorkBuddy 连接</h2>
          <dl className="diagnostics">
            <dt>路径</dt>
            <dd>{snapshot?.workbuddy.path || "未找到"}</dd>
            <dt>版本</dt>
            <dd>{snapshot?.workbuddy.version || "未检测"}</dd>
            <dt>连接状态</dt>
            <dd>{snapshot?.workbuddy.message || "正在检测…"}</dd>
            <dt>样式节点</dt>
            <dd>{snapshot?.workbuddy.styleNodeCount ?? "未检测"}</dd>
            <dt>自动保持</dt>
            <dd>{snapshot?.runtime.monitorStatus || "未检测"}</dd>
            <dt>恢复重试</dt>
            <dd>{snapshot?.runtime.retryCount ?? 0} / 3</dd>
            <dt>最近错误码</dt>
            <dd>{snapshot?.runtime.lastErrorCode || "无"}</dd>
            <dt>开机启动</dt>
            <dd>
              {snapshot?.runtime.loginAutostartEnabled ? "已开启" : "未开启"}
            </dd>
          </dl>
          <div className="button-row">
            <button
              disabled={!!snapshot?.trial}
              onClick={() => void choosePath()}
            >
              更换 WorkBuddy 路径
            </button>
            <button
              onClick={() =>
                void api.openLogs().catch((e) => notify(errorText(e)))
              }
            >
              打开日志目录
            </button>
          </div>
          <p className="footnote">
            仿真样板对应 5.2.6；主题应用前会检查当前页面结构。必要组件不匹配时会停止应用，不会通过关闭校验强行换肤。
          </p>
        </section>
        <section className="panel">
          <h2>历史凭据清理</h2>
          <p>
            本版不读取旧 API Key。若以前配置过
            AI，可选择清除本应用留下的凭据；不会触碰其他软件。
          </p>
          <button disabled={clearing} onClick={() => setClearPrompt(true)}>
            清除旧 AI 凭据
          </button>
          {clearPrompt && (
            <div role="alertdialog" aria-label="确认清除旧凭据">
              <p>
                仅删除 WorkBuddy 主题切换器的历史 AI
                密钥，删除后不能恢复。继续吗？
              </p>
              <div className="button-row">
                <button
                  disabled={clearing}
                  onClick={() => setClearPrompt(false)}
                >
                  取消
                </button>
                <button
                  disabled={clearing}
                  onClick={async () => {
                    setClearing(true);
                    try {
                      await api.clearLegacyAiCredential(true);
                      setClearPrompt(false);
                      notify("旧凭据已清除（或原本不存在）。");
                    } catch (error) {
                      notify(errorText(error));
                    } finally {
                      setClearing(false);
                    }
                  }}
                >
                  {clearing ? "清除中…" : "确认清除"}
                </button>
              </div>
            </div>
          )}
        </section>
      </div>
    </section>
  );
}
