//! Agent mode: a dictation that goes to opencode instead of the text cursor.
//!
//! The service starts its own opencode server the first time a message is
//! sent (`opencode serve` on 127.0.0.1 with a random password that only
//! travels in the environment), keeps one ongoing session, and delivers each
//! message through the server's HTTP API. Nothing is typed into a window and
//! the message never appears in a command line.
//!
//! The opencode TUI runs in a terminal attached to the same server and lives
//! in NIRUWM scratchpad slot 9 (Super+Alt+9). It is started when it is
//! missing and shown on top whenever a message is sent. Closing that window
//! stops the server; the next message starts both again and continues the
//! same session.
//!
//! Both opencode command lines are supported:
//!
//! | opencode | server            | TUI                                   | API |
//! |----------|-------------------|---------------------------------------|-----|
//! | 1.x      | `serve --port`    | `attach URL --dir D --session ID`     | `/session`, `/session/ID/prompt_async` |
//! | 2.x      | `serve --port`    | `--server URL --session ID D`         | `/api/session`, `/api/session/ID/prompt` |
//!
//! opencode's own default agent is used unless `[agent] agent` names one, and
//! permissions are never auto-approved (no `--auto`).
use crate::config::{self, Agent as Settings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, Window};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// NIRUWM scratchpad slot of the opencode window (Super+Alt+9).
pub const SLOT: u32 = 9;
/// WM_CLASS of the terminal that runs the opencode TUI.
pub const CLASS: &str = "nirudictate-agent";

fn log(line: String) {
    eprintln!("nirudictate: agent: {line}");
}

// ── opencode ──────────────────────────────────────────────────────────────

/// Which HTTP API and command line the installed opencode has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Api {
    /// opencode 1.x (yargs CLI: serve, attach, --session).
    V1,
    /// opencode 2.x (background service, --server, /api/...).
    V2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opencode {
    pub path: PathBuf,
    pub version: String,
    pub api: Api,
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Where opencode is: the configured path, else PATH, else the places its
/// installers use.
pub fn find(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let p = match configured.strip_prefix("~/") {
            Some(rest) => config::home().join(rest),
            None => PathBuf::from(configured),
        };
        return executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|d| d.join("opencode"))
        .chain(
            [
                ".opencode/bin/opencode",
                ".local/bin/opencode",
                ".bun/bin/opencode",
                ".npm-global/bin/opencode",
            ]
            .iter()
            .map(|p| config::home().join(p)),
        )
        .find(|p| executable(p))
}

/// `1.19.0` or `opencode v2.1.3` → version and API.
pub fn parse_version(out: &str) -> Option<(String, Api)> {
    let word = out
        .split_whitespace()
        .map(|w| w.trim_start_matches('v'))
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let major: u32 = word.split('.').next()?.parse().ok()?;
    let api = if major >= 2 { Api::V2 } else { Api::V1 };
    Some((word.to_string(), api))
}

/// Run `opencode --version` (a few seconds at most).
pub fn probe(path: &Path) -> Result<Opencode, String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", path.display()))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} --version did not answer", path.display()));
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    let (version, api) = parse_version(&out)
        .ok_or_else(|| format!("{} --version printed no version", path.display()))?;
    Ok(Opencode {
        path: path.to_path_buf(),
        version,
        api,
    })
}

// ── the HTTP API (pure parts are unit tested) ─────────────────────────────

fn query_dir(dir: &Path) -> String {
    let mut out = String::new();
    for b in dir.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn health_path(api: Api) -> &'static str {
    match api {
        Api::V1 => "/global/health",
        Api::V2 => "/api/info",
    }
}

/// Create a session in `dir`, with the named agent if one is configured.
pub fn create_request(api: Api, dir: &Path, agent: &str) -> (String, Value) {
    match api {
        Api::V1 => {
            let mut body = json!({});
            if !agent.is_empty() {
                body["agent"] = json!(agent);
            }
            (format!("/session?directory={}", query_dir(dir)), body)
        }
        Api::V2 => {
            let mut body = json!({ "location": { "directory": dir.to_string_lossy() } });
            if !agent.is_empty() {
                body["agent"] = json!(agent);
            }
            ("/api/session".into(), body)
        }
    }
}

