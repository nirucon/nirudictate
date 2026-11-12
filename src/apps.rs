//! Installed programs, for whisper's prompt in command mode.
//!
//! The names come from the XDG `.desktop` files (the same index the
//! launcher uses). Only the programs the user actually starts from
//! NIRULAUNCHER (`~/.local/state/nirulauncher/usage.tsv`) go into the
//! prompt, most used first, so it stays short; opening a program by voice
//! still works for every installed one, because the launcher does the
//! ranking.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![crate::config::data_home().join("applications")];
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|d| d.starts_with('/')) {
        dirs.push(Path::new(d).join("applications"));
    }
    dirs.push(crate::config::data_home().join("flatpak/exports/share/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

/// `Name=` of a visible application entry, or None.
pub fn entry_name(text: &str) -> Option<String> {
    let mut in_entry = false;
    let mut name = None;
    let mut application = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if in_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (key.trim(), value.trim()) {
            ("Name", v) if !v.is_empty() => name = Some(v.to_string()),
            ("Type", v) => application = v == "Application",
            ("NoDisplay" | "Hidden", "true") => return None,
            _ => {}
        }
    }
    name.filter(|_| application)
}

/// Desktop id (`org.gnome.Foo.desktop`, subfolders joined by `-`) → name.
pub fn installed() -> HashMap<String, String> {
    let mut out = HashMap::new();
    for dir in application_dirs() {
        scan(&dir, &dir, &mut out);
    }
    out
}

fn scan(root: &Path, dir: &Path, out: &mut HashMap<String, String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(root, &path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let id = rel.to_string_lossy().replace('/', "-");
        // The first directory wins, like the XDG lookup order.
        if out.contains_key(&id) {
            continue;
        }
        let name = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| entry_name(&t));
        if let Some(name) = name {
            out.insert(id, name);
        }
    }
}

/// Launch counts per desktop id from NIRULAUNCHER's history.
pub fn usage_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for line in text.lines() {
        let key = line.split('\t').next().unwrap_or("");
        if key.ends_with(".desktop") {
            *counts.entry(key.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

/// Up to `limit` program names for the prompt: the most launched first,
/// short names only, each once.
pub fn prompt_names(limit: usize) -> Vec<String> {
    let usage = std::fs::read_to_string(crate::config::state_home().join("nirulauncher/usage.tsv"))
        .unwrap_or_default();
    pick(&installed(), &usage_counts(&usage), limit)
}

pub fn pick(
    installed: &HashMap<String, String>,
    counts: &HashMap<String, usize>,
    limit: usize,
) -> Vec<String> {
    let mut used: Vec<(&String, usize)> = counts
        .iter()
        .filter(|(id, _)| installed.contains_key(*id))
        .map(|(id, n)| (id, *n))
        .collect();
    used.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut names: Vec<String> = Vec::new();
    for (id, _) in used {
        let name = &installed[id];
        if name.chars().count() > 28 || name.split_whitespace().count() > 3 {
            continue;
        }
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            names.push(name.clone());
        }
        if names.len() == limit {
            break;
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_visible_application_names() {
        let entry =
            "[Desktop Entry]\nType=Application\nName=Spotify\nName[sv]=Spotify SV\nExec=spotify\n";
        assert_eq!(entry_name(entry).as_deref(), Some("Spotify"));
        assert_eq!(
            entry_name("[Desktop Entry]\nType=Application\nName=X\nNoDisplay=true\n"),
            None
        );
        assert_eq!(entry_name("[Desktop Entry]\nType=Link\nName=Docs\n"), None);
        // Actions after the entry do not override its name.
        assert_eq!(
            entry_name("[Desktop Entry]\nType=Application\nName=Firefox\n[Desktop Action new]\nName=New Window\n")
                .as_deref(),
            Some("Firefox")
        );
    }

    #[test]
    fn the_prompt_lists_used_programs_first() {
        let installed: HashMap<String, String> = [
            ("spotify.desktop", "Spotify"),
            ("gimp.desktop", "GNU Image Manipulation Program"),
            ("thunderbird.desktop", "Thunderbird"),
            ("steam.desktop", "Steam"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let usage = "thunderbird.desktop\t1\nspotify.desktop\t2\nspotify.desktop\t3\naction:theme:x\t4\ngimp.desktop\t5\ngone.desktop\t6\n";
        let counts = usage_counts(usage);
        assert_eq!(counts.get("spotify.desktop"), Some(&2));
        assert!(!counts.contains_key("action:theme:x"));
        assert_eq!(pick(&installed, &counts, 10), ["Spotify", "Thunderbird"]);
        assert_eq!(pick(&installed, &counts, 1), ["Spotify"]);
        assert!(pick(&installed, &HashMap::new(), 10).is_empty());
    }
}
