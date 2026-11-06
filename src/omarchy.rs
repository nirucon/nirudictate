//! Omarchy theme discovery: the current theme's palette, read from the files
//! Omarchy itself maintains.
//!
//! Omarchy 4 copies the selected theme to `~/.local/state/omarchy/current/theme`
//! (a directory, replaced on every switch) and records its name in
//! `current/theme.name`. Omarchy 3.x pointed `~/.config/omarchy/current/theme`
//! at the theme directory instead. Since Omarchy 3.4 every theme has a unified
//! `colors.toml`; older themes only ship `alacritty.toml`, which is read the way
//! `omarchy-theme-colors-from-alacritty` reads it.
//!
//! The paths are `$HOME`-relative on purpose: Omarchy hard-codes them and does
//! not follow `XDG_STATE_HOME`/`XDG_CONFIG_HOME` for its own state.
use egui::Color32;
use std::fs;
use std::path::{Path, PathBuf};

/// The palette inputs NIRUAMP derives its whole look from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub background: Color32,
    pub foreground: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub focus: Color32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    /// Theme key as Omarchy names it, e.g. `tokyo-night`.
    pub name: String,
    /// Directory holding the current theme's files.
    pub dir: PathBuf,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Omarchy 4: `~/.local/state/omarchy/current`.
pub fn state_current() -> PathBuf {
    home().join(".local/state/omarchy/current")
}

/// Omarchy 3.x: `~/.config/omarchy/current`.
pub fn legacy_current() -> PathBuf {
    home().join(".config/omarchy/current")
}

fn has_palette(dir: &Path) -> bool {
    dir.join("colors.toml").is_file() || dir.join("alacritty.toml").is_file()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !name.starts_with('.')
}

fn name_in(current: &Path, theme_dir: &Path) -> String {
    let recorded = fs::read_to_string(current.join("theme.name"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| valid_name(s));
    if let Some(name) = recorded {
        return name;
    }
    // Omarchy 3.x before theme.name: the symlink names the theme.
    fs::read_link(theme_dir)
        .ok()
        .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|s| valid_name(s))
        .unwrap_or_else(|| "omarchy".to_string())
}

/// The theme Omarchy currently applies, or `None` on a machine without
/// Omarchy (every NIRUX machine).
pub fn current() -> Option<Current> {
    for base in [state_current(), legacy_current()] {
        let dir = base.join("theme");
        if dir.is_dir() && has_palette(&dir) {
            let name = name_in(&base, &dir);
            return Some(Current { name, dir });
        }
    }
    None
}

/// `tokyo-night` -> `Tokyo Night`.
pub fn label(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `#rrggbb`, `rrggbb`, `0xrrggbb`, optionally with alpha (`#rrggbbaa`, ignored).
pub fn parse_color(value: &str) -> Option<Color32> {
    let s = value.trim().trim_matches(|c| c == '"' || c == '\'');
    let s = s
        .strip_prefix('#')
        .or_else(|| s.strip_prefix("0x"))
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    let hex = match s.len() {
        6 => s,
        8 => &s[..6],
        _ => return None,
    };
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Color32::from_rgb(
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    ))
}

fn channel(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn rel_luminance(c: Color32) -> f32 {
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// WCAG contrast ratio, 1.0 ..= 21.0.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (rel_luminance(a), rel_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Move `c` towards `toward` until it reaches `min` contrast with `against`
/// (or is `toward` itself). Colours that already pass are kept exactly.
fn ensure_contrast(c: Color32, against: Color32, toward: Color32, min: f32) -> Color32 {
    if contrast(c, against) >= min {
        return c;
    }
    for step in 1..=20 {
        let candidate = mix(c, toward, step as f32 / 20.0);
        if contrast(candidate, against) >= min {
            return candidate;
        }
    }
    toward
}

/// Lookup in a flat `colors.toml` table.
struct Table(toml::Table);

impl Table {
    fn color(&self, key: &str) -> Option<Color32> {
        self.0
            .get(key)
            .and_then(|v| v.as_str())
            .and_then(parse_color)
    }
    fn first(&self, keys: &[&str]) -> Option<Color32> {
        keys.iter().find_map(|k| self.color(k))
    }
}

/// Map Omarchy's semantic roles onto NIRUAMP's five inputs.
///
/// * background/foreground: as given.
/// * focus: `accent`, else ANSI blue (`color4`/`blue`), as Omarchy itself
///   defaults the accent.
/// * muted (secondary text): the theme's dimmed foreground, kept exactly
///   when it reads well, otherwise moved towards the foreground until it
///   reaches 4.0:1 against the background.
/// * border: `selection` (the theme's subtle surface), else the lighter
///   background or bright black, made just visible against the background.
fn from_roles(
    background: Color32,
    foreground: Color32,
    accent: Option<Color32>,
    muted: Option<Color32>,
    border: Option<Color32>,
) -> Colors {
    let focus = accent.unwrap_or_else(|| mix(foreground, background, 0.35));
    let muted = muted.unwrap_or_else(|| mix(foreground, background, 0.38));
    let muted = ensure_contrast(muted, background, foreground, 4.0);
    let border = border.unwrap_or_else(|| mix(background, foreground, 0.16));
    let border = if border == background {
        mix(background, foreground, 0.16)
    } else {
        ensure_contrast(border, background, foreground, 1.18)
    };
    Colors {
        background,
        foreground,
        muted,
        border,
        focus,
    }
}

pub fn parse_colors_toml(text: &str) -> Result<Colors, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| format!("colors.toml invalid: {e}"))?;
    let t = Table(table);
    let background = t
        .color("background")
        .ok_or("colors.toml has no background")?;
    let foreground = t
        .color("foreground")
        .ok_or("colors.toml has no foreground")?;
    Ok(from_roles(
        background,
        foreground,
        t.first(&["accent", "blue", "color4"]),
        t.first(&["dark_foreground", "muted", "color8", "bright_black"]),
        t.first(&["selection", "lighter_background", "color8"]),
    ))
}