pub fn session_id_from(api: Api, body: &Value) -> Option<String> {
    let id = match api {
        Api::V1 => body.get("id"),
        Api::V2 => body.get("data").and_then(|d| d.get("id")),
    }?
    .as_str()?;
    id.starts_with("ses").then(|| id.to_string())
}

pub fn session_path(api: Api, id: &str, dir: &Path) -> String {
    match api {
        Api::V1 => format!("/session/{id}?directory={}", query_dir(dir)),
        Api::V2 => format!("/api/session/{id}"),
    }
}

/// Send one user message; the agent answers in the background.
pub fn prompt_request(api: Api, id: &str, dir: &Path, text: &str, agent: &str) -> (String, Value) {
    match api {
        Api::V1 => {
            let mut body = json!({ "parts": [ { "type": "text", "text": text } ] });
            if !agent.is_empty() {
                body["agent"] = json!(agent);
            }
            (
                format!("/session/{id}/prompt_async?directory={}", query_dir(dir)),
                body,
            )
        }
        Api::V2 => (format!("/api/session/{id}/prompt"), json!({ "text": text })),
    }
}

/// The opencode TUI attached to our server, on the session.
pub fn tui_args(api: Api, url: &str, id: &str, dir: &Path) -> Vec<String> {
    let dir = dir.to_string_lossy().to_string();
    match api {
        Api::V1 => vec![
            "attach".into(),
            url.into(),
            "--dir".into(),
            dir,
            "--session".into(),
            id.into(),
        ],
        Api::V2 => vec![
            "--server".into(),
            url.into(),
            "--session".into(),
            id.into(),
            dir,
        ],
    }
}

fn on_path(name: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| executable(&d.join(name)))
}

