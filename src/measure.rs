//! Speed measured on this computer, used to recommend a model.
//!
//! After every dictation the transcription time divided by the model's
//! relative cost is stored; the median of the last 20 is "seconds a tiny
//! model needs here". Whisper always encodes a 30 s window, so the time per
//! dictation is nearly independent of its length.
use crate::config::{atomic_write, measurements_path};
use crate::engine::Transcript;
use crate::models;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Measurements {
    /// Normalised seconds (tiny equivalent), newest last.
    pub tiny_seconds: Vec<f32>,
    /// Last measured seconds per model id.
    #[serde(default)]
    pub per_model: std::collections::BTreeMap<String, f32>,
}

pub fn load() -> Measurements {
    std::fs::read_to_string(measurements_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn record(t: &Transcript) {
    let Some(model) = models::find(&t.model) else {
        return;
    };
    if t.seconds <= 0.0 {
        return;
    }
    let mut m = load();
    m.tiny_seconds.push(t.seconds / model.cost);
    if m.tiny_seconds.len() > 20 {
        let extra = m.tiny_seconds.len() - 20;
        m.tiny_seconds.drain(..extra);
    }
    m.per_model.insert(t.model.clone(), t.seconds);
    if let Ok(text) = serde_json::to_string_pretty(&m) {
        let _ = atomic_write(&measurements_path(), text.as_bytes(), 0o600);
    }
}

impl Measurements {
    pub fn tiny(&self) -> Option<f32> {
        if self.tiny_seconds.len() < 2 {
            return None;
        }
        let mut v = self.tiny_seconds.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(v[v.len() / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_needs_two_samples() {
        let mut m = Measurements::default();
        assert_eq!(m.tiny(), None);
        m.tiny_seconds = vec![0.3, 0.1, 0.2];
        assert_eq!(m.tiny(), Some(0.2));
    }
}
