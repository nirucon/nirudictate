//! NIRUDICTATE: local speech-to-text dictation for the NIRUX desktop.
mod agent;
mod audio;
mod config;
mod daemon;
mod engine;
mod hardware;
mod history;
mod hotkey;
mod insert;
mod ipc;
mod live;
mod measure;
mod models;
mod omarchy;
mod overlay;
mod panel;
mod session;
mod sounds;
mod text;
mod theme;

use ipc::Request;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Which whisper.cpp build this binary contains.
pub fn engine_variant() -> &'static str {
    match option_env!("NIRUDICTATE_ENGINE") {
        Some("baseline") => "baseline (SSE4.2)",
        _ => "x86-64-v3 (AVX2, FMA)",
    }
}

/// On a CPU without AVX2/FMA, run the baseline build installed next to this
/// one instead (the fast engine would stop with an illegal instruction).
fn use_baseline_engine_if_needed() {
    if option_env!("NIRUDICTATE_ENGINE") == Some("baseline") || hardware::fast_engine_supported() {
        return;
    }
    if std::env::var_os("NIRUDICTATE_NO_REEXEC").is_some() {
        return;
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("nirudictate-baseline"));
        }
    }
    candidates.push(config::home().join(".local/lib/nirudictate/nirudictate-baseline"));
    for path in candidates {
        if path.is_file() {
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new(&path)
                .args(std::env::args_os().skip(1))
                .env("NIRUDICTATE_NO_REEXEC", "1")
                .exec();
            eprintln!("nirudictate: cannot run {}: {err}", path.display());
            return;
        }
    }
    eprintln!("nirudictate: this CPU lacks AVX2/FMA and the baseline engine is not installed; transcription may fail");
}

const USAGE: &str = "\
NIRUDICTATE — local speech-to-text dictation

Usage: nirudictate [COMMAND]

  settings [TAB]           open the control panel (models, shortcut, output,
                           vocabulary, history, advanced)
  start | stop | toggle    control a dictation (the shortcut does the same)
  cancel                   stop recording and discard it
  status [--json]          what the service is doing
  language [sv|en|toggle]  show or change the dictation language
  models [list] [--json]   speech models, sizes and what is installed
  models recommend         the best model for this computer
  models install ID        download a model (checksum verified)
  models delete ID         remove a downloaded model
  models verify ID         check a downloaded model against its checksum
  transcribe FILE.wav      transcribe a 16-bit WAV file [--model ID]
  bench FILE.wav [ID...]   time installed models on a WAV file
  agent [status]           agent mode: opencode, its window and conversation
  agent toggle             start/stop a dictation for opencode (Shift+Alt+Space)
  agent new                the next message starts a new conversation
  agent show               show the opencode window (scratchpad 9, Super+Alt+9)
  agent send               send the text in the confirmation box now
  agent file FILE.wav      transcribe a WAV and send it like a spoken message
  doctor                   check the shortcut, audio, models and display
  daemon                   run the background service (normally automatic)
  autostart                start the service if autostart is enabled
  quit                     stop the background service
  --version | --help

The default shortcut is Ctrl+Alt+Space: tap to start (it stops by itself
after a pause, or tap again), or hold while speaking and release.
Shift+Alt+Space works the same way but sends the text to opencode (agent
mode) after a short confirmation; tap twice quickly for a new conversation.";