/// The terminal command line around `program args…`, with our window class.
pub fn terminal_command(
    terminal: &str,
    available: &dyn Fn(&str) -> bool,
    program: &Path,
    args: &[String],
) -> Option<Vec<String>> {
    let chosen = match terminal {
        "auto" | "" => ["kitty", "alacritty", "xterm"]
            .into_iter()
            .find(|t| available(t))?,
        t if available(t) => t,
        _ => return None,
    };
    let mut cmd: Vec<String> = match chosen {
        "kitty" => vec!["kitty".into(), "--class".into(), CLASS.into()],
        "alacritty" => vec![
            "alacritty".into(),
            "--class".into(),
            CLASS.into(),
            "-e".into(),
        ],
        "xterm" => vec!["xterm".into(), "-class".into(), CLASS.into(), "-e".into()],
        _ => return None,
    };
    cmd.push(program.to_string_lossy().to_string());
    cmd.extend(args.iter().cloned());
    Some(cmd)
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn random_password() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

struct Http {
    base: String,
    auth: String,
    agent: ureq::Agent,
    /// For the start-up health checks: opencode accepts connections before
    /// it answers them, and a request sent too early can hang, so each try
    /// gives up quickly and the next one is sent.
    quick: ureq::Agent,
}

impl Http {
    fn new(base: String, password: &str) -> Self {
        Self {
            base,
            auth: format!(
                "Basic {}",
                base64(format!("opencode:{password}").as_bytes())
            ),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(2))
                .timeout(Duration::from_secs(20))
                .build(),
            quick: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_millis(500))
                .timeout(Duration::from_millis(800))
                .build(),
        }
    }

    fn healthy(&self, api: Api) -> bool {
        self.quick
            .get(&format!("{}{}", self.base, health_path(api)))
            .set("Authorization", &self.auth)
            .call()
            .is_ok_and(|r| r.status() == 200)
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
        let req = self
            .agent
            .request(method, &format!("{}{path}", self.base))
            .set("Authorization", &self.auth);
        let result = match body {
            Some(b) => req
                .set("Content-Type", "application/json")
                .send_string(&b.to_string()),
            None => req.call(),
        };
        let resp = match result {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Ok((code, Value::Null)),
            Err(e) => return Err(format!("opencode server: {e}")),
        };
        let code = resp.status();
        let text = resp.into_string().unwrap_or_default();
        Ok((code, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }
}

// ── the worker ────────────────────────────────────────────────────────────

/// What the service asks the agent thread to do.
pub enum Job {
    Deliver { text: String, new_session: bool },
    NewSession,
    Show,
    Settings(Settings),
    TuiExited(u64),
    Shutdown,
}

/// The answer to a `Deliver`.
#[derive(Debug)]
pub enum Outcome {
    Sent { new_session: bool, seconds: f32 },
    Failed(String),
}

/// For `status`, the panel and `doctor`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Info {
    /// The opencode program, if it was found.
    pub opencode: Option<String>,
    /// Its version, once it has been asked.
    pub version: Option<String>,
    pub server: bool,
    pub session: Option<String>,
    /// The opencode window exists.
    pub window: bool,
    /// It is in NIRUWM scratchpad slot 9.
    pub in_slot: bool,
    /// The next message starts a new conversation.
    pub fresh: bool,
    pub directory: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct Saved {
    session: Option<String>,
    directory: Option<String>,
    api: Option<Api>,
}

fn saved_path() -> PathBuf {
    config::state_dir().join("agent.json")
}

fn load_saved() -> Saved {
    std::fs::read_to_string(saved_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn store_saved(s: &Saved) {
    if let Ok(text) = serde_json::to_string(s) {
        let _ = config::atomic_write(&saved_path(), text.as_bytes(), 0o600);
    }
}

struct Server {
    child: Child,
    url: String,
    password: String,
    api: Api,
    dir: PathBuf,
    http: Http,
}

struct Tui {
    pid: i32,
    generation: u64,
    session: String,
    window: Option<Window>,
    /// NIRUWM confirmed it in slot 9.
    in_slot: bool,
}

struct Worker {
    settings: Settings,
    opencode: Option<Opencode>,
    server: Option<Server>,
    saved: Saved,
    fresh: bool,
    tui: Option<Tui>,
    generation: u64,
    x: Option<(RustConnection, Window)>,
    info: Arc<Mutex<Info>>,
    jobs: Sender<Job>,
}

/// The agent thread's handle, owned by the service.
pub struct Agent {
    tx: Sender<Job>,
    pub info: Arc<Mutex<Info>>,
}

impl Agent {
    pub fn spawn(
        settings: Settings,
        done: Sender<Outcome>,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let info = Arc::new(Mutex::new(Info {
            opencode: find(&settings.opencode).map(|p| p.display().to_string()),
            directory: settings.folder().display().to_string(),
            ..Info::default()
        }));
        let mut worker = Worker {
            settings,
            opencode: None,
            server: None,
            saved: load_saved(),
            fresh: false,
            tui: None,
            generation: 0,
            x: None,
            info: info.clone(),
            jobs: tx.clone(),
        };
        worker.publish();
        let _ = thread::Builder::new()
            .name("nirudictate-agent".into())
            .spawn(move || worker.run(rx, done, wake));
        Self { tx, info }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    pub fn info(&self) -> Info {
        self.info.lock().map(|i| i.clone()).unwrap_or_default()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.tx.send(Job::Shutdown);
    }
}

impl Worker {
    fn run(&mut self, rx: Receiver<Job>, done: Sender<Outcome>, wake: impl Fn()) {
        while let Ok(job) = rx.recv() {
            match job {
                Job::Deliver { text, new_session } => {
                    let t0 = Instant::now();
                    let outcome = match self.deliver(&text, new_session) {
                        Ok(fresh) => Outcome::Sent {
                            new_session: fresh,
                            seconds: t0.elapsed().as_secs_f32(),
                        },
                        Err(e) => {
                            log(e.clone());
                            Outcome::Failed(e)
                        }
                    };
                    let _ = done.send(outcome);
                    wake();
                }
                Job::NewSession => {
                    self.fresh = true;
                    log("the next message starts a new conversation".into());
                }
                Job::Show => {
                    if let Err(e) = self.show() {
                        log(e);
                    }
                }
                Job::Settings(s) => {
                    if s.opencode != self.settings.opencode {
                        self.opencode = None;
                    }
                    self.settings = s;
                }
                Job::TuiExited(generation) => {
                    if self
                        .tui
                        .as_ref()
                        .is_some_and(|t| t.generation == generation)
                    {
                        log("the opencode window was closed; stopping its server".into());
                        self.tui = None;
                        self.stop_server();
                    }
                }
                Job::Shutdown => {
                    let _ = self.stop_tui();
                    self.stop_server();
                    self.publish();
                    return;
                }
            }
            self.publish();
        }
        let _ = self.stop_tui();
        self.stop_server();
    }

    fn publish(&self) {
        if let Ok(mut i) = self.info.lock() {
            i.opencode = self
                .opencode
                .as_ref()
                .map(|o| o.path.display().to_string())
                .or_else(|| find(&self.settings.opencode).map(|p| p.display().to_string()));
            i.version = self.opencode.as_ref().map(|o| o.version.clone());
            i.server = self.server.is_some();
            i.session = self.saved.session.clone();
            i.window = self.tui.is_some();
            i.in_slot = self.tui.as_ref().is_some_and(|t| t.in_slot);
            i.fresh = self.fresh;
            i.directory = self.settings.folder().display().to_string();
        }
    }

    fn opencode(&mut self) -> Result<Opencode, String> {
        if let Some(o) = &self.opencode {
            if executable(&o.path) {
                return Ok(o.clone());
            }
        }
        let path = find(&self.settings.opencode)
            .ok_or("opencode is not installed (see https://opencode.ai); agent mode needs it")?;
        let o = probe(&path)?;
        log(format!(
            "using opencode {} ({})",
            o.version,
            o.path.display()
        ));
        self.opencode = Some(o.clone());
        Ok(o)
    }

    /// Returns whether a new conversation was started.
    fn deliver(&mut self, text: &str, new_session: bool) -> Result<bool, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("nothing to send".into());
        }
        let dir = self.settings.folder();
        if !dir.is_dir() {
            return Err(format!("the agent folder {} does not exist", dir.display()));
        }
        let oc = self.opencode()?;
        self.ensure_server(&oc, &dir)?;
        let (session, started_new) = self.ensure_session(&dir, new_session || self.fresh)?;
        self.fresh = false;
        let server = self.server.as_ref().ok_or("the opencode server stopped")?;
        let (path, body) = prompt_request(server.api, &session, &dir, text, &self.settings.agent);
        let (code, _) = server.http.call("POST", &path, Some(&body))?;
        if !(200..300).contains(&code) {
            return Err(format!("opencode did not take the message (HTTP {code})"));
        }
        log(format!(
            "sent {} characters to session {session}{}",
            text.chars().count(),
            if started_new { " (new)" } else { "" }
        ));
        if let Err(e) = self.ensure_tui(&session) {
            // The message is in; only the window failed.
            log(e);
        }
        Ok(started_new)
    }

    fn ensure_server(&mut self, oc: &Opencode, dir: &Path) -> Result<(), String> {
        if let Some(s) = self.server.as_mut() {
            let alive = matches!(s.child.try_wait(), Ok(None));
            if alive && s.dir == dir && s.api == oc.api {
                return Ok(());
            }
            if alive {
                log("the agent folder changed; restarting the opencode server".into());
            }
        }
        let _ = self.stop_tui();
        self.stop_server();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| format!("no free port: {e}"))?;
        let password = random_password()?;
        let mut cmd = Command::new(&oc.path);
        cmd.args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .current_dir(dir)
        // The password reaches the server and the TUI only through the
        // environment (1.x reads OPENCODE_SERVER_PASSWORD, 2.x
        // OPENCODE_PASSWORD); nothing is logged.
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .env("OPENCODE_PASSWORD", &password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        // SAFETY: setsid and prctl only change the child's own process state.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let t0 = Instant::now();
        let child = cmd
            .spawn()
            .map_err(|e| format!("cannot start opencode serve: {e}"))?;
        let url = format!("http://127.0.0.1:{port}");
        let http = Http::new(url.clone(), &password);
        let mut server = Server {
            child,
            url,
            password,
            api: oc.api,
            dir: dir.to_path_buf(),
            http,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(Some(status)) = server.child.try_wait() {
                return Err(format!("opencode serve stopped at once ({status})"));
            }
            if server.http.healthy(oc.api) {
                break;
            }
            if Instant::now() > deadline {
                stop_child(&mut server.child);
                return Err("the opencode server did not start within 30 s".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        log(format!(
            "opencode server ready on 127.0.0.1:{port} in {:.1} s",
            t0.elapsed().as_secs_f32()
        ));
        self.server = Some(server);
        Ok(())
    }

    fn stop_server(&mut self) {
        if let Some(mut s) = self.server.take() {
            stop_child(&mut s.child);
        }
    }

    fn ensure_session(&mut self, dir: &Path, new: bool) -> Result<(String, bool), String> {
        let server = self.server.as_ref().ok_or("no opencode server")?;
        let dir_text = dir.display().to_string();
        if !new && self.saved.directory.as_deref() == Some(dir_text.as_str()) {
            if let Some(id) = self.saved.session.clone() {
                let path = session_path(server.api, &id, dir);
                if let Ok((200, _)) = server.http.call("GET", &path, None) {
                    return Ok((id, false));
                }
                log("the saved conversation is gone; starting a new one".into());
            }
        }
        let (path, body) = create_request(server.api, dir, &self.settings.agent);
        let (code, reply) = server.http.call("POST", &path, Some(&body))?;
        let id = session_id_from(server.api, &reply)
            .ok_or_else(|| format!("opencode did not create a session (HTTP {code})"))?;
        self.saved = Saved {
            session: Some(id.clone()),
            directory: Some(dir_text),
            api: Some(server.api),
        };
        store_saved(&self.saved);
        Ok((id, true))
    }

    // ── the window ────────────────────────────────────────────────────────

    fn tui_alive(&self) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        self.tui
            .as_ref()
            .is_some_and(|t| unsafe { libc::kill(t.pid, 0) } == 0)
    }

    fn ensure_tui(&mut self, session: &str) -> Result<(), String> {
        if self.tui_alive() && self.tui.as_ref().is_some_and(|t| t.session == session) {
            return self.show();
        }
        if self.stop_tui() {
            // Let the old window go first: X reuses window ids, so the new
            // terminal's window can get the id the old one had.
            let deadline = Instant::now() + Duration::from_secs(3);
            while !self.agent_windows().is_empty() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
            }
        }
        let before = self.agent_windows();
        let server = self.server.as_ref().ok_or("no opencode server")?;
        let oc = self.opencode.clone().ok_or("opencode unknown")?;
        let args = tui_args(server.api, &server.url, session, &server.dir);
        let cmd = terminal_command(&self.settings.terminal, &on_path, &oc.path, &args).ok_or_else(
            || {
                format!(
                    "no terminal for the opencode window ({}; install kitty, alacritty or xterm)",
                    self.settings.terminal
                )
            },
        )?;
        let mut command = Command::new(&cmd[0]);
        command
            .args(&cmd[1..])
            .current_dir(&server.dir)
            .env("OPENCODE_SERVER_PASSWORD", &server.password)
            .env("OPENCODE_PASSWORD", &server.password)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: setsid and prctl only change the child's own process state.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", cmd[0]))?;
        self.generation += 1;
        let generation = self.generation;
        let pid = child.id() as i32;
        let jobs = self.jobs.clone();
        let _ = thread::Builder::new()
            .name("nirudictate-agent-tui".into())
            .spawn(move || {
                let _ = child.wait();
                let _ = jobs.send(Job::TuiExited(generation));
            });
        self.tui = Some(Tui {
            pid,
            generation,
            session: session.to_string(),
            window: None,
            in_slot: false,
        });
        // Wait for the terminal's window, by class and (when it says) PID.
        let deadline = Instant::now() + Duration::from_secs(10);
        let window = loop {
            // Ours by _NET_WM_PID; a terminal that does not set it, by being new.
            let found = self.agent_windows().into_iter().find(|(w, p)| match p {
                Some(p) => *p == pid as u32,
                None => !before.iter().any(|(b, _)| b == w),
            });
            if let Some((w, _)) = found {
                break Some(w);
            }
            if Instant::now() > deadline || !self.tui_alive() {
                break None;
            }
            thread::sleep(Duration::from_millis(80));
        };
        let Some(window) = window else {
            return Err("the opencode window did not appear".into());
        };
        if let Some(t) = self.tui.as_mut() {
            t.window = Some(window);
        }
        self.show()
    }

    /// Close our terminal; whether there was one.
    fn stop_tui(&mut self) -> bool {
        let Some(t) = self.tui.take() else {
            return false;
        };
        // SAFETY: the terminal is our own child in its own session.
        unsafe {
            libc::kill(-t.pid, libc::SIGTERM);
            libc::kill(t.pid, libc::SIGTERM);
        }
        true
    }

    fn x(&mut self) -> Option<&(RustConnection, Window)> {
        if self.x.is_none() {
            if let Ok((c, s)) = x11rb::connect(None) {
                let root = c.setup().roots[s].root;
                self.x = Some((c, root));
            }
        }
        self.x.as_ref()
    }

    /// Managed windows of our class, with their `_NET_WM_PID`.
    fn agent_windows(&mut self) -> Vec<(Window, Option<u32>)> {
        let Some((c, root)) = self.x() else {
            return Vec::new();
        };
        let (c, root) = (c, *root);
        let mut out = Vec::new();
        // A window sent to a scratchpad leaves _NET_CLIENT_LIST; the
        // scratchpad list names it instead.
        let mut windows = prop32(c, root, "_NET_CLIENT_LIST");
        for (_, w, _) in scratchpads(c, root) {
            if w != 0 && !windows.contains(&w) {
                windows.push(w);
            }
        }
        for w in windows {
            let class = c
                .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
                .ok()
                .and_then(|r| r.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.value).to_string())
                .unwrap_or_default();
            if class
                .split('\0')
                .any(|part| part.eq_ignore_ascii_case(CLASS))
            {
                let pid = prop32(c, w, "_NET_WM_PID").first().copied();
                out.push((w, pid));
            }
        }
        out
    }

    /// Put the opencode window in slot 9 (again, if it was taken out) and
    /// show it on top. An older NIRUWM without command 18 leaves it a normal
    /// window, which is raised instead.
    fn show(&mut self) -> Result<(), String> {
        if !self.tui_alive() {
            return Ok(());
        }
        let Some(window) = self.tui.as_ref().and_then(|t| t.window) else {
            return Ok(());
        };
        let Some((c, root)) = self.x() else {
            return Err("no X display for the opencode window".into());
        };
        let root = *root;
        let in_slot = |c: &RustConnection| {
            scratchpads(c, root)
                .iter()
                .any(|(slot, w, _)| *slot == SLOT && *w == window)
        };
        if !in_slot(c) {
            niruwm_command(c, root, [18, SLOT, window, 0, 0]);
            let deadline = Instant::now() + Duration::from_millis(1500);
            while !in_slot(c) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
            }
        }
        let placed = in_slot(c);
        if let Some(t) = self.tui.as_mut() {
            t.in_slot = placed;
        }
        let Some((c, root)) = self.x.as_ref() else {
            return Ok(());
        };
        let root = *root;
        if placed {
            niruwm_command(c, root, [15, SLOT, 1, 0, 0]);
        } else {
            log("NIRUWM did not take the opencode window into slot 9 (NIRUWM 0.17.9 or newer does); raising it as a normal window".into());
            activate(c, root, window);
        }
        Ok(())
    }
}

fn stop_child(child: &mut Child) {
    let pid = child.id() as i32;
    // SAFETY: the server is our own child and leads its own process group.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    // SAFETY: as above.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn atom(c: &RustConnection, name: &str) -> u32 {
    c.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0)
}

