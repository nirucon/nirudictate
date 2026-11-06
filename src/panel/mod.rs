//! The control panel (`nirudictate settings`): status, models, shortcut,
//! output, vocabulary, history and advanced settings. It follows the NIRUX
//! theme live, like NIRUAMP. Every change is saved at once and the running
//! service reloads it.
mod widgets;

use crate::audio::{self, Capture};
use crate::config::{self, Config, Method, Mode};
use crate::engine::Transcript;
use crate::hardware::{self, Hardware};
use crate::ipc::{self, Request, Status};
use crate::models::{self, Model, Progress};
use crate::theme::{self, Preferences, ResolvedTheme, ThemeEvent, ThemeWatcher};
use crate::{history, hotkey, measure, text};
use egui::RichText;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use widgets::{card, chip, level_meter, section_title};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Models,
    Shortcut,
    Output,
    Vocabulary,
    History,
    Advanced,
}

const TABS: &[(Tab, &str, &str)] = &[
    (Tab::Overview, "Overview", "●"),
    (Tab::Models, "Languages & Models", "◆"),
    (Tab::Shortcut, "Shortcut & Mode", "⌨"),
    (Tab::Output, "Output", "↵"),
    (Tab::Vocabulary, "Vocabulary", "Aa"),
    (Tab::History, "History", "≡"),
    (Tab::Advanced, "Advanced", "⚙"),
];

struct Download {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    done: Arc<Mutex<Option<Result<(), String>>>>,
}

enum MicTest {
    Recording { capture: Capture, started: Instant },
    Transcribing(Receiver<Result<Transcript, String>>),
}

struct App {
    cfg: Config,
    saved: Config,
    save_error: Option<String>,
    prefs: Arc<Mutex<Preferences>>,
    theme: ResolvedTheme,
    _watcher: ThemeWatcher,
    tab: Tab,
    status: Arc<Mutex<Option<Status>>>,
    polling: Arc<AtomicBool>,
    hw: Hardware,
    measured: Option<f32>,
    downloads: HashMap<&'static str, Download>,
    mic: Option<MicTest>,
    mic_result: Option<Result<Transcript, String>>,
    devices: Vec<audio::Device>,
    history: Vec<history::Entry>,
    vocab: String,
    terminals: String,
    capturing_shortcut: bool,
    confirm_clear: bool,
    confirm_delete: Option<&'static str>,
    toast: Option<(String, Instant, bool)>,
    themes: Vec<(String, String)>,
    restyle_pending: bool,
}

fn status_poller(status: Arc<Mutex<Option<Status>>>, run: Arc<AtomicBool>, ctx: egui::Context) {
    let _ = thread::Builder::new()
        .name("nirudictate-panel-status".into())
        .spawn(move || {
            let mut last: Option<Status> = None;
            while run.load(Ordering::Relaxed) {
                let now = ipc::send(&Request::Status, Duration::from_millis(800))
                    .ok()
                    .and_then(|r| r.status);
                let busy = now.as_ref().is_some_and(|s| s.state != "idle");
                if now != last {
                    if let Ok(mut s) = status.lock() {
                        *s = now.clone();
                    }
                    ctx.request_repaint();
                    last = now;
                }
                thread::sleep(Duration::from_millis(if busy { 200 } else { 1000 }));
            }
        });
}

fn reload_service() {
    let _ = ipc::send(&Request::Reload, Duration::from_secs(2));
}

