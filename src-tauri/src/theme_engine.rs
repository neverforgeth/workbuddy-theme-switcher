//! Pure theme compilation. Preview and injection use this exact stylesheet.
use super::*;
use image::{DynamicImage, ImageFormat, ImageReader};
use std::{collections::BTreeMap, io::Cursor};
use theme_model::StyleAdvice;

pub(crate) const TEMPLATE_VERSION: &str = "workbuddy-5.2.6-v1";
pub(crate) const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Palette {
    pub background: String,
    pub surface: String,
    pub surface_strong: String,
    pub sidebar: String,
    pub accent: String,
    pub accent_strong: String,
    pub on_accent: String,
    pub border: String,
    pub text: String,
    pub text_muted: String,
    pub dark: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContrastCheck {
    pub name: String,
    pub ratio: f32,
    pub required: f32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompiledTheme {
    pub css: String,
    pub palette: Palette,
    pub checks: Vec<ContrastCheck>,
    pub template_version: String,
    pub effective_opacity: u8,
    pub compile_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_design: Option<region_theme::DesignAdvice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<String>,
}

pub(crate) struct ImportedImage {
    pub hero: Vec<u8>,
    pub thumbnail: Vec<u8>,
    pub analysis: theme_compiler::ImageAnalysis,
    pub background: String,
    pub accent: String,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn import_image(filename: &str, bytes: &[u8]) -> AppResult<ImportedImage> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::new(
            "IMAGE_SIZE_INVALID",
            "图片须非空且不超过 10MB。",
        ));
    }
    let ext = source_image_extension(Path::new(filename), bytes)?;
    let format = match ext {
        "jpg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "webp" => ImageFormat::WebP,
        _ => unreachable!(),
    };
    let bad = || {
        AppError::new(
            "IMAGE_DECODE_FAILED",
            "图片损坏或无法解码，请选择有效的 JPG、PNG 或 WebP。",
        )
    };
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| bad())?;
    validate_dimensions(width, height)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| bad())?;
    // Flatten transparent pixels locally; JPEG derivatives strip EXIF and other source metadata.
    let mut rgba = decoded.to_rgba8();
    for pixel in rgba.pixels_mut() {
        let a = pixel[3] as f32 / 255.0;
        for channel in 0..3 {
            pixel[channel] = (pixel[channel] as f32 * a + 255.0 * (1.0 - a)).round() as u8;
        }
        pixel[3] = 255;
    }
    let decoded = DynamicImage::ImageRgba8(rgba);
    let hero = decoded.resize(1600, 1600, image::imageops::FilterType::Triangle);
    let thumbnail = hero.resize(640, 640, image::imageops::FilterType::Triangle);
    let (background, accent) = extract_colors(&hero);
    Ok(ImportedImage {
        hero: jpeg(&hero, 86)?,
        analysis: theme_compiler::analyze_image(&hero)?,
        thumbnail: jpeg(&thumbnail, 72)?,
        background: background.hex(),
        accent: accent.hex(),
        width,
        height,
    })
}

fn validate_dimensions(width: u32, height: u32) -> AppResult<()> {
    if width < 64 || height < 64 || u64::from(width) * u64::from(height) > 40_000_000 {
        return Err(AppError::new(
            "IMAGE_DIMENSIONS_INVALID",
            "图片至少 64×64 且不超过 4000 万像素。",
        ));
    }
    Ok(())
}

fn jpeg(image: &DynamicImage, quality: u8) -> AppResult<Vec<u8>> {
    let mut output = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality)
        .encode_image(&image.to_rgb8())
        .map_err(|_| AppError::new("IMAGE_ENCODE_FAILED", "无法生成本地图片资源。"))?;
    Ok(output)
}

