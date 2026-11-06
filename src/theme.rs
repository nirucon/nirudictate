//! NIRUX and Omarchy appearance resolution with a standalone NIRUDICTATE fallback.
//!
//! `auto` follows NIRUX inside the NIRU session, otherwise the current Omarchy
//! theme when one exists (see `omarchy.rs`), otherwise NIRUX's configuration.
//!
//! The precedence intentionally follows NIRUX: a custom user theme file wins;
//! otherwise a compiled-in builtin theme owns the base palette and the versioned
//! appearance contract can override semantic colours. Invalid appearance data
//! is ignored, while a malformed custom theme is reported without replacing the
//! working appearance.
use crate::config::config_home;
use crate::{omarchy, session};
use egui::Color32;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// The appearance choices the theme depends on.
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    /// `auto`, `nirux`, `omarchy`, `classic` or a NIRUX theme name.
    pub theme: String,
    pub scale: f32,
    pub reduced_motion: Option<bool>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "auto".into(),
            scale: 1.0,
            reduced_motion: None,
        }
    }
}

const BUILTINS: &[BuiltinTheme] = &[
    BuiltinTheme {
        name: "niru-noir",
        label: "Niru Noir",
        background: "0f0f10",
        foreground: "e5e5e5",
        muted: "a8a8a8",
        border: "2a2a2d",
        focus: "5a5a60",
    },
    BuiltinTheme {
        name: "niru-nord",
        label: "Niru Nord",
        background: "2e3440",
        foreground: "eceff4",
        muted: "9aa5b1",
        border: "3b4252",
        focus: "88c0d0",
    },
    BuiltinTheme {
        name: "niru-satie",
        label: "Niru Satie",
        background: "141414",
        foreground: "dedad4",
        muted: "8f8b86",
        border: "2a2724",
        focus: "b08d7d",
    },
    BuiltinTheme {
        name: "niru-carl-larsson",
        label: "Niru Carl Larsson",
        background: "f3ece0",
        foreground: "3b3228",
        muted: "7d6f5f",
        border: "d8cdbb",
        focus: "a63d2f",
    },
    BuiltinTheme {
        name: "niru-forest-vigil",
        label: "Niru Forest Vigil",
        background: "0b100c",
        foreground: "d8ded2",
        muted: "96a292",
        border: "2b362c",
        focus: "8aa87e",
    },
    BuiltinTheme {
        name: "niru-blood-of-urdr",
        label: "Niru Blood of Urdr",
        background: "120808",
        foreground: "e8d6cf",
        muted: "b09a92",
        border: "3a1d1b",
        focus: "c96050",
    },
    BuiltinTheme {
        name: "niru-root-and-stone",
        label: "Niru Root and Stone",
        background: "17100b",
        foreground: "eadfd2",
        muted: "b5a08c",
        border: "3a2a1e",
        focus: "c19768",
    },
    BuiltinTheme {
        name: "niru-blue-horizon",
        label: "Niru Blue Horizon",
        background: "0b1821",
        foreground: "dce6eb",
        muted: "93a6b2",
        border: "243846",
        focus: "86aec8",
    },
    BuiltinTheme {
        name: "niru-the-norns",
        label: "Niru The Norns",
        background: "111110",
        foreground: "ece9e4",
        muted: "a6a19b",
        border: "3b3936",
        focus: "c9c2b9",
    },
    BuiltinTheme {
        name: "niru-winter-nights",
        label: "Niru Winter Nights",
        background: "1f232b",
        foreground: "ead9c3",
        muted: "b9aa9a",
        border: "4e5157",
        focus: "d8c4aa",
    },
];