fn start_service() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = config::log_path();
    let _ = std::fs::create_dir_all(config::state_dir());
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err);
    // SAFETY: setsid in the child only detaches it from our session.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let _ = thread::Builder::new().spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load_or_default();
        let prefs = Arc::new(Mutex::new(Preferences {
            theme: cfg.general.theme.clone(),
            ..Preferences::default()
        }));
        let theme = theme::resolve(&prefs.lock().expect("prefs").clone())
            .or_else(|_| {
                theme::resolve(&Preferences {
                    theme: "classic".into(),
                    ..Preferences::default()
                })
            })
            .expect("classic theme");
        theme::install_style(&cc.egui_ctx, &theme, &prefs.lock().expect("prefs"));
        let watcher = ThemeWatcher::spawn(prefs.clone(), cc.egui_ctx.clone());
        let status = Arc::new(Mutex::new(None));
        let polling = Arc::new(AtomicBool::new(true));
        status_poller(status.clone(), polling.clone(), cc.egui_ctx.clone());
        let vocab = cfg.vocabulary.words.join("\n");
        let terminals = cfg.output.terminals.join(", ");
        Self {
            saved: cfg.clone(),
            cfg,
            save_error: None,
            prefs,
            theme,
            _watcher: watcher,
            tab: Tab::Overview,
            status,
            polling,
            hw: hardware::detect(),
            measured: measure::load().tiny(),
            downloads: HashMap::new(),
            mic: None,
            mic_result: None,
            devices: audio::devices(),
            history: history::load(),
            vocab,
            terminals,
            capturing_shortcut: false,
            confirm_clear: false,
            confirm_delete: None,
            toast: None,
            themes: theme::available_themes(),
            restyle_pending: true,
        }
    }

    fn p(&self) -> theme::Palette {
        self.theme.palette
    }

    fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some((text.into(), Instant::now(), error));
    }

    /// Save changed settings and let the service reload them.
    fn persist(&mut self) {
        if self.cfg == self.saved {
            return;
        }
        match self.cfg.save() {
            Ok(()) => {
                let theme_changed = self.cfg.general.theme != self.saved.general.theme;
                self.saved = self.cfg.clone();
                self.save_error = None;
                reload_service();
                if theme_changed {
                    if let Ok(mut p) = self.prefs.lock() {
                        p.theme = self.cfg.general.theme.clone();
                    }
                    if let Ok(t) = theme::resolve(&self.prefs.lock().expect("prefs").clone()) {
                        self.theme = t;
                        self.restyle_pending = true;
                    }
                }
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    fn status(&self) -> Option<Status> {
        self.status.lock().ok().and_then(|s| s.clone())
    }

    fn start_download(&mut self, model: &'static Model, ctx: &egui::Context) {
        if self.downloads.contains_key(model.id) {
            return;
        }
        let d = Download {
            progress: Arc::new(Mutex::new(Progress {
                total: model.bytes,
                ..Progress::default()
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            done: Arc::new(Mutex::new(None)),
        };
        let (p, c, done, ctx) = (
            d.progress.clone(),
            d.cancel.clone(),
            d.done.clone(),
            ctx.clone(),
        );
        let _ = thread::Builder::new()
            .name("nirudictate-download".into())
            .spawn(move || {
                let r = models::download(model, &c, |pr| {
                    if let Ok(mut x) = p.lock() {
                        *x = pr;
                    }
                    ctx.request_repaint();
                });
                if let Ok(mut x) = done.lock() {
                    *x = Some(r);
                }
                ctx.request_repaint();
            });
        self.downloads.insert(model.id, d);
    }

    fn poll_downloads(&mut self) {
        let finished: Vec<(&'static str, Result<(), String>)> = self
            .downloads
            .iter()
            .filter_map(|(id, d)| {
                d.done
                    .lock()
                    .ok()
                    .and_then(|mut x| x.take())
                    .map(|r| (*id, r))
            })
            .collect();
        for (id, result) in finished {
            self.downloads.remove(id);
            match result {
                Ok(()) => {
                    let label = models::find(id).map(|m| m.label).unwrap_or(id);
                    self.toast(format!("{label} is installed and ready"), false);
                    reload_service();
                }
                Err(e) if e == "cancelled" => {
                    self.toast("Download cancelled; it resumes where it stopped", false)
                }
                Err(e) => self.toast(e, true),
            }
        }
    }

    fn start_mic_test(&mut self, ctx: &egui::Context) {
        let c = ctx.clone();
        match Capture::start(&self.cfg.advanced.microphone, move || c.request_repaint()) {
            Ok(capture) => {
                self.mic_result = None;
                self.mic = Some(MicTest::Recording {
                    capture,
                    started: Instant::now(),
                });
            }
            Err(e) => self.mic_result = Some(Err(e)),
        }
    }

    fn finish_mic_test(&mut self, ctx: &egui::Context) {
        let Some(MicTest::Recording { capture, .. }) = self.mic.take() else {
            return;
        };
        let samples = capture.finish();
        let Some(speech) = audio::trim_silence(&samples) else {
            self.mic_result = Some(Err(
                "Nothing was heard. Check the microphone and its level below.".into(),
            ));
            return;
        };
        let speech = speech.to_vec();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let cfg = self.cfg.clone();
        let hw = self.hw.clone();
        let _ = thread::Builder::new().spawn(move || {
            let result = (|| {
                let lang = cfg.general.language.clone();
                let model = models::active(&lang, cfg.model_for(&lang), &hw)
                    .ok_or("No model is installed for this language yet.")?;
                if ipc::running() {
                    let path = config::runtime_dir().join("mic-test.wav");
                    audio::write_wav(&path, &speech)?;
                    let reply = ipc::send(
                        &Request::TranscribeFile {
                            path: path.to_string_lossy().into_owned(),
                            model: Some(model.id.to_string()),
                        },
                        Duration::from_secs(300),
                    );
                    let _ = std::fs::remove_file(&path);
                    let reply = reply?;
                    if !reply.ok {
                        return Err(reply.message.unwrap_or_default());
                    }
                    reply.transcript.ok_or_else(|| "no transcript".to_string())
                } else {
                    crate::engine::transcribe_now(
                        model,
                        speech,
                        text::prompt(&cfg.vocabulary.words, model.language),
                        cfg.threads(),
                        cfg.advanced.nice,
                    )
                }
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.mic = Some(MicTest::Transcribing(rx));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self._watcher.rx.try_recv() {
            match event {
                ThemeEvent::Updated(t) => {
                    self.theme = *t;
                    self.restyle_pending = true;
                }
                ThemeEvent::Warning(w) => self.toast(w, true),
            }
        }
        if self.restyle_pending {
            self.restyle_pending = false;
            theme::install_style(ctx, &self.theme, &self.prefs.lock().expect("prefs").clone());
            widgets::tune_style(ctx, &self.theme.palette);
        }
        self.poll_downloads();
        if let Some(MicTest::Recording { started, .. }) = &self.mic {
            if started.elapsed() >= Duration::from_secs(8) {
                self.finish_mic_test(ctx);
            } else {
                ctx.request_repaint_after(Duration::from_millis(33));
            }
        }
        if let Some(MicTest::Transcribing(rx)) = &self.mic {
            if let Ok(result) = rx.try_recv() {
                if let Ok(t) = &result {
                    measure::record(t);
                    self.measured = measure::load().tiny();
                }
                self.mic_result = Some(result);
                self.mic = None;
            }
        }
        if !self.downloads.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let p = self.p();
        egui::SidePanel::left("nav")
            .exact_width(212.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(p.panel)
                    .inner_margin(egui::Margin::symmetric(14.0, 18.0)),
            )
            .show(ctx, |ui| self.nav(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(p.background)
                    .inner_margin(egui::Margin::symmetric(28.0, 22.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(760.0);
                        match self.tab {
                            Tab::Overview => self.overview(ui, ctx),
                            Tab::Models => self.models_tab(ui, ctx),
                            Tab::Shortcut => self.shortcut_tab(ui, ctx),
                            Tab::Output => self.output_tab(ui),
                            Tab::Vocabulary => self.vocabulary_tab(ui),
                            Tab::History => self.history_tab(ui, ctx),
                            Tab::Advanced => self.advanced_tab(ui),
                        }
                        ui.add_space(24.0);
                    });
            });
        self.draw_toast(ctx);
        self.persist();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.polling.store(false, Ordering::Relaxed);
        for d in self.downloads.values() {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl App {
    fn nav(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        ui.horizontal(|ui| {
            widgets::logo(ui, &p, 28.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("NIRUDICTATE")
                        .strong()
                        .size(16.0)
                        .color(p.foreground),
                );
                ui.label(RichText::new("Local dictation").size(12.0).color(p.muted));
            });
        });
        ui.add_space(14.0);
        let status = self.status();
        let (label, color) = match status.as_ref().map(|s| s.state.as_str()) {
            Some("recording") => ("Recording", widgets::red(&p)),
            Some("transcribing") => ("Transcribing", p.warning),
            Some(_) => ("Ready", p.success),
            None => ("Service stopped", p.error),
        };
        chip(ui, &p, label, color);
        ui.add_space(18.0);
        for (tab, label, icon) in TABS {
            if widgets::nav_item(ui, &p, icon, label, self.tab == *tab).clicked() {
                self.tab = *tab;
                self.capturing_shortcut = false;
                if *tab == Tab::History {
                    self.history = history::load();
                }
                if *tab == Tab::Advanced {
                    self.devices = audio::devices();
                    self.themes = theme::available_themes();
                }
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(
                RichText::new(format!(
                    "v{} · private, on this computer",
                    env!("CARGO_PKG_VERSION")
                ))
                .size(11.0)
                .color(p.muted),
            );
            ui.label(
                RichText::new(self.theme.source.clone())
                    .size(11.0)
                    .color(p.disabled),
            );
        });
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((text, at, error)) = self.toast.clone() else {
            return;
        };
        let age = at.elapsed();
        if age > Duration::from_secs(5) {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        let p = self.p();
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(100.0, -20.0))
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(p.surface_hi)
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        if error { p.error } else { p.border_hi },
                    ))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).color(if error {
                            p.error
                        } else {
                            p.foreground
                        }));
                    });
            });
    }

    fn heading(&self, ui: &mut egui::Ui, title: &str, subtitle: &str) {
        let p = self.p();
        ui.label(RichText::new(title).size(24.0).strong().color(p.foreground));
        ui.label(RichText::new(subtitle).color(p.muted));
        ui.add_space(14.0);
        if let Some(e) = &self.save_error {
            ui.label(RichText::new(format!("Not saved: {e}")).color(p.error));
        }
    }

    // ── Overview ──────────────────────────────────────────────────────────

    fn overview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.p();
        self.heading(
            ui,
            "Overview",
            "Speak, and the text appears where you type. Nothing leaves this computer.",
        );
        let status = self.status();
        let lang = self.cfg.general.language.clone();
        let active = models::active(&lang, self.cfg.model_for(&lang), &self.hw);
        card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                let (big, color, detail) = match status.as_ref() {
                    None => (
                        "Service not running",
                        p.error,
                        "The shortcut does nothing until the service runs.".to_string(),
                    ),
                    Some(s) if s.state == "recording" => (
                        "Listening…",
                        widgets::red(&p),
                        format!("{:.0} s", s.recording_seconds),
                    ),
                    Some(s) if s.state == "transcribing" => (
                        "Transcribing…",
                        p.warning,
                        "Writing down what you said".into(),
                    ),
                    Some(s) if !s.shortcut_active => (
                        "Shortcut unavailable",
                        p.warning,
                        s.error.clone().unwrap_or_default(),
                    ),
                    Some(_) if active.is_none() => (
                        "Needs a model",
                        p.warning,
                        "Download one below to start dictating.".into(),
                    ),
                    Some(_) => (
                        "Ready",
                        p.success,
                        format!("Press {} anywhere", hotkey::display(&self.cfg.shortcut.key)),
                    ),
                };
                widgets::status_dot(ui, color, 14.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(big).size(20.0).strong().color(p.foreground));
                    ui.label(RichText::new(detail).color(p.muted));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if status.is_none() {
                        if widgets::primary_button(ui, &p, "Start service").clicked() {
                            match start_service() {
                                Ok(()) => self.toast("Starting the service…", false),
                                Err(e) => self.toast(e, true),
                            }
                        }
                    } else if status.as_ref().is_some_and(|s| s.state == "recording") {
                        if ui.button("Stop").clicked() {
                            let _ = ipc::send(&Request::Stop, Duration::from_secs(2));
                        }
                    } else if active.is_some()
                        && ui
                            .button("Dictate now")
                            .on_hover_text(
                                "Starts recording; the text goes to the window that has focus",
                            )
                            .clicked()
                    {
                        let _ = ipc::send(&Request::Start, Duration::from_secs(2));
                    }
                });
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            egui::Grid::new("ov-grid")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Language").color(p.muted));
                    ui.horizontal(|ui| {
                        for (code, label) in config::LANGUAGES {
                            if widgets::segment(ui, &p, label, self.cfg.general.language == *code)
                                .clicked()
                            {
                                self.cfg.general.language = code.to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new("Model").color(p.muted));
                    ui.label(match active {
                        Some(m) => format!(
                            "{} · {}{}",
                            m.label,
                            models::human_size(m.bytes),
                            if status.as_ref().and_then(|s| s.loaded.as_deref()) == Some(m.id) {
                                " · loaded"
                            } else {
                                ""
                            }
                        ),
                        None => "none installed".into(),
                    });
                    ui.end_row();
                    ui.label(RichText::new("Shortcut").color(p.muted));
                    ui.horizontal(|ui| {
                        widgets::keycaps(ui, &p, &self.cfg.shortcut.key);
                        ui.label(
                            RichText::new(match self.cfg.shortcut.mode {
                                Mode::Hybrid => "tap to toggle · hold to talk",
                                Mode::Toggle => "press to start and stop",
                                Mode::Hold => "hold to talk",
                            })
                            .color(p.muted),
                        );
                    });
                    ui.end_row();
                });
        });
        if active.is_none() {
            let rec = models::recommend(&lang, &self.hw, self.measured);
            ui.add_space(12.0);
            card(ui, &p, |ui| {
                section_title(ui, &p, "Get started");
                ui.label(format!(
                    "Download the recommended {} model for this computer: {} ({}).",
                    config::language_label(&lang),
                    rec.label,
                    models::human_size(rec.bytes)
                ));
                ui.add_space(8.0);
                self.download_row(ui, ctx, rec, true);
            });
        }
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Microphone test");
            ui.label(
                RichText::new("Say a sentence. It is transcribed here and not inserted anywhere.")
                    .color(p.muted),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| match &self.mic {
                Some(MicTest::Recording { capture, started }) => {
                    let levels = capture
                        .shared
                        .lock()
                        .map(|s| s.levels.clone())
                        .unwrap_or_default();
                    if widgets::primary_button(ui, &p, "Stop and transcribe").clicked() {
                        self.finish_mic_test(ctx);
                    } else {
                        level_meter(ui, &p, &levels, egui::vec2(260.0, 30.0));
                        ui.label(
                            RichText::new(format!("{:.0} s", started.elapsed().as_secs_f32()))
                                .color(p.muted),
                        );
                    }
                }
                Some(MicTest::Transcribing(_)) => {
                    ui.spinner();
                    ui.label("Transcribing…");
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                None => {
                    if widgets::primary_button(ui, &p, "Test microphone").clicked() {
                        self.start_mic_test(ctx);
                    }
                }
            });
            match &self.mic_result {
                Some(Ok(t)) => {
                    ui.add_space(8.0);
                    let shown = text::finish(
                        &t.text,
                        text::Options {
                            capitalize: true,
                            trailing_space: false,
                        },
                    )
                    .unwrap_or_else(|| "(no speech recognised)".into());
                    widgets::quote(ui, &p, &shown);
                    ui.label(
                        RichText::new(format!(
                            "{} · {:.1} s of speech · {:.2} s to transcribe{}",
                            t.model,
                            t.audio_seconds,
                            t.seconds,
                            if t.load_seconds > 0.0 {
                                format!(" (+{:.2} s loading)", t.load_seconds)
                            } else {
                                String::new()
                            }
                        ))
                        .size(12.0)
                        .color(p.muted),
                    );
                }
                Some(Err(e)) => {
                    ui.add_space(8.0);
                    ui.label(RichText::new(e).color(p.error));
                }
                None => {}
            }
        });
        if let Some(last) = status.as_ref().and_then(|s| s.last.clone()) {
            ui.add_space(12.0);
            card(ui, &p, |ui| {
                ui.horizontal(|ui| {
                    section_title(ui, &p, "Last dictation");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Copy").clicked() {
                            ctx.output_mut(|o| o.copied_text = last.text.clone());
                            self.toast("Copied", false);
                        }
                    });
                });
                widgets::quote(ui, &p, &last.text);
                ui.label(
                    RichText::new(format!(
                        "{} · {:.1} s of speech · {:.2} s · {}",
                        last.model, last.audio_seconds, last.seconds, last.inserted_by
                    ))
                    .size(12.0)
                    .color(p.muted),
                );
            });
        }
    }

    // ── Models ────────────────────────────────────────────────────────────

    fn download_row(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        m: &'static Model,
        big: bool,
    ) {
        let p = self.p();
        if let Some(d) = self.downloads.get(m.id) {
            let pr = d.progress.lock().map(|x| *x).unwrap_or_default();
            let frac = if pr.total > 0 {
                pr.done as f32 / pr.total as f32
            } else {
                0.0
            };
            let text = if pr.verifying {
                "Verifying checksum…".to_string()
            } else {
                format!(
                    "{} of {} · {:.1} MB/s",
                    models::human_size(pr.done),
                    models::human_size(pr.total),
                    pr.bytes_per_second / 1e6
                )
            };
            ui.horizontal(|ui| {
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_width(if big { 320.0 } else { 200.0 })
                        .text(text),
                );
                if ui.small_button("Cancel").clicked() {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            });
        } else if big {
            if widgets::primary_button(ui, &p, &format!("Download {}", models::human_size(m.bytes)))
                .clicked()
            {
                self.start_download(m, ctx);
            }
        } else if ui
            .button(format!("Download · {}", models::human_size(m.bytes)))
            .clicked()
        {
            self.start_download(m, ctx);
        }
    }

    fn models_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.p();
        self.heading(
            ui,
            "Languages & Models",
            "Models are downloaded once, verified, and run on this computer only.",
        );
        card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Dictation language").color(p.muted));
                for (code, label) in config::LANGUAGES {
                    if widgets::segment(ui, &p, label, self.cfg.general.language == *code).clicked()
                    {
                        self.cfg.general.language = code.to_string();
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} · {} cores · {} · {} GB memory · speed {}",
                    self.hw.model,
                    self.hw.physical,
                    if self.hw.avx2 { "AVX2" } else { "no AVX2" },
                    self.hw.memory_mb / 1024,
                    if self.measured.is_some() {
                        "measured on this computer"
                    } else {
                        "estimated (measured after a few dictations)"
                    }
                ))
                .size(12.0)
                .color(p.muted),
            );
        });
        for (code, label) in config::LANGUAGES {
            ui.add_space(14.0);
            let rec = models::recommend(code, &self.hw, self.measured);
            let active = models::active(code, self.cfg.model_for(code), &self.hw);
            ui.horizontal(|ui| {
                section_title(ui, &p, label);
                ui.label(
                    RichText::new(match *code {
                        "sv" => "KB-Whisper by KBLab (National Library of Sweden) · Apache-2.0",
                        _ => "OpenAI Whisper via whisper.cpp · MIT",
                    })
                    .size(12.0)
                    .color(p.muted),
                );
            });
            card(ui, &p, |ui| {
                for m in models::for_language(code) {
                    let installed = m.installed();
                    ui.horizontal(|ui| {
                        let is_active = active.map(|a| a.id) == Some(m.id);
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(m.label).strong().color(p.foreground));
                                if m.id == rec.id {
                                    chip(ui, &p, "Recommended", p.accent);
                                }
                                if is_active {
                                    chip(ui, &p, "In use", p.success);
                                }
                            });
                            ui.label(
                                RichText::new(format!(
                                    "{} · about {:.1} s per dictation here · {} MB memory",
                                    models::human_size(m.bytes),
                                    models::estimate(m, &self.hw, self.measured),
                                    m.memory_mb
                                ))
                                .size(12.0)
                                .color(p.muted),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if installed {
                                if self.confirm_delete == Some(m.id) {
                                    if ui.button(RichText::new("Delete").color(p.error)).clicked() {
                                        match models::delete(m) {
                                            Ok(()) => {
                                                let _ = ipc::send(
                                                    &Request::Unload,
                                                    Duration::from_secs(2),
                                                );
                                                if self.cfg.model_for(code) == m.id {
                                                    self.cfg.set_model_for(code, "");
                                                }
                                                self.toast(format!("{} deleted", m.label), false);
                                            }
                                            Err(e) => self.toast(e, true),
                                        }
                                        self.confirm_delete = None;
                                    }
                                    if ui.button("Keep").clicked() {
                                        self.confirm_delete = None;
                                    }
                                } else {
                                    if ui.small_button("Delete…").clicked() {
                                        self.confirm_delete = Some(m.id);
                                    }
                                    if !is_active && ui.button("Use").clicked() {
                                        self.cfg.set_model_for(code, m.id);
                                    }
                                }
                            } else {
                                self.download_row(ui, ctx, m, false);
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Automatic choice:").size(12.0).color(p.muted));
                    let auto = self.cfg.model_for(code).is_empty();
                    if ui
                        .radio(
                            auto,
                            RichText::new("largest fast enough installed model").size(12.0),
                        )
                        .clicked()
                    {
                        self.cfg.set_model_for(code, "");
                    }
                });
            });
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("Stored in {}", config::models_dir().display()))
                .size(12.0)
                .color(p.muted),
        );
    }

    // ── Shortcut ──────────────────────────────────────────────────────────

    fn shortcut_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.p();
        self.heading(
            ui,
            "Shortcut & Mode",
            "One key combination starts and stops dictation in any application.",
        );
        card(ui, &p, |ui| {
            section_title(ui, &p, "Shortcut");
            ui.horizontal(|ui| {
                if self.capturing_shortcut {
                    ui.label(
                        RichText::new("Press the new combination… (Esc cancels)").color(p.accent),
                    );
                    if let Some(spec) = widgets::capture_shortcut(ctx) {
                        if spec == "escape" {
                            self.capturing_shortcut = false;
                        } else if hotkey::parse(&spec).is_ok() {
                            self.cfg.shortcut.key = spec;
                            self.capturing_shortcut = false;
                        } else {
                            self.toast(
                                "Use at least one modifier (Ctrl, Alt, Super) with a key",
                                true,
                            );
                        }
                    }
                } else {
                    widgets::keycaps(ui, &p, &self.cfg.shortcut.key);
                    if ui.button("Change…").clicked() {
                        self.capturing_shortcut = true;
                    }
                    if self.cfg.shortcut.key != "ctrl+alt+space" && ui.button("Default").clicked() {
                        self.cfg.shortcut.key = "ctrl+alt+space".into();
                    }
                }
            });
            if let Some(s) = self.status() {
                if !s.shortcut_active {
                    ui.label(
                        RichText::new(
                            s.error
                                .unwrap_or_else(|| "The shortcut is not active".into()),
                        )
                        .color(p.warning),
                    );
                }
            }
            ui.label(
                RichText::new("NIRUDICTATE listens for this combination itself; the window manager is not changed. Ctrl+Alt+Space is free in NIRUWM.")
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Mode");
            for mode in [Mode::Hybrid, Mode::Toggle, Mode::Hold] {
                ui.radio_value(&mut self.cfg.shortcut.mode, mode, mode.label());
            }
            ui.add_space(8.0);
            egui::Grid::new("mode-grid")
                .num_columns(2)
                .spacing([18.0, 10.0])
                .show(ui, |ui| {
                    if self.cfg.shortcut.mode == Mode::Hybrid {
                        ui.label("Hold longer than");
                        ui.add(
                            egui::Slider::new(&mut self.cfg.shortcut.hold_ms, 150..=1000)
                                .suffix(" ms")
                                .text("for push-to-talk"),
                        );
                        ui.end_row();
                    }
                    if self.cfg.shortcut.mode != Mode::Hold {
                        ui.label("Stop after silence");
                        ui.horizontal(|ui| {
                            let mut on = self.cfg.shortcut.silence_seconds > 0.0;
                            if ui.checkbox(&mut on, "").changed() {
                                self.cfg.shortcut.silence_seconds = if on { 2.0 } else { 0.0 };
                            }
                            if on {
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.cfg.shortcut.silence_seconds,
                                        0.5..=10.0,
                                    )
                                    .step_by(0.5)
                                    .suffix(" s"),
                                );
                            } else {
                                ui.label(RichText::new("off — press again to stop").color(p.muted));
                            }
                        });
                        ui.end_row();
                    }
                    ui.label("Longest dictation");
                    ui.add(
                        egui::Slider::new(&mut self.cfg.shortcut.max_seconds, 10..=600)
                            .suffix(" s"),
                    );
                    ui.end_row();
                });
        });
    }

    // ── Output ────────────────────────────────────────────────────────────

    fn output_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Output",
            "How the text reaches the window you are typing in.",
        );
        card(ui, &p, |ui| {
            section_title(ui, &p, "Insert by");
            ui.radio_value(
                &mut self.cfg.output.method,
                Method::Paste,
                "Pasting (fast; Ctrl+Shift+V in terminals)",
            );
            ui.radio_value(
                &mut self.cfg.output.method,
                Method::Type,
                "Typing each character (slower; for apps that block paste)",
            );
            ui.add_space(6.0);
            if self.cfg.output.method == Method::Paste {
                ui.checkbox(
                    &mut self.cfg.output.restore_clipboard,
                    "Put the previous clipboard back afterwards",
                );
                ui.checkbox(
                    &mut self.cfg.output.type_fallback,
                    "Type instead when a window does not accept the paste",
                );
            }
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Text");
            ui.checkbox(
                &mut self.cfg.output.capitalize,
                "Start with a capital letter",
            );
            ui.checkbox(
                &mut self.cfg.output.trailing_space,
                "Add a space after the text",
            );
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Terminals");
            ui.label(
                RichText::new(format!(
                    "Recognised automatically: {} and more. xterm and urxvt use Shift+Insert.",
                    crate::insert::TERMINALS[..6].join(", ")
                ))
                .size(12.0)
                .color(p.muted),
            );
            ui.horizontal(|ui| {
                ui.label("Also treat as terminals (WM_CLASS):");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.terminals)
                            .hint_text("e.g. myterm, cool-retro-term")
                            .desired_width(260.0),
                    )
                    .changed()
                {
                    self.cfg.output.terminals = self
                        .terminals
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            });
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Feedback");
            ui.checkbox(
                &mut self.cfg.general.overlay,
                "Show the recording overlay at the bottom of the screen",
            );
            ui.checkbox(&mut self.cfg.general.sounds, "Play the dictation sounds");
            ui.label(
                RichText::new(if crate::sounds::available() {
                    "Sounds come from NIRUX: `nirux sounds toggle dictation` or each event, own files in ~/.config/nirux/sounds/."
                } else {
                    "Sounds need NIRUX (`nirux sounds`); without it dictation is silent."
                })
                .size(12.0)
                .color(p.muted),
            );
        });
    }

    // ── Vocabulary ────────────────────────────────────────────────────────

    fn vocabulary_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Vocabulary",
            "Names and words the model should spell your way: products, people, places, jargon.",
        );
        card(ui, &p, |ui| {
            ui.label(RichText::new("One word or phrase per line").color(p.muted));
            let changed = ui
                .add(
                    egui::TextEdit::multiline(&mut self.vocab)
                        .desired_rows(12)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if changed {
                self.cfg.vocabulary.words = self
                    .vocab
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
            }
            ui.add_space(6.0);
            let prompt = text::prompt(&self.cfg.vocabulary.words, &self.cfg.general.language);
            ui.label(
                RichText::new("Given to the model as a hint:")
                    .size(12.0)
                    .color(p.muted),
            );
            widgets::quote(
                ui,
                &p,
                if prompt.is_empty() {
                    "(nothing)"
                } else {
                    &prompt
                },
            );
            ui.label(
                RichText::new(
                    "A short list works best; the model only sees about 200 words of hint.",
                )
                .size(12.0)
                .color(p.muted),
            );
        });
    }

    // ── History ───────────────────────────────────────────────────────────

    fn history_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.p();
        self.heading(
            ui,
            "History",
            "Kept only on this computer, readable only by you. Audio is never stored.",
        );
        card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.cfg.history.enabled, "Keep a history");
                if self.cfg.history.enabled {
                    ui.label("· keep the last");
                    ui.add(
                        egui::DragValue::new(&mut self.cfg.history.keep)
                            .range(10..=5000)
                            .speed(5),
                    );
                    ui.label("dictations");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.confirm_clear {
                        if ui
                            .button(RichText::new("Delete everything").color(p.error))
                            .clicked()
                        {
                            match history::clear() {
                                Ok(()) => {
                                    self.history.clear();
                                    self.toast("History cleared", false);
                                }
                                Err(e) => self.toast(e, true),
                            }
                            self.confirm_clear = false;
                        }
                        if ui.button("Keep").clicked() {
                            self.confirm_clear = false;
                        }
                    } else if !self.history.is_empty() && ui.button("Clear all…").clicked() {
                        self.confirm_clear = true;
                    }
                });
            });
            ui.label(
                RichText::new(format!("{} (mode 0600)", config::history_path().display()))
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.add_space(12.0);
        if self.history.is_empty() {
            ui.label(RichText::new("Nothing yet.").color(p.muted));
            return;
        }
        let mut delete = None;
        for e in self.history.iter().rev().take(300) {
            card(ui, &p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(widgets::when(e.time))
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} · {:.1} s · {}",
                            e.model, e.audio_seconds, e.language
                        ))
                        .size(12.0)
                        .color(p.disabled),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Delete").clicked() {
                            delete = Some((e.time, e.text.clone()));
                        }
                        if ui.small_button("Copy").clicked() {
                            ctx.output_mut(|o| o.copied_text = e.text.clone());
                        }
                    });
                });
                ui.label(RichText::new(&e.text).color(p.foreground));
            });
            ui.add_space(6.0);
        }
        if let Some((time, text)) = delete {
            if history::delete(time, &text).is_ok() {
                self.history = history::load();
            }
        }
    }

    // ── Advanced ──────────────────────────────────────────────────────────

    fn advanced_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Advanced",
            "Microphone, performance and the background service.",
        );
        card(ui, &p, |ui| {
            section_title(ui, &p, "Microphone");
            let current = if self.cfg.advanced.microphone.is_empty() {
                "System default".to_string()
            } else {
                self.devices
                    .iter()
                    .find(|d| d.name == self.cfg.advanced.microphone)
                    .map(|d| d.description.clone())
                    .unwrap_or_else(|| self.cfg.advanced.microphone.clone())
            };
            egui::ComboBox::from_id_salt("mic")
                .width(380.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.cfg.advanced.microphone,
                        String::new(),
                        "System default",
                    );
                    for d in &self.devices {
                        ui.selectable_value(
                            &mut self.cfg.advanced.microphone,
                            d.name.clone(),
                            &d.description,
                        );
                    }
                });
            ui.label(
                RichText::new(format!(
                    "Recorded with {} only while you dictate.",
                    audio::backend()
                        .map(|b| b.name())
                        .unwrap_or("no recorder found")
                ))
                .size(12.0)
                .color(p.muted),
            );
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Performance");
            egui::Grid::new("perf")
                .num_columns(2)
                .spacing([18.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Threads");
                    ui.horizontal(|ui| {
                        let auto = hardware::threads_for(&self.hw);
                        let mut is_auto = self.cfg.advanced.threads == 0;
                        if ui
                            .checkbox(&mut is_auto, format!("Automatic ({auto})"))
                            .changed()
                        {
                            self.cfg.advanced.threads = if is_auto { 0 } else { auto };
                        }
                        if !is_auto {
                            ui.add(egui::Slider::new(
                                &mut self.cfg.advanced.threads,
                                1..=self.hw.logical.max(1),
                            ));
                        }
                    });
                    ui.end_row();
                    ui.label("Priority");
                    ui.add(
                        egui::Slider::new(&mut self.cfg.advanced.nice, 0..=19)
                            .text("nice (higher yields to other apps)"),
                    );
                    ui.end_row();
                    ui.label("Model memory");
                    ui.vertical(|ui| {
                        ui.checkbox(
                            &mut self.cfg.advanced.keep_model_loaded,
                            "Keep the model loaded (faster first dictation)",
                        );
                        if !self.cfg.advanced.keep_model_loaded {
                            ui.horizontal(|ui| {
                                ui.label("Unload after");
                                ui.add(
                                    egui::DragValue::new(
                                        &mut self.cfg.advanced.unload_after_minutes,
                                    )
                                    .range(1..=1440)
                                    .suffix(" min"),
                                );
                                ui.label("without dictation");
                            });
                        }
                    });
                    ui.end_row();
                });
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Appearance");
            let label = |key: &str, themes: &[(String, String)]| match key {
                "auto" => "Follow NIRUX (or Omarchy)".to_string(),
                "classic" => "NIRUDICTATE Classic".to_string(),
                k => themes
                    .iter()
                    .find(|t| t.0 == k)
                    .map(|t| t.1.clone())
                    .unwrap_or_else(|| k.to_string()),
            };
            egui::ComboBox::from_id_salt("theme")
                .width(300.0)
                .selected_text(label(&self.cfg.general.theme, &self.themes))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.cfg.general.theme,
                        "auto".into(),
                        label("auto", &self.themes),
                    );
                    ui.selectable_value(
                        &mut self.cfg.general.theme,
                        "classic".into(),
                        label("classic", &self.themes),
                    );
                    for (k, l) in &self.themes {
                        ui.selectable_value(&mut self.cfg.general.theme, k.clone(), l);
                    }
                });
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Background service");
            ui.checkbox(
                &mut self.cfg.general.autostart,
                "Start with the desktop session",
            );
            let status = self.status();
            ui.horizontal(|ui| match &status {
                Some(s) => {
                    ui.label(format!(
                        "Running · pid {} · {} · engine {}",
                        s.pid, s.audio, s.engine
                    ));
                }
                None => {
                    ui.label(RichText::new("Not running").color(p.error));
                }
            });
            ui.horizontal(|ui| {
                if status.is_some() {
                    if ui.button("Restart service").clicked() {
                        let _ = ipc::send(&Request::Quit, Duration::from_secs(2));
                        thread::sleep(Duration::from_millis(300));
                        match start_service() {
                            Ok(()) => self.toast("Service restarted", false),
                            Err(e) => self.toast(e, true),
                        }
                    }
                    if status.as_ref().and_then(|s| s.loaded.as_ref()).is_some()
                        && ui.button("Unload model now").clicked()
                    {
                        let _ = ipc::send(&Request::Unload, Duration::from_secs(2));
                    }
                } else if ui.button("Start service").clicked() {
                    match start_service() {
                        Ok(()) => self.toast("Starting the service…", false),
                        Err(e) => self.toast(e, true),
                    }
                }
            });
        });
        ui.add_space(12.0);
        card(ui, &p, |ui| {
            section_title(ui, &p, "Files");
            for (what, path) in [
                ("Settings", config::config_path()),
                ("Models", config::models_dir()),
                ("History", config::history_path()),
                ("Service log", config::log_path()),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(what).color(p.muted));
                    ui.label(
                        RichText::new(path.display().to_string())
                            .monospace()
                            .size(12.0),
                    );
                });
            }
        });
    }
}

