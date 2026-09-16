//! Versioned, bounded regional styling. Neither stored data nor the frontend can supply selectors or CSS.
use super::*;
use std::collections::BTreeMap;
use theme_engine::{CompiledTheme, ContrastCheck};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Region {
    Sidebar,
    Main,
    Composer,
    UserMessage,
    AssistantMessage,
    PrimaryButton,
    SecondaryButton,
    Menu,
    Dialog,
}
impl Region {
    pub const ALL: [Self; 9] = [
        Self::Sidebar,
        Self::Main,
        Self::Composer,
        Self::UserMessage,
        Self::AssistantMessage,
        Self::PrimaryButton,
        Self::SecondaryButton,
        Self::Menu,
        Self::Dialog,
    ];
    pub(crate) fn selector(self) -> &'static str {
        match self {
        Self::Sidebar => ".conversation-sidebar",
        Self::Main => ".wb-home-page,.main-content--chat,.expert-center-page,.skills-view,.connector-panel,.claw-workspace",
        Self::Composer => ".wb-home-composer__input-slot>section,.chat-container:not(.chat-container--welcome) section",
        Self::UserMessage => "[id^='user-message-'] [class*='_userMessageBubble_']",
        Self::AssistantMessage => ".cb-assistant-message",
        Self::PrimaryButton => "button[type='submit'],.cb-button--primary,[class*='_iconBtnSend_'],.t-button--theme-primary,.button-primary,[data-studio-primary]",
        Self::SecondaryButton => "button,[role='button'],[role='tab']",
        Self::Menu => "[role='menu'],[role='listbox'],[role='tooltip']",
        Self::Dialog => "[role='dialog']",
    }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RegionStyle {
    pub background: String,
    pub text: String,
    pub muted: String,
    pub border: String,
    pub hover: String,
    pub selected: String,
    pub focus_ring: String,
    pub opacity: u8,
    pub shadow: Shadow,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Shadow {
    None,
    Soft,
    Raised,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesignAdvice {
    pub regions: BTreeMap<Region, RegionStyle>,
    pub background_x: u8,
    pub background_y: u8,
    pub veil: u8,
    pub explanation: String,
}
fn color(s: &str) -> RgbColor {
    parse_hex_color(s).expect("validated design color")
}
fn black() -> RgbColor {
    color("#000000")
}
fn white() -> RgbColor {
    color("#FFFFFF")
}
fn foreground(bg: RgbColor) -> RgbColor {
    if contrast_ratio(black(), bg) >= contrast_ratio(white(), bg) {
        black()
    } else {
        white()
    }
}
pub(crate) fn sample_colors(hero: &[u8]) -> AppResult<Vec<String>> {
    let image = image::load_from_memory(hero)
        .map_err(|_| AppError::new("IMAGE_DECODE_FAILED", "缓存图片无法读取。"))?
        .thumbnail(12, 12)
        .to_rgb8();
    Ok(image
        .pixels()
        .map(|p| {
            RgbColor {
                red: p[0],
                green: p[1],
                blue: p[2],
            }
            .hex()
        })
        .collect())
}
pub(crate) fn defaults(p: &theme_engine::Palette) -> DesignAdvice {
    let mut regions = BTreeMap::new();
    for region in Region::ALL {
        let background = match region {
            Region::PrimaryButton => p.accent.clone(),
            Region::Sidebar => p.sidebar.clone(),
            _ => p.surface.clone(),
        };
        let text = foreground(color(&background)).hex();
        regions.insert(
            region,
            RegionStyle {
                background: background.clone(),
                text: text.clone(),
                muted: text,
                border: p.border.clone(),
                hover: mix(color(&background), color(&p.accent), 0.12).hex(),
                selected: mix(color(&background), color(&p.accent), 0.2).hex(),
                focus_ring: p.accent_strong.clone(),
                opacity: match region {
                    Region::Main => 12,
                    Region::Sidebar => 92,
                    Region::Composer => 78,
                    Region::PrimaryButton => 100,
                    Region::Dialog | Region::Menu => 98,
                    _ => 88,
                },
                shadow: if matches!(region, Region::Dialog | Region::Menu) {
                    Shadow::Raised
                } else {
                    Shadow::None
                },
            },
        );
    }
    DesignAdvice {
        regions,
        background_x: 50,
        background_y: 50,
        veil: 8,
        explanation: "本地分区方案：保留主背景、侧栏清晰、输入框透景。".into(),
    }
}
pub(crate) fn validate(d: &DesignAdvice) -> AppResult<()> {
    let invalid = || {
        AppError::new(
            "DESIGN_INVALID",
            "分区设计包含无效颜色、范围或区域，已保留当前方案。",
        )
    };
    if d.regions.len() != 9
        || Region::ALL.iter().any(|r| !d.regions.contains_key(r))
        || d.background_x > 100
        || d.background_y > 100
        || d.veil > 80
        || d.explanation.chars().count() > 800
    {
        return Err(invalid());
    }
    for s in d.regions.values() {
        if s.opacity > 100
            || [
                &s.background,
                &s.text,
                &s.muted,
                &s.border,
                &s.hover,
                &s.selected,
                &s.focus_ring,
            ]
            .iter()
            .any(|s| parse_hex_color(s).is_none())
        {
            return Err(invalid());
        }
    }
    Ok(())
}
pub(crate) fn compile_controls(
    base: CompiledTheme,
    d: &DesignAdvice,
    samples: &[String],
    controls: &ThemeControls,
) -> AppResult<CompiledTheme> {
    let mut effective = d.clone();
    if let Some(accent) = &controls.accent {
        if parse_hex_color(accent).is_none() {
            return Err(AppError::new("DESIGN_INVALID", "强调色无效。"));
        }
        let primary = effective
            .regions
            .get_mut(&Region::PrimaryButton)
            .ok_or_else(|| AppError::new("DESIGN_INVALID", "缺少主要按钮区域。"))?;
        primary.background = accent.clone();
        primary.text = foreground(color(accent)).hex();
        primary.hover = mix(color(accent), foreground(color(accent)), 0.08).hex();
    }
    let samples: Vec<String> = samples
        .iter()
        .filter_map(|s| parse_hex_color(s))
        .map(|c| adjust_brightness(c, controls.brightness).hex())
        .collect();
    compile(
        base,
        &effective,
        &samples,
        i16::from(controls.panel_opacity) - 82,
    )
}
pub(crate) fn compile(
    mut base: CompiledTheme,
    d: &DesignAdvice,
    samples: &[String],
    opacity_delta: i16,
) -> AppResult<CompiledTheme> {
    validate(d)?;
    let samples: Vec<RgbColor> = samples.iter().filter_map(|s| parse_hex_color(s)).collect();
    let samples = if samples.is_empty() {
        vec![black(), white()]
    } else {
        samples
    };
    base.checks.clear();
    // Generic button rules first, role-specific buttons last, so primary controls retain emphasis.
    let order = [
        Region::SecondaryButton,
        Region::Main,
        Region::Sidebar,
        Region::Composer,
        Region::UserMessage,
        Region::AssistantMessage,
        Region::Menu,
        Region::Dialog,
        Region::PrimaryButton,
    ];
    for region in order {
        let s = &d.regions[&region];
        let surface = color(&s.background);
        let mut alpha = (i16::from(s.opacity) + opacity_delta).clamp(0, 100) as f32 / 100.0;
        let underlays: Vec<RgbColor> = samples
            .iter()
            .map(|bg| mix(*bg, white(), f32::from(d.veil) / 100.0))
            .collect();
        // Include both wallpaper and configured parent panels. These are conservative samples,
        // not a claim that every inherited or dynamic WorkBuddy pixel has been verified.
        let mut underlays = underlays;
        let parents: &[Region] = match region {
            Region::Sidebar | Region::Main => &[],
            Region::PrimaryButton | Region::SecondaryButton => &[
                Region::Sidebar,
                Region::Main,
                Region::Composer,
                Region::UserMessage,
                Region::AssistantMessage,
                Region::Menu,
                Region::Dialog,
            ],
            _ => &[Region::Main],
        };
        for parent in parents {
            let s = &d.regions[&parent];
            let a = (i16::from(s.opacity) + opacity_delta).clamp(0, 100) as f32 / 100.0;
            underlays.extend(samples.iter().map(|bg| mix(*bg, color(&s.background), a)));
        }
        let worst = |fg: RgbColor, a: f32| {
            underlays
                .iter()
                .map(|bg| contrast_ratio(fg, mix(*bg, surface, a)))
                .fold(f32::INFINITY, f32::min)
        };
        let mut text = color(&s.text);
        if worst(text, alpha) < 4.5 && !matches!(region, Region::Main) {
            let candidate = foreground(surface);
            if worst(candidate, alpha) > worst(text, alpha) {
                text = candidate;
            }
            while alpha < 1.0 && worst(text, alpha) < 4.5 {
                alpha = (alpha + 0.01).min(1.0);
            }
        }
        let muted = if worst(color(&s.muted), alpha) >= 4.5 {
            color(&s.muted)
        } else {
            text
        };
        let hover = color(&s.hover);
        let selected = color(&s.selected);
        let shadow = match s.shadow {
            Shadow::None => "none",
            Shadow::Soft => "0 3px 12px #00000012",
            Shadow::Raised => "0 10px 28px #00000026",
        };
        let selector = format!("html.codedrobe-host-workbuddy :is({})", region.selector());
        base.css+=&format!("\n{selector}{{background:{}!important;color:{}!important;border-color:{}!important;box-shadow:{shadow}!important}}\n{selector} :is(small,[class*='placeholder']){{color:{}!important}}\n{selector}:hover{{background-color:{}!important;color:{}!important}}\n{selector}:is([aria-selected='true'],[aria-pressed='true']){{background-color:{}!important;color:{}!important}}\n{selector}:focus-visible{{outline:2px solid {}!important;outline-offset:2px}}\n{selector}:disabled{{opacity:.6;}}",
            css_rgba(surface,alpha),text.css(),s.border,muted.css(),
            if matches!(region,Region::PrimaryButton|Region::SecondaryButton) {hover.css()} else {css_rgba(surface,alpha)},
            if matches!(region,Region::PrimaryButton|Region::SecondaryButton) {foreground(hover).css()} else {text.css()},selected.css(),foreground(selected).css(),
            if contrast_ratio(color(&s.focus_ring),surface)>=3.0 {s.focus_ring.clone()} else {foreground(surface).hex()});
        if matches!(region, Region::Composer) {
            base.css+=&format!("\n{selector} :is(input,textarea,[contenteditable='true']){{background:transparent!important;color:{}!important;caret-color:{}!important}}\n{selector} :is(input,textarea)::placeholder{{color:{}!important}}",text.css(),text.css(),muted.css());
        }
        base.checks.push(ContrastCheck {
            name: format!("{region:?} / 选中状态正文"),
            ratio: contrast_ratio(foreground(selected), selected),
            required: 4.5,
            status: "pass".into(),
        });
        if matches!(region, Region::PrimaryButton | Region::SecondaryButton) {
            base.checks.push(ContrastCheck {
                name: format!("{region:?} / 悬停状态正文"),
                ratio: contrast_ratio(foreground(hover), hover),
                required: 4.5,
                status: "pass".into(),
            });
        }
        base.checks.push(ContrastCheck {
            name: format!("{region:?} / 叠图抽样正文"),
            ratio: (worst(text, alpha) * 100.0).floor() / 100.0,
            required: 4.5,
            status: if worst(text, alpha) >= 4.5 {
                "pass"
            } else {
                "warning"
            }
            .into(),
        });
        base.checks.push(ContrastCheck {
            name: format!("{region:?} / 实机继承及未覆盖状态"),
            ratio: 0.0,
            required: 4.5,
            status: "unverified".into(),
        });
    }
    // Background positioning is decorative only; never alter component geometry.
    base.css+=&format!("\nhtml.codedrobe-host-workbuddy .teams-container::before{{background-position:{}% {}%!important;box-shadow:inset 0 0 0 100vmax rgba(255,255,255,{:.2})}}",d.background_x,d.background_y,f32::from(d.veil)/100.0);
    base.template_version = "workbuddy-5.2.6-regions-v2".into();
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_compile_rejects_invalid_design_and_corrects_low_contrast() {
        let base = theme_engine::compile(
            "#AABBCC",
            "#336699",
            &ThemeControls::default(),
            &theme_model::StyleAdvice::default(),
        )
        .unwrap();
        let mut d = defaults(&base.palette);
        d.regions.get_mut(&Region::Composer).unwrap().text = "#FFFFFF".into();
        let out = compile(base.clone(), &d, &["#FFFFFF".into(), "#000000".into()], 0).unwrap();
        assert!(out
            .checks
            .iter()
            .filter(|c| c.status == "pass")
            .all(|c| c.ratio >= 4.5));
        assert!(!out.css.contains("backdrop-filter"));
        assert!(!out.css.contains("url("));
        d.regions.get_mut(&Region::Sidebar).unwrap().background = "url(https://bad)".into();
        assert!(compile(base, &d, &[], 0).is_err());
    }
}