struct BuiltinTheme {
    name: &'static str,
    label: &'static str,
    background: &'static str,
    foreground: &'static str,
    muted: &'static str,
    border: &'static str,
    focus: &'static str,
}

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // shared with NIRUAMP; not every role is drawn here
pub struct Palette {
    pub background: Color32,
    pub surface: Color32,
    pub surface_hi: Color32,
    pub panel: Color32,
    pub inset: Color32,
    pub foreground: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub border_hi: Color32,
    pub focus: Color32,
    pub accent: Color32,
    pub selected: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub disabled: Color32,
    pub led: Color32,
    pub led_dim: Color32,
    pub vis: Color32,
    pub vis_peak: Color32,
    pub shadow: Color32,
    pub light: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Appearance {
    pub font_size: f32,
    pub font_path: Option<PathBuf>,
    pub gap: f32,
    pub padding: f32,
    pub margin: f32,
    pub focus_width: f32,
    pub reduced_motion: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_size: 15.0,
            font_path: None,
            gap: 8.0,
            padding: 12.0,
            margin: 16.0,
            focus_width: 2.0,
            reduced_motion: false,
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ResolvedTheme {
    pub palette: Palette,
    pub appearance: Appearance,
    pub name: String,
    pub label: String,
    pub source: String,
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
struct ThemeFile {
    #[serde(default)]
    name: Option<String>,
    background: String,
    foreground: String,
    muted: String,
    border: String,
    focus: String,
}

fn niruwm_dir() -> PathBuf {
    config_home().join("niruwm")
}

fn builtin(name: &str) -> Option<&'static BuiltinTheme> {
    BUILTINS.iter().find(|t| t.name == name)
}

fn parse_hex(value: &str) -> Option<Color32> {
    let s = value.trim().trim_start_matches('#');
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb(
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    ))
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn lighten(c: Color32, t: f32) -> Color32 {
    blend(c, Color32::WHITE, t)
}

fn darken(c: Color32, t: f32) -> Color32 {
    blend(c, Color32::BLACK, t)
}

fn luminance(c: Color32) -> f32 {
    (0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32) / 255.0
}

fn build_palette(
    background: Color32,
    foreground: Color32,
    muted: Color32,
    border: Color32,
    focus: Color32,
) -> Palette {
    let light = luminance(background) > 0.55;
    let surface = if light {
        blend(background, foreground, 0.055)
    } else {
        blend(background, foreground, 0.065)
    };
    let surface_hi = blend(background, foreground, if light { 0.11 } else { 0.13 });
    let panel = if light {
        blend(background, Color32::WHITE, 0.22)
    } else {
        blend(background, Color32::BLACK, 0.16)
    };
    let inset = if light {
        darken(background, 0.045)
    } else {
        darken(background, 0.30)
    };
    let accent = focus;
    let led = if light {
        darken(focus, 0.18)
    } else {
        lighten(focus, 0.30)
    };
    let vis = focus;
    let vis_peak = if light {
        darken(focus, 0.25)
    } else {
        lighten(focus, 0.55)
    };
    Palette {
        background,
        surface,
        surface_hi,
        panel,
        inset,
        foreground,
        muted,
        border,
        border_hi: blend(border, foreground, 0.24),
        focus,
        accent,
        selected: blend(surface, focus, if light { 0.18 } else { 0.26 }),
        success: if light {
            Color32::from_rgb(47, 111, 65)
        } else {
            Color32::from_rgb(143, 191, 127)
        },
        warning: if light {
            Color32::from_rgb(140, 92, 26)
        } else {
            Color32::from_rgb(215, 179, 106)
        },
        error: if light {
            Color32::from_rgb(155, 55, 55)
        } else {
            Color32::from_rgb(215, 122, 122)
        },
        disabled: blend(muted, background, 0.55),
        led,
        led_dim: blend(led, surface, 0.80),
        vis,
        vis_peak,
        shadow: blend(background, Color32::BLACK, if light { 0.35 } else { 0.55 }),
        light,
    }
}

fn classic_palette() -> Palette {
    build_palette(
        Color32::from_rgb(26, 28, 29),
        Color32::from_rgb(231, 225, 211),
        Color32::from_rgb(162, 155, 141),
        Color32::from_rgb(54, 56, 57),
        Color32::from_rgb(214, 168, 95),
    )
}

#[derive(Debug, Default)]
struct AppearanceFile {
    colors: BTreeMap<String, Color32>,
    font_size: Option<f32>,
    font_path: Option<PathBuf>,
    gap: Option<f32>,
    padding: Option<f32>,
    margin: Option<f32>,
    focus_width: Option<f32>,
    reduced_motion: Option<bool>,
    warnings: Vec<String>,
}

fn read_appearance() -> AppearanceFile {
    let mut out = AppearanceFile::default();
    let path = niruwm_dir().join("appearance.toml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => {
            out.warnings
                .push(format!("appearance.toml unreadable: {e}"));
            return out;
        }
    };
    let value: toml::Value = match text.parse() {
        Ok(v) => v,
        Err(e) => {
            out.warnings.push(format!("appearance.toml ignored: {e}"));
            return out;
        }
    };
    if value.get("schema").and_then(toml::Value::as_integer) != Some(1) {
        out.warnings
            .push("appearance.toml ignored: unsupported schema".into());
        return out;
    }
    if let Some(colors) = value.get("colors").and_then(toml::Value::as_table) {
        for (key, raw) in colors {
            let text = raw.as_str().unwrap_or("");
            match parse_hex(text) {
                Some(color) => {
                    out.colors.insert(key.clone(), color);
                }
                None => out
                    .warnings
                    .push(format!("appearance colour {key} is invalid")),
            }
        }
    }
    if let Some(font) = value.get("font") {
        if let Some(size) = font.get("size").and_then(toml::Value::as_float) {
            if size.is_finite() && (8.0..=48.0).contains(&size) {
                out.font_size = Some(size as f32);
            } else {
                out.warnings
                    .push("appearance font.size is out of range".into());
            }
        }
        if let Some(path) = font.get("path").and_then(toml::Value::as_str) {
            let p = PathBuf::from(path);
            if p.is_file() {
                out.font_path = Some(p);
            } else {
                out.warnings
                    .push(format!("appearance font not found: {path}"));
            }
        }
    }
    if let Some(spacing) = value.get("spacing") {
        for (key, slot) in [
            ("gap", &mut out.gap),
            ("padding", &mut out.padding),
            ("margin", &mut out.margin),
        ] {
            if let Some(n) = spacing.get(key).and_then(toml::Value::as_integer) {
                *slot = Some((n as f32).clamp(0.0, 64.0));
            }
        }
    }
    if let Some(width) = value
        .get("focus")
        .and_then(|f| f.get("width"))
        .and_then(toml::Value::as_integer)
    {
        out.focus_width = Some((width as f32).clamp(1.0, 6.0));
    }
    if let Some(reduced) = value
        .get("motion")
        .and_then(|m| m.get("reduced"))
        .and_then(toml::Value::as_bool)
    {
        out.reduced_motion = Some(reduced);
    }
    out
}

fn theme_name_from_config() -> Result<Option<String>, String> {
    let path = niruwm_dir().join("config.toml");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("NIRUX config unreadable: {e}")),
    };
    let value: toml::Value = text
        .parse()
        .map_err(|e| format!("NIRUX config ignored: {e}"))?;
    Ok(value
        .get("theme")
        .and_then(toml::Value::as_str)
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty()))
}

