//! The control socket: one JSON request line, one JSON reply line.
//!
//! `$XDG_RUNTIME_DIR/nirudictate/control.sock`, mode 0600 in a 0700
//! directory. Used by the CLI, the settings panel and scripts.
use crate::config::socket_path;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    Status,
    Start,
    Stop,
    Toggle,
    Cancel,
    Reload,
    Unload,
    Language {
        language: String,
    },
    /// Transcribe a WAV file without inserting it (tests and benchmarks).
    TranscribeFile {
        path: String,
        model: Option<String>,
    },
    Quit,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LastResult {
    pub text: String,
    pub time: u64,
    pub model: String,
    pub audio_seconds: f32,
    pub seconds: f32,
    pub inserted_by: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub version: String,
    /// idle, recording, transcribing
    pub state: String,
    pub pid: u32,
    pub language: String,
    /// The model a dictation would use now, if one is installed.
    pub model: Option<String>,
    /// The model currently in memory.
    pub loaded: Option<String>,
    pub recording_seconds: f32,
    pub shortcut: String,
    pub shortcut_active: bool,
    pub mode: String,
    pub audio: String,
    pub engine: String,
    pub last: Option<LastResult>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<crate::engine::Transcript>,
}

impl Reply {
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            ..Self::default()
        }
    }
}

/// Send one request to the running service.
pub fn send(request: &Request, timeout: Duration) -> Result<Reply, String> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|_| {
        "the NIRUDICTATE service is not running (start it with `nirudictate daemon`)".to_string()
    })?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let mut line = serde_json::to_string(request).map_err(|e| e.to_string())?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("cannot talk to the service: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut answer = String::new();
    reader
        .read_line(&mut answer)
        .map_err(|e| format!("no answer from the service: {e}"))?;
    serde_json::from_str(answer.trim()).map_err(|e| format!("bad answer from the service: {e}"))
}

pub fn running() -> bool {
    send(&Request::Status, Duration::from_secs(1)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_have_a_stable_wire_form() {
        assert_eq!(
            serde_json::to_string(&Request::Status).unwrap(),
            r#"{"cmd":"status"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(
                r#"{"cmd":"transcribe-file","path":"/x.wav","model":null}"#
            )
            .unwrap(),
            Request::TranscribeFile {
                path: "/x.wav".into(),
                model: None
            }
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"language","language":"en"}"#).unwrap(),
            Request::Language {
                language: "en".into()
            }
        );
    }
}
