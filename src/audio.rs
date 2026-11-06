//! Microphone capture. The microphone is opened only while recording: a
//! `pw-record` (PipeWire), `parec` (PulseAudio) or `arecord` (ALSA) child
//! delivers 16 kHz mono 16-bit samples on a pipe and exits when the recording
//! ends. No audio library is linked and nothing listens while idle.
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const RATE: u32 = 16_000;
/// Level/VAD frame: 20 ms.
const FRAME: usize = (RATE as usize) / 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    PipeWire,
    Pulse,
    Alsa,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::PipeWire => "PipeWire (pw-record)",
            Backend::Pulse => "PulseAudio (parec)",
            Backend::Alsa => "ALSA (arecord)",
        }
    }
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let p = dir.join(program);
                p.is_file()
                    && std::fs::metadata(&p)
                        .map(|m| {
                            std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0
                        })
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn backend() -> Option<Backend> {
    if which("pw-record") {
        Some(Backend::PipeWire)
    } else if which("parec") {
        Some(Backend::Pulse)
    } else if which("arecord") {
        Some(Backend::Alsa)
    } else {
        None
    }
}

fn command(backend: Backend, device: &str) -> Command {
    let mut cmd = match backend {
        Backend::PipeWire => {
            let mut c = Command::new("pw-record");
            c.args(["--rate", "16000", "--channels", "1", "--format", "s16"]);
            c.args(["--latency", "20ms"]);
            c.args(["-P", "{ media.role = \"Communication\" application.name = \"NIRUDICTATE\" node.description = \"NIRUDICTATE\" }"]);
            if !device.is_empty() {
                c.args(["--target", device]);
            }
            c.arg("-");
            c
        }
        Backend::Pulse => {
            let mut c = Command::new("parec");
            c.args([
                "--rate=16000",
                "--channels=1",
                "--format=s16le",
                "--latency-msec=20",
                "--raw",
                "--client-name=NIRUDICTATE",
            ]);
            if !device.is_empty() {
                c.arg(format!("--device={device}"));
            }
            c
        }
        Backend::Alsa => {
            let mut c = Command::new("arecord");
            c.args(["-q", "-f", "S16_LE", "-r", "16000", "-c", "1", "-t", "raw"]);
            if !device.is_empty() {
                c.args(["-D", device]);
            }
            c
        }
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

/// The data offset of a Sun AU (".snd") header with 16-bit linear PCM.
pub fn au_header(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 24 || &bytes[..4] != b".snd" {
        return None;
    }
    let offset = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    (24..=4096).contains(&offset).then_some(offset)
}

/// Speech/silence decisions on 20 ms frames with an adaptive noise floor.
#[derive(Clone, Debug)]
pub struct Vad {
    floor_db: f32,
    pub speech_frames: u32,
    pub silent_frames: u32,
    pub frames: u32,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            floor_db: -60.0,
            speech_frames: 0,
            silent_frames: 0,
            frames: 0,
        }
    }
}

impl Vad {
    /// Feed one frame's level in dBFS; returns whether it is speech.
    pub fn push(&mut self, db: f32) -> bool {
        self.frames += 1;
        // The floor follows quiet frames quickly and loud ones very slowly.
        if db < self.floor_db {
            self.floor_db = 0.7 * self.floor_db + 0.3 * db;
        } else {
            self.floor_db += 0.02 * (db - self.floor_db).min(6.0);
        }
        self.floor_db = self.floor_db.clamp(-90.0, -30.0);
        let speech = db > -48.0 && db > self.floor_db + 9.0;
        if speech {
            self.speech_frames += 1;
            self.silent_frames = 0;
        } else {
            self.silent_frames += 1;
        }
        speech
    }

    pub fn heard_speech(&self) -> bool {
        // 200 ms of speech in total.
        self.speech_frames >= 10
    }

    pub fn silence(&self) -> Duration {
        Duration::from_millis(u64::from(self.silent_frames) * 20)
    }
}

pub fn level_db(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return -90.0;
    }
    let energy = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
    (10.0 * energy.max(1e-9).log10()).max(-90.0)
}

/// Meter position 0..1 for a dBFS level (-60 dB .. -6 dB).
pub fn meter(db: f32) -> f32 {
    ((db + 60.0) / 54.0).clamp(0.0, 1.0)
}

#[derive(Default)]
pub struct Shared {
    pub samples: Vec<f32>,
    /// Recent meter values (0..1), newest last; the overlay draws these.
    pub levels: Vec<f32>,
    pub vad: Vad,
    pub ended: bool,
    pub error: Option<String>,
}

pub struct Capture {
    child: Child,
    pub shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
    pub started: Instant,
}