fn extract_colors(image: &DynamicImage) -> (RgbColor, RgbColor) {
    let sample = image.thumbnail(96, 96).to_rgb8();
    let mut bins: BTreeMap<(u8, u8, u8), (u32, [u32; 3])> = BTreeMap::new();
    for p in sample.pixels() {
        let bucket = bins.entry((p[0] / 24, p[1] / 24, p[2] / 24)).or_default();
        bucket.0 += 1;
        for i in 0..3 {
            bucket.1[i] += p[i] as u32;
        }
    }
    let color = |item: &(u32, [u32; 3])| RgbColor {
        red: (item.1[0] / item.0) as u8,
        green: (item.1[1] / item.0) as u8,
        blue: (item.1[2] / item.0) as u8,
    };
    let dominant = color(
        bins.values()
            .max_by_key(|v| v.0)
            .expect("validated nonempty image"),
    );
    let mut accent = dominant;
    let mut best = -1.0f32;
    for value in bins.values() {
        let c = color(value);
        let max = c.red.max(c.green).max(c.blue) as f32;
        let min = c.red.min(c.green).min(c.blue) as f32;
        let score = ((max - min) / 255.0)
            * (value.0 as f32).sqrt()
            * (1.0 - (relative_luminance(c) - 0.35).abs());
        if score > best {
            best = score;
            accent = c;
        }
    }
    (dominant, accent)
}

fn rgb(value: &str) -> RgbColor {
    parse_hex_color(value).expect("validated color")
}
fn exact_black() -> RgbColor {
    RgbColor {
        red: 0,
        green: 0,
        blue: 0,
    }
}
fn exact_white() -> RgbColor {
    RgbColor {
        red: 255,
        green: 255,
        blue: 255,
    }
}
fn worst_contrast(text: RgbColor, surface: RgbColor, alpha: f32) -> f32 {
    [exact_black(), exact_white()]
        .into_iter()
        .map(|bg| contrast_ratio(text, mix(bg, surface, alpha)))
        .fold(f32::INFINITY, f32::min)
}
fn best_foreground(surface: RgbColor) -> RgbColor {
    if contrast_ratio(exact_black(), surface) >= contrast_ratio(exact_white(), surface) {
        exact_black()
    } else {
        exact_white()
    }
}

