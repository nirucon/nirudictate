//! The background service: owns the shortcut, the microphone while
//! recording, the overlay, and hands audio to the engine and text to the
//! inserter.
//!
//! One thread waits in `poll` on the X connection, a wake pipe and a signal
//! pipe. While idle nothing runs and nothing times out: no microphone, no
//! model, no animation, no timers.
use crate::audio::{self, Capture};
use crate::config::{self, Config};
use crate::engine::{self, Engine, Job, Loaded, Transcript};
use crate::hardware::{self, Hardware};
use crate::hotkey::{self, Action, Controller};
use crate::insert::{Insert, Inserter};
use crate::ipc::{LastResult, Reply, Request, Status};
use crate::models::{self, Model};
use crate::overlay::{self, Overlay, View};
use crate::sounds::{self, Cue};
use crate::{history, live, text, theme};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, GrabMode, ModMask, PropMode};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub fn pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe2 fills two new descriptors that we take ownership of.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return None;
    }
    // SAFETY: both descriptors were just created and are owned by nobody else.
    unsafe { Some((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

fn drain(fd: &OwnedFd) {
    let mut buf = [0u8; 256];
    loop {
        // SAFETY: reading into a local buffer from our own non-blocking pipe.
        let n = unsafe {
            libc::read(
                fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n <= 0 {
            return;
        }
    }
}

/// Wakes the main loop from another thread.
#[derive(Clone)]
pub struct Waker(Arc<OwnedFd>);

impl Waker {
    pub fn wake(&self) {
        // SAFETY: one byte from a local buffer to our own pipe.
        unsafe { libc::write(self.0.as_raw_fd(), [1u8].as_ptr() as *const libc::c_void, 1) };
    }
}

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write(2) is async-signal-safe.
        unsafe { libc::write(fd, [1u8].as_ptr() as *const libc::c_void, 1) };
    }
}

/// Hold an exclusive lock for the life of the service; a second one exits.
fn single_instance() -> Result<Option<std::fs::File>, String> {
    let dir = config::runtime_dir();
    config::ensure_private_dir(&dir)?;
    let path = dir.join("daemon.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: flock on a descriptor we own.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(None);
    }
    Ok(Some(file))
}

enum Phase {
    Idle,
    Recording {
        capture: Capture,
        model: &'static Model,
        levels_seen: usize,
    },
    Transcribing {
        rx: Receiver<Result<Transcript, String>>,
        started: Instant,
    },
    Inserting {
        rx: Receiver<Result<String, String>>,
        transcript: Transcript,
        text: String,
    },
}

impl Phase {
    fn word(&self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Recording { .. } => "recording",
            Phase::Transcribing { .. } | Phase::Inserting { .. } => "transcribing",
        }
    }
}

type Client = (Request, Sender<Reply>);
type PendingFile = (Sender<Reply>, Receiver<Result<Transcript, String>>);

struct Daemon {
    conn: RustConnection,
    screen: usize,
    root: u32,
    cfg: Config,
    hw: Hardware,
    engine: Engine,
    loaded: Arc<Mutex<Loaded>>,
    inserter: Option<Inserter>,
    overlay: Option<Overlay>,
    font: Option<ab_glyph::FontArc>,
    palette: Option<theme::Palette>,
    scale: f32,
    controller: Controller,
    phase: Phase,
    grabbed: Vec<(u8, u16)>,
    shortcut_error: Option<String>,
    last: Option<LastResult>,
    error: Option<String>,
    message: Option<(View, Instant)>,
    pending_files: Vec<PendingFile>,
    state_atom: u32,
    utf8: u32,
    waker: Waker,
    clients: Receiver<Client>,
    started_panel: Option<Instant>,
}

fn intern(conn: &RustConnection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0)
}

fn dpi_scale(conn: &RustConnection, root: u32) -> f32 {
    // Xft.dpi from the resource database, like GTK and Qt.
    let dpi = conn
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            65536,
        )
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| {
            String::from_utf8_lossy(&r.value)
                .lines()
                .find_map(|l| {
                    l.strip_prefix("Xft.dpi:")
                        .map(|v| v.trim().parse::<f32>().ok())
                })
                .flatten()
        })
        .unwrap_or(96.0);
    (dpi / 96.0).clamp(1.0, 3.0)
}

