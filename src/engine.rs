//! Transcription with whisper.cpp on one background thread.
//!
//! The model is loaded on first use and dropped again after a quiet period, so
//! an idle service holds no model memory. The thread runs at a low priority
//! (nice, SCHED_BATCH) and whisper.cpp's worker threads inherit that, so music,
//! visualizers and the desktop keep their CPU time while a dictation is
//! transcribed.
use crate::models::Model;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Transcript {
    pub text: String,
    pub model: String,
    pub language: String,
    /// Seconds of audio given to the model.
    pub audio_seconds: f32,
    /// Seconds spent loading the model for this request (0 when it was loaded).
    pub load_seconds: f32,
    /// Seconds spent transcribing.
    pub seconds: f32,
    /// The language whisper detected, when it was asked to (command mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected: Option<String>,
}

pub struct Request {
    pub samples: Vec<f32>,
    pub model: &'static Model,
    pub prompt: String,
    pub threads: u32,
    /// Encode the whole 30 s window (slower, the reference behaviour).
    pub full_window: bool,
    /// Another language than the model's: `auto` detects it (multilingual
    /// models only; command mode).
    pub language: Option<String>,
    pub reply: Sender<Result<Transcript, String>>,
}

pub enum Job {
    Transcribe(Request),
    /// Load a model ahead of time (while the user is still speaking).
    Preload(&'static Model),
    Unload,
    /// Change the idle unload policy.
    Policy {
        keep_loaded: bool,
        unload_after: Duration,
        nice: i32,
    },
}

#[derive(Clone)]
pub struct Engine {
    tx: Sender<Job>,
}

/// What the worker reports back for status displays.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub model: Option<String>,
}