/// Alacritty colour lookup that accepts both `[colors.primary] background`
/// and `[colors] primary.background` spellings.
fn alacritty_color(root: &toml::Table, section: &str, key: &str) -> Option<Color32> {
    root.get("colors")
        .and_then(|c| c.as_table())
        .and_then(|c| c.get(section))
        .and_then(|s| s.as_table())
        .and_then(|s| s.get(key))
        .and_then(|v| v.as_str())
        .and_then(parse_color)
}

pub fn parse_alacritty(text: &str) -> Result<Colors, String> {
    let root: toml::Table = text
        .parse()
        .map_err(|e| format!("alacritty.toml invalid: {e}"))?;
    let normal = |k| alacritty_color(&root, "normal", k);
    let background = alacritty_color(&root, "primary", "background")
        .or_else(|| normal("black"))
        .ok_or("alacritty.toml has no background")?;
    let foreground = alacritty_color(&root, "primary", "foreground")
        .or_else(|| normal("white"))
        .ok_or("alacritty.toml has no foreground")?;
    Ok(from_roles(
        background,
        foreground,
        normal("blue"),
        alacritty_color(&root, "bright", "black"),
        alacritty_color(&root, "selection", "background")
            .or_else(|| alacritty_color(&root, "bright", "black")),
    ))
}

/// Read the palette of a theme directory.
pub fn read_colors(dir: &Path) -> Result<Colors, String> {
    let colors = dir.join("colors.toml");
    if colors.is_file() {
        let text =
            fs::read_to_string(&colors).map_err(|e| format!("colors.toml unreadable: {e}"))?;
        return parse_colors_toml(&text);
    }
    let alacritty = dir.join("alacritty.toml");
    let text =
        fs::read_to_string(&alacritty).map_err(|e| format!("alacritty.toml unreadable: {e}"))?;
    parse_alacritty(&text)
}

/// Directories whose changes can change the current Omarchy theme, for the
/// theme watcher. Only existing ones; a machine without Omarchy has none.
pub fn watch_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in [state_current(), legacy_current()] {
        if base.is_dir() {
            let theme = base.join("theme");
            if theme.is_dir() {
                out.push(theme);
            }
            out.push(base);
        }
    }
    out
}