pub fn run(foreground_message: bool) -> Result<(), String> {
    let Some(_lock) = single_instance()? else {
        if foreground_message {
            println!("NIRUDICTATE is already running");
        }
        return Ok(());
    };
    let (conn, screen) =
        x11rb::connect(None).map_err(|e| format!("cannot open the X display: {e}"))?;
    let root = conn.setup().roots[screen].root;
    // Key releases without autorepeat noise: a held key sends one press and
    // one release.
    if conn
        .xkb_use_extension(1, 0)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| r.supported)
    {
        let flag = xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT;
        let _ = conn
            .xkb_per_client_flags(
                xkb::ID::USE_CORE_KBD.into(),
                flag,
                flag,
                xkb::BoolCtrl::from(0u32),
                xkb::BoolCtrl::from(0u32),
                xkb::BoolCtrl::from(0u32),
            )
            .ok()
            .and_then(|c| c.reply().ok());
    }
    let cfg = Config::load_or_default();
    let (wake_read, wake_write) = pipe().ok_or("cannot create a pipe")?;
    let waker = Waker(Arc::new(wake_write));
    let (sig_read, sig_write) = pipe().ok_or("cannot create a pipe")?;
    SIGNAL_FD.store(sig_write.as_raw_fd(), Ordering::Relaxed);
    // SAFETY: installing a handler that only writes to a pipe.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let loaded = Arc::new(Mutex::new(Loaded::default()));
    let w = waker.clone();
    let engine = Engine::spawn(
        cfg.advanced.keep_model_loaded,
        Duration::from_secs(u64::from(cfg.advanced.unload_after_minutes) * 60),
        cfg.advanced.nice,
        loaded.clone(),
        move || w.wake(),
    );
    let inserter = match Inserter::spawn() {
        Ok(i) => Some(i),
        Err(e) => {
            eprintln!("nirudictate: {e}");
            None
        }
    };
    let clients = listen(waker.clone())?;
    let state_atom = intern(&conn, "_NIRUDICTATE_STATE");
    let utf8 = intern(&conn, "UTF8_STRING");
    let scale = dpi_scale(&conn, root);
    let controller = Controller::new(
        cfg.shortcut.mode,
        Duration::from_millis(cfg.shortcut.hold_ms),
    );
    let mut d = Daemon {
        conn,
        screen,
        root,
        cfg,
        hw: hardware::detect(),
        engine,
        loaded,
        inserter,
        overlay: None,
        font: None,
        palette: None,
        scale,
        controller,
        phase: Phase::Idle,
        grabbed: Vec::new(),
        shortcut_error: None,
        last: None,
        error: None,
        message: None,
        pending_files: Vec::new(),
        state_atom,
        utf8,
        waker,
        clients,
        started_panel: None,
    };
    d.grab();
    d.publish();
    eprintln!(
        "nirudictate {}: ready ({}, {}, engine {})",
        env!("CARGO_PKG_VERSION"),
        hotkey::display(&d.cfg.shortcut.key),
        d.cfg.general.language,
        crate::engine_variant()
    );
    let result = d.run(wake_read, sig_read);
    d.shutdown();
    result
}

/// Accept control connections on a thread; each request is handed to the
/// main loop and answered from there.
fn listen(waker: Waker) -> Result<Receiver<Client>, String> {
    let path = config::socket_path();
    config::ensure_private_dir(&config::runtime_dir())?;
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let _ = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    let (tx, rx) = mpsc::channel::<Client>();
    thread::Builder::new()
        .name("nirudictate-control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                let waker = waker.clone();
                let _ = thread::Builder::new()
                    .name("nirudictate-client".into())
                    .spawn(move || serve_client(stream, tx, waker));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

fn serve_client(stream: UnixStream, tx: Sender<Client>, waker: Waker) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => {
            let (rtx, rrx) = mpsc::channel();
            if tx.send((request, rtx)).is_err() {
                return;
            }
            waker.wake();
            rrx.recv_timeout(Duration::from_secs(600))
                .unwrap_or_else(|_| Reply::error("the service did not answer"))
        }
        Err(e) => Reply::error(format!("bad request: {e}")),
    };
    let mut stream = stream;
    if let Ok(mut text) = serde_json::to_string(&reply) {
        text.push('\n');
        let _ = stream.write_all(text.as_bytes());
    }
}

