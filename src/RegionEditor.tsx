import { useState } from "react";
import type { DesignAdvice, Region, RegionStyle } from "./types";
export const regionNames: Record<Region, string> = {
  sidebar: "侧栏",
  main: "主背景",
  composer: "输入框",
  userMessage: "用户消息",
  assistantMessage: "助手消息",
  primaryButton: "主要按钮",
  secondaryButton: "次要按钮",
  menu: "菜单",
  dialog: "弹窗",
};
export function RegionEditor({
  design,
  disabled,
  onChange,
  onReset,
  effective,
}: {
  design: DesignAdvice;
  disabled: boolean;
  onChange: (d: DesignAdvice) => void;
  onReset?: () => void;
  effective?: DesignAdvice | null;
}) {
  const [region, setRegion] = useState<Region>("composer");
  const value = design;
  const change = (patch: Partial<RegionStyle>) => {
    onChange({
      ...value,
      regions: {
        ...value.regions,
        [region]: { ...value.regions[region], ...patch },
      },
    });
  };
  const backdrop = (patch: Partial<DesignAdvice>) => {
    onChange({ ...value, ...patch });
  };
  return (
    <details className="region-editor">
      <summary>高级调整 · 背景定位与九区配色</summary>
      <fieldset disabled={disabled}>
        {onReset && <button onClick={onReset}>恢复当前风格默认值</button>}
        <label>
          编辑区域
          <select
            value={region}
            onChange={(e) => setRegion(e.target.value as Region)}
          >
            {Object.entries(regionNames).map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        {(
          [
            ["background", "区域底色"],
            ["text", "正文颜色"],
            ["muted", "辅助文字"],
            ["border", "边框"],
            ["hover", "悬停"],
            ["selected", "选中"],
            ["focusRing", "聚焦提示"],
          ] as const
        ).map(([key, label]) => (
          <label className="region-color" key={key}>
            {label}
            <input
              aria-label={regionNames[region] + label}
              type="color"
              value={value.regions[region][key]}
              onChange={(e) => change({ [key]: e.target.value })}
            />
          </label>
        ))}
        <label>
          区域不透明度 {value.regions[region].opacity}%
          <input
            aria-label="区域不透明度"
            type="range"
            min={0}
            max={100}
            value={value.regions[region].opacity}
            onChange={(e) => change({ opacity: Number(e.target.value) })}
          />
        </label>
        <label>
          阴影
          <select
            value={value.regions[region].shadow}
            onChange={(e) =>
              change({ shadow: e.target.value as RegionStyle["shadow"] })
            }
          >
            <option value="none">无</option>
            <option value="soft">柔和</option>
            <option value="raised">浮层</option>
          </select>
        </label>
        {(
          [
            ["backgroundX", "背景水平位置", 100],
            ["backgroundY", "背景垂直位置", 100],
            ["veil", "背景柔光遮罩", 80],
          ] as const
        ).map(([key, label, max]) => (
          <label key={key}>
            {label} {value[key]}%
            <input
              aria-label={label}
              type="range"
              min={0}
              max={max}
              value={value[key]}
              onChange={(e) => backdrop({ [key]: Number(e.target.value) })}
            />
          </label>
        ))}
        {effective && (
          <p className="footnote">
            当前区域实际生效：不透明度 {effective.regions[region].opacity}
            %，正文 {effective.regions[region].text}，辅助文字{" "}
            {effective.regions[region].muted}。
          </p>
        )}
        <p className="footnote">
          参数只影响外观；可读性修正可能提高当前区域的遮盖度。主背景保留透景，警告项目需实机确认。
        </p>
      </fieldset>
    </details>
  );
}