impl Capture {
    /// Open the microphone. `wake` is called for every ~60 ms of audio so the
    /// caller can redraw meters and check for silence.
    pub fn start(device: &str, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let backend = backend().ok_or_else(|| {
            "no audio recorder found: install pipewire (pw-record), pulseaudio-utils (parec) or alsa-utils (arecord)".to_string()
        })?;
        let mut child = command(backend, device)
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", backend.name()))?;
        let mut stdout = child.stdout.take().ok_or("recorder has no output")?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, stop2) = (shared.clone(), stop.clone());
        let reader = thread::Builder::new()
            .name("nirudictate-audio".into())
            .spawn(move || {
                let mut raw = vec![0u8; FRAME * 2 * 3];
                let mut carry: Vec<u8> = Vec::new();
                let mut frame: Vec<f32> = Vec::with_capacity(FRAME);
                // pw-record writes a Sun AU header (big-endian samples) to a
                // pipe unless --raw is given; older versions have no --raw.
                let mut header_checked = false;
                let mut big_endian = false;
                loop {
                    let n = match stdout.read(&mut raw) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            if let Ok(mut s) = s2.lock() {
                                s.error = Some(format!("microphone read failed: {e}"));
                            }
                            break;
                        }
                    };
                    carry.extend_from_slice(&raw[..n]);
                    if !header_checked {
                        if carry.len() < 24 {
                            continue;
                        }
                        header_checked = true;
                        if let Some(skip) = au_header(&carry) {
                            big_endian = true;
                            if carry.len() < skip {
                                header_checked = false;
                                continue;
                            }
                            carry.drain(..skip);
                        }
                    }
                    let whole = carry.len() / 2 * 2;
                    let mut new: Vec<f32> = Vec::with_capacity(whole / 2);
                    for pair in carry[..whole].as_chunks::<2>().0 {
                        let v = if big_endian {
                            i16::from_be_bytes([pair[0], pair[1]])
                        } else {
                            i16::from_le_bytes([pair[0], pair[1]])
                        };
                        new.push(v as f32 / 32768.0);
                    }
                    carry.drain(..whole);
                    if let Ok(mut s) = s2.lock() {
                        for v in &new {
                            frame.push(*v);
                            if frame.len() == FRAME {
                                let db = level_db(&frame);
                                s.vad.push(db);
                                s.levels.push(meter(db));
                                if s.levels.len() > 64 {
                                    let extra = s.levels.len() - 64;
                                    s.levels.drain(..extra);
                                }
                                frame.clear();
                            }
                        }
                        s.samples.extend_from_slice(&new);
                    }
                    wake();
                    if stop2.load(Ordering::Relaxed) {
                        break;
                    }
                }
                if let Ok(mut s) = s2.lock() {
                    s.ended = true;
                }
                wake();
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            child,
            shared,
            stop,
            reader: Some(reader),
            started: Instant::now(),
        })
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Close the microphone and return what was recorded.
    pub fn finish(mut self) -> Vec<f32> {
        self.close();
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut s.samples)
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // SAFETY: signalling our own child.
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if self.reader.is_some() {
            self.close();
        }
    }
}

/// Remove leading/trailing silence (keeping a little margin) so whisper does
/// not invent words in the gaps. Returns None when there is no speech at all.
pub fn trim_silence(samples: &[f32]) -> Option<&[f32]> {
    let frames: Vec<f32> = samples.chunks(FRAME).map(level_db).collect();
    let mut vad = Vad::default();
    let speech: Vec<bool> = frames.iter().map(|db| vad.push(*db)).collect();
    if !vad.heard_speech() {
        return None;
    }
    let first = speech.iter().position(|s| *s)?;
    let last = speech.iter().rposition(|s| *s)?;
    let margin = 15; // 300 ms
    let start = first.saturating_sub(margin) * FRAME;
    let end = ((last + 1 + margin) * FRAME).min(samples.len());
    Some(&samples[start..end])
}

/// A microphone the user can choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub description: String,
}

/// Input devices from `pw-dump` (or `pactl`), without monitors of outputs.
pub fn devices() -> Vec<Device> {
    if let Some(list) = pw_devices() {
        return list;
    }
    pactl_devices().unwrap_or_default()
}

fn pw_devices() -> Option<Vec<Device>> {
    let out = Command::new("pw-dump")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(parse_pw_dump(&value))
}

pub fn parse_pw_dump(value: &serde_json::Value) -> Vec<Device> {
    let mut list = Vec::new();
    for obj in value.as_array().into_iter().flatten() {
        if obj.get("type").and_then(|t| t.as_str()) != Some("PipeWire:Interface:Node") {
            continue;
        }
        let Some(props) = obj.pointer("/info/props") else {
            continue;
        };
        if props.get("media.class").and_then(|c| c.as_str()) != Some("Audio/Source") {
            continue;
        }
        let Some(name) = props.get("node.name").and_then(|n| n.as_str()) else {
            continue;
        };
        let description = props
            .get("node.description")
            .or_else(|| props.get("node.nick"))
            .and_then(|d| d.as_str())
            .unwrap_or(name);
        list.push(Device {
            name: name.to_string(),
            description: description.to_string(),
        });
    }
    list
}