fn custom_theme_path(name: &str) -> PathBuf {
    niruwm_dir().join("themes").join(format!("{name}.toml"))
}

fn read_theme(name: &str) -> Result<(Palette, String, String, bool), String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("invalid theme name".into());
    }
    let custom = custom_theme_path(name);
    if custom.exists() {
        let text = fs::read_to_string(&custom)
            .map_err(|e| format!("theme {} unreadable: {e}", custom.display()))?;
        let file: ThemeFile = toml::from_str(&text)
            .map_err(|e| format!("theme {} invalid: {e}", custom.display()))?;
        let palette = build_palette(
            parse_hex(&file.background).ok_or_else(|| "invalid background".to_string())?,
            parse_hex(&file.foreground).ok_or_else(|| "invalid foreground".to_string())?,
            parse_hex(&file.muted).ok_or_else(|| "invalid muted".to_string())?,
            parse_hex(&file.border).ok_or_else(|| "invalid border".to_string())?,
            parse_hex(&file.focus).ok_or_else(|| "invalid focus".to_string())?,
        );
        let label = file.name.unwrap_or_else(|| name.to_string());
        return Ok((palette, name.to_string(), label, true));
    }
    let b = builtin(name).ok_or_else(|| format!("theme {name} is not installed"))?;
    let palette = build_palette(
        parse_hex(b.background).expect("builtin background"),
        parse_hex(b.foreground).expect("builtin foreground"),
        parse_hex(b.muted).expect("builtin muted"),
        parse_hex(b.border).expect("builtin border"),
        parse_hex(b.focus).expect("builtin focus"),
    );
    Ok((palette, name.to_string(), b.label.to_string(), false))
}

/// Where `auto` takes its colours from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AutoSource {
    Nirux,
    Omarchy(omarchy::Current),
}

/// `auto` precedence: the NIRU session always follows NIRUX exactly as
/// before; outside it the current Omarchy theme wins when there is one; then
/// a NIRUX configuration; then Classic (via the NIRUX path, unchanged).
fn auto_source() -> AutoSource {
    if session::niru() {
        return AutoSource::Nirux;
    }
    match omarchy::current() {
        Some(current) => AutoSource::Omarchy(current),
        None => AutoSource::Nirux,
    }
}

fn read_omarchy(current: &omarchy::Current) -> Result<(Palette, String, String, bool), String> {
    let c = omarchy::read_colors(&current.dir)
        .map_err(|e| format!("Omarchy theme {}: {e}", current.name))?;
    Ok((
        build_palette(c.background, c.foreground, c.muted, c.border, c.focus),
        current.name.clone(),
        omarchy::label(&current.name),
        false,
    ))
}

fn nirux_or_classic() -> Result<(Palette, String, String, bool), String> {
    match theme_name_from_config()? {
        Some(name) => {
            read_theme(&name).map_err(|e| format!("{e}; keeping the previous appearance"))
        }
        None => Ok((
            classic_palette(),
            "classic".to_string(),
            "NIRUDICTATE Classic".to_string(),
            false,
        )),
    }
}

