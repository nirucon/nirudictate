//! Local dictation history: one JSON object per line in
//! `~/.local/state/nirudictate/history.jsonl`, readable only by the user
//! (mode 0600), trimmed to the configured number of entries. Audio is never
//! stored.
use crate::config::{atomic_write, ensure_private_dir, history_path, state_dir};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Unix time in seconds.
    pub time: u64,
    pub text: String,
    pub language: String,
    pub model: String,
    pub audio_seconds: f32,
    pub seconds: f32,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn load() -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(history_path()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn write_all(entries: &[Entry]) -> Result<(), String> {
    let mut out = String::new();
    for e in entries {
        out.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    atomic_write(&history_path(), out.as_bytes(), 0o600)
}

/// Append one entry, then trim to `keep` entries.
pub fn append(entry: &Entry, keep: usize) -> Result<(), String> {
    if keep == 0 {
        return clear();
    }
    ensure_private_dir(&state_dir())?;
    let path = history_path();
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| e.to_string())?;
    }
    let entries = load();
    if entries.len() > keep {
        write_all(&entries[entries.len() - keep..])?;
    }
    Ok(())
}

pub fn trim(keep: usize) -> Result<(), String> {
    let entries = load();
    if entries.len() > keep {
        write_all(&entries[entries.len() - keep..])?;
    }
    Ok(())
}

pub fn clear() -> Result<(), String> {
    match fs::remove_file(history_path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete(time: u64, text: &str) -> Result<(), String> {
    let entries: Vec<Entry> = load()
        .into_iter()
        .filter(|e| !(e.time == time && e.text == text))
        .collect();
    write_all(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_trims_and_is_private() {
        let _g = crate::session::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("nirudictate-hist-{}", std::process::id()));
        std::env::set_var("XDG_STATE_HOME", &dir);
        let e = |n: u64| Entry {
            time: n,
            text: format!("rad {n} åäö"),
            language: "sv".into(),
            model: "sv-small".into(),
            audio_seconds: 2.0,
            seconds: 0.5,
        };
        for n in 0..5 {
            append(&e(n), 3).unwrap();
        }
        let got = load();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].time, 2);
        assert_eq!(got[2].text, "rad 4 åäö");
        let mode = fs::metadata(history_path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        delete(3, "rad 3 åäö").unwrap();
        assert_eq!(load().len(), 2);
        clear().unwrap();
        assert!(load().is_empty());
        std::env::remove_var("XDG_STATE_HOME");
        let _ = fs::remove_dir_all(&dir);
    }
}
