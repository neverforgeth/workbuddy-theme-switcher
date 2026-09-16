//! Offline v3 compiler. This is the only owner of profile, overrides and rendering order.
//! Keep the legacy engine/region path unchanged: saved v1/v2 CSS is never recompiled on read.
use super::*;
use region_theme::{DesignAdvice, Region, RegionStyle, Shadow};
use std::collections::BTreeMap;
use theme_engine::{CompiledTheme, ContrastCheck};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StyleId {
    AiryLight,
    WarmPaper,
    CalmDark,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StyleSelection {
    pub id: StyleId,
    pub version: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ColorCandidate {
    pub color: String,
    pub area: f32,
    pub score: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImageAnalysis {
    pub version: u32,
    pub candidates: Vec<ColorCandidate>,
    pub luminance: [f32; 3],
    pub warmth: f32,
    pub samples: Vec<String>,
    pub recommended: StyleId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<SampleGrid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SampleGrid {
    pub width: u8,
    pub height: u8,
    pub mean: f32,
    pub deviation: f32,
    pub max_neighbor_delta: f32,
    pub saturation: f32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManualOverrides {
    pub regions: BTreeMap<Region, RegionStyle>,
    pub global: bool,
    pub position: bool,
}
fn rgb(s: &str) -> RgbColor {
    parse_hex_color(s).expect("validated compiler color")
}
fn foreground(bg: RgbColor) -> RgbColor {
    if contrast_ratio(rgb("#111820"), bg) >= 4.5 {
        rgb("#111820")
    } else if contrast_ratio(rgb("#F3F5F7"), bg) >= 4.5 {
        rgb("#F3F5F7")
    } else if contrast_ratio(rgb("#000000"), bg) > contrast_ratio(rgb("#FFFFFF"), bg) {
        rgb("#000000")
    } else {
        rgb("#FFFFFF")
    }
}
pub(crate) fn analyze(samples: Vec<String>) -> AppResult<ImageAnalysis> {
    if samples.is_empty()
        || samples.len() > 256
        || samples.iter().any(|s| parse_hex_color(s).is_none())
    {
        return Err(AppError::new("ANALYSIS_INVALID", "图片分析缓存无效。"));
    }
    let mut bins: BTreeMap<(u8, u8, u8), (usize, [u32; 3])> = BTreeMap::new();
    let mut lights = Vec::new();
    let mut warmth = 0.0;
    for s in &samples {
        let c = rgb(s);
        let b = bins
            .entry((c.red / 32, c.green / 32, c.blue / 32))
            .or_default();
        b.0 += 1;
        for (i, v) in [c.red, c.green, c.blue].iter().enumerate() {
            b.1[i] += u32::from(*v);
        }
        lights.push(relative_luminance(c));
        warmth += (f32::from(c.red) - f32::from(c.blue)) / 255.0;
    }
    lights.sort_by(f32::total_cmp);
    warmth /= samples.len() as f32;
    let mut candidates: Vec<ColorCandidate> = bins
        .values()
        .map(|(n, sum)| {
            let c = RgbColor {
                red: (sum[0] / *n as u32) as u8,
                green: (sum[1] / *n as u32) as u8,
                blue: (sum[2] / *n as u32) as u8,
            };
            let area = *n as f32 / samples.len() as f32;
            let saturation =
                f32::from(c.red.max(c.green).max(c.blue) - c.red.min(c.green).min(c.blue)) / 255.0;
            ColorCandidate {
                color: c.hex(),
                area,
                score: area.powf(0.7)
                    * (0.25 + saturation)
                    * (1.0 - (relative_luminance(c) - 0.4).abs()),
            }
        })
        .filter(|c| c.area >= 0.015)
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.color.cmp(&b.color))
    });
    candidates.truncate(8);
    // If a highly textured image has no large bucket, its mean is a stable fallback.
    if candidates.is_empty() {
        candidates.push(ColorCandidate {
            color: samples[samples.len() / 2].clone(),
            area: 1.0,
            score: 0.0,
        });
    }
    let luminance = [
        lights[lights.len() / 10],
        lights[lights.len() / 2],
        lights[lights.len() * 9 / 10],
    ];
    let recommended = if luminance[1] < 0.16 {
        StyleId::CalmDark
    } else if warmth > 0.055 {
        StyleId::WarmPaper
    } else {
        StyleId::AiryLight
    };
    Ok(ImageAnalysis {
        version: 1,
        candidates,
        luminance,
        warmth,
        samples,
        recommended,
        grid: None,
    })
}
/// Called once on the decoded controlled image, never from ordinary draft edits.
pub(crate) fn analyze_image(image: &image::DynamicImage) -> AppResult<ImageAnalysis> {
    let pixels = image
        .resize_exact(16, 16, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let samples: Vec<String> = pixels
        .pixels()
        .map(|p| {
            RgbColor {
                red: p[0],
                green: p[1],
                blue: p[2],
            }
            .hex()
        })
        .collect();
    let mut a = analyze(samples)?;
    let lights: Vec<f32> = a
        .samples
        .iter()
        .map(|s| relative_luminance(rgb(s)))
        .collect();
    let mean = lights.iter().sum::<f32>() / 256.0;
    let deviation = (lights.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / 256.0).sqrt();
    let mut delta: f32 = 0.0;
    for i in 0..256 {
        for j in [
            if i % 16 < 15 { Some(i + 1) } else { None },
            if i < 240 { Some(i + 16) } else { None },
        ]
        .into_iter()
        .flatten()
        {
            delta = delta.max((lights[i] - lights[j]).abs());
        }
    }
    let saturation = a
        .samples
        .iter()
        .map(|s| {
            let c = rgb(s);
            f32::from(c.red.max(c.green).max(c.blue) - c.red.min(c.green).min(c.blue)) / 255.0
        })
        .sum::<f32>()
        / 256.0;
    a.version = 2;
    a.grid = Some(SampleGrid {
        width: 16,
        height: 16,
        mean,
        deviation,
        max_neighbor_delta: delta,
        saturation,
    });
    Ok(a)
}
pub(crate) fn validate(style: &StyleSelection, a: &ImageAnalysis) -> AppResult<()> {
    if !matches!((style.version, a.version), (1, 1) | (2, 2)) {
        return Err(AppError::new(
            "STYLE_VERSION_UNSUPPORTED",
            "此风格或分析版本尚不支持编辑；已保存效果未被改变。",
        ));
    }
    if a.version == 2
        && (a.samples.len() != 256
            || a.grid.as_ref().is_none_or(|g| {
                g.width != 16
                    || g.height != 16
                    || [g.mean, g.deviation, g.max_neighbor_delta, g.saturation]
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            }))
    {
        return Err(AppError::new("ANALYSIS_INVALID", "新版图片采样缓存无效。"));
    }
    if a.candidates.is_empty()
        || a.candidates.len() > 8
        || a.candidates.iter().any(|c| {
            parse_hex_color(&c.color).is_none()
                || !c.area.is_finite()
                || !(0.0..=1.0).contains(&c.area)
                || !c.score.is_finite()
        })
        || a.luminance
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        || !a.warmth.is_finite()
        || a.warmth.abs() > 1.0
    {
        return Err(AppError::new("ANALYSIS_INVALID", "图片分析缓存无效。"));
    }
    if a.samples.is_empty()
        || a.samples.len() > 256
        || a.samples.iter().any(|s| parse_hex_color(s).is_none())
    {
        return Err(AppError::new("ANALYSIS_INVALID", "图片采样缓存无效。"));
    }
    Ok(())
}
#[cfg(test)]
pub(crate) fn controls(id: StyleId) -> ThemeControls {
    controls_for(&StyleSelection { id, version: 1 })
}
pub(crate) fn controls_for(style: &StyleSelection) -> ThemeControls {
    #[derive(Deserialize)]
    struct Profile {
        id: StyleId,
        controls: ThemeControls,
    }
    let profiles: Vec<Profile> = serde_json::from_str(if style.version == 1 {
        include_str!("../../shared/offline-styles-v1.json")
    } else {
        include_str!("../../shared/offline-styles.json")
    })
    .expect("versioned built-in profiles");
    profiles
        .into_iter()
        .find(|p| p.id == style.id)
        .expect("known profile")
        .controls
}

/// Return one compiled artifact and the requested region values; corrections are reported separately.
pub(crate) fn compile(
    style: &StyleSelection,
    analysis: &ImageAnalysis,
    controls: &ThemeControls,
    overrides: &ManualOverrides,
    position: Option<&DesignAdvice>,
) -> AppResult<(CompiledTheme, DesignAdvice)> {
    validate(style, analysis)?;
    if style.version == 2 {
        return theme_fusion::compile(style, analysis, controls, overrides, position);
    }
    let started = Instant::now();
    validate(style, analysis)?;
    validate_controls(controls)?;
    let dark = style.id == StyleId::CalmDark;
    let neutral = rgb(match style.id {
        StyleId::AiryLight => "#F7FAFC",
        StyleId::WarmPaper => "#FAF4E8",
        StyleId::CalmDark => "#20262E",
    });
    let raw = rgb(&analysis.candidates[0].color);
    let surface = mix(neutral, raw, if dark { 0.06 } else { 0.035 });
    let mut accent = controls.accent.as_deref().map(rgb).unwrap_or_else(|| {
        mix(
            raw,
            neutral,
            if style.id == StyleId::WarmPaper {
                0.22
            } else {
                0.08
            },
        )
    });
    if controls.accent.is_none() {
        let anchor = rgb(if dark {
            "#CADFED"
        } else if style.id == StyleId::WarmPaper {
            "#594432"
        } else {
            "#243E50"
        });
        for _ in 0..50 {
            if contrast_ratio(accent, surface) >= 3.0 {
                break;
            }
            accent = mix(accent, anchor, 0.1);
        }
    }
    let strong = mix(
        neutral,
        if dark { rgb("#151B23") } else { rgb("#FFFFFF") },
        0.55,
    );
    let sidebar = mix(neutral, raw, 0.055);
    let advice = theme_model::StyleAdvice {
        background: Some(if dark { "#18212B" } else { "#EDF2F6" }.into()),
        surface: Some(surface.hex()),
        sidebar: Some(sidebar.hex()),
        accent: Some(accent.hex()),
        mood: None,
    };
    let mut base = theme_engine::compile(&surface.hex(), &accent.hex(), controls, &advice)?;
    base.palette.dark = dark;
    base.palette.surface_strong = strong.hex();
    let mut design = region_theme::defaults(&base.palette);
    design.explanation =
        "离线风格 v1：图片轻染色、单层背景、分区可读性修正；动态实机场景仍需确认。".into();
    design.veil = if dark { 12 } else { 8 };
    if let Some(p) = position {
        design.background_x = p.background_x;
        design.background_y = p.background_y;
        design.veil = p.veil;
    }
    for (r, s) in &mut design.regions {
        let bg = match r {
            Region::Main => surface,
            Region::Sidebar => sidebar,
            Region::Menu | Region::Dialog => strong,
            Region::PrimaryButton => accent,
            Region::UserMessage => mix(surface, accent, 0.12),
            Region::SecondaryButton => mix(surface, accent, 0.04),
            _ => surface,
        };
        s.background = bg.hex();
        s.text = foreground(bg).hex();
        s.muted = mix(foreground(bg), bg, 0.2).hex();
        s.hover = mix(
            bg,
            accent,
            if *r == Region::PrimaryButton {
                0.0
            } else {
                0.13
            },
        )
        .hex();
        if *r == Region::PrimaryButton {
            s.hover = mix(bg, foreground(bg), 0.08).hex();
        }
        s.selected = mix(bg, accent, 0.24).hex();
        s.focus_ring = foreground(bg).hex();
        s.border = mix(bg, foreground(bg), 0.32).hex();
        s.opacity = match r {
            Region::Main => 12,
            Region::Sidebar => 96,
            Region::Composer => controls.panel_opacity,
            Region::PrimaryButton => 100,
            Region::Menu | Region::Dialog => 98,
            Region::AssistantMessage => 90,
            Region::SecondaryButton => 94,
            _ => 92,
        };
        if let Some(custom) = overrides.regions.get(r) {
            *s = custom.clone();
        }
    }
    region_theme::validate(&design)?;
    base.checks.clear();
    let mut effective = design.clone();
    let mut corrections = Vec::new();
    let veil = if dark { rgb("#000000") } else { rgb("#FFFFFF") };
    // CSS brightness multiplies encoded RGB channels; cover both ends of the gradient.
    let mut underlays: Vec<RgbColor> = analysis
        .samples
        .iter()
        .flat_map(|s| {
            let factor = 1.0 + f32::from(controls.brightness) / 100.0;
            [
                f32::from(design.veil) / 100.0,
                (f32::from(design.veil) + 10.0) / 100.0,
            ]
            .map(|a| {
                let c = mix(rgb(s), veil, a);
                RgbColor {
                    red: (f32::from(c.red) * factor).round().clamp(0.0, 255.0) as u8,
                    green: (f32::from(c.green) * factor).round().clamp(0.0, 255.0) as u8,
                    blue: (f32::from(c.blue) * factor).round().clamp(0.0, 255.0) as u8,
                }
            })
        })
        .collect();
    // Include neutral panel endpoints for nested and portalled controls; no assumption about crop position.
    underlays.extend([surface, strong, sidebar]);
    for r in [
        Region::Main,
        Region::Sidebar,
        Region::Composer,
        Region::UserMessage,
        Region::AssistantMessage,
        Region::Menu,
        Region::Dialog,
        Region::SecondaryButton,
        Region::PrimaryButton,
    ] {
        let s = effective.regions.get_mut(&r).unwrap();
        let bg = rgb(&s.background);
        let mut alpha = f32::from(s.opacity) / 100.0;
        if !matches!(r, Region::Main | Region::PrimaryButton | Region::Composer)
            && !overrides.regions.contains_key(&r)
        {
            alpha = (alpha + (f32::from(controls.panel_opacity) - 82.0) / 200.0).clamp(0.0, 1.0);
        }
        let worst = |fg: RgbColor, a: f32| {
            underlays
                .iter()
                .map(|under| contrast_ratio(fg, mix(*under, bg, a)))
                .fold(f32::INFINITY, f32::min)
        };
        let requested = s.clone();
        let mut text = rgb(&s.text);
        if worst(text, alpha) < 4.5 {
            text = foreground(bg);
            while alpha < 1.0 && worst(text, alpha) < 4.5 {
                alpha = (alpha + 0.01).min(1.0);
            }
        }
        s.text = text.hex();
        s.opacity = (alpha * 100.0).round() as u8;
        if worst(rgb(&s.muted), alpha) < 4.5 {
            s.muted = s.text.clone();
        }
        if worst(rgb(&s.focus_ring), alpha) < 3.0 {
            s.focus_ring = s.text.clone();
        }
        // Borders are required for the composer and both button types, decorative elsewhere.
        if matches!(r, Region::Composer | Region::PrimaryButton)
            && worst(rgb(&s.border), alpha) < 3.0
        {
            s.border = s.text.clone();
        }
        if s.opacity != requested.opacity
            || s.text != requested.text
            || s.muted != requested.muted
            || s.border != requested.border
            || s.focus_ring != requested.focus_ring
        {
            corrections.push(format!("{}：生效不透明度 {}%，正文 {}、辅助文字 {}、边框 {}、聚焦 {}；按叠图抽样修正可读性。",label(r),s.opacity,s.text,s.muted,s.border,s.focus_ring));
        }
        let selector = format!("html.codedrobe-host-workbuddy :is({})", r.selector());
        let shadow = match s.shadow {
            Shadow::None => "none",
            Shadow::Soft => "0 3px 12px #00000012",
            Shadow::Raised => "0 10px 28px #00000030",
        };
        base.css+=&format!("\n{selector}{{--region-text:{};--region-muted:{};background:{}!important;color:{}!important;border-color:{}!important;box-shadow:{shadow}!important}}",s.text,s.muted,css_rgba(bg,alpha),s.text,s.border);
        // Inherit into known text nodes only; do not recolor status indicators or SVGs wholesale.
        base.css+=&format!("\n{selector} :is(p,h1,h2,h3,label,.text-primary){{color:var(--region-text)!important}}\n{selector} :is(small,.text-secondary,[class*='placeholder']){{color:var(--region-muted)!important}}\n{selector}:focus-visible{{outline:2px solid {}!important;outline-offset:2px!important}}",s.focus_ring);
        if matches!(r, Region::PrimaryButton | Region::SecondaryButton) {
            for (state, c) in [
                (
                    ":hover:not(:disabled):not([aria-disabled='true'])",
                    &s.hover,
                ),
                (
                    ":is([aria-selected='true'],[aria-pressed='true'])",
                    &s.selected,
                ),
            ] {
                let fg = foreground(rgb(c));
                base.css+=&format!("\n{selector}{state}{{--region-text:{};background:{}!important;color:{}!important}}",fg.hex(),c,fg.hex());
                base.checks.push(check(
                    format!(
                        "{} / {}正文",
                        label(r),
                        if state.contains("hover") {
                            "悬停"
                        } else {
                            "选中"
                        }
                    ),
                    contrast_ratio(fg, rgb(c)),
                    4.5,
                ));
            }
            base.css+=&format!("\n{selector}:is(:disabled,[aria-disabled='true']){{opacity:1!important;background:{}!important;color:{}!important;cursor:not-allowed}}",css_rgba(bg,alpha),s.muted);
        }
        if r == Region::Composer {
            base.css+=&format!("\n{selector} :is(input,textarea,[contenteditable='true']){{background:transparent!important;color:{}!important;caret-color:{}!important}}\n{selector} :is(input,textarea)::placeholder{{color:{}!important;opacity:1}}",s.text,s.focus_ring,s.muted);
        }
        if matches!(r, Region::UserMessage | Region::AssistantMessage) {
            base.css+=&format!("\n{selector} :is(pre,code,th,td){{background:{}!important;color:{}!important;border-color:{}!important}}",css_rgba(bg,alpha),s.text,s.border);
        }
        base.checks.push(check(
            format!("{} / 叠图抽样正文", label(r)),
            worst(text, alpha),
            4.5,
        ));
        base.checks.push(check(
            format!("{} / 辅助及占位文字", label(r)),
            worst(rgb(&s.muted), alpha),
            4.5,
        ));
        base.checks.push(check(
            format!("{} / 聚焦提示", label(r)),
            worst(rgb(&s.focus_ring), alpha),
            3.0,
        ));
        if matches!(r, Region::Composer | Region::PrimaryButton) {
            base.checks.push(check(
                format!("{} / 控件边界", label(r)),
                worst(rgb(&s.border), alpha),
                3.0,
            ));
        }
    }
    base.css+=&format!("\nhtml.codedrobe-host-workbuddy .teams-container::before{{background-position:{}% {}%!important;box-shadow:none!important;background-image:linear-gradient(180deg,{},{}) ,var(--codedrobe-image-hero)!important}}",design.background_x,design.background_y,css_rgba(veil,f32::from(design.veil)/100.0),css_rgba(veil,(f32::from(design.veil)+10.0)/100.0));
    base.checks.push(ContrastCheck {
        name: "动态状态、状态提示色与实机继承尚未验证".into(),
        ratio: 0.0,
        required: 4.5,
        status: "unverified".into(),
    });
    base.effective_opacity = effective.regions[&Region::Composer].opacity;
    base.palette.text = effective.regions[&Region::Main].text.clone();
    base.palette.text_muted = effective.regions[&Region::Main].muted.clone();
    base.palette.accent = effective.regions[&Region::PrimaryButton].background.clone();
    base.palette.on_accent = effective.regions[&Region::PrimaryButton].text.clone();
    base.template_version = "workbuddy-5.2.6-offline-v3.2".into();
    base.effective_design = Some(effective);
    base.corrections = corrections;
    base.compile_ms = started.elapsed().as_millis();
    Ok((base, design))
}
/// Repair only the known 1.6.0 adapter bug, never rerun palette analysis or alter saved revisions.
pub(crate) fn repair_draft_background(doc: &mut theme_library::ThemeDocument) -> AppResult<bool> {
    if doc.schema_version != 3 || doc.compiled.template_version != "workbuddy-5.2.6-offline-v3.1" {
        return Ok(false);
    }
    let next = doc
        .edit_sequence
        .checked_add(1)
        .ok_or_else(|| AppError::new("THEME_STORE_ERROR", "草稿序号溢出，未修改原稿。"))?;
    doc.compiled.css = doc.compiled.css.replace(
        ".workbuddy-topbar,#workbuddy-menubar-container,.wb-home-page",
        ".workbuddy-topbar,:where(#workbuddy-menubar-container),.wb-home-page",
    );
    let patch = include_str!("workbuddy-background-compat.css");
    if !doc.compiled.css.contains(patch) {
        doc.compiled.css.push_str(patch);
    }
    doc.compiled.template_version = "workbuddy-5.2.6-offline-v3.2".into();
    doc.compiled.corrections.push(
        "已修复 1.6.0 背景遮挡和面板优先级；保留原图与参数，需重新预览并保存，旧修订未覆盖。"
            .into(),
    );
    doc.edit_sequence = next;
    doc.saved_sequence = None;
    Ok(true)
}
fn check(name: String, ratio: f32, required: f32) -> ContrastCheck {
    ContrastCheck {
        name,
        ratio: (ratio * 100.0).floor() / 100.0,
        required,
        status: if ratio >= required { "pass" } else { "warning" }.into(),
    }
}
fn label(r: Region) -> &'static str {
    match r {
        Region::Main => "主背景",
        Region::Sidebar => "侧栏",
        Region::Composer => "输入框",
        Region::UserMessage => "用户消息",
        Region::AssistantMessage => "助手消息",
        Region::PrimaryButton => "主要按钮",
        Region::SecondaryButton => "次要按钮",
        Region::Menu => "菜单",
        Region::Dialog => "弹窗",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_buttons_stay_distinct_on_neutral_images() {
        for color in ["#FFFFFF", "#222222", "#BBBBBB"] {
            for id in [StyleId::AiryLight, StyleId::WarmPaper, StyleId::CalmDark] {
                let a = analyze(vec![color.into(); 256]).unwrap();
                let (compiled, _) = compile(
                    &StyleSelection { id, version: 1 },
                    &a,
                    &controls(id),
                    &ManualOverrides::default(),
                    None,
                )
                .unwrap();
                assert!(
                    contrast_ratio(
                        rgb(&compiled.palette.accent),
                        rgb(&compiled.palette.surface)
                    ) >= 3.0
                );
            }
        }
    }
    #[test]
    fn recommends_once_and_ignores_tiny_saturated_accents() {
        for (color, id) in [
            ("#F0F8FF", StyleId::AiryLight),
            ("#F7DDBB", StyleId::WarmPaper),
            ("#101820", StyleId::CalmDark),
        ] {
            let mut samples = vec![color.into(); 255];
            samples.push("#FF0000".into());
            let a = analyze(samples).unwrap();
            assert_eq!(a.recommended, id);
            assert_eq!(a.candidates[0].color, color);
        }
    }
    #[test]
    fn every_profile_is_deterministic_bounded_and_reports_real_corrections() {
        let a = analyze(
            ["#000000", "#FFFFFF", "#FF0088", "#0077CC"]
                .repeat(32)
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
        .unwrap();
        for id in [StyleId::AiryLight, StyleId::WarmPaper, StyleId::CalmDark] {
            let style = StyleSelection { id, version: 1 };
            for brightness in [-40, 0, 40] {
                for opacity in [55, 82, 96] {
                    let c = ThemeControls {
                        brightness,
                        panel_opacity: opacity,
                        ..controls(id)
                    };
                    let (out, _) =
                        compile(&style, &a, &c, &ManualOverrides::default(), None).unwrap();
                    let (again, _) =
                        compile(&style, &a, &c, &ManualOverrides::default(), None).unwrap();
                    assert_eq!(out.css, again.css);
                    assert!(
                        out.checks
                            .iter()
                            .all(|v| v.status == "unverified" || v.ratio >= v.required),
                        "{:?}",
                        out.checks
                    );
                    assert!(out.checks.iter().any(|v| v.status == "unverified"));
                    assert!(out.checks.iter().any(|v| v.required == 3.0));
                    assert!(!out.corrections.is_empty());
                    assert_eq!(out.css.matches("filter: brightness").count(), 1);
                    assert!(!out.css.contains("backdrop-filter"));
                    for forbidden in [
                        "display:none",
                        "height:",
                        "overflow:",
                        "url(",
                        "@import",
                        "pointer-events: auto",
                    ] {
                        assert!(!out.css.contains(forbidden));
                    }
                }
            }
            assert!(compile(
                &StyleSelection { id, version: 99 },
                &a,
                &controls(id),
                &ManualOverrides::default(),
                None
            )
            .is_err());
        }
    }
}

pub(crate) fn edit(
    mut doc: theme_library::ThemeDocument,
    update: theme_library::DraftUpdate,
) -> AppResult<theme_library::ThemeDocument> {
    validate_controls(&update.controls)?;
    if let Some(d) = &update.design {
        region_theme::validate(d)?;
    }
    let switch = update
        .style
        .as_ref()
        .is_some_and(|s| Some(s) != doc.style.as_ref())
        || update.reset_style;
    if switch {
        let style = update
            .style
            .clone()
            .or(doc.style.clone())
            .ok_or_else(|| AppError::new("STYLE_REQUIRED", "请先选择离线风格。"))?;
        if doc
            .style
            .as_ref()
            .is_some_and(|old| old.version != style.version)
        {
            return Err(AppError::new(
                "STYLE_COPY_REQUIRED",
                "请创建新版融合副本，原稿和旧修订不会自动升级。",
            ));
        }
        if doc.analysis.is_none() {
            doc.analysis = Some(analyze(doc.background_samples.clone())?);
        }
        validate(&style, doc.analysis.as_ref().unwrap())?;
        doc.schema_version = 3;
        // v2 receives one complete edit intent. The UI has already reset to style defaults;
        // keep any slider input made in the same debounce window instead of discarding it.
        doc.controls = if style.version == 2 {
            update.controls.clone()
        } else {
            controls_for(&style)
        };
        doc.style = Some(style);
        doc.manual_overrides = ManualOverrides::default();
        if let Some(d) = &update.design {
            doc.design = Some(d.clone());
        }
        if let Some(d) = &mut doc.design {
            d.veil = if doc.style.as_ref().unwrap().version == 2 {
                0
            } else if doc.style.as_ref().unwrap().id == StyleId::CalmDark {
                12
            } else {
                8
            };
        }
    } else {
        doc.controls = update.controls;
        if doc.schema_version == 3 {
            if let Some(d) = &update.design {
                for (r, s) in &d.regions {
                    if doc.design.as_ref().is_none_or(|old| {
                        serde_json::to_value(&old.regions[r]).unwrap()
                            != serde_json::to_value(s).unwrap()
                    }) {
                        doc.manual_overrides.regions.insert(*r, s.clone());
                    }
                }
                doc.design = Some(d.clone());
            }
        } else if let Some(d) = update.design {
            doc.design = Some(d);
            doc.schema_version = 2;
        }
    }
    if doc.schema_version == 3 {
        let style = doc
            .style
            .as_ref()
            .ok_or_else(|| AppError::new("STYLE_REQUIRED", "缺少风格版本。"))?;
        let a = doc
            .analysis
            .as_ref()
            .ok_or_else(|| AppError::new("ANALYSIS_INVALID", "缺少图片分析缓存。"))?;
        let (compiled, design) = compile(
            style,
            a,
            &doc.controls,
            &doc.manual_overrides,
            doc.design.as_ref(),
        )?;
        doc.manual_overrides.global = serde_json::to_value(&doc.controls).unwrap()
            != serde_json::to_value(controls_for(style)).unwrap();
        doc.manual_overrides.position = design.background_x != 50
            || design.background_y != 50
            || design.veil
                != if style.version == 2 {
                    0
                } else if style.id == StyleId::CalmDark {
                    12
                } else {
                    8
                };
        doc.compiled = compiled;
        doc.design = Some(design);
    } else {
        let base = theme_engine::compile(
            &doc.background,
            &doc.extracted_accent,
            &doc.controls,
            &doc.advice,
        )?;
        doc.compiled = if let Some(d) = &doc.design {
            region_theme::compile_controls(base, d, &doc.background_samples, &doc.controls)?
        } else {
            base
        };
    }
    doc.name = update.name;
    doc.edit_sequence = update.sequence;
    Ok(doc)
}