pub fn resolve(prefs: &Preferences) -> Result<ResolvedTheme, String> {
    let mode = prefs.theme.trim();
    let appearance = read_appearance();
    let mut warnings = appearance.warnings.clone();
    // `nirux` is `auto` pinned to NIRUX; `omarchy` is `auto` pinned to Omarchy.
    let source = match mode {
        "auto" => Some(auto_source()),
        "nirux" => Some(AutoSource::Nirux),
        "omarchy" => Some(AutoSource::Omarchy(omarchy::current().ok_or_else(
            || "no current Omarchy theme found; keeping the previous appearance".to_string(),
        )?)),
        _ => None,
    };
    let from_omarchy = matches!(source, Some(AutoSource::Omarchy(_)));
    let follows_nirux = source == Some(AutoSource::Nirux);
    let (mut palette, name, label, custom) = if mode == "classic" {
        (
            classic_palette(),
            "classic".to_string(),
            "NIRUDICTATE Classic".to_string(),
            false,
        )
    } else if let Some(AutoSource::Omarchy(current)) = &source {
        read_omarchy(current).map_err(|e| format!("{e}; keeping the previous appearance"))?
    } else if follows_nirux {
        nirux_or_classic()?
    } else {
        read_theme(mode).map_err(|e| format!("{e}; keeping the previous appearance"))?
    };
    // NIRUX lets a user theme file take precedence over appearance colour
    // overrides. Extended semantic roles are honoured when no custom file wins.
    if follows_nirux && !custom {
        for (key, color) in &appearance.colors {
            match key.as_str() {
                "background" => palette.background = *color,
                "surface" => palette.surface = *color,
                "foreground" => palette.foreground = *color,
                "muted" => palette.muted = *color,
                "border" => palette.border = *color,
                "focus" => palette.focus = *color,
                "accent" => palette.accent = *color,
                "selected" => palette.selected = *color,
                "success" => palette.success = *color,
                "warning" => palette.warning = *color,
                "error" => palette.error = *color,
                "disabled" => palette.disabled = *color,
                _ => warnings.push(format!("appearance role {key} is not used")),
            }
        }
        if !appearance.colors.is_empty() {
            palette.surface_hi = blend(
                palette.background,
                palette.foreground,
                if palette.light { 0.11 } else { 0.13 },
            );
            palette.panel = if palette.light {
                blend(palette.background, Color32::WHITE, 0.22)
            } else {
                blend(palette.background, Color32::BLACK, 0.16)
            };
            palette.inset = if palette.light {
                darken(palette.background, 0.045)
            } else {
                darken(palette.background, 0.30)
            };
            palette.border_hi = blend(palette.border, palette.foreground, 0.24);
            palette.led_dim = blend(palette.led, palette.surface, 0.80);
            palette.shadow = blend(
                palette.background,
                Color32::BLACK,
                if palette.light { 0.35 } else { 0.55 },
            );
        }
    }
    let appearance = Appearance {
        font_size: appearance.font_size.unwrap_or(15.0),
        font_path: appearance.font_path,
        gap: appearance.gap.unwrap_or(8.0),
        padding: appearance.padding.unwrap_or(12.0),
        margin: appearance.margin.unwrap_or(16.0),
        focus_width: appearance.focus_width.unwrap_or(2.0),
        reduced_motion: appearance.reduced_motion.unwrap_or(false),
    };
    let source = if from_omarchy {
        format!("Omarchy · {name}")
    } else if mode == "classic" || (follows_nirux && name == "classic") {
        "NIRUDICTATE fallback".to_string()
    } else if custom {
        format!("NIRUX custom · {}", custom_theme_path(&name).display())
    } else {
        format!("NIRUX · {name}")
    };
    Ok(ResolvedTheme {
        palette,
        appearance,
        name,
        label,
        source,
        warnings,
    })
}

pub fn available_themes() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = BUILTINS
        .iter()
        .map(|b| (b.name.to_string(), b.label.to_string()))
        .collect();
    if let Ok(entries) = fs::read_dir(niruwm_dir().join("themes")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if key.is_empty()
                || key.len() > 80
                || !key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                continue;
            }
            if out.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let label = fs::read_to_string(&path)
                .ok()
                .and_then(|text| toml::from_str::<ThemeFile>(&text).ok())
                .and_then(|t| t.name)
                .unwrap_or_else(|| key.clone());
            out.push((key, label));
        }
    }
    out.sort_by_key(|a| a.1.to_lowercase());
    out
}

pub fn install_style(ctx: &egui::Context, theme: &ResolvedTheme, prefs: &Preferences) {
    let p = theme.palette;
    let mut visuals = if p.light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    visuals.panel_fill = p.background;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.inset;
    visuals.faint_bg_color = p.surface;
    visuals.code_bg_color = p.inset;
    visuals.selection.bg_fill = p.selected;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, p.foreground);
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.error;
    visuals.window_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.bg_fill = p.surface;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, p.foreground);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.bg_fill = p.surface_hi;
    visuals.widgets.inactive.weak_bg_fill = p.surface;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, p.foreground);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, p.border);
    visuals.widgets.hovered.bg_fill = blend(p.surface_hi, p.focus, 0.18);
    visuals.widgets.hovered.weak_bg_fill = p.surface_hi;
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, p.foreground);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, p.focus);
    visuals.widgets.active.bg_fill = p.selected;
    visuals.widgets.active.weak_bg_fill = p.selected;
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, p.foreground);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, p.focus);
    visuals.widgets.open.bg_fill = p.surface_hi;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, p.focus);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(2.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(2.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(2.0);
    visuals.widgets.active.rounding = egui::Rounding::same(2.0);
    visuals.widgets.open.rounding = egui::Rounding::same(2.0);
    visuals.window_rounding = egui::Rounding::same(3.0);
    visuals.menu_rounding = egui::Rounding::same(2.0);
    ctx.set_visuals(visuals);

    let base = (theme.appearance.font_size * prefs.scale).clamp(10.0, 32.0);
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (
            egui::TextStyle::Heading,
            egui::FontId::proportional(base * 1.30),
        ),
        (egui::TextStyle::Body, egui::FontId::proportional(base)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(base)),
        (egui::TextStyle::Button, egui::FontId::proportional(base)),
        (
            egui::TextStyle::Small,
            egui::FontId::proportional(base * 0.80),
        ),
    ]
    .into();
    let gap = theme.appearance.gap * prefs.scale;
    style.spacing.item_spacing = egui::vec2(gap, gap * 0.7);
    style.spacing.button_padding = egui::vec2(gap, gap * 0.35);
    style.spacing.window_margin = egui::Margin::same(theme.appearance.padding * prefs.scale);
    style.animation_time = if theme.appearance.reduced_motion || prefs.reduced_motion == Some(true)
    {
        0.0
    } else {
        0.12
    };
    ctx.set_style(style);

    if let Some(path) = theme.appearance.font_path.as_ref() {
        if let Ok(bytes) = fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("nirux-font".into(), egui::FontData::from_owned(bytes));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, "nirux-font".into());
            }
            ctx.set_fonts(fonts);
        }
    }
}

