//! Style v2 paint compiler. Frozen v1 lives in theme_compiler; no storage or image decoding here.
use super::*;
use region_theme::{DesignAdvice, Region, RegionStyle, Shadow};
use std::collections::BTreeMap;
use theme_compiler::{ImageAnalysis, ManualOverrides, StyleId, StyleSelection};
use theme_engine::{CompiledTheme, ContrastCheck, Palette};

fn rgb(s: &str) -> RgbColor {
    parse_hex_color(s).expect("validated fusion color")
}
fn worst(fg: RgbColor, backgrounds: &[RgbColor]) -> f32 {
    backgrounds
        .iter()
        .map(|bg| contrast_ratio(fg, *bg))
        .fold(f32::INFINITY, f32::min)
}
fn composite(under: &[RgbColor], s: &RegionStyle) -> Vec<RgbColor> {
    under
        .iter()
        .map(|bg| mix(*bg, rgb(&s.background), f32::from(s.opacity) / 100.0))
        .collect()
}
fn readable(fg: RgbColor, bg: &[RgbColor], target: f32) -> RgbColor {
    if worst(fg, bg) >= target {
        return fg;
    }
    let black = rgb("#101820");
    let white = rgb("#F7FAFC");
    let anchor = if worst(black, bg) > worst(white, bg) {
        rgb("#000000")
    } else {
        rgb("#FFFFFF")
    };
    for step in 1..=20 {
        let candidate = mix(fg, anchor, step as f32 / 20.0);
        if worst(candidate, bg) >= target {
            return candidate;
        }
    }
    anchor
}
fn label(r: Region) -> &'static str {
    match r {
        Region::Main => "主画布",
        Region::Sidebar => "侧栏",
        Region::Composer => "输入框",
        Region::AssistantMessage => "助手消息",
        Region::UserMessage => "用户消息",
        Region::PrimaryButton => "主按钮",
        Region::SecondaryButton => "次按钮",
        Region::Menu => "菜单",
        Region::Dialog => "弹窗",
    }
}
fn key(r: Region) -> &'static str {
    match r {
        Region::Main => "main",
        Region::Sidebar => "sidebar",
        Region::Composer => "composer",
        Region::AssistantMessage => "assistant",
        Region::UserMessage => "user",
        Region::PrimaryButton => "primary",
        Region::SecondaryButton => "secondary",
        Region::Menu => "menu",
        Region::Dialog => "dialog",
    }
}
fn selector(r: Region) -> &'static str {
    match r {
    // The wrapper owns the canvas, so the top bar and chat share one continuous veil.
    Region::Main=>".teams-content-wrapper",
    Region::PrimaryButton=>"button[type='submit'],.cb-button--primary,[class*='_iconBtnSend_'],.t-button--theme-primary,.button-primary,[data-studio-primary],.chat-container section [class*='_icon_'][class*='_large_'],.wb-home-composer__input-slot>section [class*='_icon_'][class*='_large_']",
    _=>r.selector(),
}
}
fn check(name: String, ratio: f32, required: f32) -> ContrastCheck {
    ContrastCheck {
        name,
        ratio: (ratio * 100.0).floor() / 100.0,
        required,
        status: if ratio >= required { "pass" } else { "warning" }.into(),
    }
}