fn prop32(c: &RustConnection, w: Window, name: &str) -> Vec<u32> {
    let a = atom(c, name);
    c.get_property(false, w, a, AtomEnum::ANY, 0, 4096)
        .ok()
        .and_then(|r| r.reply().ok())
        .and_then(|r| r.value32().map(|v| v.collect()))
        .unwrap_or_default()
}

/// NIRUWM's `_NIRUWM_SCRATCHPADS`: `slot<TAB>state<TAB>window<TAB>class` lines.
pub fn parse_scratchpads(text: &str) -> Vec<(u32, Window, String)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let slot = f.next()?.trim().parse().ok()?;
            let state = f.next()?.to_string();
            let window = f.next()?.trim().parse().ok()?;
            Some((slot, window, state))
        })
        .collect()
}

fn scratchpads(c: &RustConnection, root: Window) -> Vec<(u32, Window, String)> {
    let a = atom(c, "_NIRUWM_SCRATCHPADS");
    let utf8 = atom(c, "UTF8_STRING");
    c.get_property(false, root, a, utf8, 0, 4096)
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| parse_scratchpads(&String::from_utf8_lossy(&r.value)))
        .unwrap_or_default()
}

/// A `_NIRUWM_COMMAND` like `niructl` sends it.
fn niruwm_command(c: &RustConnection, root: Window, data: [u32; 5]) {
    let target = prop32(c, root, "_NET_SUPPORTING_WM_CHECK")
        .first()
        .copied()
        .unwrap_or(root);
    let a = atom(c, "_NIRUWM_COMMAND");
    let _ = c.send_event(
        false,
        target,
        EventMask::NO_EVENT,
        ClientMessageEvent::new(32, target, a, data),
    );
    let _ = c.sync();
}