/// Bring an already open panel to the front instead of opening a second one.
fn raise_existing() -> bool {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask};
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return false;
    };
    let root = conn.setup().roots[screen].root;
    let atom = |n: &str| {
        conn.intern_atom(false, n.as_bytes())
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
    };
    let (Some(list), Some(active)) = (atom("_NET_CLIENT_LIST"), atom("_NET_ACTIVE_WINDOW")) else {
        return false;
    };
    let Some(windows) = conn
        .get_property(false, root, list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| {
            r.value32()
                .map(|v| v.collect::<Vec<u32>>())
                .unwrap_or_default()
        })
    else {
        return false;
    };
    for w in windows {
        let class = conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.value)
            .unwrap_or_default();
        if class.split(|b| *b == 0).any(|s| s == b"nirudictate") {
            let ev = ClientMessageEvent::new(32, w, active, [1u32, 0, 0, 0, 0]);
            let _ = conn.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                ev,
            );
            let _ = conn.flush();
            return true;
        }
    }
    false
}

pub fn run() -> Result<(), String> {
    let lock_dir = config::runtime_dir();
    config::ensure_private_dir(&lock_dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_dir.join("panel.lock"))
        .map_err(|e| e.to_string())?;
    use std::os::fd::AsRawFd;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        if raise_existing() {
            return Ok(());
        }
        return Err("the settings panel is already open".into());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NIRUDICTATE")
            .with_app_id("nirudictate")
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(widgets::icon()),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "NIRUDICTATE",
        options,
        Box::new(|cc| {
            let mut app = App::new(cc);
            app.restyle_pending = true;
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| format!("cannot open the window: {e}"))?;
    drop(lock);
    Ok(())
}
