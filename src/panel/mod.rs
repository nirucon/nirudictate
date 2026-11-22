//! The control panel (`nirudictate settings`): status, models, shortcut,
//! output, vocabulary, history and advanced settings. It follows the NIRUX
//! theme live, like NIRUAMP. Every change is saved at once and the running
//! service reloads it.
mod widgets;

use crate::audio::{self, Capture};
use crate::config::{self, Config, Method, Mode};
use crate::engine::Transcript;
use crate::gui::ui::{red, Ctx, Rt, Ui};
use crate::gui::{self, Outside, Waker};
use crate::hardware::{self, Hardware};
use crate::ipc::{self, Request, Status};
use crate::models::{self, Model, Progress};
use crate::theme::{self, Preferences, ResolvedTheme, ThemeEvent, ThemeWatcher};
use crate::{history, hotkey, measure, text};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Models,
    Shortcut,
    Agent,
    Commands,
    Output,
    Vocabulary,
    History,
    Advanced,
}

const TABS: &[(Tab, &str)] = &[
    (Tab::Overview, "Overview"),
    (Tab::Models, "Languages & Models"),
    (Tab::Shortcut, "Shortcut & Mode"),
    (Tab::Agent, "Agent mode"),
    (Tab::Commands, "Voice commands"),
    (Tab::Output, "Output"),
    (Tab::Vocabulary, "Vocabulary"),
    (Tab::History, "History"),
    (Tab::Advanced, "Advanced"),
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
    watcher: ThemeWatcher,
    wake: Waker,
    tab: Tab,
    status: Arc<Mutex<Option<Status>>>,
    refresh: Sender<()>,
    hw: Hardware,
    measured: Option<f32>,
    downloads: HashMap<&'static str, Download>,
    mic: Option<MicTest>,
    mic_result: Option<Result<Transcript, String>>,
    devices: Arc<Mutex<Vec<audio::Device>>>,
    history: Vec<history::Entry>,
    vocab: String,
    terminals: String,
    capturing_shortcut: bool,
    capturing_agent: bool,
    capturing_command: bool,
    agent_dir: String,
    agent_name: String,
    agent_program: String,
    opencode: Arc<Mutex<Option<Result<crate::agent::Opencode, String>>>>,
    confirm_clear: bool,
    confirm_delete: Option<&'static str>,
    toast: Option<(String, Instant, bool)>,
    themes: Vec<(String, String)>,
    scroll: f32,
    scroll_drag: Option<(f32, f32)>,
    last_tab: Tab,
    font_path: Option<std::path::PathBuf>,
}

/// Asks the service for its status when told to (the service changed its
/// root window property, the window got the focus, the panel did
/// something), and every 200 ms only while it records or transcribes.
fn status_thread(status: Arc<Mutex<Option<Status>>>, wake: Waker) -> Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let _ = thread::Builder::new()
        .name("nirudictate-panel-status".into())
        .spawn(move || {
            let mut last: Option<Status> = None;
            let mut busy = false;
            loop {
                let got = if busy {
                    rx.recv_timeout(Duration::from_millis(200))
                        .map_err(|e| matches!(e, mpsc::RecvTimeoutError::Disconnected))
                } else {
                    rx.recv().map_err(|_| true)
                };
                if got == Err(true) {
                    return;
                }
                // Coalesce a burst of requests.
                while rx.try_recv().is_ok() {}
                let now = ipc::send(&Request::Status, Duration::from_millis(800))
                    .ok()
                    .and_then(|r| r.status);
                busy = now.as_ref().is_some_and(|s| s.state != "idle");
                if now != last {
                    if let Ok(mut s) = status.lock() {
                        *s = now.clone();
                    }
                    wake();
                    last = now;
                }
            }
        });
    let _ = tx.send(());
    tx
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

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = ui.p();
    ui.frame(p.surface, Some(p.border), 12.0, (18.0, 14.0), add)
}

fn section_title(ui: &mut Ui, text: &str) {
    let p = ui.p();
    ui.label(Rt::new(text).strong().size(15.0).color(p.foreground));
}

/// The label column of a form: its widest label and the gap after it.
fn col(ui: &mut Ui, labels: &[&str]) -> f32 {
    let body = ui.cx.body;
    labels
        .iter()
        .map(|l| ui.cx.tw(crate::fonts::Face::Sans, body, l))
        .fold(0.0, f32::max)
        + 18.0
}

/// One row of a two-column form: the label column is `col` wide.
fn grid_row(ui: &mut Ui, col: f32, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        let r = ui.label(label);
        let pad = col - r.rect.w - crate::gui::ui::SPACING_X;
        if pad > 0.0 {
            ui.add_space(pad);
        }
        add(ui);
    });
}

fn resolve_theme(prefs: &Preferences) -> ResolvedTheme {
    theme::resolve(prefs)
        .or_else(|_| {
            theme::resolve(&Preferences {
                theme: "classic".into(),
                ..Preferences::default()
            })
        })
        .expect("classic theme")
}

/// Load the microphone list off the UI thread (pw-dump takes a while).
fn load_devices(slot: Arc<Mutex<Vec<audio::Device>>>, wake: Waker) {
    let _ = thread::Builder::new()
        .name("nirudictate-panel-devices".into())
        .spawn(move || {
            let list = audio::devices();
            if let Ok(mut d) = slot.lock() {
                *d = list;
            }
            wake();
        });
}

impl App {
    fn new(cfg: Config, prefs: Arc<Mutex<Preferences>>, theme: ResolvedTheme, wake: Waker) -> Self {
        let watcher = ThemeWatcher::spawn(prefs.clone(), wake.clone());
        let status = Arc::new(Mutex::new(None));
        let refresh = status_thread(status.clone(), wake.clone());
        let vocab = cfg.vocabulary.words.join("\n");
        let agent_dir = cfg.agent.directory.clone();
        let agent_name = cfg.agent.agent.clone();
        let agent_program = cfg.agent.opencode.clone();
        let terminals = cfg.output.terminals.join(", ");
        let font_path = theme.appearance.font_path.clone();
        let devices = Arc::new(Mutex::new(Vec::new()));
        load_devices(devices.clone(), wake.clone());
        Self {
            saved: cfg.clone(),
            cfg,
            save_error: None,
            prefs,
            theme,
            watcher,
            wake,
            tab: Tab::Overview,
            status,
            refresh,
            hw: hardware::detect(),
            measured: measure::load().tiny(),
            downloads: HashMap::new(),
            mic: None,
            mic_result: None,
            devices,
            history: history::load(),
            vocab,
            terminals,
            capturing_shortcut: false,
            capturing_agent: false,
            capturing_command: false,
            agent_dir,
            agent_name,
            agent_program,
            opencode: Arc::new(Mutex::new(None)),
            confirm_clear: false,
            confirm_delete: None,
            toast: None,
            themes: theme::available_themes(),
            scroll: 0.0,
            scroll_drag: None,
            last_tab: Tab::Overview,
            font_path,
        }
    }