pub(crate) fn compile(
    background: &str,
    accent: &str,
    controls: &ThemeControls,
    advice: &StyleAdvice,
) -> AppResult<CompiledTheme> {
    let started = Instant::now();
    validate_controls(controls)?;
    if parse_hex_color(background).is_none() || parse_hex_color(accent).is_none() {
        return Err(AppError::new("PALETTE_INVALID", "主题配色无效。"));
    }
    for value in [
        &advice.background,
        &advice.surface,
        &advice.sidebar,
        &advice.accent,
    ]
    .into_iter()
    .flatten()
    {
        if parse_hex_color(value).is_none() {
            return Err(AppError::new("PALETTE_INVALID", "主题配色无效。"));
        }
    }
    let bg = adjust_brightness(
        rgb(advice.background.as_deref().unwrap_or(background)),
        controls.brightness,
    );
    let dark = relative_luminance(bg) < 0.38;
    let base = if dark { exact_black() } else { exact_white() };
    let surface = advice
        .surface
        .as_deref()
        .map(rgb)
        .unwrap_or_else(|| mix(bg, base, if dark { 0.76 } else { 0.91 }));
    let side = advice
        .sidebar
        .as_deref()
        .map(rgb)
        .unwrap_or_else(|| mix(surface, bg, 0.12));
    // One readable foreground across all text surfaces. Mixed light/dark advice is normalized.
    let text = best_foreground(surface);
    let side = if contrast_ratio(text, side) < 4.5 {
        surface
    } else {
        side
    };
    let strong = mix(
        surface,
        if text.red == 0 {
            exact_white()
        } else {
            exact_black()
        },
        0.6,
    );
    let mut alpha = controls.panel_opacity as f32 / 100.0;
    while alpha < 1.0
        && [surface, side, strong]
            .iter()
            .any(|s| worst_contrast(text, *s, alpha) < 4.5)
    {
        alpha = (alpha + 0.01).min(1.0);
    }
    let muted_candidate = mix(text, surface, 0.24);
    let muted = if [surface, side, strong]
        .iter()
        .all(|s| worst_contrast(muted_candidate, *s, alpha) >= 4.5)
    {
        muted_candidate
    } else {
        text
    };
    let ac = controls
        .accent
        .as_deref()
        .filter(|v| !v.is_empty())
        .or(advice.accent.as_deref())
        .map(rgb)
        .unwrap_or_else(|| rgb(accent));
    let ac = if controls.accent.is_none() && advice.mood.as_deref() == Some("calm") {
        mix(ac, surface, 0.15)
    } else {
        ac
    };
    let on_accent = best_foreground(ac);
    let accent_strong = if worst_contrast(ac, surface, alpha) >= 4.5 {
        ac
    } else {
        text
    };
    let border = mix(text, surface, 0.48);
    let mut selected = mix(
        surface,
        ac,
        if advice.mood.as_deref() == Some("vivid") {
            0.2
        } else {
            0.12
        },
    );
    if contrast_ratio(text, selected) < 4.5 {
        selected = surface;
    }
    let palette = Palette {
        background: bg.hex(),
        surface: surface.hex(),
        surface_strong: strong.hex(),
        sidebar: side.hex(),
        accent: ac.hex(),
        accent_strong: accent_strong.hex(),
        on_accent: on_accent.hex(),
        border: border.hex(),
        text: text.hex(),
        text_muted: muted.hex(),
        dark,
    };
    let checks = [
        ("正文 / 叠图面板", worst_contrast(text, surface, alpha)),
        ("辅助文字 / 叠图面板", worst_contrast(muted, surface, alpha)),
        ("侧栏文字 / 叠图背景", worst_contrast(text, side, alpha)),
        ("输入框及弹窗", worst_contrast(text, strong, alpha)),
        ("主要按钮文字", contrast_ratio(on_accent, ac)),
        ("悬停与选中", contrast_ratio(text, selected)),
    ]
    .into_iter()
    .map(|(name, ratio)| ContrastCheck {
        name: name.into(),
        ratio: (ratio * 100.0).floor() / 100.0,
        required: 4.5,
        status: if ratio >= 4.5 { "pass" } else { "warning" }.into(),
    })
    .collect();
    let tokens = [
        ("BACKGROUND", bg.css()),
        ("SURFACE", css_rgba(surface, alpha)),
        ("STRONG", css_rgba(strong, alpha)),
        ("SIDEBAR", css_rgba(side, alpha)),
        ("ACCENT", ac.css()),
        ("ON_ACCENT", on_accent.css()),
        ("ACCENT_TEXT", accent_strong.css()),
        ("TEXT", text.css()),
        ("MUTED", muted.css()),
        ("BORDER", border.css()),
        ("SELECTED", selected.css()),
        (
            "BRIGHTNESS",
            format!("{:.2}", 1.0 + controls.brightness as f32 / 100.0),
        ),
        ("BLUR", controls.blur.to_string()),
        ("SCHEME", if dark { "dark" } else { "light" }.into()),
    ];
    let mut css = include_str!("theme-template.css").to_string();
    css.push_str(include_str!("workbuddy-background-compat.css"));
    for (name, value) in tokens {
        css = css.replace(&format!("{{{{{name}}}}}"), &value);
    }
    if css.contains("{{") || css.contains("@import") || css.contains("url(") {
        return Err(AppError::new("THEME_COMPILE_FAILED", "主题样式模板无效。"));
    }
    Ok(CompiledTheme {
        css,
        palette,
        checks,
        template_version: TEMPLATE_VERSION.into(),
        effective_opacity: (alpha * 100.0).round() as u8,
        compile_ms: started.elapsed().as_millis(),
        effective_design: None,
        corrections: Vec::new(),
    })
}