fn main() -> ExitCode {
    use_baseline_engine_if_needed();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("settings");
    let rest = &args[args.len().min(1)..];
    let result = match cmd {
        "--version" | "-V" | "version" => {
            println!("NIRUDICTATE {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "settings" | "panel" => panel::run(rest.first().map(String::as_str)),
        "daemon" | "service" => daemon::run(true),
        "autostart" => autostart(),
        "start" => control(Request::Start),
        "stop" => control(Request::Stop),
        "toggle" => control(Request::Toggle),
        "cancel" => control(Request::Cancel),
        "unload" => control(Request::Unload),
        "reload" => control(Request::Reload),
        "quit" => control(Request::Quit),
        "status" => status(rest.iter().any(|a| a == "--json")),
        "state" => {
            println!("{}", live::read());
            Ok(())
        }
        "language" => language(rest),
        "models" | "model" => models_cmd(rest),
        "transcribe" => transcribe(rest),
        "bench" => bench(rest),
        "agent" => agent_cmd(rest),
        "doctor" => doctor(),
        other => Err(format!("unknown command {other:?}; see nirudictate --help")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nirudictate: {e}");
            ExitCode::FAILURE
        }
    }
}

fn autostart() -> Result<(), String> {
    let cfg = config::Config::load_or_default();
    if !cfg.general.autostart {
        return Ok(());
    }
    if std::env::var_os("DISPLAY").is_none() {
        return Err("no X display; the service starts with the desktop session".into());
    }
    daemon::run(false)
}

fn control(request: Request) -> Result<(), String> {
    let reply = ipc::send(&request, Duration::from_secs(10))?;
    if !reply.ok {
        return Err(reply.message.unwrap_or_else(|| "failed".into()));
    }
    if let Some(s) = reply.status {
        println!("{}", s.state);
    }
    Ok(())
}

fn status(json: bool) -> Result<(), String> {
    let reply = ipc::send(&Request::Status, Duration::from_secs(3));
    let status = match reply {
        Ok(r) => r.status,
        Err(e) if !json => return Err(e),
        Err(_) => None,
    };
    if json {
        let value = match &status {
            Some(s) => serde_json::to_value(s).map_err(|e| e.to_string())?,
            None => serde_json::json!({"state": "stopped", "version": env!("CARGO_PKG_VERSION")}),
        };
        println!("{value}");
        return Ok(());
    }
    let s = status.ok_or("no status")?;
    println!("state:     {}", s.state);
    println!(
        "language:  {} ({})",
        config::language_label(&s.language),
        s.language
    );
    println!(
        "model:     {}{}",
        s.model.as_deref().unwrap_or("none installed"),
        match &s.loaded {
            Some(m) => format!(" (loaded: {m})"),
            None => " (not loaded)".into(),
        }
    );
    println!(
        "shortcut:  {} — {}{}",
        hotkey::display(&s.shortcut),
        s.mode,
        if s.shortcut_active {
            ""
        } else {
            " (NOT ACTIVE)"
        }
    );
    println!("audio:     {}", s.audio);
    println!("engine:    {}", s.engine);
    if let Some(l) = &s.last {
        println!(
            "last:      {:?} ({:.1} s audio, {:.2} s, {})",
            l.text, l.audio_seconds, l.seconds, l.inserted_by
        );
    }
    if let Some(e) = &s.error {
        println!("problem:   {e}");
    }
    if let Some(a) = &s.agent {
        println!(
            "agent:     {}{}",
            hotkey::display(&a.shortcut),
            match &a.problem {
                Some(p) => format!(" (OFF: {p})"),
                None if a.shortcut_active => String::new(),
                None => " (NOT ACTIVE)".into(),
            }
        );
    }
    Ok(())
}

fn agent_cmd(rest: &[String]) -> Result<(), String> {
    let sub = rest.first().map(String::as_str).unwrap_or("status");
    let request = match sub {
        "status" => {
            let reply = ipc::send(&Request::Status, Duration::from_secs(3))?;
            let a = reply
                .status
                .and_then(|s| s.agent)
                .ok_or("this service has no agent mode (update NIRUDICTATE)")?;
            if rest.iter().any(|x| x == "--json") {
                println!("{}", serde_json::to_string(&a).map_err(|e| e.to_string())?);
                return Ok(());
            }
            println!(
                "shortcut:  {}{}",
                hotkey::display(&a.shortcut),
                match &a.problem {
                    Some(p) => format!(" (off: {p})"),
                    None if a.shortcut_active => " (active)".into(),
                    None => " (not active)".into(),
                }
            );
            println!(
                "opencode:  {}{}",
                a.info.opencode.as_deref().unwrap_or("not found"),
                a.info
                    .version
                    .as_ref()
                    .map(|v| format!(" ({v})"))
                    .unwrap_or_default()
            );
            println!("folder:    {}", a.info.directory);
            println!(
                "server:    {}",
                if a.info.server {
                    "running"
                } else {
                    "not running"
                }
            );
            println!(
                "window:    {}",
                match (a.info.window, a.info.in_slot) {
                    (true, true) => "open, in scratchpad 9 (Super+Alt+9)",
                    (true, false) => "open",
                    _ => "not open (starts with the next message)",
                }
            );
            println!(
                "session:   {}{}",
                a.info.session.as_deref().unwrap_or("none yet"),
                if a.info.fresh {
                    " (the next message starts a new one)"
                } else {
                    ""
                }
            );
            return Ok(());
        }
        "toggle" => Request::AgentToggle,
        "new" => Request::AgentNew,
        "show" => Request::AgentShow,
        "send" => Request::AgentSend,
        "file" => {
            let path = rest
                .get(1)
                .ok_or("usage: nirudictate agent file FILE.wav")?;
            let path = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
            Request::AgentFile {
                path: path.display().to_string(),
            }
        }
        other => {
            return Err(format!(
                "unknown agent command {other:?}; try status, toggle, new, show, send or file"
            ))
        }
    };
    control(request)
}

fn language(rest: &[String]) -> Result<(), String> {
    let mut cfg = config::Config::load()?;
    let Some(want) = rest.first() else {
        println!("{}", cfg.general.language);
        return Ok(());
    };
    let next = match want.as_str() {
        "toggle" => {
            if cfg.general.language == "sv" {
                "en"
            } else {
                "sv"
            }
        }
        "sv" | "swedish" => "sv",
        "en" | "english" => "en",
        other => return Err(format!("unknown language {other:?} (sv or en)")),
    };
    if ipc::running() {
        let reply = ipc::send(
            &Request::Language {
                language: next.into(),
            },
            Duration::from_secs(3),
        )?;
        if !reply.ok {
            return Err(reply.message.unwrap_or_default());
        }
    } else {
        cfg.general.language = next.into();
        cfg.save()?;
    }
    println!("{next}");
    Ok(())
}

fn models_cmd(rest: &[String]) -> Result<(), String> {
    let action = rest.first().map(String::as_str).unwrap_or("list");
    let hw = hardware::detect();
    let measured = measure::load().tiny();
    match action {
        "list" | "--json" => {
            let json = rest.iter().any(|a| a == "--json");
            if json {
                let list: Vec<_> = models::CATALOG
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": m.id, "language": m.language, "label": m.label,
                            "bytes": m.bytes, "installed": m.installed(),
                            "license": m.license(), "url": m.url(), "sha256": m.sha256,
                            "estimated_seconds": models::estimate(m, &hw, measured),
                        })
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(list));
                return Ok(());
            }
            println!(
                "{:<10} {:<40} {:>9}  {:<9} EST. TIME",
                "ID", "MODEL", "SIZE", "INSTALLED"
            );
            for m in models::CATALOG {
                println!(
                    "{:<10} {:<40} {:>9}  {:<9} {:.1} s",
                    m.id,
                    m.label,
                    models::human_size(m.bytes),
                    if m.installed() { "yes" } else { "-" },
                    models::estimate(m, &hw, measured)
                );
            }
            Ok(())
        }
        "recommend" => {
            for lang in ["sv", "en"] {
                let m = models::recommend(lang, &hw, measured);
                println!("{lang}: {} ({})", m.id, m.label);
            }
            println!(
                "computer: {} — {} cores / {} threads, {}, {} MB RAM{}",
                hw.model,
                hw.physical,
                hw.logical,
                if hw.avx2 { "AVX2" } else { "no AVX2" },
                hw.memory_mb,
                if measured.is_some() {
                    ", measured"
                } else {
                    ", estimated"
                }
            );
            Ok(())
        }
        "install" | "download" => {
            let id = rest
                .get(1)
                .ok_or("models install needs an ID (see nirudictate models)")?;
            let id = if id == "recommended" {
                let cfg = config::Config::load_or_default();
                models::recommend(&cfg.general.language, &hw, measured)
                    .id
                    .to_string()
            } else {
                id.clone()
            };
            let model = models::find(&id).ok_or_else(|| format!("unknown model {id}"))?;
            if model.installed() {
                println!("{} is already installed", model.id);
                return Ok(());
            }
            println!(
                "Downloading {} ({}, {})",
                model.label,
                models::human_size(model.bytes),
                model.license()
            );
            let cancel = AtomicBool::new(false);
            let mut last = 0u64;
            models::download(model, &cancel, |p| {
                if p.verifying {
                    eprint!("\rverifying checksum…                              ");
                } else if p.done / 5_000_000 != last {
                    last = p.done / 5_000_000;
                    eprint!(
                        "\r{} / {}  {:.1} MB/s   ",
                        models::human_size(p.done),
                        models::human_size(p.total),
                        p.bytes_per_second / 1e6
                    );
                }
            })?;
            eprintln!();
            println!("Installed {} at {}", model.id, model.path().display());
            let _ = ipc::send(&Request::Reload, Duration::from_secs(2));
            Ok(())
        }
        "delete" | "remove" => {
            let id = rest.get(1).ok_or("models delete needs an ID")?;
            let model = models::find(id).ok_or_else(|| format!("unknown model {id}"))?;
            models::delete(model)?;
            let _ = ipc::send(&Request::Unload, Duration::from_secs(2));
            println!("Deleted {}", model.id);
            Ok(())
        }
        "verify" => {
            let id = rest.get(1).ok_or("models verify needs an ID")?;
            let model = models::find(id).ok_or_else(|| format!("unknown model {id}"))?;
            if !model.installed() {
                return Err(format!("{id} is not installed"));
            }
            models::verify(model)?;
            println!("{id}: checksum OK");
            Ok(())
        }
        other => Err(format!("unknown models action {other:?}")),
    }
}

