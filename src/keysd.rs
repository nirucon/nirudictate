//! Register the shortcuts in NIRUWM's cheat sheet:
//! `~/.local/share/niru/keys.d/nirudictate.toml` (`$XDG_DATA_HOME`).
//!
//! The service writes the file from the keys it actually uses each time it
//! (re)grabs them, so the sheet follows the settings. `uninstall.sh` removes
//! it. The key syntax is NIRUWM's `[[keys]]` syntax; a key NIRUWM cannot
//! name (Insert, Home, …) is left out rather than making the file invalid.
use crate::config::{self, Config};
use crate::hotkey;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    config::data_home().join("niru/keys.d/nirudictate.toml")
}

/// A shortcut in NIRUWM's syntax (`ctrl+alt+space`, `super+F9`), or `None`
/// when it does not parse or NIRUWM has no name for the key.
pub fn niruwm_keys(spec: &str) -> Option<String> {
    let parsed = hotkey::parse(spec).ok()?;
    let mut parts: Vec<String> = Vec::new();
    // Modifiers in the order they were written, each once.
    for m in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let name = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "ctrl",
            "alt" | "mod1" => "alt",
            "shift" => "shift",
            "super" | "mod4" | "win" => "super",
            _ => continue,
        };
        if !parts.iter().any(|p| p == name) {
            parts.push(name.to_string());
        }
    }
    let key = match parsed.keysym {
        0x20 => "space".to_string(),
        0xff0d => "Return".to_string(),
        0xff09 => "Tab".to_string(),
        0xff1b => "Escape".to_string(),
        0xff61 => "Print".to_string(),
        0x60 => "grave".to_string(),
        0x2d => "minus".to_string(),
        0x3d => "equal".to_string(),
        0x2c => "comma".to_string(),
        0x2e => "period".to_string(),
        0x2f => "slash".to_string(),
        0x3b => ";".to_string(),
        0x27 => "'".to_string(),
        0xa7 => "§".to_string(),
        0x1008ffb2 => "XF86AudioMicMute".to_string(),
        k @ 0xffbe..=0xffd5 => format!("F{}", k - 0xffbe + 1),
        k if (0x30..=0x39).contains(&k) || (0x61..=0x7a).contains(&k) => {
            char::from_u32(k)?.to_string()
        }
        _ => return None,
    };
    parts.push(key);
    Some(parts.join("+"))
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The file's text. `agent` is whether the agent shortcut is in use
/// (enabled and opencode installed).
pub fn render(cfg: &Config, agent: bool) -> String {
    let mut out = String::from(
        "# Written by NIRUDICTATE from its settings each time it starts.\n\
         # Edits are overwritten; change the keys in `nirudictate settings`.\n\
         app = \"NIRUDICTATE\"\ngroup = \"Dictation\"\n",
    );
    let mut add = |spec: &str, description: &str, run: &[&str]| {
        if let Some(keys) = niruwm_keys(spec) {
            let run: Vec<String> = run.iter().map(|a| quote(a)).collect();
            out.push_str(&format!(
                "\n[[keys]]\nkeys = {}\ndescription = {}\nrun = [{}]\n",
                quote(&keys),
                quote(description),
                run.join(", ")
            ));
        }
    };
    add(
        &cfg.shortcut.key,
        "Dictate: tap to start and stop, hold to talk",
        &["nirudictate", "toggle"],
    );
    if agent {
        add(
            &cfg.agent.key,
            "Dictate to the opencode agent",
            &["nirudictate", "agent", "toggle"],
        );
    }
    out
}

/// Write the file when its content changed. Errors are returned in words;
/// the caller only logs them.
pub fn write(cfg: &Config, agent: bool) -> Result<(), String> {
    let path = path();
    let text = render(cfg, agent);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    config::atomic_write(&path, text.as_bytes(), 0o644)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_use_niruwm_syntax() {
        assert_eq!(
            niruwm_keys("ctrl+alt+space").as_deref(),
            Some("ctrl+alt+space")
        );
        assert_eq!(
            niruwm_keys("Control + Mod1 + Space").as_deref(),
            Some("ctrl+alt+space")
        );
        assert_eq!(niruwm_keys("win+f9").as_deref(), Some("super+F9"));
        assert_eq!(niruwm_keys("f12").as_deref(), Some("F12"));
        assert_eq!(niruwm_keys("super+D").as_deref(), Some("super+d"));
        assert_eq!(niruwm_keys("ctrl+section").as_deref(), Some("ctrl+§"));
        assert_eq!(
            niruwm_keys("xf86audiomicmute").as_deref(),
            Some("XF86AudioMicMute")
        );
        assert_eq!(niruwm_keys("ctrl+return").as_deref(), Some("ctrl+Return"));
        // NIRUWM has no name for these: left out, never an invalid file.
        assert_eq!(niruwm_keys("ctrl+insert"), None);
        assert_eq!(niruwm_keys("nonsense+x"), None);
    }

    #[test]
    fn the_file_follows_the_settings() {
        let mut cfg = Config::default();
        let text = render(&cfg, true);
        let v: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(v["app"].as_str(), Some("NIRUDICTATE"));
        assert_eq!(v["group"].as_str(), Some("Dictation"));
        let keys = v["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0]["keys"].as_str(), Some("ctrl+alt+space"));
        assert_eq!(keys[1]["keys"].as_str(), Some("shift+alt+space"));
        assert_eq!(
            keys[1]["run"].as_array().unwrap().len(),
            3,
            "run is an argv list"
        );
        for k in keys {
            assert!(!k["description"].as_str().unwrap().is_empty());
            let table = k.as_table().unwrap();
            for field in table.keys() {
                assert!(
                    ["keys", "description", "run"].contains(&field.as_str()),
                    "NIRUWM rejects unknown fields: {field}"
                );
            }
        }

        cfg.shortcut.key = "super+f9".into();
        let v: toml::Value = toml::from_str(&render(&cfg, false)).unwrap();
        let keys = v["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 1, "no agent key when agent mode is off");
        assert_eq!(keys[0]["keys"].as_str(), Some("super+F9"));

        cfg.shortcut.key = "ctrl+insert".into();
        let v: toml::Value = toml::from_str(&render(&cfg, false)).unwrap();
        assert!(v.get("keys").is_none(), "an unnamed key is left out");
    }

    #[test]
    fn write_creates_and_keeps_the_file() {
        let dir = std::env::temp_dir().join(format!("nirudictate-keysd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("niru/keys.d/nirudictate.toml");
        let text = render(&Config::default(), true);
        config::atomic_write(&path, text.as_bytes(), 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