fn activate(c: &RustConnection, root: Window, window: Window) {
    let a = atom(c, "_NET_ACTIVE_WINDOW");
    let _ = c.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        ClientMessageEvent::new(32, window, a, [2, 0, 0, 0, 0]),
    );
    let _ = c.sync();
}

/// XKB options that make the agent shortcut switch the keyboard layout too.
pub fn conflicting_xkb_options(options: &str, key: &str) -> Vec<String> {
    let Ok(shortcut) = crate::hotkey::parse(key) else {
        return Vec::new();
    };
    let shift_alt =
        shortcut.mods & crate::hotkey::SHIFT != 0 && shortcut.mods & crate::hotkey::MOD1 != 0;
    let alt_space = shortcut.mods & crate::hotkey::MOD1 != 0 && shortcut.keysym == 0x20;
    let win_space = shortcut.mods & crate::hotkey::MOD4 != 0 && shortcut.keysym == 0x20;
    options
        .split(',')
        .map(str::trim)
        .filter(|o| {
            let name = o.strip_prefix("grp:").unwrap_or("");
            (shift_alt
                && matches!(
                    name,
                    "alt_shift_toggle"
                        | "alt_shift_toggle_bidir"
                        | "lalt_lshift_toggle"
                        | "ralt_rshift_toggle"
                        | "lshift_lalt_toggle"
                ))
                || (alt_space && name == "alt_space_toggle")
                || (win_space && name == "win_space_toggle")
        })
        .map(str::to_string)
        .collect()
}