pub(crate) fn package(
    runtime_id: &str,
    name: &str,
    compiled: &CompiledTheme,
    hero: &[u8],
) -> Value {
    json!({"format":"codedrobe-theme","schemaVersion":1,"theme":{"id":runtime_id,"displayName":name,"version":"1.4.0","catalog":{"name":{"zh":name,"en":name}}},"targets":{"workbuddy":{"css":compiled.css}},"assets":{"images":{"hero":{"filename":"hero.jpg","mimeType":"image/jpeg","base64":base64_encode(hero)}}}})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contradictory_ai_surfaces_are_repaired() {
        for background in ["#000000", "#FFFFFF"] {
            for surface in ["#000000", "#FFFFFF", "#777777"] {
                let advice = StyleAdvice {
                    background: Some(background.into()),
                    surface: Some(surface.into()),
                    sidebar: Some("#FF0000".into()),
                    ..StyleAdvice::default()
                };
                let compiled =
                    compile(background, "#FF0000", &ThemeControls::default(), &advice).unwrap();
                assert!(
                    compiled.checks.iter().all(|r| r.status == "pass"),
                    "{background} {surface}"
                );
            }
        }
    }
    pub(crate) fn test_png(color: [u8; 3]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(96, 96, image::Rgb(color)))
            .write_to(&mut buf, ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }
    #[test]
    fn rejects_fake_and_oversized_images() {
        assert!(import_image("x.jpg", b"\xff\xd8\xffbad").is_err());
        assert!(import_image("x.png", &vec![0; MAX_IMAGE_BYTES + 1]).is_err());
        assert!(validate_dimensions(10, 80).is_err());
        assert!(validate_dimensions(100_000, 100_000).is_err());
        assert!(import_image("x.jpg", &test_png([3, 5, 7])).is_err());
    }
    #[test]
    fn extracts_light_dark_and_saturated_images() {
        for color in [[250, 245, 235], [8, 10, 20], [245, 20, 80]] {
            let image = import_image("photo.png", &test_png(color)).unwrap();
            assert_eq!((image.width, image.height), (96, 96));
            assert!(image.hero.starts_with(&[255, 216, 255]));
            assert_eq!(rgb(&image.background).red, color[0]);
        }
    }
    #[test]
    fn all_controls_keep_text_readable_across_wallpaper_extremes() {
        for bg in ["#FFFFFF", "#000000", "#FFFF00", "#FF00CC", "#7A8899"] {
            for opacity in [55, 82, 96] {
                for brightness in [-40, 0, 40] {
                    let c = compile(
                        bg,
                        "#FFFF00",
                        &ThemeControls {
                            panel_opacity: opacity,
                            brightness,
                            ..ThemeControls::default()
                        },
                        &StyleAdvice::default(),
                    )
                    .unwrap();
                    assert!(c.checks.iter().all(|r| r.status == "pass"));
                    assert!(!c.css.contains("backdrop-filter"));
                    assert!(!c.css.contains("url("));
                    assert!(c.css.contains("--studio-on-accent"));
                }
            }
        }
    }
    #[test]
    fn preview_and_package_share_exact_css() {
        let c = compile(
            "#AACCBB",
            "#5F8345",
            &ThemeControls::default(),
            &StyleAdvice::default(),
        )
        .unwrap();
        let p = package("test", "Test", &c, b"jpeg");
        assert_eq!(
            p.pointer("/targets/workbuddy/css")
                .unwrap()
                .as_str()
                .unwrap(),
            c.css
        );
    }
    #[test]
    fn compiling_cached_palette_is_fast() {
        let mut times = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            compile(
                "#669988",
                "#FFCC00",
                &ThemeControls::default(),
                &StyleAdvice::default(),
            )
            .unwrap();
            times.push(start.elapsed());
        }
        times.sort();
        assert!(times[94] < Duration::from_millis(50));
    }
}