#[allow(dead_code)]
pub fn reduced_motion(theme: &ResolvedTheme, prefs: &Preferences) -> bool {
    prefs
        .reduced_motion
        .unwrap_or(theme.appearance.reduced_motion)
}

#[derive(Debug)]
pub enum ThemeEvent {
    Updated(Box<ResolvedTheme>),
    Warning(String),
}

pub struct ThemeWatcher {
    stop: Arc<AtomicBool>,
    /// Write end of the wake-up pipe; one byte unblocks the watcher's poll.
    wake: Option<std::os::fd::OwnedFd>,
    handle: Option<thread::JoinHandle<()>>,
    pub rx: Receiver<ThemeEvent>,
}

impl ThemeWatcher {
    pub fn spawn(prefs: Arc<Mutex<Preferences>>, ctx: egui::Context) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let (tx, rx) = mpsc::channel();
        let (wake_read, wake) = match pipe() {
            Some((r, w)) => (Some(r), Some(w)),
            None => (None, None),
        };
        let handle = thread::Builder::new()
            .name("nirudictate-theme".into())
            .spawn(move || {
                let notify = wake_read.and_then(|r| Inotify::new().map(|i| (i, r)));
                match notify {
                    Some((inotify, wake_read)) => {
                        watch_inotify(prefs, ctx, tx, stop2, inotify, wake_read)
                    }
                    // Without inotify (or a pipe) fall back to the 0.3 timer.
                    None => watch_loop(prefs, ctx, tx, stop2),
                }
            })
            .ok();
        Self {
            stop,
            wake,
            handle,
            rx,
        }
    }
}

impl Drop for ThemeWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(fd) = self.wake.take() {
            use std::io::Write;
            let mut file = fs::File::from(fd);
            let _ = file.write_all(&[1]);
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn pipe() -> Option<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::FromRawFd;
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe2 fills two new descriptors that we take ownership of.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    unsafe {
        Some((
            std::os::fd::OwnedFd::from_raw_fd(fds[0]),
            std::os::fd::OwnedFd::from_raw_fd(fds[1]),
        ))
    }
}

/// Minimal inotify wrapper: the watcher thread sleeps in `poll` until a
/// watched directory changes or the watcher is dropped.
struct Inotify {
    fd: std::os::fd::OwnedFd,
}

const DIR_MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

impl Inotify {
    fn new() -> Option<Self> {
        use std::os::fd::FromRawFd;
        // SAFETY: plain syscall; a valid descriptor is owned from here on.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            return None;
        }
        Some(Self {
            fd: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) },
        })
    }

    /// Watch a directory (re-adding an existing watch is a no-op).
    fn watch(&self, dir: &Path) -> bool {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: valid descriptor and NUL-terminated path.
        unsafe {
            libc::inotify_add_watch(self.fd.as_raw_fd(), c.as_ptr(), DIR_MASK | libc::IN_ONLYDIR)
                >= 0
        }
    }

    /// Discard queued events; the fingerprint decides what changed.
    fn drain(&self) {
        use std::os::fd::AsRawFd;
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: reading into a local buffer from our own descriptor.
            let n = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n <= 0 {
                return;
            }
        }
    }

    /// Wait for an event (`timeout_ms` < 0 waits forever). Returns
    /// (inotify readable, stop requested).
    fn wait(&self, stop: &std::os::fd::OwnedFd, timeout_ms: i32) -> (bool, bool) {
        use std::os::fd::AsRawFd;
        let mut fds = [
            libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stop.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfd entries.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };
        if n < 0 {
            let interrupted =
                std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted;
            return (false, !interrupted);
        }
        (fds[0].revents != 0, fds[1].revents != 0)
    }
}

/// The directories whose changes can change the resolved theme: the NIRUWM
/// config directory and its themes directory, or the nearest parent that
/// exists so their creation is noticed too.
fn watch_dirs(inotify: &Inotify) {
    for dir in omarchy::watch_dirs() {
        inotify.watch(&dir);
    }
    let niruwm = niruwm_dir();
    if niruwm.is_dir() {
        inotify.watch(&niruwm);
        let themes = niruwm.join("themes");
        if themes.is_dir() {
            inotify.watch(&themes);
        }
    } else {
        let config = config_home();
        if config.is_dir() {
            inotify.watch(&config);
        }
    }
}

fn watch_inotify(
    prefs: Arc<Mutex<Preferences>>,
    ctx: egui::Context,
    tx: Sender<ThemeEvent>,
    stop: Arc<AtomicBool>,
    inotify: Inotify,
    wake: std::os::fd::OwnedFd,
) {
    let mut last = {
        let p = prefs.lock().map(|p| p.clone()).unwrap_or_default();
        fingerprint(&p)
    };
    while !stop.load(Ordering::Relaxed) {
        watch_dirs(&inotify);
        let (_, stopped) = inotify.wait(&wake, -1);
        if stopped || stop.load(Ordering::Relaxed) {
            break;
        }
        // Debounce: editors write in several steps. Wait until the directory
        // has been quiet for 320 ms (at most ~2 s) before resolving.
        let started = Instant::now();
        loop {
            inotify.drain();
            let (more, stopped) = inotify.wait(&wake, 320);
            if stopped {
                return;
            }
            if !more || started.elapsed() > Duration::from_secs(2) {
                break;
            }
        }
        inotify.drain();
        let current = prefs.lock().map(|p| p.clone()).unwrap_or_default();
        let now = fingerprint(&current);
        if now == last {
            continue;
        }
        last = now;
        match resolve(&current) {
            Ok(theme) => {
                let _ = tx.send(ThemeEvent::Updated(Box::new(theme)));
            }
            Err(e) => {
                let _ = tx.send(ThemeEvent::Warning(e));
            }
        }
        ctx.request_repaint();
    }
}