/// The XKB options of the running X server (`_XKB_RULES_NAMES`).
pub fn xkb_options(c: &RustConnection, root: Window) -> String {
    let a = atom(c, "_XKB_RULES_NAMES");
    c.get_property(false, root, a, AtomEnum::STRING, 0, 1024)
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| {
            String::from_utf8_lossy(&r.value)
                .split('\0')
                .nth(4)
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_opencode_version_styles_parse() {
        assert_eq!(parse_version("1.19.0\n"), Some(("1.19.0".into(), Api::V1)));
        assert_eq!(
            parse_version("opencode v2.1.3\n"),
            Some(("2.1.3".into(), Api::V2))
        );
        assert_eq!(parse_version("0.15.2"), Some(("0.15.2".into(), Api::V1)));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn requests_follow_each_api() {
        let dir = Path::new("/home/niru/my src");
        let (p, b) = create_request(Api::V1, dir, "");
        assert_eq!(p, "/session?directory=/home/niru/my%20src");
        assert_eq!(b, json!({}));
        let (_, b) = create_request(Api::V1, dir, "plan");
        assert_eq!(b, json!({"agent": "plan"}));
        let (p, b) = create_request(Api::V2, dir, "");
        assert_eq!(p, "/api/session");
        assert_eq!(b, json!({"location": {"directory": "/home/niru/my src"}}));
        assert_eq!(
            session_id_from(Api::V1, &json!({"id": "ses_abc", "directory": "/x"})),
            Some("ses_abc".into())
        );
        assert_eq!(
            session_id_from(Api::V2, &json!({"data": {"id": "ses_def"}})),
            Some("ses_def".into())
        );
        assert_eq!(session_id_from(Api::V1, &json!({"id": "nope"})), None);
        let (p, b) = prompt_request(Api::V1, "ses_abc", dir, "Hej på dig", "");
        assert_eq!(
            p,
            "/session/ses_abc/prompt_async?directory=/home/niru/my%20src"
        );
        assert_eq!(
            b,
            json!({"parts": [{"type": "text", "text": "Hej på dig"}]})
        );
        assert!(
            b.get("agent").is_none(),
            "the default agent unless configured"
        );
        let (p, b) = prompt_request(Api::V2, "ses_abc", dir, "Hej", "build");
        assert_eq!(p, "/api/session/ses_abc/prompt");
        assert_eq!(b, json!({"text": "Hej"}));
        assert_eq!(health_path(Api::V1), "/global/health");
        assert_eq!(health_path(Api::V2), "/api/info");
    }

    #[test]
    fn the_tui_attaches_to_the_session_without_auto_approval() {
        let dir = Path::new("/home/niru");
        let v1 = tui_args(Api::V1, "http://127.0.0.1:4000", "ses_a", dir);
        assert_eq!(
            v1,
            [
                "attach",
                "http://127.0.0.1:4000",
                "--dir",
                "/home/niru",
                "--session",
                "ses_a"
            ]
        );
        let v2 = tui_args(Api::V2, "http://127.0.0.1:4000", "ses_a", dir);
        assert_eq!(
            v2,
            [
                "--server",
                "http://127.0.0.1:4000",
                "--session",
                "ses_a",
                "/home/niru"
            ]
        );
        for args in [v1, v2] {
            assert!(!args.iter().any(|a| a == "--auto" || a.contains("password")));
        }
    }

    #[test]
    fn terminals_get_the_agent_class() {
        let oc = Path::new("/usr/bin/opencode");
        let args = vec!["attach".to_string(), "URL".to_string()];
        let all = |_: &str| true;
        let only_xterm = |t: &str| t == "xterm";
        let none = |_: &str| false;
        assert_eq!(
            terminal_command("auto", &all, oc, &args).unwrap(),
            [
                "kitty",
                "--class",
                CLASS,
                "/usr/bin/opencode",
                "attach",
                "URL"
            ]
        );
        assert_eq!(
            terminal_command("auto", &only_xterm, oc, &args).unwrap(),
            [
                "xterm",
                "-class",
                CLASS,
                "-e",
                "/usr/bin/opencode",
                "attach",
                "URL"
            ]
        );
        assert_eq!(
            terminal_command("alacritty", &all, oc, &args).unwrap()[..4],
            ["alacritty", "--class", CLASS, "-e"]
        );
        assert!(terminal_command("kitty", &only_xterm, oc, &args).is_none());
        assert!(terminal_command("auto", &none, oc, &args).is_none());
    }

    #[test]
    fn basic_auth_is_standard_base64() {
        assert_eq!(base64(b"opencode:pw"), "b3BlbmNvZGU6cHc=");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(random_password().unwrap().len(), 48);
    }

    #[test]
    fn scratchpad_lines_parse() {
        let rows = parse_scratchpads(
            "0\thidden\t4194305\tscratchpad\n9\tshown\t8388609\tnirudictate-agent\n",
        );
        assert_eq!(rows[1], (9, 8388609, "shown".to_string()));
        assert!(parse_scratchpads("garbage").is_empty());
    }

    #[test]
    fn alt_shift_layout_switching_is_a_conflict() {
        let found = conflicting_xkb_options("grp:alt_shift_toggle,ctrl:nocaps", "shift+alt+space");
        assert_eq!(found, ["grp:alt_shift_toggle"]);
        assert_eq!(
            conflicting_xkb_options("grp:alt_space_toggle", "shift+alt+space"),
            ["grp:alt_space_toggle"]
        );
        assert!(conflicting_xkb_options("grp:alt_shift_toggle", "super+f9").is_empty());
        assert!(conflicting_xkb_options("grp:win_space_toggle", "shift+alt+space").is_empty());
        assert_eq!(
            conflicting_xkb_options("grp:win_space_toggle", "super+alt+space"),
            ["grp:win_space_toggle"]
        );
        assert_eq!(
            conflicting_xkb_options("grp:alt_space_toggle", "super+alt+space"),
            ["grp:alt_space_toggle"]
        );
        assert!(conflicting_xkb_options("", "shift+alt+space").is_empty());
    }
}