pub(crate) fn compile(
    style: &StyleSelection,
    a: &ImageAnalysis,
    controls: &ThemeControls,
    overrides: &ManualOverrides,
    position: Option<&DesignAdvice>,
) -> AppResult<(CompiledTheme, DesignAdvice)> {
    let start = Instant::now();
    theme_compiler::validate(style, a)?;
    validate_controls(controls)?;
    let dark = style.id == StyleId::CalmDark;
    let neutral = rgb(match style.id {
        StyleId::AiryLight => "#F3F8FA",
        StyleId::WarmPaper => "#F7EFDF",
        StyleId::CalmDark => "#20262D",
    });
    let raw = rgb(&a.candidates[0].color);
    let saturation = a.grid.as_ref().unwrap().saturation;
    // Keep a visible family resemblance without letting saturated pixels tint every surface.
    let tint = (0.15 - saturation * 0.09).clamp(0.05, 0.15);
    let surface = mix(neutral, raw, tint);
    let sidebar = mix(neutral, raw, tint * 0.65);
    let strong = mix(neutral, rgb(if dark { "#171D24" } else { "#FFFFFF" }), 0.7);
    let fg = mix(rgb(if dark { "#F0F3F5" } else { "#20313D" }), raw, 0.06);
    let accent = controls.accent.as_deref().map(rgb).unwrap_or_else(|| {
        readable(
            mix(
                raw,
                neutral,
                if style.id == StyleId::WarmPaper {
                    0.28
                } else {
                    0.12
                },
            ),
            &[surface],
            3.0,
        )
    });
    let palette = Palette {
        background: neutral.hex(),
        surface: surface.hex(),
        surface_strong: strong.hex(),
        sidebar: sidebar.hex(),
        accent: accent.hex(),
        accent_strong: accent.hex(),
        on_accent: readable(fg, &[accent], 4.5).hex(),
        border: mix(surface, fg, 0.22).hex(),
        text: fg.hex(),
        text_muted: mix(fg, surface, 0.18).hex(),
        dark,
    };
    let mut requested = region_theme::defaults(&palette);
    requested.veil = 0;
    requested.explanation =
        "图片融合 v2：清晰背景、分层叠色；每个组件仅一个底色所有者。裁切和动态实机场景待验证。"
            .into();
    if let Some(p) = position {
        requested.background_x = p.background_x;
        requested.background_y = p.background_y;
        requested.veil = p.veil;
    }
    let opacities = match style.id {
        StyleId::AiryLight => [28, 72, 12, 36, 62],
        StyleId::WarmPaper => [40, 82, 18, 44, 76],
        StyleId::CalmDark => [34, 80, 18, 40, 74],
    };
    let delta = i16::from(controls.panel_opacity)
        - i16::from(theme_compiler::controls_for(style).panel_opacity);
    for (r, s) in &mut requested.regions {
        let bg = match r {
            Region::Sidebar => sidebar,
            Region::Main => surface,
            Region::PrimaryButton => accent,
            Region::Menu | Region::Dialog => strong,
            Region::UserMessage => mix(surface, accent, 0.12),
            Region::Composer => mix(surface, strong, 0.35),
            _ => surface,
        };
        s.background = bg.hex();
        s.text = fg.hex();
        s.muted = mix(fg, bg, 0.17).hex();
        s.border = mix(bg, fg, 0.2).hex();
        s.focus_ring = accent.hex();
        s.hover = mix(bg, accent, 0.14).hex();
        s.selected = mix(bg, accent, 0.25).hex();
        let alpha = match r {
            Region::Main => opacities[0],
            Region::Sidebar => opacities[1],
            Region::AssistantMessage => opacities[2],
            Region::UserMessage => opacities[3],
            Region::Composer => opacities[4],
            Region::PrimaryButton => 100,
            Region::Menu | Region::Dialog => 98,
            _ => 12,
        };
        s.opacity = if matches!(r, Region::PrimaryButton | Region::Menu | Region::Dialog) {
            alpha
        } else {
            (i16::from(alpha) + delta).clamp(0, 100) as u8
        };
        s.shadow = if matches!(r, Region::Menu | Region::Dialog) {
            Shadow::Raised
        } else {
            Shadow::None
        };
        if let Some(custom) = overrides.regions.get(r) {
            *s = custom.clone();
        }
    }
    region_theme::validate(&requested)?;
    let factor = 1.0 + f32::from(controls.brightness) / 100.0;
    let veil = rgb(if dark { "#000000" } else { "#FFFFFF" });
    let image: Vec<RgbColor> = a
        .samples
        .iter()
        .map(|s| {
            let c = rgb(s);
            let lit = RgbColor {
                red: (f32::from(c.red) * factor).round().clamp(0.0, 255.0) as u8,
                green: (f32::from(c.green) * factor).round().clamp(0.0, 255.0) as u8,
                blue: (f32::from(c.blue) * factor).round().clamp(0.0, 255.0) as u8,
            };
            mix(lit, veil, f32::from(requested.veil) / 100.0)
        })
        .collect();
    let mut effective = requested.clone();
    let mut layers: BTreeMap<Region, Vec<RgbColor>> = BTreeMap::new();
    let mut checks = Vec::new();
    let mut corrections = Vec::new();
    for r in [
        Region::Main,
        Region::Sidebar,
        Region::Composer,
        Region::AssistantMessage,
        Region::UserMessage,
        Region::Menu,
        Region::Dialog,
        Region::SecondaryButton,
        Region::PrimaryButton,
    ] {
        let under = match r {
            Region::Main | Region::Sidebar => image.clone(),
            Region::Menu | Region::Dialog => {
                let mut all = image.clone();
                all.extend([rgb("#000000"), rgb("#FFFFFF")]);
                all
            }
            Region::PrimaryButton | Region::SecondaryButton => {
                layers.values().flatten().copied().collect()
            }
            _ => layers[&Region::Main].clone(),
        };
        let s = effective.regions.get_mut(&r).unwrap();
        let before = s.clone();
        let mut backgrounds = composite(&under, s);
        // Correct foreground first, then owned surface, then only this layer's opacity.
        // Keep the polarity appropriate for the OWNED surface. Picking black/white from
        // the wallpaper can choose black for a dark panel, then force it fully opaque.
        let anchor = if contrast_ratio(rgb("#000000"), rgb(&s.background))
            > contrast_ratio(rgb("#FFFFFF"), rgb(&s.background))
        {
            rgb("#000000")
        } else {
            rgb("#FFFFFF")
        };
        for step in 0..=20 {
            s.text = mix(rgb(&before.text), anchor, step as f32 / 20.0).hex();
            if worst(rgb(&s.text), &backgrounds) >= 4.5 {
                break;
            }
        }
        if worst(rgb(&s.text), &backgrounds) < 4.5 {
            let anchor = rgb(if relative_luminance(rgb(&s.text)) > 0.5 {
                "#101820"
            } else {
                "#FFFFFF"
            });
            let initial = rgb(&s.background);
            for step in 1..=5 {
                s.background = mix(initial, anchor, step as f32 * 0.05).hex();
                backgrounds = composite(&under, s);
                if worst(rgb(&s.text), &backgrounds) >= 4.5 {
                    break;
                }
            }
            while s.opacity < 100 && worst(rgb(&s.text), &backgrounds) < 4.5 {
                s.opacity += 1;
                backgrounds = composite(&under, s);
            }
        }
        // Secondary text approaches the corrected body color, never flips to the opposite
        // polarity simply because both black and white happen to pass on middle gray.
        for step in 0..=20 {
            s.muted = mix(rgb(&before.muted), rgb(&s.text), step as f32 / 20.0).hex();
            if worst(rgb(&s.muted), &backgrounds) >= 4.5 {
                break;
            }
        }
        s.focus_ring = readable(rgb(&s.focus_ring), &backgrounds, 3.0).hex();
        if s.opacity != before.opacity
            || s.background != before.background
            || s.text != before.text
            || s.muted != before.muted
            || s.focus_ring != before.focus_ring
        {
            corrections.push(format!("{}：按{}叠色校验，生效不透明度 {}%（初始 {}%），底色 {}，正文 {}，辅助文字 {}，聚焦 {}；先修正前景，再修正本层底色及不透明度。",label(r),if matches!(r,Region::Main|Region::Sidebar|Region::Menu|Region::Dialog){"背景 → 本层"}else{"背景 → 画布/所属面板 → 本层"},s.opacity,before.opacity,s.background,s.text,s.muted,s.focus_ring));
        }
        for (name, c, target) in [
            ("正文", &s.text, 4.5),
            ("辅助/占位文字", &s.muted, 4.5),
            ("聚焦指示", &s.focus_ring, 3.0),
        ] {
            checks.push(check(
                format!("{} / 分层抽样{}", label(r), name),
                worst(rgb(c), &backgrounds),
                target,
            ));
        }
        layers.insert(r, backgrounds);
    }
    // Markdown details are another real layer. Its tint must remain readable even when
    // code is nested in a quote/table, rather than repeating the translucent message paint.
    let detail_text = rgb(&effective.regions[&Region::AssistantMessage].text);
    let detail_anchor = if contrast_ratio(detail_text, rgb("#FFFFFF"))
        > contrast_ratio(detail_text, rgb("#000000"))
    {
        rgb("#FFFFFF")
    } else {
        rgb("#000000")
    };
    let mut detail_bg = strong;
    for step in 0..=20 {
        detail_bg = mix(strong, detail_anchor, step as f32 / 20.0);
        if contrast_ratio(detail_text, detail_bg) >= 4.5 {
            break;
        }
    }
    let detail_layers: Vec<_> = layers[&Region::AssistantMessage]
        .iter()
        .map(|c| mix(*c, detail_bg, 0.18))
        .collect();
    checks.push(check(
        "表格 / 代码 / 引用正文（消息内叠色）".into(),
        worst(detail_text, &detail_layers),
        4.5,
    ));
    if detail_bg.hex() != strong.hex() {
        corrections.push(format!(
            "表格、代码与引用：细节底色调整为 {}、不透明度 18%，避免重复叠色降低正文对比度。",
            detail_bg.hex()
        ));
    }
    let mut css=String::from("/* WorkBuddy fusion style v2 — paint only; one surface owner per component. */\n:root.codedrobe-host-workbuddy{");
    css += &format!("--fusion-detail-bg:{};", css_rgba(detail_bg, 0.18));
    for r in Region::ALL {
        let s = &effective.regions[&r];
        let k = key(r);
        css+=&format!("--fusion-{k}-bg:{};--fusion-{k}-text:{};--fusion-{k}-muted:{};--fusion-{k}-edge:{};--fusion-{k}-focus:{};--fusion-{k}-hover:{};--fusion-{k}-selected:{};",css_rgba(rgb(&s.background),f32::from(s.opacity)/100.0),s.text,s.muted,css_rgba(rgb(&s.border),0.35),s.focus_ring,s.hover,s.selected);
    }
    css+=&format!("--fusion-backdrop:{};--fusion-veil:{};--fusion-brightness:{factor};--fusion-blur:{}px;--fusion-position:{}% {}%;color-scheme:{};}}\n",palette.background,css_rgba(veil,f32::from(requested.veil)/100.0),controls.blur,requested.background_x,requested.background_y,if dark {"dark"} else {"light"});
    // The shared template controls the stacking contract, never dimensions or scroll behavior.
    css += include_str!("theme-fusion.css");
    for r in [
        Region::Main,
        Region::Sidebar,
        Region::Composer,
        Region::AssistantMessage,
        Region::UserMessage,
        Region::Menu,
        Region::Dialog,
        Region::SecondaryButton,
        Region::PrimaryButton,
    ] {
        let s = &effective.regions[&r];
        let k = key(r);
        let sel = format!("html.codedrobe-host-workbuddy :is({})", selector(r));
        css+=&format!("\n{sel}{{--fusion-text:var(--fusion-{k}-text);--fusion-muted:var(--fusion-{k}-muted);--fusion-focus:var(--fusion-{k}-focus);background:var(--fusion-{k}-bg)!important;color:var(--fusion-text)!important;border-color:transparent!important;box-shadow:inset 0 0 0 1px var(--fusion-{k}-edge)!important;}}");
        match s.shadow {
            Shadow::Soft => css+=&format!("\n{sel}{{box-shadow:inset 0 0 0 1px var(--fusion-{k}-edge),0 3px 12px {}!important}}",css_rgba(rgb(&palette.text),0.07)),
            Shadow::Raised => css+=&format!("\n{sel}{{box-shadow:inset 0 0 0 1px var(--fusion-{k}-edge),0 8px 24px {}!important}}",css_rgba(rgb(&palette.text),0.12)),
            Shadow::None => {},
        }
        if matches!(r, Region::Main | Region::Sidebar) && matches!(s.shadow, Shadow::None) {
            css += &format!("\n{sel}{{box-shadow:none!important}}");
        }
        if r == Region::PrimaryButton {
            css += &format!("\n{sel}:focus-visible{{outline-offset:-3px!important}}");
        }
        if matches!(
            r,
            Region::PrimaryButton | Region::SecondaryButton | Region::Sidebar | Region::Menu
        ) {
            let state_sel = if r == Region::Sidebar {
                format!("{sel} :is(button,.conversation-agent-card,[role='button'])")
            } else if r == Region::Menu {
                format!("{sel} :is([role='menuitem'],[role='option'])")
            } else {
                sel.clone()
            };
            for (state,color) in [(":hover:not(:disabled):not([aria-disabled='true']):not([class*='_disabled_'])",&s.hover),(":is([aria-selected='true'],[aria-pressed='true'],[aria-current='page'],[class*='_selected_'],.active)",&s.selected)] {
                let text=readable(rgb(&s.text),&[rgb(color)],4.5).hex();
                css+=&format!("\n{state_sel}{state}{{--fusion-text:{text};background:{color}!important;color:{text}!important;}}");
                checks.push(check(format!("{} / {}文字",label(r),if state.starts_with(":hover") {"悬停"}else{"选中"}),contrast_ratio(rgb(&text),rgb(color)),4.5));
            }
            css+=&format!("\n{state_sel}:is(:disabled,[aria-disabled='true'],[class*='_disabled_']){{opacity:1!important;color:var(--fusion-{k}-muted)!important;}}");
        }
    }
    // Explicit ownership exceptions must come after regional paint rules.
    css += include_str!("theme-fusion-inner.css");
    checks.push(ContrastCheck {
        name: "图片裁切、未采样纹理、状态提示与动态实机场景待验证".into(),
        ratio: 0.0,
        required: 4.5,
        status: "unverified".into(),
    });
    let mut palette = palette;
    palette.text = effective.regions[&Region::Main].text.clone();
    palette.text_muted = effective.regions[&Region::Main].muted.clone();
    palette.accent = effective.regions[&Region::PrimaryButton].background.clone();
    palette.on_accent = effective.regions[&Region::PrimaryButton].text.clone();
    let out = CompiledTheme {
        css,
        palette,
        checks,
        template_version: "workbuddy-5.2.6-fusion-v2.0".into(),
        effective_opacity: effective.regions[&Region::Composer].opacity,
        compile_ms: start.elapsed().as_millis(),
        effective_design: Some(effective),
        corrections,
    };
    Ok((out, requested))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layered_contrast_matches_composited_canvas_and_never_darkens_decorative_edges() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 64, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgb([255, 240, 190])
            } else {
                image::Rgb([0, 45, 80])
            }
        }));
        let analysis = theme_compiler::analyze_image(&image).unwrap();
        for id in [StyleId::AiryLight, StyleId::WarmPaper, StyleId::CalmDark] {
            let style = StyleSelection { id, version: 2 };
            let controls = theme_compiler::controls_for(&style);
            assert_eq!(controls.blur, 0);
            let (out, requested) = compile(
                &style,
                &analysis,
                &controls,
                &ManualOverrides::default(),
                None,
            )
            .unwrap();
            let effective = out.effective_design.as_ref().unwrap();
            assert_eq!(
                effective.regions[&Region::AssistantMessage].border,
                requested.regions[&Region::AssistantMessage].border
            );
            let factor = 1.0 + f32::from(controls.brightness) / 100.0;
            let pixels: Vec<_> = analysis
                .samples
                .iter()
                .map(|s| {
                    let c = rgb(s);
                    RgbColor {
                        red: (f32::from(c.red) * factor).round().clamp(0.0, 255.0) as u8,
                        green: (f32::from(c.green) * factor).round().clamp(0.0, 255.0) as u8,
                        blue: (f32::from(c.blue) * factor).round().clamp(0.0, 255.0) as u8,
                    }
                })
                .collect();
            let canvas = composite(&pixels, &effective.regions[&Region::Main]);
            let message = composite(&canvas, &effective.regions[&Region::AssistantMessage]);
            let measured = worst(
                rgb(&effective.regions[&Region::AssistantMessage].text),
                &message,
            );
            let check = out
                .checks
                .iter()
                .find(|c| c.name == "助手消息 / 分层抽样正文")
                .unwrap();
            assert!((check.ratio - measured).abs() < 0.011);
            assert!(
                out.checks
                    .iter()
                    .all(|c| c.status == "unverified" || c.ratio >= c.required),
                "{:?}",
                out.checks
            );
            assert_eq!(
                effective.regions[&Region::AssistantMessage].opacity,
                requested.regions[&Region::AssistantMessage].opacity
            );
            let mut changed = controls.clone();
            changed.panel_opacity -= 10;
            let (_, less) = compile(
                &style,
                &analysis,
                &changed,
                &ManualOverrides::default(),
                None,
            )
            .unwrap();
            assert_eq!(
                requested.regions[&Region::Sidebar].opacity
                    - less.regions[&Region::Sidebar].opacity,
                10
            );
            assert_eq!(
                requested.regions[&Region::Composer].opacity
                    - less.regions[&Region::Composer].opacity,
                10
            );
            assert_ne!(
                less.regions[&Region::Composer].opacity,
                less.regions[&Region::Sidebar].opacity
            );
        }
    }
    #[test]
    fn fixed_grid_analysis_is_deterministic_and_rejects_invalid_v2_caches() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(320, 64, |x, _| {
            if x < 160 {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([255, 255, 255])
            }
        }));
        let a = theme_compiler::analyze_image(&image).unwrap();
        let style = StyleSelection {
            id: StyleId::AiryLight,
            version: 2,
        };
        assert_eq!(a.samples.len(), 256);
        let grid = a.grid.as_ref().unwrap();
        assert!(grid.deviation > 0.4 && grid.max_neighbor_delta > 0.5);
        let (first, _) = compile(
            &style,
            &a,
            &theme_compiler::controls_for(&style),
            &ManualOverrides::default(),
            None,
        )
        .unwrap();
        let (second, _) = compile(
            &style,
            &a,
            &theme_compiler::controls_for(&style),
            &ManualOverrides::default(),
            None,
        )
        .unwrap();
        assert_eq!(first.css, second.css);
        let mut bad = a.clone();
        bad.samples.pop();
        assert!(theme_compiler::validate(&style, &bad).is_err());
        bad = a.clone();
        bad.grid.as_mut().unwrap().deviation = f32::NAN;
        assert!(theme_compiler::validate(&style, &bad).is_err());
        let legacy = theme_compiler::analyze(a.samples).unwrap();
        assert!(theme_compiler::validate(&style, &legacy).is_err());
    }
}