impl Daemon {
    fn run(&mut self, wake: OwnedFd, signals: OwnedFd) -> Result<(), String> {
        loop {
            loop {
                match self.conn.poll_for_event() {
                    Ok(Some(event)) => self.event(event),
                    Ok(None) => break,
                    Err(e) => return Err(format!("X connection closed: {e}")),
                }
            }
            while let Ok((request, reply)) = self.clients.try_recv() {
                if request == Request::Quit {
                    let _ = reply.send(Reply::ok());
                    return Ok(());
                }
                self.request(request, reply);
            }
            self.tick();
            let _ = self.conn.flush();
            let timeout = self.timeout_ms();
            let mut fds = [
                libc::pollfd {
                    fd: self.conn.stream().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wake.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: signals.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: three valid pollfds for the duration of the call.
            unsafe { libc::poll(fds.as_mut_ptr(), 3, timeout) };
            if fds[1].revents != 0 {
                drain(&wake);
            }
            if fds[2].revents != 0 {
                return Ok(());
            }
        }
    }

    /// How long the loop may sleep: forever when idle.
    fn timeout_ms(&self) -> i32 {
        if !matches!(self.phase, Phase::Idle) || !self.pending_files.is_empty() {
            return 33;
        }
        if let Some((_, until)) = &self.message {
            return until
                .saturating_duration_since(Instant::now())
                .as_millis()
                .clamp(1, 60_000) as i32;
        }
        -1
    }

    fn shutdown(&mut self) {
        self.phase = Phase::Idle;
        self.ungrab();
        let _ = self.conn.delete_property(self.root, self.state_atom);
        let _ = self.conn.flush();
        live::remove();
        let _ = std::fs::remove_file(config::socket_path());
    }

    // ── shortcut ──────────────────────────────────────────────────────────

    fn keycodes(&self, keysym: u32) -> Vec<u8> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let Ok(Ok(map)) = self
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .map(|c| c.reply())
        else {
            return Vec::new();
        };
        let per = map.keysyms_per_keycode as usize;
        map.keysyms
            .chunks(per.max(1))
            .enumerate()
            .filter(|(_, syms)| syms.first() == Some(&keysym))
            .map(|(i, _)| min + i as u8)
            .collect()
    }

    fn ungrab(&mut self) {
        for (code, mods) in self.grabbed.drain(..) {
            for extra in [0u16, 2, 16, 18] {
                let _ = self
                    .conn
                    .ungrab_key(code, self.root, ModMask::from(mods | extra));
            }
        }
        let _ = self.conn.flush();
    }

    fn grab(&mut self) {
        self.ungrab();
        self.shortcut_error = None;
        let shortcut = match hotkey::parse(&self.cfg.shortcut.key) {
            Ok(s) => s,
            Err(e) => {
                self.shortcut_error = Some(e);
                return;
            }
        };
        let codes = self.keycodes(shortcut.keysym);
        if codes.is_empty() {
            self.shortcut_error = Some(format!(
                "{} is not on this keyboard layout",
                hotkey::display(&self.cfg.shortcut.key)
            ));
            return;
        }
        let mut failed = false;
        for code in codes {
            let mut ok = true;
            for extra in [0u16, 2, 16, 18] {
                let cookie = self.conn.grab_key(
                    true,
                    self.root,
                    ModMask::from(shortcut.mods | extra),
                    code,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                );
                if cookie.map(|c| c.check().is_err()).unwrap_or(true) {
                    ok = false;
                }
            }
            if ok {
                self.grabbed.push((code, shortcut.mods));
            } else {
                failed = true;
                for extra in [0u16, 2, 16, 18] {
                    let _ =
                        self.conn
                            .ungrab_key(code, self.root, ModMask::from(shortcut.mods | extra));
                }
            }
        }
        if self.grabbed.is_empty() && failed {
            self.shortcut_error = Some(format!(
                "{} is already used by another program; choose another shortcut",
                hotkey::display(&self.cfg.shortcut.key)
            ));
        }
        let _ = self.conn.flush();
    }

    fn matches(&self, code: u8, state: u16) -> bool {
        const MEANINGFUL: u16 = hotkey::SHIFT | hotkey::CONTROL | hotkey::MOD1 | hotkey::MOD4;
        self.grabbed
            .iter()
            .any(|(c, m)| *c == code && (state & MEANINGFUL) == *m)
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::KeyPress(e) => {
                if self.matches(e.detail, u16::from(e.state)) {
                    let busy = matches!(
                        self.phase,
                        Phase::Transcribing { .. } | Phase::Inserting { .. }
                    );
                    let action = self.controller.press(Instant::now(), busy);
                    self.act(action);
                }
            }
            Event::KeyRelease(e) => {
                // The release may come with different modifiers (Ctrl let go
                // first); any release of the grabbed key counts.
                if self.grabbed.iter().any(|(c, _)| *c == e.detail) {
                    let action = self.controller.release(Instant::now());
                    self.act(action);
                }
            }
            Event::MappingNotify(_) => self.grab(),
            _ => {}
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Start => self.start(),
            Action::Stop => self.stop(),
            Action::None => {}
        }
    }