impl Engine {
    pub fn spawn(
        keep_loaded: bool,
        unload_after: Duration,
        nice: i32,
        loaded: std::sync::Arc<std::sync::Mutex<Loaded>>,
        on_change: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("nirudictate-engine".into())
            .spawn(move || worker(rx, keep_loaded, unload_after, nice, loaded, on_change))
            .expect("spawn engine thread");
        Self { tx }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

fn lower_priority(nice: i32) {
    // SAFETY: both calls only change this thread's own scheduling.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, nice);
        let param = libc::sched_param { sched_priority: 0 };
        libc::sched_setscheduler(0, libc::SCHED_BATCH, &param);
    }
}

struct LoadedModel {
    id: &'static str,
    ctx: WhisperContext,
}

fn load(model: &'static Model) -> Result<LoadedModel, String> {
    // NIRUDICTATE_MODEL_FILE swaps in another ggml file for measurements.
    let path = std::env::var_os("NIRUDICTATE_MODEL_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| model.path());
    let path = path.to_str().ok_or("model path is not UTF-8")?;
    let mut params = WhisperContextParameters::default();
    params.use_gpu(false);
    params.flash_attn = std::env::var("NIRUDICTATE_FLASH_ATTN").is_ok_and(|v| v == "1");
    let ctx = WhisperContext::new_with_params(path, params)
        .map_err(|e| format!("cannot load {}: {e}", model.id))?;
    Ok(LoadedModel { id: model.id, ctx })
}

fn release_memory() {
    // Give freed model memory back to the system right away.
    #[cfg(target_env = "gnu")]
    // SAFETY: plain glibc call without arguments that matter.
    unsafe {
        libc::malloc_trim(0);
    }
}

fn worker(
    rx: Receiver<Job>,
    mut keep_loaded: bool,
    mut unload_after: Duration,
    mut nice: i32,
    loaded: std::sync::Arc<std::sync::Mutex<Loaded>>,
    on_change: impl Fn(),
) {
    lower_priority(nice);
    whisper_rs::install_logging_hooks();
    let mut current: Option<LoadedModel> = None;
    let set_loaded = |id: Option<&str>| {
        if let Ok(mut l) = loaded.lock() {
            l.model = id.map(str::to_string);
        }
        on_change();
    };
    loop {
        let job = if current.is_some() && !keep_loaded {
            match rx.recv_timeout(unload_after) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => Job::Unload,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(job) => job,
                Err(_) => return,
            }
        };
        match job {
            Job::Unload => {
                if current.take().is_some() {
                    release_memory();
                    set_loaded(None);
                }
            }
            Job::Preload(model) => {
                if current.as_ref().map(|c| c.id) != Some(model.id) {
                    if current.take().is_some() {
                        release_memory();
                    }
                    match load(model) {
                        Ok(m) => {
                            current = Some(m);
                            set_loaded(Some(model.id));
                        }
                        Err(e) => {
                            eprintln!("nirudictate: {e}");
                            set_loaded(None);
                        }
                    }
                }
            }
            Job::Policy {
                keep_loaded: k,
                unload_after: u,
                nice: n,
            } => {
                keep_loaded = k;
                unload_after = u;
                if n != nice {
                    nice = n;
                    lower_priority(nice);
                }
            }
            Job::Transcribe(req) => {
                let mut load_seconds = 0.0;
                if current.as_ref().map(|c| c.id) != Some(req.model.id) {
                    if current.take().is_some() {
                        release_memory();
                    }
                    let t = Instant::now();
                    match load(req.model) {
                        Ok(m) => {
                            current = Some(m);
                            load_seconds = t.elapsed().as_secs_f32();
                            set_loaded(Some(req.model.id));
                        }
                        Err(e) => {
                            set_loaded(None);
                            let _ = req.reply.send(Err(e));
                            continue;
                        }
                    }
                }
                let ctx = &current.as_ref().expect("model loaded").ctx;
                let t = Instant::now();
                let result = run(ctx, &req).map(|(text, detected)| Transcript {
                    text,
                    detected,
                    model: req.model.id.to_string(),
                    language: req.model.language.to_string(),
                    audio_seconds: req.samples.len() as f32 / crate::audio::RATE as f32,
                    load_seconds,
                    seconds: t.elapsed().as_secs_f32(),
                });
                let _ = req.reply.send(result);
            }
        }
    }
}

fn run(ctx: &WhisperContext, req: &Request) -> Result<(String, Option<String>), String> {
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(req.threads.max(1) as i32);
    // KB-Whisper keeps the multilingual vocabulary; the `.en` models do not.
    let language = match req.language.as_deref() {
        Some("auto") if req.model.multilingual() => "auto",
        Some(code) if req.model.multilingual() && code != "auto" => code,
        _ => req.model.whisper_language(),
    };
    params.set_language(Some(language));
    params.set_translate(false);
    params.set_no_context(true);
    params.set_no_timestamps(true);
    params.set_single_segment(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    if !req.prompt.is_empty() {
        params.set_initial_prompt(&req.prompt);
    }
    // Whisper works on 30 s windows; pad very short clips to one second so
    // the encoder sees enough context.
    let mut samples = req.samples.clone();
    let min = crate::audio::RATE as usize;
    if samples.len() < min {
        samples.resize(min, 0.0);
    }
    // Whisper encodes a 30 s window however short the dictation is.
    if let Some(ctx) = audio_ctx(samples.len(), req.full_window) {
        params.set_audio_ctx(ctx);
    }
    state.full(params, &samples).map_err(|e| e.to_string())?;
    let mut out = String::new();
    for segment in state.as_iter() {
        if segment.no_speech_probability() > 0.8 {
            continue;
        }
        if let Ok(text) = segment.to_str_lossy() {
            out.push_str(&text);
        }
    }
    let detected = (language == "auto")
        .then(|| whisper_rs::get_lang_str(state.full_lang_id_from_state()))
        .flatten()
        .map(str::to_string);
    Ok((out, detected))
}

/// Encoder frames (50 per second) for a shortened window, or None for the
/// full 30 s window. Shortening is several times faster but makes the small
/// models repeat themselves (see docs/MEASUREMENTS.md), so it is off unless
/// `NIRUDICTATE_AUDIO_CTX=short` asks for it.
pub fn audio_ctx(samples: usize, full_window: bool) -> Option<i32> {
    let floor: i32 = match std::env::var("NIRUDICTATE_AUDIO_CTX") {
        Ok(v) if v == "short" => 256,
        Ok(v) => v.parse().unwrap_or(1500),
        Err(_) => 1500,
    };
    if full_window || floor >= 1500 {
        return None;
    }
    let seconds = samples as f32 / crate::audio::RATE as f32;
    let frames = (seconds * 50.0).ceil() as i32 + 128;
    (frames < 1400).then_some(frames.max(floor))
}

/// Transcribe on the calling thread (CLI `transcribe` and benchmarks without
/// a running service).
pub fn transcribe_now(
    model: &'static Model,
    samples: Vec<f32>,
    prompt: String,
    threads: u32,
    nice: i32,
) -> Result<Transcript, String> {
    let loaded = std::sync::Arc::new(std::sync::Mutex::new(Loaded::default()));
    let engine = Engine::spawn(false, Duration::from_secs(1), nice, loaded, || {});
    let (tx, rx) = mpsc::channel();
    engine.send(Job::Transcribe(Request {
        samples,
        model,
        prompt,
        threads,
        full_window: false,
        language: None,
        reply: tx,
    }));
    rx.recv().map_err(|_| "engine stopped".to_string())?
}