fn pactl_devices() -> Option<Vec<Device>> {
    let out = Command::new("pactl")
        .args(["list", "short", "sources"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.lines()
            .filter_map(|l| l.split('\t').nth(1))
            .filter(|n| !n.ends_with(".monitor"))
            .map(|n| Device {
                name: n.to_string(),
                description: n.to_string(),
            })
            .collect(),
    )
}

/// Read a 16-bit PCM WAV file as 16 kHz mono (down-mixed, linearly resampled).
pub fn read_wav(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_wav(&bytes)
}

pub fn parse_wav(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut pos = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = pos + 8;
        let end = (body + size).min(bytes.len());
        if id == b"fmt " && size >= 16 {
            let f = &bytes[body..];
            format = Some((
                u16::from_le_bytes([f[0], f[1]]),
                u16::from_le_bytes([f[2], f[3]]),
                u32::from_le_bytes([f[4], f[5], f[6], f[7]]),
                u16::from_le_bytes([f[14], f[15]]),
            ));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or("WAV data before format")?;
            if !(tag == 1 || tag == 0xfffe) || bits != 16 || channels == 0 {
                return Err("only 16-bit PCM WAV is supported".into());
            }
            let ch = channels as usize;
            let mono: Vec<f32> = bytes[body..end]
                .chunks_exact(2 * ch)
                .map(|frame| {
                    frame
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| i16::from_le_bytes(*s) as f32 / 32768.0)
                        .sum::<f32>()
                        / ch as f32
                })
                .collect();
            return Ok(resample(&mono, rate, RATE));
        }
        pos = body + size + (size & 1);
    }
    Err("WAV file has no data".into())
}

pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (input.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let j = x as usize;
            let t = (x - j as f64) as f32;
            let a = input[j.min(input.len() - 1)];
            let b = input[(j + 1).min(input.len() - 1)];
            a + (b - a) * t
        })
        .collect()
}

pub fn write_wav(path: &std::path::Path, samples: &[f32]) -> Result<(), String> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    crate::config::atomic_write(path, &out, 0o600)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f32, amp: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| amp * (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin())
            .collect()
    }

    #[test]
    fn vad_detects_speech_and_following_silence() {
        let mut vad = Vad::default();
        for _ in 0..50 {
            vad.push(-70.0);
        }
        assert!(!vad.heard_speech());
        for _ in 0..30 {
            vad.push(-20.0);
        }
        assert!(vad.heard_speech());
        for _ in 0..100 {
            vad.push(-68.0);
        }
        assert_eq!(vad.silence(), Duration::from_secs(2));
    }

    #[test]
    fn trimming_keeps_speech_with_a_margin() {
        let mut s = vec![0.0001f32; RATE as usize];
        s.extend(tone(1.0, 0.3));
        s.extend(vec![0.0001f32; RATE as usize * 2]);
        let t = trim_silence(&s).unwrap();
        let secs = t.len() as f32 / RATE as f32;
        assert!((1.4..=1.8).contains(&secs), "trimmed to {secs}");
        assert!(trim_silence(&vec![0.0001f32; RATE as usize]).is_none());
    }

    #[test]
    fn wav_round_trip_and_resample() {
        let dir = std::env::temp_dir().join(format!("nirudictate-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        let s = tone(0.5, 0.5);
        write_wav(&path, &s).unwrap();
        let back = read_wav(&path).unwrap();
        assert_eq!(back.len(), s.len());
        assert!((back[100] - s[100]).abs() < 0.001);
        assert_eq!(resample(&vec![0.0; 48_000], 48_000, 16_000).len(), 16_000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recognises_the_au_header_pw_record_writes_to_pipes() {
        let mut h = b".snd".to_vec();
        h.extend_from_slice(&24u32.to_be_bytes());
        h.extend_from_slice(&[0xff; 4]);
        h.extend_from_slice(&3u32.to_be_bytes());
        h.extend_from_slice(&16000u32.to_be_bytes());
        h.extend_from_slice(&1u32.to_be_bytes());
        assert_eq!(au_header(&h), Some(24));
        assert_eq!(au_header(&[0u8; 24]), None);
    }

    #[test]
    fn pw_dump_lists_sources_only() {
        let v: serde_json::Value = serde_json::json!([
            {"type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Source", "node.name": "mic", "node.description": "USB Mic"}}},
            {"type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink", "node.name": "spk"}}},
            {"type": "PipeWire:Interface:Port"}
        ]);
        assert_eq!(
            parse_pw_dump(&v),
            vec![Device {
                name: "mic".into(),
                description: "USB Mic".into()
            }]
        );
    }
}
