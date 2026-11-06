//! Speech models: the catalog, what is installed, downloads with progress and
//! checksum verification, and which model this computer should use.
//!
//! Every URL is pinned to a Hugging Face commit and every file to its SHA-256,
//! so a download is exactly the file that was tested.
use crate::config::models_dir;
use crate::hardware::Hardware;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// KBLab KB-Whisper, fine-tuned on 50 000 hours of Swedish. Apache-2.0.
    KbWhisper,
    /// OpenAI Whisper converted by whisper.cpp. MIT.
    Whisper,
}

#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub language: &'static str,
    pub label: &'static str,
    pub family: Family,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// Relative compute cost, tiny = 1 (measured; encoder dominated).
    pub cost: f32,
    /// Memory while loaded, in MB (approximate).
    pub memory_mb: u32,
}

macro_rules! url {
    ($base:ident, $file:literal) => {
        concat!(url_base!($base), $file)
    };
}
/// SV: KB-Whisper converted to 8-bit (q8_0) with whisper.cpp 1.8.3's
/// `whisper-quantize` from KBLab's own ggml files (Hugging Face
/// KBLab/kb-whisper-*, Apache-2.0), published as the `models-sv-1` release of
/// this repository; see docs/MODELS.md for provenance.
/// EN: whisper.cpp's converted OpenAI models, pinned to a Hugging Face commit.
macro_rules! url_base {
    (SV) => {
        "https://github.com/nirucon/nirudictate/releases/download/models-sv-1/"
    };
    (EN) => {
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/"
    };
}

pub const CATALOG: &[Model] = &[
    Model {
        id: "sv-tiny",
        language: "sv",
        label: "KB-Whisper Tiny",
        family: Family::KbWhisper,
        url: url!(SV, "kb-whisper-tiny-q8_0.bin"),
        bytes: 43_537_450,
        sha256: "c16fb9e07e9287d931d0c0d9a6683367dc8683eac8df34a0b89bb4ec98893fb0",
        cost: 1.0,
        memory_mb: 120,
    },
    Model {
        id: "sv-base",
        language: "sv",
        label: "KB-Whisper Base",
        family: Family::KbWhisper,
        url: url!(SV, "kb-whisper-base-q8_0.bin"),
        bytes: 81_768_602,
        sha256: "2027be2ad0a60dcb3fa389889669e3b6f5d1e45df7d7a39f090055ad0ec5de30",
        cost: 2.0,
        memory_mb: 200,
    },
    Model {
        id: "sv-small",
        language: "sv",
        label: "KB-Whisper Small",
        family: Family::KbWhisper,
        url: url!(SV, "kb-whisper-small-q8_0.bin"),
        bytes: 264_464_624,
        sha256: "0073fd16fca4ec90e37703c07a20f717f536ef718a7fcb050dbf3979b18255a3",
        cost: 6.5,
        memory_mb: 420,
    },
    Model {
        id: "sv-medium",
        language: "sv",
        label: "KB-Whisper Medium",
        family: Family::KbWhisper,
        url: url!(SV, "kb-whisper-medium-q8_0.bin"),
        bytes: 823_369_796,
        sha256: "1dd1ace68932cb3f3647882dd572ade251dbc6e4d8e96363e8979fcd434ca358",
        cost: 19.0,
        memory_mb: 1100,
    },
    Model {
        id: "sv-large",
        language: "sv",
        label: "KB-Whisper Large",
        family: Family::KbWhisper,
        url: url!(SV, "kb-whisper-large-q8_0.bin"),
        bytes: 1_656_538_283,
        sha256: "339d086f52e4f5752647ce6bf67cc02cebf197703a5ce4cc2f4252b0c47b38d8",
        cost: 38.0,
        memory_mb: 2100,
    },
    Model {
        id: "en-tiny",
        language: "en",
        label: "Whisper Tiny (English)",
        family: Family::Whisper,
        url: url!(EN, "ggml-tiny.en-q8_0.bin"),
        bytes: 43_550_795,
        sha256: "5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94",
        cost: 1.0,
        memory_mb: 120,
    },
    Model {
        id: "en-base",
        language: "en",
        label: "Whisper Base (English)",
        family: Family::Whisper,
        url: url!(EN, "ggml-base.en-q8_0.bin"),
        bytes: 81_781_811,
        sha256: "a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e",
        cost: 2.0,
        memory_mb: 200,
    },
    Model {
        id: "en-small",
        language: "en",
        label: "Whisper Small (English)",
        family: Family::Whisper,
        url: url!(EN, "ggml-small.en-q8_0.bin"),
        bytes: 264_477_561,
        sha256: "67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067",
        cost: 6.5,
        memory_mb: 420,
    },
    Model {
        id: "en-medium",
        language: "en",
        label: "Whisper Medium (English)",
        family: Family::Whisper,
        url: url!(EN, "ggml-medium.en-q8_0.bin"),
        bytes: 823_382_461,
        sha256: "43fa2cd084de5a04399a896a9a7a786064e221365c01700cea4666005218f11c",
        cost: 19.0,
        memory_mb: 1100,
    },
    Model {
        id: "en-turbo",
        language: "en",
        label: "Whisper Large v3 Turbo (multilingual)",
        family: Family::Whisper,
        url: url!(EN, "ggml-large-v3-turbo-q8_0.bin"),
        bytes: 874_188_075,
        sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
        cost: 22.0,
        memory_mb: 1300,
    },
];