/// What identifies the current theme's palette, for the watcher fingerprint.
pub fn fingerprint() -> String {
    use std::os::unix::fs::MetadataExt;
    let Some(cur) = current() else {
        return String::new();
    };
    let meta = |p: PathBuf| {
        fs::metadata(p)
            .ok()
            .map(|m| (m.ino(), m.mtime(), m.mtime_nsec(), m.len()))
    };
    format!(
        "{}|{:?}|{:?}|{:?}",
        cur.name,
        meta(cur.dir.clone()),
        meta(cur.dir.join("colors.toml")),
        meta(cur.dir.join("alacritty.toml"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO: &str = r##"mode = "dark"
accent = "#7aa2f7"
selection = "#292e42"
muted = "#414868"
background = "#1a1b26"
lighter_background = "#24283b"
foreground = "#a9b1d6"
dark_foreground = "#565f89"
blue = "#7aa2f7"
"##;

    const FLEXOKI_LIGHT: &str = r##"mode = "light"
accent = "#205EA6"
selection = "#CECDC3"
muted = "#B7B5AC"
background = "#FFFCF0"
lighter_background = "#E6E4D9"
foreground = "#100F0F"
dark_foreground = "#878580"
"##;

    const FOREST_VIGIL: &str = r##"mode = "dark"
accent = "#8aa87e"
selection = "#2b362c"
muted = "#96a292"
background = "#0b100c"
foreground = "#d8ded2"
dark_foreground = "#96a292"
"##;

    #[test]
    fn colors_toml_dark_theme() {
        let c = parse_colors_toml(TOKYO).unwrap();
        assert_eq!(c.background, parse_color("#1a1b26").unwrap());
        assert_eq!(c.foreground, parse_color("#a9b1d6").unwrap());
        assert_eq!(c.focus, parse_color("#7aa2f7").unwrap());
        assert_eq!(c.border, parse_color("#292e42").unwrap());
        assert!(contrast(c.muted, c.background) >= 4.0);
        assert!(contrast(c.muted, c.background) < contrast(c.foreground, c.background));
    }

    #[test]
    fn colors_toml_light_theme_stays_readable() {
        let c = parse_colors_toml(FLEXOKI_LIGHT).unwrap();
        assert_eq!(c.background, parse_color("FFFCF0").unwrap());
        assert_eq!(c.focus, parse_color("205ea6").unwrap());
        assert!(contrast(c.muted, c.background) >= 4.0);
        assert!(contrast(c.border, c.background) >= 1.18);
    }

    #[test]
    fn nirux_port_maps_back_to_the_nirux_palette() {
        // The owner's Forest Vigil port carries NIRUWM's own five colours.
        let c = parse_colors_toml(FOREST_VIGIL).unwrap();
        assert_eq!(c.muted, parse_color("96a292").unwrap());
        assert_eq!(c.border, parse_color("2b362c").unwrap());
        assert_eq!(c.focus, parse_color("8aa87e").unwrap());
    }

    #[test]
    fn generated_colors_toml_without_semantic_roles() {
        // What omarchy-theme-colors-from-alacritty writes.
        let text = "accent = \"#81a1c1\"\nselection = \"#434c5e\"\nbackground = \"#2e3440\"\nforeground = \"#d8dee9\"\ncolor4 = \"#81a1c1\"\ncolor8 = \"#4c566a\"\n";
        let c = parse_colors_toml(text).unwrap();
        assert_eq!(c.focus, parse_color("81a1c1").unwrap());
        assert_eq!(c.border, parse_color("434c5e").unwrap());
        assert!(contrast(c.muted, c.background) >= 4.0);
    }

    #[test]
    fn alacritty_fallback_for_old_themes() {
        let text = "[colors.primary]\nbackground = '0x1a1b26'\nforeground = '#a9b1d6'\n[colors.normal]\nblack = '#32344a'\nblue = '#7aa2f7'\nwhite = '#787c99'\n[colors.bright]\nblack = '#444b6a'\n[colors.selection]\nbackground = '#7aa2f7'\n";
        let c = parse_alacritty(text).unwrap();
        assert_eq!(c.background, parse_color("1a1b26").unwrap());
        assert_eq!(c.focus, parse_color("7aa2f7").unwrap());
        assert!(contrast(c.muted, c.background) >= 4.0);
    }

    #[test]
    fn missing_or_invalid_colors_are_errors() {
        assert!(parse_colors_toml("foreground = \"#ffffff\"\n").is_err());
        assert!(parse_colors_toml("background = \"nope\"\nforeground = \"#fff\"\n").is_err());
        assert!(parse_colors_toml("not toml [").is_err());
    }

    #[test]
    fn color_spellings() {
        let want = Color32::from_rgb(0x12, 0x34, 0x56);
        for s in ["#123456", "123456", "0x123456", "'#123456'", "#123456ff"] {
            assert_eq!(parse_color(s), Some(want), "{s}");
        }
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgb(1,2,3)"), None);
    }

    #[test]
    fn labels() {
        assert_eq!(label("tokyo-night"), "Tokyo Night");
        assert_eq!(label("catppuccin-latte"), "Catppuccin Latte");
        assert_eq!(label("retro-82"), "Retro 82");
    }

    #[test]
    fn current_theme_in_both_layouts() {
        let root = std::env::temp_dir().join(format!(
            "niruamp-omarchy-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        // Omarchy 4: a copied directory plus theme.name.
        let v4 = root.join("v4");
        let dir = v4.join(".local/state/omarchy/current/theme");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("colors.toml"), TOKYO).unwrap();
        fs::write(
            v4.join(".local/state/omarchy/current/theme.name"),
            "tokyo-night\n",
        )
        .unwrap();
        // Omarchy 3.x: a symlink, no theme.name, alacritty only.
        let v3 = root.join("v3");
        let target = v3.join(".config/omarchy/themes/nord");
        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join("alacritty.toml"),
            "[colors.primary]\nbackground = '#2e3440'\nforeground = '#d8dee9'\n",
        )
        .unwrap();
        fs::create_dir_all(v3.join(".config/omarchy/current")).unwrap();
        std::os::unix::fs::symlink(&target, v3.join(".config/omarchy/current/theme")).unwrap();

        let found = |home: &Path| {
            let base4 = home.join(".local/state/omarchy/current");
            let base3 = home.join(".config/omarchy/current");
            [base4, base3].into_iter().find_map(|base| {
                let dir = base.join("theme");
                (dir.is_dir() && has_palette(&dir)).then(|| (name_in(&base, &dir), dir))
            })
        };
        let (name, dir) = found(&v4).unwrap();
        assert_eq!(name, "tokyo-night");
        assert_eq!(
            read_colors(&dir).unwrap().focus,
            parse_color("7aa2f7").unwrap()
        );
        let (name, dir) = found(&v3).unwrap();
        assert_eq!(name, "nord");
        assert_eq!(
            read_colors(&dir).unwrap().background,
            parse_color("2e3440").unwrap()
        );
        assert!(found(&root.join("none")).is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
