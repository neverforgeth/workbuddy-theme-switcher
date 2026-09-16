import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { previewDocument, type PreviewScene } from "./preview-fixture";
import type { ThemePreview } from "./types";

export function WorkBuddyThemePreview({
  preview,
  imageUrl,
  updating = false,
}: {
  preview: ThemePreview;
  imageUrl: string;
  updating?: boolean;
}) {
  const [scene, setScene] = useState<PreviewScene>("home");
  const [fullSize, setFullSize] = useState(false);
  const frame = useRef<HTMLIFrameElement>(null);
  // Only structural scene/image changes navigate the iframe. Parameter updates replace one style
  // node in place, preserving focus, scroll, sample input and the already decoded background.
  const source = useMemo(
    () => previewDocument("", imageUrl, scene),
    [imageUrl, scene],
  );
  const updateStyle = useCallback(() => {
    const style = frame.current?.contentDocument?.getElementById(
      "codedrobe-theme-style-workbuddy",
    );
    if (style) style.textContent = preview.css;
  }, [preview.css]);
  useLayoutEffect(updateStyle, [updateStyle, source]);
  return (
    <section className="theme-preview" aria-label="主题效果预览">
      <div className="preview-heading">
        <div>
          <span className="eyebrow">仿真预览 · WorkBuddy 5.2.6 样板</span>
          <h2>{preview.name}</h2>
        </div>
        <span className="preview-state" role="status">
          {updating ? "正在更新…" : "与主题包共用样式"}
        </span>
      </div>
      <div className="preview-toolbar">
        <div className="segmented" role="tablist" aria-label="预览场景">
          {(
            [
              ["home", "工作台"],
              ["chat", "对话"],
              ["dialog", "弹窗与菜单"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              role="tab"
              aria-selected={scene === id}
              onClick={() => setScene(id)}
            >
              {label}
            </button>
          ))}
        </div>
        <button className="text-button" onClick={() => setFullSize(!fullSize)}>
          {fullSize ? "适应宽度" : "100% 缩放"}
        </button>
      </div>
      <div className="preview-viewport">
        <iframe
          ref={frame}
          onLoad={updateStyle}
          title="WorkBuddy 仿真效果"
          sandbox="allow-same-origin"
          referrerPolicy="no-referrer"
          srcDoc={source}
          className={fullSize ? "preview-frame full-size" : "preview-frame"}
        />
      </div>
      {preview.compiled ? (
        <details className="readability">
          <summary>
            可读性检查 ·{" "}
            {preview.compiled.checks.filter((c) => c.status === "pass").length}/
            {preview.compiled.checks.length} 项通过
          </summary>
          <div className="palette-row">
            {Object.entries(preview.compiled.palette)
              .filter(
                (pair): pair is [string, string] => typeof pair[1] === "string",
              )
              .map(([name, color]) => (
                <span key={name} title={name + " " + color}>
                  <i style={{ background: color }} />
                  <code>{color}</code>
                </span>
              ))}
          </div>
          <ul>
            {preview.compiled.corrections?.map((correction) => (
              <li key={correction}>{correction}</li>
            ))}
            {preview.compiled.checks.map((check) => (
              <li key={check.name}>
                {check.name}
                <span className={check.status === "pass" ? "good" : "warning"}>
                  {check.status === "unverified"
                    ? "未验证"
                    : `${check.ratio.toFixed(2)} : 1 / ≥${check.required}`}
                </span>
              </li>
            ))}
          </ul>
          <p>
            按模板版本采用图片采样与面板叠加或黑白极值校验；这不等于实机全部通过，动态状态仍需验证。
          </p>
        </details>
      ) : (
        <p className="preview-note">
          旧主题未提供结构化可读性数据，未验证；实机效果以 WorkBuddy 为准。
        </p>
      )}
    </section>
  );
}