pub fn find(id: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn for_language(language: &str) -> impl Iterator<Item = &'static Model> + '_ {
    CATALOG.iter().filter(move |m| m.language == language)
}

impl Model {
    pub fn url(&self) -> &'static str {
        self.url
    }

    pub fn path(&self) -> PathBuf {
        models_dir().join(format!("{}.bin", self.id))
    }

    fn part_path(&self) -> PathBuf {
        models_dir().join(format!("{}.bin.part", self.id))
    }

    /// Installed: the file exists with exactly the expected size. Its content
    /// was verified when it was downloaded; `verify` checks it again.
    pub fn installed(&self) -> bool {
        fs::metadata(self.path())
            .map(|m| m.is_file() && m.len() == self.bytes)
            .unwrap_or(false)
    }

    pub fn license(&self) -> &'static str {
        match self.family {
            Family::KbWhisper => "Apache-2.0 (KBLab, National Library of Sweden)",
            Family::Whisper => "MIT (OpenAI Whisper, converted by whisper.cpp)",
        }
    }

    /// Initial prompt language hint passed to whisper.
    pub fn whisper_language(&self) -> &'static str {
        self.language
    }
}

pub fn human_size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.2} GB", mb / 1000.0)
    } else {
        format!("{mb:.0} MB")
    }
}

pub fn installed() -> Vec<&'static Model> {
    CATALOG.iter().filter(|m| m.installed()).collect()
}

/// Progress of a running download.
#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub bytes_per_second: f64,
    pub verifying: bool,
}

fn sha256_file(path: &PathBuf, cancel: &AtomicBool) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Check an installed model against its pinned checksum.
pub fn verify(model: &Model) -> Result<(), String> {
    let path = model.path();
    let got = sha256_file(&path, &AtomicBool::new(false))?;
    if got != model.sha256 {
        return Err(format!(
            "{} does not match its checksum (got {got}); delete and download it again",
            path.display()
        ));
    }
    Ok(())
}

/// Download (or resume) a model, verify its SHA-256 and move it into place.
/// `progress` is called about ten times per second.
pub fn download(
    model: &Model,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<(), String> {
    if model.installed() {
        return Ok(());
    }
    let dir = models_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let part = model.part_path();
    // One download per model at a time, across processes.
    let lock = dir.join(format!(".{}.lock", model.id));
    let _guard = DownloadLock::take(&lock)?;

    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > model.bytes {
        let _ = fs::remove_file(&part);
        have = 0;
    }
    if have < model.bytes {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .user_agent(concat!("nirudictate/", env!("CARGO_PKG_VERSION")))
            .build();
        let mut request = agent.get(model.url());
        if have > 0 {
            request = request.set("Range", &format!("bytes={have}-"));
        }
        let response = request
            .call()
            .map_err(|e| format!("download failed: {e}"))?;
        if have > 0 && response.status() != 206 {
            // The server ignored the range: start over.
            have = 0;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .map_err(|e| format!("{}: {e}", part.display()))?;
        let mut reader = response.into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let started = Instant::now();
        let start_have = have;
        let mut last = Instant::now() - Duration::from_secs(1);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("{}: {e}", part.display()))?;
            have += n as u64;
            if have > model.bytes {
                return Err("the server sent more data than expected".into());
            }
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                let secs = started.elapsed().as_secs_f64().max(0.001);
                progress(Progress {
                    done: have,
                    total: model.bytes,
                    bytes_per_second: (have - start_have) as f64 / secs,
                    verifying: false,
                });
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        if have != model.bytes {
            return Err(format!(
                "download incomplete ({} of {}); run it again to resume",
                human_size(have),
                human_size(model.bytes)
            ));
        }
    }
    progress(Progress {
        done: model.bytes,
        total: model.bytes,
        bytes_per_second: 0.0,
        verifying: true,
    });
    let got = sha256_file(&part, cancel)?;
    if got != model.sha256 {
        let _ = fs::remove_file(&part);
        return Err(format!(
            "checksum mismatch for {} (expected {}, got {got}); the file was removed",
            model.id, model.sha256
        ));
    }
    fs::rename(&part, model.path()).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(
        model.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o644),
    );
    Ok(())
}