#[derive(Clone, PartialEq)]
struct Fingerprint {
    mode: String,
    config: Option<(SystemTime, u64)>,
    appearance: Option<(SystemTime, u64)>,
    themes_dir: Option<SystemTime>,
}

fn fingerprint(prefs: &Preferences) -> Fingerprint {
    let meta = |p: &Path| {
        fs::metadata(p)
            .ok()
            .map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
    };
    let mode = prefs.theme.clone();
    let resolved_name = if mode == "auto" {
        theme_name_from_config().ok().flatten().unwrap_or_default()
    } else if mode == "classic" {
        String::new()
    } else {
        mode.clone()
    };
    let theme_file = if resolved_name.is_empty() {
        None
    } else {
        meta(&custom_theme_path(&resolved_name))
    };
    // Outside the NIRU session `auto` can follow Omarchy; there its current
    // theme directory (replaced on every switch) and colour file count too.
    let omarchy = if mode == "omarchy" || (mode == "auto" && !session::niru()) {
        omarchy::fingerprint()
    } else {
        String::new()
    };
    Fingerprint {
        mode: format!("{mode}|{resolved_name}|{theme_file:?}|{omarchy}"),
        config: meta(&niruwm_dir().join("config.toml")),
        appearance: meta(&niruwm_dir().join("appearance.toml")),
        themes_dir: fs::metadata(niruwm_dir().join("themes"))
            .ok()
            .and_then(|m| m.modified().ok()),
    }
}

