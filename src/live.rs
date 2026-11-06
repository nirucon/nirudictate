//! Live state for status bars and scripts.
//!
//! `$XDG_RUNTIME_DIR/nirudictate/state.json` is replaced atomically on every
//! change (never on a timer):
//!
//! ```json
//! {"version":1,"state":"recording","pid":1234,"pid_start":5678,"since":1793900000.5,"language":"sv"}
//! ```
//!
//! `state` is `idle`, `recording` or `transcribing`. A reader treats the file
//! as idle when the process `pid` with start time `pid_start` (field 22 of
//! /proc/PID/stat) is gone. The root window property `_NIRUDICTATE_STATE`
//! carries the same state word, so an X11 bar that watches root properties
//! repaints at once without polling.
use crate::config::{atomic_write, ensure_private_dir, live_state_path, runtime_dir};

pub fn pid_start(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|text| {
            let close = text.rfind(')')?;
            text[close + 1..].split_whitespace().nth(19)?.parse().ok()
        })
        .unwrap_or(0)
}

pub fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn write(state: &str, language: &str) {
    let pid = std::process::id();
    let body = serde_json::json!({
        "version": 1,
        "state": state,
        "pid": pid,
        "pid_start": pid_start(pid),
        "since": now_f64(),
        "language": language,
    });
    let _ = ensure_private_dir(&runtime_dir());
    let _ = atomic_write(&live_state_path(), body.to_string().as_bytes(), 0o644);
}

pub fn remove() {
    let _ = std::fs::remove_file(live_state_path());
}

/// The state word a reader should show, with the liveness check.
pub fn read() -> String {
    let Ok(text) = std::fs::read_to_string(live_state_path()) else {
        return "stopped".into();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return "stopped".into();
    };
    let pid = v.get("pid").and_then(|p| p.as_u64()).unwrap_or(0) as u32;
    let start = v.get("pid_start").and_then(|p| p.as_u64()).unwrap_or(0);
    if pid == 0 || pid_start(pid) == 0 || (start != 0 && pid_start(pid) != start) {
        return "stopped".into();
    }
    v.get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("idle")
        .to_string()
}
