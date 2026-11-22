//! Panel helpers that are not drawing: the window icon, the shortcut
//! recorder's key names and local time stamps.
use crate::gui::ui::Key;

/// The window icon: an accent disc with the microphone (straight RGBA).
pub fn icon() -> (u32, u32, Vec<u8>) {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let accent = [214.0, 168.0, 95.0];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
            let disc = (31.0 - (fx * fx + fy * fy).sqrt()).clamp(0.0, 1.0);
            // Microphone body: a capsule.
            let by = (fy + 5.0).abs() - 7.0;
            let capsule =
                (7.5 - ((fx.abs().max(0.0)).powi(2) + by.max(0.0).powi(2)).sqrt()).clamp(0.0, 1.0);
            let ring_r = (fx * fx + (fy + 2.0) * (fy + 2.0)).sqrt();
            let ring = if fy > -2.0 {
                (1.8 - (ring_r - 13.0).abs()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let stem = if fx.abs() < 1.8 && fy > 11.0 && fy < 19.0 {
                1.0
            } else {
                0.0
            };
            let mark = capsule.max(ring).max(stem);
            let i = (y * n + x) * 4;
            for k in 0..3 {
                rgba[i + k] = (accent[k] * (1.0 - mark) + 250.0 * mark) as u8;
            }
            rgba[i + 3] = (disc * 255.0) as u8;
        }
    }
    (n as u32, n as u32, rgba)
}

/// The name `hotkey::parse` uses for an unshifted keysym.
fn key_name(sym: u32) -> Option<String> {
    let name = match sym {
        0x20 => "space",
        0xff0d => "return",
        0xff09 => "tab",
        0xff08 => "backspace",
        0xff63 => "insert",
        0xffff => "delete",
        0xff50 => "home",
        0xff57 => "end",
        0xff13 => "pause",
        0xff14 => "scroll_lock",
        0xff61 => "print",
        0xff67 => "menu",
        0x2d => "minus",
        0x3d => "equal",
        0x2c => "comma",
        0x2e => "period",
        0x2f => "slash",
        0x3b => "semicolon",
        0x27 => "apostrophe",
        0x60 => "grave",
        0xa7 => "section",
        0x30..=0x39 | 0x61..=0x7a => return char::from_u32(sym).map(|c| c.to_string()),
        0x41..=0x5a => return char::from_u32(sym + 0x20).map(|c| c.to_string()),
        0xffbe..=0xffd5 => return Some(format!("f{}", sym - 0xffbe + 1)),
        _ => return None,
    };
    Some(name.to_string())
}

/// The key combination pressed in this frame, as `ctrl+alt+d`; `escape`
/// when Esc was pressed.
pub fn capture_shortcut(keys: &[Key]) -> Option<String> {
    for k in keys {
        if k.base == 0xff1b || k.sym == 0xff1b {
            return Some("escape".to_string());
        }
        let Some(name) = key_name(k.base) else {
            continue;
        };
        let mut parts = Vec::new();
        if k.sup {
            parts.push("super".to_string());
        }
        if k.ctrl {
            parts.push("ctrl".to_string());
        }
        if k.alt {
            parts.push("alt".to_string());
        }
        if k.shift {
            parts.push("shift".to_string());
        }
        parts.push(name);
        return Some(parts.join("+"));
    }
    None
}

/// Local date and time for a Unix timestamp.
pub fn when(time: u64) -> String {
    let t = time as libc::time_t;
    // SAFETY: localtime_r writes into our zeroed struct.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(base: u32, ctrl: bool, alt: bool, shift: bool, sup: bool) -> Key {
        Key {
            sym: base,
            base,
            ctrl,
            alt,
            shift,
            sup,
            text: None,
        }
    }

    #[test]
    fn recorded_shortcuts_parse() {
        let spec = capture_shortcut(&[key(0x64, true, true, false, false)]).unwrap();
        assert_eq!(spec, "ctrl+alt+d");
        assert!(crate::hotkey::parse(&spec).is_ok());
        let spec = capture_shortcut(&[key(0x20, false, true, false, true)]).unwrap();
        assert_eq!(spec, "super+alt+space");
        assert!(crate::hotkey::parse(&spec).is_ok());
        assert_eq!(
            capture_shortcut(&[key(0xffbf, false, false, true, false)]).as_deref(),
            Some("shift+f2")
        );
        assert_eq!(
            capture_shortcut(&[key(0xff1b, false, false, false, false)]).as_deref(),
            Some("escape")
        );
        // A modifier alone is not a shortcut.
        assert_eq!(
            capture_shortcut(&[key(0xffe3, true, false, false, false)]),
            None
        );
    }

    #[test]
    fn the_icon_is_a_disc() {
        let (w, h, px) = icon();
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert_eq!(px[3], 0, "transparent corner");
        assert_eq!(px[((32 * w + 32) * 4 + 3) as usize], 255);
    }
}