/// Timer fallback, used only when inotify is unavailable.
fn watch_loop(
    prefs: Arc<Mutex<Preferences>>,
    ctx: egui::Context,
    tx: Sender<ThemeEvent>,
    stop: Arc<AtomicBool>,
) {
    let mut last = {
        let p = prefs.lock().map(|p| p.clone()).unwrap_or_default();
        fingerprint(&p)
    };
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(900));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let snapshot = prefs.lock().map(|p| p.clone()).unwrap_or_default();
        let now = fingerprint(&snapshot);
        if now == last {
            continue;
        }
        last = now;
        let debounce = Instant::now();
        while debounce.elapsed() < Duration::from_millis(320) && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(80));
        }
        let current = prefs.lock().map(|p| p.clone()).unwrap_or_default();
        match resolve(&current) {
            Ok(theme) => {
                let _ = tx.send(ThemeEvent::Updated(Box::new(theme)));
            }
            Err(e) => {
                let _ = tx.send(ThemeEvent::Warning(e));
            }
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;

    fn env_lock() -> &'static Mutex<()> {
        crate::session::env_lock()
    }

    struct TempEnv {
        root: PathBuf,
        old: Vec<(&'static str, Option<OsString>)>,
    }

    impl TempEnv {
        fn new(name: &str) -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "niruamp-theme-test-{name}-{}-{stamp}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("niruwm/themes")).unwrap();
            let old = [
                "XDG_CONFIG_HOME",
                "HOME",
                "XDG_CURRENT_DESKTOP",
                "DESKTOP_SESSION",
                "DISPLAY",
                "WAYLAND_DISPLAY",
            ]
            .into_iter()
            .map(|k| (k, std::env::var_os(k)))
            .collect();
            std::env::set_var("XDG_CONFIG_HOME", &root);
            std::env::set_var("HOME", &root);
            for k in [
                "XDG_CURRENT_DESKTOP",
                "DESKTOP_SESSION",
                "DISPLAY",
                "WAYLAND_DISPLAY",
            ] {
                std::env::remove_var(k);
            }
            Self { root, old }
        }
        fn dir(&self) -> PathBuf {
            self.root.join("niruwm")
        }
        /// An Omarchy 4 current theme (replaced the way omarchy-theme-set does).
        fn omarchy(&self, name: &str, colors: &str) {
            let current = self.root.join(".local/state/omarchy/current");
            fs::create_dir_all(&current).unwrap();
            let next = current.join("next-theme");
            let _ = fs::remove_dir_all(&next);
            fs::create_dir_all(&next).unwrap();
            fs::write(next.join("colors.toml"), colors).unwrap();
            let _ = fs::remove_dir_all(current.join("theme"));
            fs::rename(&next, current.join("theme")).unwrap();
            fs::write(current.join("theme.name"), format!("{name}\n")).unwrap();
        }
        fn write(&self, relative: &str, text: &str) {
            let path = self.dir().join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for TempEnv {
        fn drop(&mut self) {
            for (key, value) in &self.old {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn pref(theme: &str) -> Preferences {
        Preferences {
            theme: theme.into(),
            ..Preferences::default()
        }
    }

    #[test]
    fn standalone_fallback_without_nirux() {
        let _guard = env_lock().lock().unwrap();
        let _env = TempEnv::new("standalone");
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.name, "classic");
        assert_eq!(theme.source, "NIRUDICTATE fallback");
        assert_eq!(theme.palette.background, Color32::from_rgb(26, 28, 29));
        assert!(!theme.palette.light);
    }

    #[test]
    fn builtin_theme_without_custom_file() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("builtin");
        env.write("config.toml", "theme = \"niru-nord\"\n");
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.name, "niru-nord");
        assert_eq!(
            theme.palette.background,
            Color32::from_rgb(0x2e, 0x34, 0x40)
        );
        assert!(theme.source.contains("niru-nord"));
        assert!(!theme.appearance.reduced_motion);
    }

    #[test]
    fn custom_theme_takes_precedence() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("custom");
        env.write("config.toml", "theme = \"niru-nord\"\n");
        env.write("themes/niru-nord.toml", "name = \"Mine\"\nbackground = \"101010\"\nforeground = \"eeeeee\"\nmuted = \"999999\"\nborder = \"333333\"\nfocus = \"4488cc\"\n");
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.label, "Mine");
        assert_eq!(
            theme.palette.background,
            Color32::from_rgb(0x10, 0x10, 0x10)
        );
        assert!(theme.source.starts_with("NIRUX custom"));
    }

    #[test]
    fn sparse_appearance_overrides_semantic_roles() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("appearance");
        env.write("config.toml", "theme = \"niru-noir\"\n");
        env.write("appearance.toml", "schema = 1\n[colors]\nbackground = \"010203\"\nfocus = \"ff0000\"\nwarning = \"00ff00\"\n[font]\nsize = 17.0\n[spacing]\ngap = 10\n[motion]\nreduced = true\n");
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.palette.background, Color32::from_rgb(1, 2, 3));
        assert_eq!(theme.palette.focus, Color32::from_rgb(255, 0, 0));
        assert_eq!(theme.palette.warning, Color32::from_rgb(0, 255, 0));
        assert_eq!(theme.appearance.font_size, 17.0);
        assert_eq!(theme.appearance.gap, 10.0);
        assert!(theme.appearance.reduced_motion);
    }

    #[test]
    fn malformed_appearance_is_ignored() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("bad-appearance");
        env.write("config.toml", "theme = \"niru-satie\"\n");
        env.write(
            "appearance.toml",
            "schema = 99\n[colors]\nbackground = \"zzz\"\n",
        );
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.name, "niru-satie");
        assert_eq!(
            theme.palette.background,
            Color32::from_rgb(0x14, 0x14, 0x14)
        );
        assert!(theme.warnings.iter().any(|w| w.contains("schema")));
    }

    #[test]
    fn malformed_custom_theme_is_an_error_not_partial_apply() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("bad-custom");
        env.write("config.toml", "theme = \"mine\"\n");
        env.write("themes/mine.toml", "background = \"nope\"\n");
        assert!(resolve(&Preferences::default()).is_err());
    }

    #[test]
    fn manual_classic_ignores_nirux_colour_overrides() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("manual-classic");
        env.write("config.toml", "theme = \"niru-nord\"\n");
        env.write(
            "appearance.toml",
            "schema = 1\n[colors]\nbackground = \"ff00ff\"\n",
        );
        let theme = resolve(&pref("classic")).unwrap();
        assert_eq!(theme.name, "classic");
        assert_eq!(theme.palette.background, Color32::from_rgb(26, 28, 29));
    }

    #[test]
    fn light_theme_derives_readable_roles() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("light");
        env.write("config.toml", "theme = \"niru-carl-larsson\"\n");
        let theme = resolve(&Preferences::default()).unwrap();
        assert!(theme.palette.light);
        assert!(luminance(theme.palette.foreground) < 0.35);
        assert!(luminance(theme.palette.background) > 0.75);
        assert_ne!(theme.palette.selected, theme.palette.surface);
    }

    #[test]
    fn watcher_applies_live_theme_changes_after_debounce() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("watch");
        env.write("config.toml", "theme = \"niru-nord\"\n");
        let initial = resolve(&Preferences::default()).unwrap();
        assert_eq!(initial.name, "niru-nord");
        let prefs = Arc::new(Mutex::new(Preferences::default()));
        let ctx = egui::Context::default();
        let watcher = ThemeWatcher::spawn(prefs.clone(), ctx);
        thread::sleep(Duration::from_millis(1300));
        env.write("config.toml", "theme = \"niru-satie\"\n");
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut updated = None;
        while Instant::now() < deadline {
            if let Ok(ThemeEvent::Updated(theme)) =
                watcher.rx.recv_timeout(Duration::from_millis(250))
            {
                updated = Some(theme.name.clone());
                break;
            }
        }
        assert_eq!(updated.as_deref(), Some("niru-satie"));
        // A malformed custom theme must not replace the working appearance.
        thread::sleep(Duration::from_millis(1100));
        env.write("config.toml", "theme = \"broken\"\n");
        env.write("themes/broken.toml", "background = \"nope\"\n");
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut warned = false;
        while Instant::now() < deadline {
            if let Ok(ThemeEvent::Warning(msg)) =
                watcher.rx.recv_timeout(Duration::from_millis(250))
            {
                warned = msg.contains("broken");
                break;
            }
        }
        assert!(warned);
    }

    #[test]
    fn watcher_notices_a_niruwm_directory_created_later() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("watch-late");
        let _ = fs::remove_dir_all(env.dir());
        let prefs = Arc::new(Mutex::new(Preferences::default()));
        let watcher = ThemeWatcher::spawn(prefs, egui::Context::default());
        thread::sleep(Duration::from_millis(200));
        assert!(watcher.rx.try_recv().is_err(), "no event without a change");
        env.write("config.toml", "theme = \"niru-satie\"\n");
        let event = watcher.rx.recv_timeout(Duration::from_secs(4));
        assert!(
            matches!(event, Ok(ThemeEvent::Updated(ref t)) if t.name == "niru-satie"),
            "{event:?}"
        );
        let started = Instant::now();
        drop(watcher);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "drop is prompt"
        );
    }

    #[test]
    fn available_themes_include_builtins_and_custom_files() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("catalog");
        env.write("themes/my-theme.toml", "name = \"My Theme\"\nbackground = \"101010\"\nforeground = \"eeeeee\"\nmuted = \"999999\"\nborder = \"333333\"\nfocus = \"4488cc\"\n");
        let themes = available_themes();
        assert!(themes.iter().any(|(k, _)| k == "niru-noir"));
        assert!(themes
            .iter()
            .any(|(k, label)| k == "my-theme" && label == "My Theme"));
    }

    const OMARCHY_TOKYO: &str = "accent = \"#7aa2f7\"\nselection = \"#292e42\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"\ndark_foreground = \"#565f89\"\n";
    const OMARCHY_LATTE: &str = "mode = \"light\"\naccent = \"#1e66f5\"\nselection = \"#ccd0da\"\nbackground = \"#eff1f5\"\nforeground = \"#4c4f69\"\ndark_foreground = \"#8c8fa1\"\n";

    #[test]
    fn auto_follows_omarchy_outside_the_niru_session() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("omarchy-auto");
        env.omarchy("tokyo-night", OMARCHY_TOKYO);
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.source, "Omarchy · tokyo-night");
        assert_eq!(theme.label, "Tokyo Night");
        assert_eq!(
            theme.palette.background,
            Color32::from_rgb(0x1a, 0x1b, 0x26)
        );
        assert_eq!(theme.palette.focus, Color32::from_rgb(0x7a, 0xa2, 0xf7));
        assert!(!theme.palette.light);
    }

    #[test]
    fn niru_session_keeps_nirux_even_with_omarchy_present() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("omarchy-niru");
        env.omarchy("tokyo-night", OMARCHY_TOKYO);
        env.write("config.toml", "theme = \"niru-nord\"\n");
        std::env::set_var("XDG_CURRENT_DESKTOP", "NIRU");
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.name, "niru-nord");
        assert_eq!(theme.source, "NIRUX · niru-nord");
        // Without a NIRUX config the NIRU session still gets Classic, as before.
        fs::remove_file(env.dir().join("config.toml")).unwrap();
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(theme.source, "NIRUDICTATE fallback");
    }

    #[test]
    fn explicit_nirux_and_omarchy_modes() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("omarchy-explicit");
        assert!(resolve(&pref("omarchy")).is_err(), "no Omarchy theme yet");
        env.omarchy("catppuccin-latte", OMARCHY_LATTE);
        env.write("config.toml", "theme = \"niru-satie\"\n");
        let o = resolve(&pref("omarchy")).unwrap();
        assert_eq!(o.label, "Catppuccin Latte");
        assert!(o.palette.light);
        assert!(luminance(o.palette.foreground) < 0.35);
        let n = resolve(&pref("nirux")).unwrap();
        assert_eq!(n.name, "niru-satie");
    }

    #[test]
    fn nirux_appearance_overrides_do_not_touch_omarchy_colours() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("omarchy-appearance");
        env.omarchy("tokyo-night", OMARCHY_TOKYO);
        env.write(
            "appearance.toml",
            "schema = 1\n[colors]\nbackground = \"ff00ff\"\n",
        );
        let theme = resolve(&Preferences::default()).unwrap();
        assert_eq!(
            theme.palette.background,
            Color32::from_rgb(0x1a, 0x1b, 0x26)
        );
    }

    #[test]
    fn watcher_follows_an_omarchy_theme_switch() {
        let _guard = env_lock().lock().unwrap();
        let env = TempEnv::new("omarchy-watch");
        env.omarchy("tokyo-night", OMARCHY_TOKYO);
        let prefs = Arc::new(Mutex::new(Preferences::default()));
        let watcher = ThemeWatcher::spawn(prefs, egui::Context::default());
        thread::sleep(Duration::from_millis(300));
        env.omarchy("catppuccin-latte", OMARCHY_LATTE);
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut updated = None;
        while Instant::now() < deadline {
            if let Ok(ThemeEvent::Updated(theme)) =
                watcher.rx.recv_timeout(Duration::from_millis(250))
            {
                updated = Some(theme.source.clone());
                break;
            }
        }
        assert_eq!(updated.as_deref(), Some("Omarchy · catppuccin-latte"));
    }
}