pub fn delete(model: &Model) -> Result<(), String> {
    for path in [model.path(), model.part_path()] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(())
}

struct DownloadLock(PathBuf);

impl DownloadLock {
    fn take(path: &PathBuf) -> Result<Self, String> {
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(Self(path.clone()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pid: i32 = fs::read_to_string(path)
                        .ok()
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0);
                    // SAFETY: signal 0 only checks that the process exists.
                    if pid > 0 && unsafe { libc::kill(pid, 0) } == 0 {
                        return Err("this model is already being downloaded".into());
                    }
                    let _ = fs::remove_file(path);
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err("could not lock the download".into())
    }
}

impl Drop for DownloadLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Seconds the reference model (`tiny`, cost 1) needs for one dictation of up
/// to 30 s on this computer, estimated from the hardware. Measured on an
/// Intel Xeon W-1250 (6 cores, AVX2) with 5 threads; see docs/MEASUREMENTS.md.
pub fn estimated_tiny_seconds(hw: &Hardware) -> f32 {
    let threads = crate::hardware::threads_for(hw) as f32;
    let reference = TINY_SECONDS_REFERENCE * (5.0 / threads).powf(0.85);
    if hw.avx2 {
        reference
    } else {
        reference * 2.8
    }
}

/// The measured time of the reference: KB-Whisper Tiny (q8_0) per dictation
/// on othala (Xeon W-1250, 5 threads, nice 10, desktop in use).
pub const TINY_SECONDS_REFERENCE: f32 = 0.47;

/// How long a dictation may take to come back after you stop speaking.
pub const FAST_ENOUGH_SECONDS: f32 = 3.5;

/// Estimated seconds for `model` on 10 s of speech. A measurement from this
/// computer wins over the hardware estimate.
pub fn estimate(model: &Model, hw: &Hardware, measured_tiny: Option<f32>) -> f32 {
    measured_tiny.unwrap_or_else(|| estimated_tiny_seconds(hw)) * model.cost
}

/// The largest model for `language` that is fast enough here and fits in
/// memory comfortably.
pub fn recommend(language: &str, hw: &Hardware, measured_tiny: Option<f32>) -> &'static Model {
    let mut best = None;
    for m in for_language(language) {
        let fits = hw.memory_mb == 0 || u64::from(m.memory_mb) * 4 <= hw.memory_mb;
        if fits && estimate(m, hw, measured_tiny) <= FAST_ENOUGH_SECONDS {
            best = Some(m);
        }
    }
    best.unwrap_or_else(|| {
        for_language(language)
            .next()
            .expect("catalog has the language")
    })
}

/// The model the service uses for a language: the configured one when it is
/// installed, otherwise the best installed one (largest that is fast enough,
/// else the smallest installed).
pub fn active(language: &str, configured: &str, hw: &Hardware) -> Option<&'static Model> {
    if let Some(m) = find(configured).filter(|m| m.installed() && m.language == language) {
        return Some(m);
    }
    let installed: Vec<_> = for_language(language).filter(|m| m.installed()).collect();
    let recommended = recommend(language, hw, None);
    installed
        .iter()
        .rev()
        .find(|m| m.cost <= recommended.cost)
        .or_else(|| installed.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        let mut ids = std::collections::HashSet::new();
        for m in CATALOG {
            assert!(ids.insert(m.id), "duplicate id {}", m.id);
            assert_eq!(m.sha256.len(), 64);
            assert!(m.url().starts_with("https://"));
            assert!(m.url().ends_with(".bin"));
            assert!(m.id.starts_with(m.language));
        }
        assert!(for_language("sv").count() >= 3);
        assert!(for_language("en").count() >= 3);
    }

    fn machine(physical: u32, avx2: bool, memory_mb: u64) -> Hardware {
        Hardware {
            model: "test".into(),
            logical: physical * 2,
            physical,
            avx2,
            memory_mb,
        }
    }

    #[test]
    fn faster_machines_get_larger_models() {
        let slow = recommend("sv", &machine(2, false, 4000), None);
        let mid = recommend("sv", &machine(4, true, 8000), None);
        let fast = recommend("sv", &machine(6, true, 32000), None);
        assert!(slow.cost <= mid.cost && mid.cost <= fast.cost);
        assert!(
            fast.cost >= 6.0,
            "a 6-core AVX2 desktop handles small or more"
        );
    }

    #[test]
    fn a_measurement_overrides_the_estimate() {
        let hw = machine(6, true, 32000);
        assert_eq!(recommend("sv", &hw, Some(1.0)).id, "sv-base");
        assert_eq!(recommend("sv", &hw, Some(0.05)).id, "sv-large");
    }

    #[test]
    fn memory_limits_the_recommendation() {
        let hw = machine(16, true, 3000);
        assert!(recommend("sv", &hw, Some(0.01)).memory_mb * 4 <= 3000);
    }
}