    // ── recording ─────────────────────────────────────────────────────────

    fn language(&self) -> String {
        self.cfg.general.language.clone()
    }

    fn current_model(&self) -> Option<&'static Model> {
        let lang = self.language();
        models::active(&lang, self.cfg.model_for(&lang), &self.hw)
    }

    fn start(&mut self) {
        if !matches!(self.phase, Phase::Idle) {
            return;
        }
        let Some(model) = self.current_model() else {
            self.controller.reset();
            let lang = config::language_label(&self.cfg.general.language);
            self.notice(format!("No {lang} model yet — opening settings"), true);
            self.cue(Cue::Error);
            self.open_panel();
            return;
        };
        let waker = self.waker.clone();
        match Capture::start(&self.cfg.advanced.microphone, move || waker.wake()) {
            Ok(capture) => {
                self.error = None;
                self.message = None;
                // Load the model while the user speaks.
                self.engine.send(Job::Preload(model));
                self.palette = Some(self.resolve_palette());
                self.phase = Phase::Recording {
                    capture,
                    model,
                    levels_seen: 0,
                };
                self.cue(Cue::Start);
                self.publish();
                self.draw();
            }
            Err(e) => {
                self.controller.reset();
                self.fail(e);
            }
        }
    }

    fn stop(&mut self) {
        let Phase::Recording { capture, model, .. } =
            std::mem::replace(&mut self.phase, Phase::Idle)
        else {
            return;
        };
        self.controller.reset();
        let samples = capture.finish();
        self.cue(Cue::Stop);
        let Some(speech) = audio::trim_silence(&samples) else {
            self.notice("Nothing heard".into(), false);
            self.publish();
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.engine.send(Job::Transcribe(engine::Request {
            samples: speech.to_vec(),
            model,
            prompt: text::prompt(&self.cfg.vocabulary.words, model.language),
            threads: self.cfg.threads(),
            full_window: false,
            reply: tx,
        }));
        self.phase = Phase::Transcribing {
            rx,
            started: Instant::now(),
        };
        self.publish();
        self.draw();
    }

    fn cancel(&mut self) {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Recording { capture, .. } => {
                drop(capture);
                self.cue(Cue::Stop);
            }
            other => self.phase = other,
        }
        if matches!(self.phase, Phase::Transcribing { .. }) {
            // The engine finishes the job; its result is dropped.
            self.phase = Phase::Idle;
        }
        self.controller.reset();
        self.hide();
        self.publish();
    }

    fn fail(&mut self, error: String) {
        eprintln!("nirudictate: {error}");
        self.error = Some(error.clone());
        self.phase = Phase::Idle;
        self.cue(Cue::Error);
        self.notice(error, true);
        self.publish();
    }

    fn notice(&mut self, text: String, error: bool) {
        if !self.cfg.general.overlay {
            self.hide();
            return;
        }
        let secs = if error { 4 } else { 2 };
        self.message = Some((
            View::Message { text, error },
            Instant::now() + Duration::from_secs(secs),
        ));
        self.palette = Some(self.resolve_palette());
        self.draw();
    }

    fn cue(&self, cue: Cue) {
        if self.cfg.general.sounds {
            sounds::play(cue);
        }
    }

    fn open_panel(&mut self) {
        if self
            .started_panel
            .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        self.started_panel = Some(Instant::now());
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(mut child) = std::process::Command::new(exe)
                .arg("settings")
                .stdin(std::process::Stdio::null())
                .spawn()
            {
                let _ = thread::Builder::new().spawn(move || {
                    let _ = child.wait();
                });
            }
        }
    }

    /// Periodic work while something is happening.
    fn tick(&mut self) {
        if let Some((_, until)) = &self.message {
            if Instant::now() >= *until {
                self.message = None;
                if matches!(self.phase, Phase::Idle) {
                    self.hide();
                }
            }
        }
        self.pending_files
            .retain(|(reply, rx)| match rx.try_recv() {
                Ok(result) => {
                    let _ = reply.send(match result {
                        Ok(t) => Reply {
                            ok: true,
                            transcript: Some(t),
                            ..Reply::default()
                        },
                        Err(e) => Reply::error(e),
                    });
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => {
                    let _ = reply.send(Reply::error("the engine stopped"));
                    false
                }
            });
        match &mut self.phase {
            Phase::Idle => {}
            Phase::Recording {
                capture,
                levels_seen,
                ..
            } => {
                let (ended, error, silence, heard, frames) = {
                    let s = capture.shared.lock().unwrap_or_else(|e| e.into_inner());
                    (
                        s.ended,
                        s.error.clone(),
                        s.vad.silence(),
                        s.vad.heard_speech(),
                        s.vad.frames as usize,
                    )
                };
                *levels_seen = frames;
                let elapsed = capture.elapsed();
                let holding = self.controller.holding();
                let silence_limit = self.cfg.shortcut.silence_seconds;
                if let Some(e) = error {
                    self.cancel();
                    self.fail(e);
                    return;
                }
                if ended && elapsed > Duration::from_millis(300) {
                    self.cancel();
                    self.fail(
                        "the microphone recorder stopped; check the microphone in settings".into(),
                    );
                    return;
                }
                if elapsed >= Duration::from_secs(u64::from(self.cfg.shortcut.max_seconds)) {
                    self.stop();
                    return;
                }
                if !holding && silence_limit > 0.0 {
                    if heard && silence >= Duration::from_secs_f32(silence_limit) {
                        self.stop();
                        return;
                    }
                    if !heard && elapsed >= Duration::from_secs_f32((silence_limit * 4.0).max(8.0))
                    {
                        self.cancel();
                        self.notice("Nothing heard".into(), false);
                        return;
                    }
                }
                self.draw();
            }
            Phase::Transcribing { rx, .. } => match rx.try_recv() {
                Ok(Ok(transcript)) => self.transcribed(transcript),
                Ok(Err(e)) => self.fail(e),
                Err(TryRecvError::Empty) => self.draw(),
                Err(TryRecvError::Disconnected) => self.fail("the engine stopped".into()),
            },
            Phase::Inserting { rx, .. } => match rx.try_recv() {
                Ok(result) => {
                    let Phase::Inserting {
                        transcript, text, ..
                    } = std::mem::replace(&mut self.phase, Phase::Idle)
                    else {
                        return;
                    };
                    match result {
                        Ok(how) => self.inserted(transcript, text, how),
                        Err(e) => self.fail(e),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.fail("text insertion stopped".into()),
            },
        }
    }

    fn transcribed(&mut self, transcript: Transcript) {
        let options = text::Options {
            capitalize: self.cfg.output.capitalize,
            trailing_space: self.cfg.output.trailing_space,
        };
        let Some(text) = text::finish(&transcript.text, options) else {
            self.phase = Phase::Idle;
            self.notice("No speech recognised".into(), false);
            self.publish();
            return;
        };
        crate::measure::record(&transcript);
        self.hide();
        let Some(inserter) = &self.inserter else {
            self.fail("text insertion is unavailable (XTest missing?)".into());
            return;
        };
        let (tx, rx) = mpsc::channel();
        inserter.insert(Insert {
            text: text.clone(),
            paste: self.cfg.output.method == config::Method::Paste,
            restore: self.cfg.output.restore_clipboard,
            type_fallback: self.cfg.output.type_fallback,
            terminals: self.cfg.output.terminals.clone(),
            reply: tx,
        });
        self.phase = Phase::Inserting {
            rx,
            transcript,
            text,
        };
    }

    fn inserted(&mut self, transcript: Transcript, text: String, how: String) {
        let time = history::now();
        if self.cfg.history.enabled {
            let entry = history::Entry {
                time,
                text: text.trim_end().to_string(),
                language: transcript.language.clone(),
                model: transcript.model.clone(),
                audio_seconds: transcript.audio_seconds,
                seconds: transcript.seconds + transcript.load_seconds,
            };
            if let Err(e) = history::append(&entry, self.cfg.history.keep) {
                eprintln!("nirudictate: history: {e}");
            }
        }
        self.last = Some(LastResult {
            text: text.trim_end().to_string(),
            time,
            model: transcript.model,
            audio_seconds: transcript.audio_seconds,
            seconds: transcript.seconds + transcript.load_seconds,
            inserted_by: how,
        });
        self.cue(Cue::Done);
        self.publish();
    }

    // ── overlay ───────────────────────────────────────────────────────────

    fn resolve_palette(&self) -> theme::Palette {
        let prefs = theme::Preferences {
            theme: self.cfg.general.theme.clone(),
            ..Default::default()
        };
        theme::resolve(&prefs)
            .or_else(|_| theme::resolve(&theme::Preferences::default()))
            .map(|t| t.palette)
            .unwrap_or_else(|_| {
                theme::resolve(&theme::Preferences {
                    theme: "classic".into(),
                    ..Default::default()
                })
                .expect("classic theme")
                .palette
            })
    }

    fn view(&self) -> Option<View> {
        match &self.phase {
            Phase::Recording { capture, .. } => {
                let levels = capture
                    .shared
                    .lock()
                    .map(|s| s.levels.clone())
                    .unwrap_or_default();
                Some(View::Recording {
                    seconds: capture.elapsed().as_secs_f32(),
                    levels,
                    language: self.language(),
                    holding: self.controller.holding(),
                })
            }
            Phase::Transcribing { started, .. } => Some(View::Transcribing {
                phase: started.elapsed().as_secs_f32(),
            }),
            Phase::Inserting { .. } => None,
            Phase::Idle => self.message.as_ref().map(|(v, _)| v.clone()),
        }
    }

    fn draw(&mut self) {
        if !self.cfg.general.overlay {
            return;
        }
        let Some(view) = self.view() else {
            self.hide();
            return;
        };
        if self.overlay.is_none() {
            match Overlay::new(&self.conn, self.screen) {
                Ok(o) => self.overlay = Some(o),
                Err(e) => {
                    eprintln!("nirudictate: overlay unavailable: {e}");
                    return;
                }
            }
        }
        if self.font.is_none() {
            self.font = Some(overlay::font(None));
        }
        let palette = match self.palette {
            Some(p) => p,
            None => {
                let p = self.resolve_palette();
                self.palette = Some(p);
                p
            }
        };
        let (Some(ov), Some(font)) = (self.overlay.as_mut(), self.font.as_ref()) else {
            return;
        };
        let canvas = overlay::render(&view, &palette, font, self.scale, ov.translucent());
        ov.show(&self.conn, self.screen, &canvas);
    }

    fn hide(&mut self) {
        if let Some(ov) = self.overlay.as_mut() {
            ov.hide(&self.conn);
        }
    }

    // ── state for others ──────────────────────────────────────────────────

    fn publish(&mut self) {
        let word = self.phase.word();
        let lang = self.language();
        live::write(word, &lang);
        let _ = self.conn.change_property8(
            PropMode::REPLACE,
            self.root,
            self.state_atom,
            self.utf8,
            word.as_bytes(),
        );
        let _ = self.conn.flush();
    }

    fn status(&self) -> Status {
        let recording_seconds = match &self.phase {
            Phase::Recording { capture, .. } => capture.elapsed().as_secs_f32(),
            _ => 0.0,
        };
        Status {
            version: env!("CARGO_PKG_VERSION").into(),
            state: self.phase.word().into(),
            pid: std::process::id(),
            language: self.language(),
            model: self.current_model().map(|m| m.id.to_string()),
            loaded: self.loaded.lock().ok().and_then(|l| l.model.clone()),
            recording_seconds,
            shortcut: self.cfg.shortcut.key.clone(),
            shortcut_active: !self.grabbed.is_empty(),
            mode: format!("{:?}", self.cfg.shortcut.mode).to_lowercase(),
            audio: audio::backend()
                .map(|b| b.name().to_string())
                .unwrap_or_else(|| "none".into()),
            engine: crate::engine_variant().into(),
            last: self.last.clone(),
            error: self.shortcut_error.clone().or_else(|| self.error.clone()),
        }
    }

    fn reload(&mut self) -> Result<(), String> {
        let cfg = Config::load()?;
        self.cfg = cfg;
        self.controller = Controller::new(
            self.cfg.shortcut.mode,
            Duration::from_millis(self.cfg.shortcut.hold_ms),
        );
        self.engine.send(Job::Policy {
            keep_loaded: self.cfg.advanced.keep_model_loaded,
            unload_after: Duration::from_secs(
                u64::from(self.cfg.advanced.unload_after_minutes) * 60,
            ),
            nice: self.cfg.advanced.nice,
        });
        if self.cfg.history.enabled {
            let _ = history::trim(self.cfg.history.keep);
        }
        self.palette = None;
        self.grab();
        self.publish();
        Ok(())
    }

    fn request(&mut self, request: Request, reply: Sender<Reply>) {
        let answer = match request {
            Request::Status => Reply {
                ok: true,
                status: Some(self.status()),
                ..Reply::default()
            },
            Request::Start => {
                if matches!(self.phase, Phase::Idle) {
                    self.start();
                    if matches!(self.phase, Phase::Recording { .. }) {
                        self.controller.started_elsewhere();
                    }
                }
                self.ok_status()
            }
            Request::Stop => {
                self.stop();
                self.ok_status()
            }
            Request::Toggle => {
                if matches!(self.phase, Phase::Recording { .. }) {
                    self.stop();
                } else if matches!(self.phase, Phase::Idle) {
                    self.start();
                    if matches!(self.phase, Phase::Recording { .. }) {
                        self.controller.started_elsewhere();
                    }
                }
                self.ok_status()
            }
            Request::Cancel => {
                self.cancel();
                self.ok_status()
            }
            Request::Reload => match self.reload() {
                Ok(()) => self.ok_status(),
                Err(e) => Reply::error(e),
            },
            Request::Unload => {
                self.engine.send(Job::Unload);
                Reply::ok()
            }
            Request::Language { language } => {
                let mut cfg = self.cfg.clone();
                cfg.general.language = language;
                match cfg.save() {
                    Ok(()) => {
                        self.cfg = cfg;
                        self.publish();
                        self.ok_status()
                    }
                    Err(e) => Reply::error(e),
                }
            }
            Request::TranscribeFile { path, model } => {
                let lang = self.language();
                let chosen = match model {
                    Some(id) => models::find(&id).filter(|m| m.installed()),
                    None => self.current_model(),
                };
                match (chosen, audio::read_wav(std::path::Path::new(&path))) {
                    (None, _) => Reply::error(format!("no installed model for {lang}")),
                    (_, Err(e)) => Reply::error(e),
                    (Some(model), Ok(samples)) => {
                        let (tx, rx) = mpsc::channel();
                        self.engine.send(Job::Transcribe(engine::Request {
                            samples,
                            model,
                            prompt: text::prompt(&self.cfg.vocabulary.words, model.language),
                            threads: self.cfg.threads(),
                            full_window: false,
                            reply: tx,
                        }));
                        self.pending_files.push((reply, rx));
                        return;
                    }
                }
            }
            Request::Quit => Reply::ok(),
        };
        let _ = reply.send(answer);
    }

    fn ok_status(&self) -> Reply {
        Reply {
            ok: true,
            status: Some(self.status()),
            ..Reply::default()
        }
    }
}