fn model_arg(rest: &[String]) -> Option<String> {
    rest.iter()
        .position(|a| a == "--model")
        .and_then(|i| rest.get(i + 1))
        .cloned()
}

fn transcribe(rest: &[String]) -> Result<(), String> {
    let file = rest
        .iter()
        .find(|a| !a.starts_with("--") && Some(*a) != model_arg(rest).as_ref())
        .ok_or("transcribe needs a WAV file")?;
    let path = std::fs::canonicalize(file).map_err(|e| format!("{file}: {e}"))?;
    let model = model_arg(rest);
    let started = std::time::Instant::now();
    let t = if ipc::running() {
        let reply = ipc::send(
            &Request::TranscribeFile {
                path: path.to_string_lossy().into_owned(),
                model,
            },
            Duration::from_secs(600),
        )?;
        if !reply.ok {
            return Err(reply.message.unwrap_or_default());
        }
        reply.transcript.ok_or("no transcript")?
    } else {
        let cfg = config::Config::load_or_default();
        let hw = hardware::detect();
        let m = match model {
            Some(id) => models::find(&id).filter(|m| m.installed()),
            None => models::active(
                &cfg.general.language,
                cfg.model_for(&cfg.general.language),
                &hw,
            ),
        }
        .ok_or("no installed model (nirudictate models install recommended)")?;
        let samples = audio::read_wav(&path)?;
        engine::transcribe_now(
            m,
            samples,
            text::prompt(&cfg.vocabulary.words, m.language),
            cfg.threads(),
            cfg.advanced.nice,
        )?
    };
    let options = text::Options {
        capitalize: true,
        trailing_space: false,
    };
    println!("{}", text::finish(&t.text, options).unwrap_or_default());
    eprintln!(
        "[{}: {:.1} s audio, load {:.2} s, transcribe {:.2} s, total {:.2} s]",
        t.model,
        t.audio_seconds,
        t.load_seconds,
        t.seconds,
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

fn bench(rest: &[String]) -> Result<(), String> {
    let file = rest.first().ok_or("bench needs a WAV file")?;
    let samples = audio::read_wav(std::path::Path::new(file))?;
    let cfg = config::Config::load_or_default();
    let ids: Vec<&str> = if rest.len() > 1 {
        rest[1..].iter().map(String::as_str).collect()
    } else {
        models::installed().iter().map(|m| m.id).collect()
    };
    println!(
        "{} threads, engine {}, {:.1} s of audio",
        cfg.threads(),
        engine_variant(),
        samples.len() as f32 / audio::RATE as f32
    );
    for id in ids {
        let m = models::find(id)
            .filter(|m| m.installed())
            .ok_or_else(|| format!("{id} is not installed"))?;
        // First run includes loading; the second is the steady state.
        let prompt = text::prompt(&cfg.vocabulary.words, m.language);
        let first = engine::transcribe_now(
            m,
            samples.clone(),
            prompt.clone(),
            cfg.threads(),
            cfg.advanced.nice,
        )?;
        let second =
            engine::transcribe_now(m, samples.clone(), prompt, cfg.threads(), cfg.advanced.nice)?;
        println!(
            "{:<10} load {:>5.2} s  transcribe {:>5.2} s  {}",
            m.id,
            second.load_seconds,
            second.seconds.min(first.seconds),
            text::finish(
                &second.text,
                text::Options {
                    capitalize: true,
                    trailing_space: false
                }
            )
            .unwrap_or_default()
        );
    }
    Ok(())
}

fn doctor() -> Result<(), String> {
    let problems = std::cell::Cell::new(0);
    let line = |ok: bool, what: &str, detail: String| {
        if !ok {
            problems.set(problems.get() + 1);
        }
        println!("{} {what}: {detail}", if ok { "ok  " } else { "FAIL" });
    };
    let warn = |what: &str, detail: String| println!("WARN {what}: {detail}");
    let hw = hardware::detect();
    line(
        true,
        "computer",
        format!("{} ({} cores, {} MB)", hw.model, hw.physical, hw.memory_mb),
    );
    line(true, "engine", engine_variant().into());
    line(
        hardware::fast_engine_supported() || option_env!("NIRUDICTATE_ENGINE") == Some("baseline"),
        "cpu",
        if hw.avx2 {
            "AVX2/FMA".into()
        } else {
            "no AVX2 (baseline engine needed)".into()
        },
    );
    let display = std::env::var("DISPLAY").unwrap_or_default();
    match x11rb::connect(None) {
        Ok((conn, screen)) => {
            use x11rb::connection::{Connection, RequestConnection};
            let xtest = conn
                .extension_information(x11rb::protocol::xtest::X11_EXTENSION_NAME)
                .ok()
                .flatten()
                .is_some();
            line(true, "display", format!("X11 {display}"));
            let root = conn.setup().roots[screen].root;
            let cfg = config::Config::load().unwrap_or_default();
            let options = agent::xkb_options(&conn, root);
            let clash = agent::conflicting_xkb_options(&options, &cfg.agent.key);
            if clash.is_empty() {
                line(
                    true,
                    "agent key",
                    format!(
                        "{}: no keyboard-layout option uses it",
                        hotkey::display(&cfg.agent.key)
                    ),
                );
            } else {
                warn(
                    "agent key",
                    format!(
                        "{} clashes with the XKB option {} (it also switches the keyboard layout); choose another agent shortcut or remove the option",
                        hotkey::display(&cfg.agent.key),
                        clash.join(", ")
                    ),
                );
            }
            line(
                xtest,
                "XTest",
                if xtest {
                    "available".into()
                } else {
                    "missing (cannot paste or type)".into()
                },
            );
        }
        Err(e) => line(
            false,
            "display",
            format!("cannot open X display {display:?}: {e}"),
        ),
    }
    match audio::backend() {
        Some(b) => line(true, "audio", b.name().into()),
        None => line(false, "audio", "no pw-record, parec or arecord".into()),
    }
    let cfg = config::Config::load();
    match &cfg {
        Ok(_) => line(true, "config", config::config_path().display().to_string()),
        Err(e) => line(false, "config", e.clone()),
    }
    let cfg = cfg.unwrap_or_default();
    let lang = cfg.general.language.clone();
    match models::active(&lang, cfg.model_for(&lang), &hw) {
        Some(m) => line(true, "model", format!("{} for {lang}", m.id)),
        None => line(
            false,
            "model",
            format!(
                "none installed for {lang}; try: nirudictate models install {}",
                models::recommend(&lang, &hw, measure::load().tiny()).id
            ),
        ),
    }
    match ipc::send(&Request::Status, Duration::from_secs(2)) {
        Ok(r) => {
            let s = r.status.unwrap_or_default();
            line(
                true,
                "service",
                format!("running (pid {}, {})", s.pid, s.state),
            );
            line(
                s.shortcut_active,
                "shortcut",
                if s.shortcut_active {
                    format!("{} grabbed", hotkey::display(&s.shortcut))
                } else {
                    s.error.unwrap_or_else(|| "not active".into())
                },
            );
        }
        Err(e) => line(false, "service", e),
    }
    line(
        true,
        "sounds",
        if sounds::available() {
            "through nirux sounds".into()
        } else {
            "nirux not found (silent)".into()
        },
    );
    let cfg = config::Config::load().unwrap_or_default();
    if !cfg.agent.enabled {
        warn("agent", "agent mode is turned off in the settings".into());
    } else {
        match agent::find(&cfg.agent.opencode) {
            None => warn(
                "agent",
                format!(
                    "opencode is not installed, so agent mode ({}) is off and its shortcut is not grabbed; install opencode from https://opencode.ai",
                    hotkey::display(&cfg.agent.key)
                ),
            ),
            Some(path) => match agent::probe(&path) {
                Ok(o) => line(
                    true,
                    "agent",
                    format!(
                        "opencode {} ({}, {} API), folder {}",
                        o.version,
                        o.path.display(),
                        match o.api {
                            agent::Api::V1 => "1.x",
                            agent::Api::V2 => "2.x",
                        },
                        cfg.agent.folder().display()
                    ),
                ),
                Err(e) => line(false, "agent", e),
            },
        }
        if !cfg.agent.folder().is_dir() {
            line(
                false,
                "agent folder",
                format!("{} does not exist", cfg.agent.folder().display()),
            );
        }
        if let Ok(r) = ipc::send(&Request::Status, Duration::from_secs(2)) {
            if let Some(a) = r.status.and_then(|s| s.agent) {
                match (&a.problem, a.shortcut_active) {
                    (None, true) => line(
                        true,
                        "agent shortcut",
                        format!("{} grabbed", hotkey::display(&a.shortcut)),
                    ),
                    (Some(p), _) if a.info.opencode.is_none() || !a.enabled => {
                        let _ = p;
                    }
                    (p, _) => line(
                        false,
                        "agent shortcut",
                        p.clone().unwrap_or_else(|| "not active".into()),
                    ),
                }
            }
        }
    }
    if problems.get() > 0 {
        return Err(format!("{} problem(s) found", problems.get()));
    }
    Ok(())
}
