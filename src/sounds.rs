//! Dictation sounds through the NIRUX sound system.
//!
//! NIRUX owns the sounds and the policy (`nirux sounds`, category
//! `dictation`: `dictation-start`, `dictation-stop`, `dictation-done`,
//! `dictation-error`, each with its own switch and an override in
//! `~/.config/nirux/sounds/EVENT.ogg`). Without NIRUX nothing plays.
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Start,
    Stop,
    Done,
    Error,
}

impl Cue {
    pub fn event(self) -> &'static str {
        match self {
            Cue::Start => "dictation-start",
            Cue::Stop => "dictation-stop",
            Cue::Done => "dictation-done",
            Cue::Error => "dictation-error",
        }
    }
}

fn nirux() -> Option<std::path::PathBuf> {
    let local = crate::config::home().join(".local/bin/nirux");
    if local.is_file() {
        return Some(local);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("nirux"))
            .find(|p| p.is_file())
    })
}

pub fn available() -> bool {
    nirux().is_some()
}

/// Play a cue without waiting for it.
pub fn play(cue: Cue) {
    let Some(nirux) = nirux() else {
        return;
    };
    let child = Command::new(nirux)
        .args(["sounds", "play", cue.event()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        // Reap it without blocking the caller.
        let _ = std::thread::Builder::new()
            .name("nirudictate-sound".into())
            .spawn(move || {
                let _ = child.wait();
            });
    }
}