    fn p(&self) -> theme::Palette {
        self.theme.palette
    }

    fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some((text.into(), Instant::now(), error));
    }

    fn refresh_status(&self) {
        let _ = self.refresh.send(());
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
                    }
                }
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    fn status(&self) -> Option<Status> {
        self.status.lock().ok().and_then(|s| s.clone())
    }

    fn start_download(&mut self, model: &'static Model) {
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
        let (p, c, done, wake) = (
            d.progress.clone(),
            d.cancel.clone(),
            d.done.clone(),
            self.wake.clone(),
        );
        let _ = thread::Builder::new()
            .name("nirudictate-download".into())
            .spawn(move || {
                // Progress wakes the window at most five times a second.
                let mut shown = Instant::now() - Duration::from_secs(1);
                let r = models::download(model, &c, |pr| {
                    if let Ok(mut x) = p.lock() {
                        *x = pr;
                    }
                    if shown.elapsed() >= Duration::from_millis(200) {
                        shown = Instant::now();
                        wake();
                    }
                });
                if let Ok(mut x) = done.lock() {
                    *x = Some(r);
                }
                wake();
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

    fn start_mic_test(&mut self) {
        let wake = self.wake.clone();
        match Capture::start(&self.cfg.advanced.microphone, move || wake()) {
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

    fn finish_mic_test(&mut self) {
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
        let wake = self.wake.clone();
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
            wake();
        });
        self.mic = Some(MicTest::Transcribing(rx));
    }

    /// What `update` did before laying out: events from the workers.
    fn before_frame(&mut self, cx: &mut Ctx) {
        while let Ok(event) = self.watcher.rx.try_recv() {
            match event {
                ThemeEvent::Updated(t) => {
                    self.theme = *t;
                }
                ThemeEvent::Warning(w) => self.toast(w, true),
            }
        }
        cx.p = self.theme.palette;
        // The NIRUX appearance may change the font and its size live.
        let body = (self.theme.appearance.font_size
            * self.prefs.lock().map(|p| p.scale).unwrap_or(1.0))
        .clamp(10.0, 32.0);
        if body != cx.body {
            cx.body = body;
        }
        if self.theme.appearance.font_path != self.font_path {
            self.font_path = self.theme.appearance.font_path.clone();
            if let Ok(t) = gui::text::Text::new(self.font_path.as_deref()) {
                cx.text = t;
            }
        }
        self.poll_downloads();
        if let Some(MicTest::Recording { started, .. }) = &self.mic {
            if started.elapsed() >= Duration::from_secs(8) {
                self.finish_mic_test();
            } else {
                cx.request_repaint_after(Duration::from_millis(33));
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
    }

    fn layout(&mut self, cx: &mut Ctx) {
        self.before_frame(cx);
        let p = self.p();
        let (w, h) = cx.size;
        // A tiling window manager may give the panel a narrow column: the
        // navigation then moves to a wrapped tab row at the top.
        let narrow = w < 780.0;
        let central = if narrow {
            let mut ui = Ui::new(cx, gui::canvas::Rect::new(12.0, 10.0, w - 24.0, h));
            let slot = ui.cx.cmds.len();
            ui.cx
                .rect(gui::canvas::Rect::new(0.0, 0.0, w, 0.0), 0.0, p.panel);
            self.tab_row(&mut ui);
            let th = ui.used_height() + 20.0;
            if let gui::ui::Cmd::Rect { r, .. } = &mut cx.cmds[slot] {
                r.h = th;
            }
            gui::canvas::Rect::new(0.0, th, w, h - th)
        } else {
            cx.rect(gui::canvas::Rect::new(0.0, 0.0, 212.0, h), 0.0, p.panel);
            {
                let mut ui = Ui::new(
                    cx,
                    gui::canvas::Rect::new(14.0, 18.0, 212.0 - 28.0, h - 36.0),
                );
                self.nav(&mut ui, h);
            }
            gui::canvas::Rect::new(212.0, 0.0, w - 212.0, h)
        };
        let margin = if narrow { (12.0, 12.0) } else { (28.0, 22.0) };
        if self.tab != self.last_tab {
            self.scroll = 0.0;
            self.last_tab = self.tab;
        }
        // Scrolling: the wheel over the content, PageUp/PageDown, the bar.
        if let Some((x, y)) = cx.input.pos {
            if central.contains(x, y) && cx.input.wheel != 0.0 {
                self.scroll += cx.input.wheel;
                cx.input.wheel = 0.0;
            }
        }
        if !cx.has_focus() {
            if cx.take_key(0xff55) {
                self.scroll -= central.h * 0.85;
            }
            if cx.take_key(0xff56) {
                self.scroll += central.h * 0.85;
            }
        }
        let content_w = (central.w - 2.0 * margin.0 - 10.0).min(760.0);
        let content_h = {
            cx.rect(central, 0.0, p.background);
            let mut ui = Ui::new(
                cx,
                gui::canvas::Rect::new(
                    central.x + margin.0,
                    central.y + margin.1 - self.scroll,
                    content_w,
                    0.0,
                ),
            );
            ui.set_clip(Some(central));
            match self.tab {
                Tab::Overview => self.overview(&mut ui),
                Tab::Models => self.models_tab(&mut ui),
                Tab::Shortcut => self.shortcut_tab(&mut ui),
                Tab::Agent => self.agent_tab(&mut ui),
                Tab::Commands => self.commands_tab(&mut ui),
                Tab::Output => self.output_tab(&mut ui),
                Tab::Vocabulary => self.vocabulary_tab(&mut ui),
                Tab::History => self.history_tab(&mut ui),
                Tab::Advanced => self.advanced_tab(&mut ui),
            }
            ui.add_space(24.0);
            let used = ui.used_height() + 24.0;
            ui.set_clip(None);
            used
        };
        let view = central.h - 2.0 * margin.1;
        let max_scroll = (content_h - view).max(0.0);
        let clamped = self.scroll.clamp(0.0, max_scroll);
        if (clamped - self.scroll).abs() > 0.5 {
            self.scroll = clamped;
            cx.changed = true;
        }
        self.scroll = clamped;
        if max_scroll > 0.0 {
            self.scrollbar(cx, central, content_h + 2.0 * margin.1, max_scroll);
        }
        self.draw_toast(cx, central);
        if let Some(t) = cx.tooltip.clone() {
            draw_tooltip(cx, &t);
        }
        self.persist();
    }

    fn scrollbar(&mut self, cx: &mut Ctx, area: gui::canvas::Rect, total: f32, max_scroll: f32) {
        let p = self.p();
        let track = gui::canvas::Rect::new(area.right() - 10.0, area.y + 4.0, 6.0, area.h - 8.0);
        let th = (track.h * area.h / total).max(30.0).min(track.h);
        let ty = track.y + (track.h - th) * (self.scroll / max_scroll).clamp(0.0, 1.0);
        let thumb = gui::canvas::Rect::new(track.x, ty, track.w, th);
        let hot = cx.input.pos.is_some_and(|(x, y)| {
            gui::canvas::Rect::new(track.x - 4.0, track.y, 14.0, track.h).contains(x, y)
        });
        if cx.input.pressed && hot {
            let y = cx.input.pos.map_or(0.0, |p| p.1);
            if !thumb.contains(thumb.x + 1.0, y) {
                // Jump so the thumb centres on the pointer.
                let t = ((y - track.y - th / 2.0) / (track.h - th)).clamp(0.0, 1.0);
                self.scroll = t * max_scroll;
            }
            self.scroll_drag = Some((y, self.scroll));
            cx.changed = true;
        }
        if cx.input.released {
            self.scroll_drag = None;
        }
        if let (Some((y0, s0)), Some((_, y)), true) =
            (self.scroll_drag, cx.input.pos, cx.input.down)
        {
            let per = max_scroll / (track.h - th).max(1.0);
            self.scroll = (s0 + (y - y0) * per).clamp(0.0, max_scroll);
        }
        let c = if hot || self.scroll_drag.is_some() {
            p.muted
        } else {
            gui::ui::mix(p.background, p.muted, 0.5)
        };
        cx.rect(thumb, 3.0, c);
    }
}

fn draw_tooltip(cx: &mut Ctx, text: &str) {
    let Some((x, y)) = cx.input.pos else { return };
    let p = cx.p;
    let size = 13.0;
    let lh = cx.line_h(crate::fonts::Face::Sans, size);
    let tw = cx.tw(crate::fonts::Face::Sans, size, text);
    let (w, h) = (tw + 16.0, lh + 10.0);
    let rx = (x + 12.0).min(cx.size.0 - w - 4.0).max(4.0);
    let ry = if y + 22.0 + h < cx.size.1 {
        y + 22.0
    } else {
        y - h - 8.0
    };
    let r = gui::canvas::Rect::new(rx, ry, w, h);
    let start = cx.cmds.len();
    cx.framed(r, 6.0, p.surface_hi, Some(p.border_hi));
    cx.text_at(
        rx + 8.0,
        ry + 5.0,
        size,
        crate::fonts::Face::Sans,
        p.foreground,
        text,
    );
    let moved: Vec<_> = cx.cmds.drain(start..).collect();
    cx.over.extend(moved);
}

impl gui::App for App {
    fn frame(&mut self, cx: &mut Ctx) {
        self.layout(cx);
    }

    fn outside(&mut self, event: Outside) {
        match event {
            Outside::ServiceState | Outside::FocusIn => self.refresh_status(),
            Outside::Woken => {}
        }
    }

    fn exit(&mut self) {
        self.cfg.agent.opencode = self.agent_program.trim().to_string();
        self.persist();
        for d in self.downloads.values() {
            d.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl App {
    fn nav(&mut self, ui: &mut Ui, height: f32) {
        let p = self.p();
        ui.horizontal(|ui| {
            ui.logo(28.0);
            ui.vertical(|ui| {
                ui.label(
                    Rt::new("NIRUDICTATE")
                        .strong()
                        .size(16.0)
                        .color(p.foreground),
                );
                ui.label(Rt::new("Local dictation").size(12.0).color(p.muted));
            });
        });
        ui.add_space(14.0);
        self.status_chip(ui);
        ui.add_space(18.0);
        for (tab, label) in TABS {
            if ui.nav_item(label, self.tab == *tab).clicked {
                self.select(*tab);
            }
        }
        // The version and the theme at the bottom.
        let lh = ui.cx.line_h(crate::fonts::Face::Sans, 11.0);
        let y = 18.0 + height - 36.0 - 2.0 * lh - 4.0;
        if y > ui.cursor_y() {
            let version = format!("v{} · private, on this computer", env!("CARGO_PKG_VERSION"));
            let source = self.theme.source.clone();
            ui.cx
                .text_at(14.0, y, 11.0, crate::fonts::Face::Sans, p.disabled, &source);
            ui.cx.text_at(
                14.0,
                y + lh + 4.0,
                11.0,
                crate::fonts::Face::Sans,
                p.muted,
                &version,
            );
        }
    }

    fn select(&mut self, tab: Tab) {
        if self.tab == Tab::Agent && tab != Tab::Agent {
            // Keep what was typed into the program field.
            let program = self.agent_program.trim().to_string();
            if program != self.cfg.agent.opencode {
                self.cfg.agent.opencode = program;
            }
        }
        self.tab = tab;
        self.capturing_shortcut = false;
        self.capturing_agent = false;
        self.capturing_command = false;
        if tab == Tab::Agent {
            self.probe_opencode();
        }
        if tab == Tab::History {
            self.history = history::load();
        }
        if tab == Tab::Advanced {
            load_devices(self.devices.clone(), self.wake.clone());
            self.themes = theme::available_themes();
        }
    }

    fn status_chip(&self, ui: &mut Ui) {
        let p = self.p();
        let (label, color) = match self.status().as_ref().map(|s| s.state.as_str()) {
            Some("recording") => ("Recording", red(&p)),
            Some("transcribing") => ("Transcribing", p.warning),
            Some(_) => ("Ready", p.success),
            None => ("Service stopped", p.error),
        };
        ui.horizontal(|ui| ui.chip(label, color));
    }

    /// Navigation for a narrow window.
    fn tab_row(&mut self, ui: &mut Ui) {
        let p = self.p();
        ui.horizontal(|ui| {
            ui.logo(22.0);
            ui.label(Rt::new("NIRUDICTATE").strong().color(p.foreground));
            self.status_chip(ui);
        });
        ui.add_space(-2.0);
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in TABS {
                if ui.segment(label, self.tab == *tab).clicked {
                    self.select(*tab);
                }
            }
        });
    }

    fn draw_toast(&mut self, cx: &mut Ctx, area: gui::canvas::Rect) {
        let Some((text, at, error)) = self.toast.clone() else {
            return;
        };
        let age = at.elapsed();
        if age > Duration::from_secs(5) {
            self.toast = None;
            return;
        }
        cx.request_repaint_after(Duration::from_secs(5) - age + Duration::from_millis(10));
        let p = self.p();
        let size = cx.body;
        let lh = cx.line_h(crate::fonts::Face::Sans, size);
        let max = (area.w - 40.0).max(100.0);
        let tw = cx.tw(crate::fonts::Face::Sans, size, &text).min(max);
        let shown = {
            let s = cx.scale;
            cx.text
                .ellipsize(crate::fonts::Face::Sans, size * s, &text, max * s)
        };
        let (w, h) = (tw + 32.0, lh + 20.0);
        let r = gui::canvas::Rect::new(
            (area.x + (area.w - w) / 2.0).max(4.0),
            cx.size.1 - 20.0 - h,
            w,
            h,
        );
        let start = cx.cmds.len();
        cx.framed(
            r,
            10.0,
            p.surface_hi,
            Some(if error { p.error } else { p.border_hi }),
        );
        cx.text_at(
            r.x + 16.0,
            r.y + 10.0,
            size,
            crate::fonts::Face::Sans,
            if error { p.error } else { p.foreground },
            &shown,
        );
        let moved: Vec<_> = cx.cmds.drain(start..).collect();
        cx.over.extend(moved);
    }

    fn heading(&self, ui: &mut Ui, title: &str, subtitle: &str) {
        let p = self.p();
        ui.label(Rt::new(title).size(24.0).strong().color(p.foreground));
        ui.label(Rt::new(subtitle).color(p.muted));
        ui.add_space(14.0);
        if let Some(e) = &self.save_error {
            ui.label(Rt::new(format!("Not saved: {e}")).color(p.error));
        }
    }

    // ── Overview ──────────────────────────────────────────────────────────

    fn overview(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Overview",
            "Speak, and the text appears where you type. Nothing leaves this computer.",
        );
        let status = self.status();
        let lang = self.cfg.general.language.clone();
        let active = models::active(&lang, self.cfg.model_for(&lang), &self.hw);
        card(ui, |ui| {
            let (big, color, detail) = match status.as_ref() {
                None => (
                    "Service not running",
                    p.error,
                    "The shortcut does nothing until the service runs.".to_string(),
                ),
                Some(s) if s.state == "recording" => (
                    "Listening…",
                    red(&p),
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
            ui.horizontal(|ui| {
                ui.status_dot(color, 14.0);
                ui.label(Rt::new(big).size(20.0).strong().color(p.foreground));
            });
            ui.label(Rt::new(detail).color(p.muted));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if status.is_none() {
                    if ui.primary_button("Start service").clicked {
                        match start_service() {
                            Ok(()) => self.toast("Starting the service…", false),
                            Err(e) => self.toast(e, true),
                        }
                    }
                } else if status.as_ref().is_some_and(|s| s.state == "recording") {
                    if ui.button("Stop").clicked {
                        let _ = ipc::send(&Request::Stop, Duration::from_secs(2));
                    }
                } else if active.is_some()
                    && ui
                        .button("Dictate now")
                        .on_hover_text(
                            ui,
                            "Starts recording; the text goes to the window that has focus",
                        )
                        .clicked
                {
                    let _ = ipc::send(&Request::Start, Duration::from_secs(2));
                }
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            {
                {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(Rt::new("Language").color(p.muted));
                        for (code, label) in config::LANGUAGES {
                            if ui
                                .segment(label, self.cfg.general.language == *code)
                                .clicked
                            {
                                self.cfg.general.language = code.to_string();
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(Rt::new("Model").color(p.muted));
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
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(Rt::new("Shortcut").color(p.muted));
                        ui.keycaps(&self.cfg.shortcut.key);
                        ui.label(
                            Rt::new(match self.cfg.shortcut.mode {
                                Mode::Hybrid => "tap to toggle · hold to talk",
                                Mode::Toggle => "press to start and stop",
                                Mode::Hold => "hold to talk",
                            })
                            .color(p.muted),
                        );
                    });
                }
            }
        });
        if active.is_none() {
            let rec = models::recommend(&lang, &self.hw, self.measured);
            ui.add_space(12.0);
            card(ui, |ui| {
                section_title(ui, "Get started");
                ui.label(format!(
                    "Download the recommended {} model for this computer: {} ({}).",
                    config::language_label(&lang),
                    rec.label,
                    models::human_size(rec.bytes)
                ));
                ui.add_space(8.0);
                self.download_row(ui, rec, true);
            });
        }
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Microphone test");
            ui.label(
                Rt::new("Say a sentence. It is transcribed here and not inserted anywhere.")
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
                    if ui.primary_button("Stop and transcribe").clicked {
                        self.finish_mic_test();
                    } else {
                        ui.level_meter(&levels, 260.0, 30.0);
                        ui.label(
                            Rt::new(format!("{:.0} s", started.elapsed().as_secs_f32()))
                                .color(p.muted),
                        );
                    }
                }
                Some(MicTest::Transcribing(_)) => {
                    ui.spinner();
                    ui.label("Transcribing…");
                    ui.cx.request_repaint_after(Duration::from_millis(100));
                }
                None => {
                    if ui.primary_button("Test microphone").clicked {
                        self.start_mic_test();
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
                    ui.quote(&shown);
                    ui.label(
                        Rt::new(format!(
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
                    ui.label(Rt::new(e).color(p.error));
                }
                None => {}
            }
        });
        if let Some(last) = status.as_ref().and_then(|s| s.last.clone()) {
            ui.add_space(12.0);
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    section_title(ui, "Last dictation");
                    ui.right_to_left(|ui| {
                        if ui.small_button("Copy").clicked {
                            ui.cx.copy = Some(last.text.clone());
                            self.toast("Copied", false);
                        }
                    });
                });
                ui.quote(&last.text);
                ui.label(
                    Rt::new(format!(
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

    fn download_row(&mut self, ui: &mut Ui, m: &'static Model, big: bool) {
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
            ui.horizontal_wrapped(|ui| {
                ui.progress_bar(frac, if big { 320.0 } else { 200.0 }, &text);
                if ui.small_button("Cancel").clicked {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            });
        } else if big {
            if ui
                .primary_button(&format!("Download {}", models::human_size(m.bytes)))
                .clicked
            {
                self.start_download(m);
            }
        } else if ui
            .button(format!("Download · {}", models::human_size(m.bytes)))
            .clicked
        {
            self.start_download(m);
        }
    }

    fn models_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Languages & Models",
            "Models are downloaded once, verified, and run on this computer only.",
        );
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(Rt::new("Dictation language").color(p.muted));
                for (code, label) in config::LANGUAGES {
                    if ui
                        .segment(label, self.cfg.general.language == *code)
                        .clicked
                    {
                        self.cfg.general.language = code.to_string();
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                Rt::new(format!(
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
            ui.horizontal_wrapped(|ui| {
                section_title(ui, label);
                ui.label(
                    Rt::new(match *code {
                        "sv" => "KB-Whisper by KBLab (National Library of Sweden) · Apache-2.0",
                        _ => "OpenAI Whisper via whisper.cpp · MIT",
                    })
                    .size(12.0)
                    .color(p.muted),
                );
            });
            card(ui, |ui| {
                for m in models::for_language(code) {
                    let installed = m.installed();
                    let is_active = active.map(|a| a.id) == Some(m.id);
                    {
                        {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(Rt::new(m.label).strong().color(p.foreground));
                                if m.id == rec.id {
                                    ui.chip("Recommended", p.accent);
                                }
                                if is_active {
                                    ui.chip("In use", p.success);
                                }
                            });
                            ui.label(
                                Rt::new(format!(
                                    "{} · about {:.1} s per dictation here · {} MB memory",
                                    models::human_size(m.bytes),
                                    models::estimate(m, &self.hw, self.measured),
                                    m.memory_mb
                                ))
                                .size(12.0)
                                .color(p.muted),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            if installed {
                                if self.confirm_delete == Some(m.id) {
                                    if ui.button(Rt::new("Delete").color(p.error)).clicked {
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
                                    if ui.button("Keep").clicked {
                                        self.confirm_delete = None;
                                    }
                                } else {
                                    if !is_active && ui.button("Use").clicked {
                                        self.cfg.set_model_for(code, m.id);
                                    }
                                    if ui.small_button("Delete…").clicked {
                                        self.confirm_delete = Some(m.id);
                                    }
                                }
                            } else {
                                self.download_row(ui, m, false);
                            }
                        });
                    }
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(Rt::new("Automatic choice:").size(12.0).color(p.muted));
                    let auto = self.cfg.model_for(code).is_empty();
                    if ui
                        .radio(auto, "largest fast enough installed model")
                        .clicked
                    {
                        self.cfg.set_model_for(code, "");
                    }
                });
            });
        }
        ui.add_space(12.0);
        ui.label(
            Rt::new(format!("Stored in {}", config::models_dir().display()))
                .size(12.0)
                .color(p.muted),
        );
    }

    // ── Shortcut ──────────────────────────────────────────────────────────

    fn shortcut_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Shortcut & Mode",
            "One key combination starts and stops dictation in any application.",
        );
        card(ui, |ui| {
            section_title(ui, "Shortcut");
            ui.horizontal_wrapped(|ui| {
                if self.capturing_shortcut {
                    ui.label(Rt::new("Press the new combination… (Esc cancels)").color(p.accent));
                    if let Some(spec) = widgets::capture_shortcut(&ui.cx.input.keys) {
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
                    ui.keycaps(&self.cfg.shortcut.key);
                    if ui.button("Change…").clicked {
                        self.capturing_shortcut = true;
                    }
                    if self.cfg.shortcut.key != "ctrl+alt+space" && ui.button("Default").clicked {
                        self.cfg.shortcut.key = "ctrl+alt+space".into();
                    }
                }
            });
            if let Some(s) = self.status() {
                if !s.shortcut_active {
                    ui.label(
                        Rt::new(
                            s.error
                                .unwrap_or_else(|| "The shortcut is not active".into()),
                        )
                        .color(p.warning),
                    );
                }
            }
            ui.label(
                Rt::new("NIRUDICTATE listens for this combination itself; the window manager is not changed. Ctrl+Alt+Space is free in NIRUWM.")
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Mode");
            for mode in [Mode::Hybrid, Mode::Toggle, Mode::Hold] {
                ui.radio_value(&mut self.cfg.shortcut.mode, mode, mode.label());
            }
            ui.add_space(8.0);
            let col = col(
                ui,
                &[
                    "Hold longer than",
                    "Stop after silence",
                    "Longest dictation",
                ],
            );
            if self.cfg.shortcut.mode == Mode::Hybrid {
                grid_row(ui, col, "Hold longer than", |ui| {
                    if let Some(v) = ui.slider(
                        "hold",
                        self.cfg.shortcut.hold_ms as f64,
                        150.0,
                        1000.0,
                        1.0,
                        0,
                        " ms",
                        "for push-to-talk",
                    ) {
                        self.cfg.shortcut.hold_ms = v as u64;
                    }
                });
            }
            if self.cfg.shortcut.mode != Mode::Hold {
                grid_row(ui, col, "Stop after silence", |ui| {
                    let mut on = self.cfg.shortcut.silence_seconds > 0.0;
                    if ui.checkbox(&mut on, "").changed {
                        self.cfg.shortcut.silence_seconds = if on { 2.0 } else { 0.0 };
                    }
                    if on {
                        if let Some(v) = ui.slider(
                            "silence",
                            self.cfg.shortcut.silence_seconds as f64,
                            0.5,
                            10.0,
                            0.5,
                            1,
                            " s",
                            "",
                        ) {
                            self.cfg.shortcut.silence_seconds = v as f32;
                        }
                    } else {
                        ui.label(Rt::new("off — press again to stop").color(p.muted));
                    }
                });
            }
            grid_row(ui, col, "Longest dictation", |ui| {
                if let Some(v) = ui.slider(
                    "longest",
                    self.cfg.shortcut.max_seconds as f64,
                    10.0,
                    600.0,
                    1.0,
                    0,
                    " s",
                    "",
                ) {
                    self.cfg.shortcut.max_seconds = v as u32;
                }
            });
        });
    }

    // ── Agent mode ────────────────────────────────────────────────────────

    /// Ask opencode for its version off the UI thread.
    fn probe_opencode(&mut self) {
        let slot = self.opencode.clone();
        let configured = self.cfg.agent.opencode.clone();
        let _ = thread::Builder::new()
            .name("nirudictate-panel-opencode".into())
            .spawn(move || {
                let result = match crate::agent::find(&configured) {
                    Some(path) => crate::agent::probe(&path),
                    None => Err("opencode is not installed".into()),
                };
                if let Ok(mut s) = slot.lock() {
                    *s = Some(result);
                }
            });
    }

    fn agent_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Agent mode",
            "Talk to opencode: speak, check the text, and it goes to the agent in scratchpad 9.",
        );
        let status = self.status().and_then(|s| s.agent);
        let found = self.opencode.lock().ok().and_then(|o| o.clone());
        card(ui, |ui| {
            section_title(ui, "opencode");
            match &found {
                Some(Ok(o)) => {
                    ui.label(
                        Rt::new(format!("opencode {} — {}", o.version, o.path.display()))
                            .color(p.success),
                    );
                }
                Some(Err(e)) => {
                    ui.label(Rt::new(e.clone()).color(p.warning));
                    ui.label(
                        Rt::new("Agent mode needs opencode (https://opencode.ai). Without it the agent shortcut is not taken.")
                            .size(12.0)
                            .color(p.muted),
                    );
                }
                None => {
                    ui.label(Rt::new("Looking for opencode…").color(p.muted));
                    ui.cx.request_repaint_after(Duration::from_millis(200));
                }
            }
            ui.checkbox(&mut self.cfg.agent.enabled, "Use agent mode");
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Shortcut");
            ui.horizontal_wrapped(|ui| {
                if self.capturing_agent {
                    ui.label(Rt::new("Press the new combination… (Esc cancels)").color(p.accent));
                    if let Some(spec) = widgets::capture_shortcut(&ui.cx.input.keys) {
                        if spec == "escape" {
                            self.capturing_agent = false;
                        } else if hotkey::parse(&spec).is_ok()
                            && hotkey::parse(&spec).ok()
                                != hotkey::parse(&self.cfg.shortcut.key).ok()
                        {
                            self.cfg.agent.key = spec;
                            self.capturing_agent = false;
                        } else {
                            self.toast(
                                "Use a modifier with a key, different from the dictation shortcut",
                                true,
                            );
                        }
                    }
                } else {
                    ui.keycaps(&self.cfg.agent.key);
                    if ui.button("Change…").clicked {
                        self.capturing_agent = true;
                    }
                    if self.cfg.agent.key != "shift+alt+space" && ui.button("Default").clicked {
                        self.cfg.agent.key = "shift+alt+space".into();
                    }
                }
            });
            if let Some(a) = &status {
                if self.cfg.agent.enabled && (a.problem.is_some() || !a.shortcut_active) {
                    ui.label(
                        Rt::new(
                            a.problem
                                .clone()
                                .unwrap_or_else(|| "The agent shortcut is not active".into()),
                        )
                        .color(p.warning),
                    );
                }
            }
            ui.label(
                Rt::new(format!(
                    "Works like the dictation shortcut ({}): tap to start, it stops after a pause or a second tap; or hold while you speak. Tap twice quickly to start a new conversation.",
                    self.cfg.shortcut.mode.label()
                ))
                .size(12.0)
                .color(p.muted),
            );
            ui.checkbox(
                &mut self.cfg.agent.double_tap_new,
                "A quick double tap starts a new conversation",
            );
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Before sending");
            let c = col(ui, &["Send by itself after"]);
            grid_row(ui, c, "Send by itself after", |ui| {
                if let Some(v) = ui.slider(
                    "confirm",
                    self.cfg.agent.confirm_seconds as f64,
                    0.0,
                    10.0,
                    0.5,
                    1,
                    " s",
                    "",
                ) {
                    self.cfg.agent.confirm_seconds = v as f32;
                }
            });
            ui.label(
                Rt::new("A small box shows what will be sent. Enter sends at once, Esc cancels. 0 s sends without the box.")
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Conversation");
            let c = col(ui, &["Working folder", "opencode agent"]);
            grid_row(ui, c, "Working folder", |ui| {
                if ui
                    .text_edit("agent-dir", &mut self.agent_dir, "~", Some(280.0), 0, false)
                    .changed
                {
                    let d = self.agent_dir.trim();
                    if d.is_empty() || d.starts_with('~') || d.starts_with('/') {
                        self.cfg.agent.directory = if d.is_empty() { "~".into() } else { d.into() };
                    }
                }
            });
            grid_row(ui, c, "opencode agent", |ui| {
                if ui
                    .text_edit(
                        "agent-name",
                        &mut self.agent_name,
                        "default agent",
                        Some(280.0),
                        0,
                        false,
                    )
                    .changed
                {
                    let n = self.agent_name.trim();
                    if !n.chars().any(char::is_whitespace) {
                        self.cfg.agent.agent = n.into();
                    }
                }
            });
            ui.label(
                Rt::new("opencode works in this folder (~ is your home folder). Leave the agent empty for opencode's default agent. Nothing is approved automatically: opencode asks in its window as usual.")
                    .size(12.0)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui.primary_button("Start a new conversation").clicked {
                    match ipc::send(&Request::AgentNew, Duration::from_secs(2)) {
                        Ok(r) if r.ok => {
                            self.toast("The next message starts a new conversation", false)
                        }
                        Ok(r) => self.toast(r.message.unwrap_or_else(|| "failed".into()), true),
                        Err(e) => self.toast(e, true),
                    }
                }
                if ui.button("Show opencode (Super+Alt+9)").clicked {
                    if let Err(e) = ipc::send(&Request::AgentShow, Duration::from_secs(2)) {
                        self.toast(e, true);
                    }
                }
            });
            if let Some(a) = &status {
                let line = match (&a.info.session, a.info.fresh) {
                    (_, true) => "The next message starts a new conversation.".to_string(),
                    (Some(id), false) => format!("Continuing conversation {id}."),
                    (None, false) => "The first message starts a conversation.".to_string(),
                };
                ui.label(Rt::new(line).size(12.0).color(p.muted));
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Window");
            let c = col(ui, &["Terminal", "opencode program"]);
            grid_row(ui, c, "Terminal", |ui| {
                let items: Vec<String> = config::TERMINALS.iter().map(|t| t.to_string()).collect();
                if let Some(i) = ui.combo("agent-terminal", &self.cfg.agent.terminal, None, &items)
                {
                    self.cfg.agent.terminal = items[i].clone();
                }
            });
            let mut probe = false;
            grid_row(ui, c, "opencode program", |ui| {
                if ui
                    .text_edit(
                        "agent-program",
                        &mut self.agent_program,
                        "found automatically",
                        Some(280.0),
                        0,
                        false,
                    )
                    .lost_focus
                {
                    self.cfg.agent.opencode = self.agent_program.trim().to_string();
                    probe = true;
                }
            });
            if probe {
                self.probe_opencode();
            }
            ui.label(
                Rt::new("opencode runs in NIRUWM scratchpad 9 (Super+Alt+9) and comes to the front when a message is sent. Closing that window ends its opencode server; the next message opens it again and continues the conversation.")
                    .size(12.0)
                    .color(p.muted),
            );
        });
    }

    // ── Voice commands ────────────────────────────────────────────────────

    fn commands_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Voice commands",
            "Say what to do in Swedish or English: \"öppna terminalen\", \"go to workspace 3\", \"lås skärmen\".",
        );
        let status = self.status().and_then(|s| s.command);
        card(ui, |ui| {
            section_title(ui, "Shortcut");
            ui.checkbox(&mut self.cfg.command.enabled, "Use voice commands");
            ui.horizontal_wrapped(|ui| {
                if self.capturing_command {
                    ui.label(
                        Rt::new("Press the new combination… (Esc cancels)").color(p.accent),
                    );
                    if let Some(spec) = widgets::capture_shortcut(&ui.cx.input.keys) {
                        let taken = [&self.cfg.shortcut.key, &self.cfg.agent.key]
                            .iter()
                            .any(|k| hotkey::parse(k).ok() == hotkey::parse(&spec).ok());
                        if spec == "escape" {
                            self.capturing_command = false;
                        } else if hotkey::parse(&spec).is_ok() && !taken {
                            self.cfg.command.key = spec;
                            self.capturing_command = false;
                        } else {
                            self.toast(
                                "Use a modifier with a key, different from the dictation and agent shortcuts",
                                true,
                            );
                        }
                    }
                } else {
                    ui.keycaps(&self.cfg.command.key);
                    if ui.button("Change…").clicked {
                        self.capturing_command = true;
                    }
                    if self.cfg.command.key != "super+alt+space" && ui.button("Default").clicked
                    {
                        self.cfg.command.key = "super+alt+space".into();
                    }
                }
            });
            if let Some(c) = &status {
                if self.cfg.command.enabled && (c.problem.is_some() || !c.shortcut_active) {
                    ui.label(
                        Rt::new(
                            c.problem
                                .clone()
                                .unwrap_or_else(|| "The command shortcut is not active".into()),
                        )
                        .color(p.warning),
                    );
                }
            }
            ui.label(
                Rt::new(format!(
                    "Works like the dictation shortcut ({}): tap, say the command, and it runs after a short pause; or hold while you speak. Nothing else is ever run: only the commands listed below.",
                    self.cfg.shortcut.mode.label()
                ))
                .size(12.0)
                .color(p.muted),
            );
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Listening");
            let label = |l: &str| match l {
                "both" => "Dictation language, then the other",
                "auto" => "Detect the language (slower)",
                "sv" => "Swedish",
                "en" => "English",
                _ => "Only the dictation language",
            };
            let c = col(ui, &["Language"]);
            grid_row(ui, c, "Language", |ui| {
                let items: Vec<String> = config::COMMAND_LANGUAGES
                    .iter()
                    .map(|l| label(l).to_string())
                    .collect();
                if let Some(i) = ui.combo(
                    "command-language",
                    label(&self.cfg.command.language),
                    None,
                    &items,
                ) {
                    self.cfg.command.language = config::COMMAND_LANGUAGES[i].to_string();
                }
            });
            ui.label(
                Rt::new("With both models installed, a command that the dictation model did not understand is tried once more with the other language's model (Languages & Models). With the Swedish model alone, many English commands still work: they come out in Swedish and match.")
                    .size(12.0)
                    .color(p.muted),
            );
            ui.checkbox(
                &mut self.cfg.command.app_names,
                "Help the model with the names of the programs you start most",
            );
            ui.checkbox(
                &mut self.cfg.command.confirm_dangerous,
                "Ask before closing a window, suspending, restarting, shutting down or logging out",
            );
            if !self.cfg.command.confirm_dangerous {
                ui.label(
                    Rt::new("These run at once when they are understood.")
                        .size(12.0)
                        .color(p.warning),
                );
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "What you can say");
            ui.label(
                Rt::new("Politeness and fillers are fine (\"kan du\", \"snälla\", \"please\"). Numbers work as words or digits. When a command is not understood nothing runs.")
                    .size(12.0)
                    .color(p.muted),
            );
            ui.add_space(4.0);
            let mut last = "";
            for (lang, label, examples) in crate::command::phrase_list() {
                if label != last {
                    ui.add_space(4.0);
                    ui.label(Rt::new(label).strong().color(p.foreground));
                    last = label;
                }
                ui.label(
                    Rt::new(format!("{}  {}", lang.code(), examples.join(" · ")))
                        .size(12.0)
                        .color(p.muted),
                );
            }
        });
    }

    // ── Output ────────────────────────────────────────────────────────────

    fn output_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Output",
            "How the text reaches the window you are typing in.",
        );
        card(ui, |ui| {
            section_title(ui, "Insert by");
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
        card(ui, |ui| {
            section_title(ui, "Text");
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
        card(ui, |ui| {
            section_title(ui, "Terminals");
            ui.label(
                Rt::new(format!(
                    "Recognised automatically: {} and more. xterm and urxvt use Shift+Insert.",
                    crate::insert::TERMINALS[..6].join(", ")
                ))
                .size(12.0)
                .color(p.muted),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label("Also treat as terminals (WM_CLASS):");
                if ui
                    .text_edit(
                        "terminals",
                        &mut self.terminals,
                        "e.g. myterm, cool-retro-term",
                        Some(260.0),
                        0,
                        false,
                    )
                    .changed
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
        card(ui, |ui| {
            section_title(ui, "Feedback");
            ui.checkbox(
                &mut self.cfg.general.overlay,
                "Show the recording overlay at the bottom of the screen",
            );
            ui.checkbox(&mut self.cfg.general.sounds, "Play the dictation sounds");
            ui.label(
                Rt::new(if crate::sounds::available() {
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

    fn vocabulary_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Vocabulary",
            "Names and words the model should spell your way: products, people, places, jargon.",
        );
        card(ui, |ui| {
            ui.label(Rt::new("One word or phrase per line").color(p.muted));
            let changed = ui
                .text_edit("vocabulary", &mut self.vocab, "", None, 12, true)
                .changed;
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
                Rt::new("Given to the model as a hint:")
                    .size(12.0)
                    .color(p.muted),
            );
            ui.quote(if prompt.is_empty() {
                "(nothing)"
            } else {
                &prompt
            });
            ui.label(
                Rt::new("A short list works best; the model only sees about 200 words of hint.")
                    .size(12.0)
                    .color(p.muted),
            );
        });
    }

    // ── History ───────────────────────────────────────────────────────────

    fn history_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "History",
            "Kept only on this computer, readable only by you. Audio is never stored.",
        );
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.cfg.history.enabled, "Keep a history");
                if self.cfg.history.enabled {
                    ui.label("· keep the last");
                    let mut keep = self.cfg.history.keep.min(u32::MAX as usize) as u32;
                    if ui.drag_value("keep", &mut keep, 10, 5000, 5.0, "").changed {
                        self.cfg.history.keep = keep as usize;
                    }
                    ui.label("dictations");
                }
                ui.right_to_left(|ui| {
                    if self.confirm_clear {
                        if ui
                            .button(Rt::new("Delete everything").color(p.error))
                            .clicked
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
                        if ui.button("Keep").clicked {
                            self.confirm_clear = false;
                        }
                    } else if !self.history.is_empty() && ui.button("Clear all…").clicked {
                        self.confirm_clear = true;
                    }
                });
            });
            ui.label(
                Rt::new(format!("{} (mode 0600)", config::history_path().display()))
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.add_space(12.0);
        if self.history.is_empty() {
            ui.label(Rt::new("Nothing yet.").color(p.muted));
            return;
        }
        let mut delete = None;
        for e in self.history.iter().rev().take(300) {
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(Rt::new(widgets::when(e.time)).size(12.0).color(p.muted));
                    ui.label(
                        Rt::new(format!(
                            "{} · {:.1} s · {}",
                            e.model, e.audio_seconds, e.language
                        ))
                        .size(12.0)
                        .color(p.disabled),
                    );
                    ui.right_to_left(|ui| {
                        if ui.small_button("Delete").clicked {
                            delete = Some((e.time, e.text.clone()));
                        }
                        if ui.small_button("Copy").clicked {
                            ui.cx.copy = Some(e.text.clone());
                        }
                    });
                });
                ui.label(Rt::new(&e.text).color(p.foreground));
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

    fn advanced_tab(&mut self, ui: &mut Ui) {
        let p = self.p();
        self.heading(
            ui,
            "Advanced",
            "Microphone, performance and the background service.",
        );
        card(ui, |ui| {
            section_title(ui, "Microphone");
            let devices = self.devices.lock().map(|d| d.clone()).unwrap_or_default();
            let current = if self.cfg.advanced.microphone.is_empty() {
                "System default".to_string()
            } else {
                devices
                    .iter()
                    .find(|d| d.name == self.cfg.advanced.microphone)
                    .map(|d| d.description.clone())
                    .unwrap_or_else(|| self.cfg.advanced.microphone.clone())
            };
            let mut items = vec!["System default".to_string()];
            items.extend(devices.iter().map(|d| d.description.clone()));
            ui.horizontal(|ui| {
                if let Some(i) = ui.combo("mic", &current, Some(380.0), &items) {
                    self.cfg.advanced.microphone = if i == 0 {
                        String::new()
                    } else {
                        devices[i - 1].name.clone()
                    };
                }
            });
            ui.label(
                Rt::new(format!(
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
        card(ui, |ui| {
            section_title(ui, "Performance");
            let col = col(ui, &["Threads", "Priority", "Model memory"]);
            grid_row(ui, col, "Threads", |ui| {
                let auto = hardware::threads_for(&self.hw);
                let mut is_auto = self.cfg.advanced.threads == 0;
                if ui
                    .checkbox(&mut is_auto, &format!("Automatic ({auto})"))
                    .changed
                {
                    self.cfg.advanced.threads = if is_auto { 0 } else { auto };
                }
                if !is_auto {
                    if let Some(v) = ui.slider(
                        "threads",
                        self.cfg.advanced.threads as f64,
                        1.0,
                        self.hw.logical.max(1) as f64,
                        1.0,
                        0,
                        "",
                        "",
                    ) {
                        self.cfg.advanced.threads = v as u32;
                    }
                }
            });
            grid_row(ui, col, "Priority", |ui| {
                if let Some(v) = ui.slider(
                    "nice",
                    self.cfg.advanced.nice as f64,
                    0.0,
                    19.0,
                    1.0,
                    0,
                    "",
                    "nice",
                ) {
                    self.cfg.advanced.nice = v as i32;
                }
                if ui.cx.slider_hovered {
                    ui.cx.tooltip = Some(
                        "Higher values let music, visualizers and the desktop go first".into(),
                    );
                }
            });
            grid_row(ui, col, "Model memory", |ui| {
                ui.vertical(|ui| {
                    ui.checkbox(
                        &mut self.cfg.advanced.keep_model_loaded,
                        "Keep the model loaded (faster first dictation)",
                    );
                    if !self.cfg.advanced.keep_model_loaded {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Unload after");
                            ui.drag_value(
                                "unload",
                                &mut self.cfg.advanced.unload_after_minutes,
                                1,
                                1440,
                                1.0,
                                " min",
                            );
                            ui.label("without dictation");
                        });
                    }
                });
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Appearance");
            let label = |key: &str, themes: &[(String, String)]| match key {
                "auto" => "Follow NIRUX (or Omarchy)".to_string(),
                "classic" => "NIRUDICTATE Classic".to_string(),
                k => themes
                    .iter()
                    .find(|t| t.0 == k)
                    .map(|t| t.1.clone())
                    .unwrap_or_else(|| k.to_string()),
            };
            let mut keys = vec!["auto".to_string(), "classic".to_string()];
            keys.extend(self.themes.iter().map(|t| t.0.clone()));
            let items: Vec<String> = keys.iter().map(|k| label(k, &self.themes)).collect();
            let current = label(&self.cfg.general.theme, &self.themes);
            ui.horizontal(|ui| {
                if let Some(i) = ui.combo("theme", &current, Some(300.0), &items) {
                    self.cfg.general.theme = keys[i].clone();
                }
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Background service");
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
                    ui.label(Rt::new("Not running").color(p.error));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if status.is_some() {
                    if ui.button("Restart service").clicked {
                        let _ = ipc::send(&Request::Quit, Duration::from_secs(2));
                        thread::sleep(Duration::from_millis(300));
                        match start_service() {
                            Ok(()) => self.toast("Service restarted", false),
                            Err(e) => self.toast(e, true),
                        }
                    }
                    if status.as_ref().and_then(|s| s.loaded.as_ref()).is_some()
                        && ui.button("Unload model now").clicked
                    {
                        let _ = ipc::send(&Request::Unload, Duration::from_secs(2));
                    }
                } else if ui.button("Start service").clicked {
                    match start_service() {
                        Ok(()) => self.toast("Starting the service…", false),
                        Err(e) => self.toast(e, true),
                    }
                }
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section_title(ui, "Files");
            for (what, path) in [
                ("Settings", config::config_path()),
                ("Models", config::models_dir()),
                ("History", config::history_path()),
                ("Service log", config::log_path()),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(Rt::new(what).color(p.muted));
                    ui.label(Rt::new(path.display().to_string()).monospace().size(12.0));
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
    let mut windows = conn
        .get_property(false, root, list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
        .unwrap_or_default();
    if windows.is_empty() {
        // No EWMH window manager: the top-level windows.
        windows = conn
            .query_tree(root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|t| t.children)
            .unwrap_or_default();
    }
    for w in windows {
        let class = conn
            .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.value)
            .unwrap_or_default();
        if class.split(|b| *b == 0).any(|s| s == b"nirudictate") {
            let ev = ClientMessageEvent::new(32, w, active, [1u32, 0, 0, 0, 0]);
            let _ = conn.configure_window(
                w,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
            );
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

pub fn run(tab: Option<&str>) -> Result<(), String> {
    let start_tab = match tab.unwrap_or("overview") {
        "models" | "languages" => Tab::Models,
        "shortcut" | "mode" => Tab::Shortcut,
        "agent" => Tab::Agent,
        "commands" | "command" | "voice" => Tab::Commands,
        "output" => Tab::Output,
        "vocabulary" => Tab::Vocabulary,
        "history" => Tab::History,
        "advanced" => Tab::Advanced,
        _ => Tab::Overview,
    };
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
    let cfg = Config::load_or_default();
    let prefs = Arc::new(Mutex::new(Preferences {
        theme: cfg.general.theme.clone(),
        ..Preferences::default()
    }));
    let theme = resolve_theme(&prefs.lock().expect("prefs").clone());
    let body = {
        let scale = prefs.lock().map(|p| p.scale).unwrap_or(1.0);
        (theme.appearance.font_size * scale).clamp(10.0, 32.0)
    };
    let window = gui::open(
        "NIRUDICTATE",
        (980.0, 680.0),
        (760.0, 520.0),
        widgets::icon(),
        theme.appearance.font_path.as_deref(),
        theme.palette,
        body,
    )?;
    let mut app = App::new(cfg, prefs, theme, window.waker());
    app.tab = start_tab;
    app.last_tab = start_tab;
    if start_tab != Tab::Overview {
        app.select(start_tab);
    }
    window.run(&mut app)?;
    drop(lock);
    Ok(())
}
