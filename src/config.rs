//! Settings and the places NIRUDICTATE keeps its files.
//!
//! Everything lives in the user's XDG directories:
//!
//! - `~/.config/nirudictate/config.toml`   settings (this module)
//! - `~/.local/share/nirudictate/models/`  downloaded speech models
//! - `~/.local/state/nirudictate/`         history (0600) and measurements
//! - `$XDG_RUNTIME_DIR/nirudictate/`       control socket and live state
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

pub fn config_home() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn state_home() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn data_home() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

pub fn config_dir() -> PathBuf {
    config_home().join("nirudictate")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn models_dir() -> PathBuf {
    data_home().join("nirudictate/models")
}

pub fn state_dir() -> PathBuf {
    state_home().join("nirudictate")
}

pub fn history_path() -> PathBuf {
    state_dir().join("history.jsonl")
}

pub fn measurements_path() -> PathBuf {
    state_dir().join("measurements.json")
}

pub fn log_path() -> PathBuf {
    state_dir().join("daemon.log")
}

/// The private runtime directory: `$XDG_RUNTIME_DIR/nirudictate`, or a 0700
/// directory under the state directory when there is no runtime directory.
pub fn runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir());
    match base {
        Some(base) => base.join("nirudictate"),
        None => state_dir().join("run"),
    }
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("control.sock")
}

/// Live state for status bars: one small JSON object, replaced atomically.
pub fn live_state_path() -> PathBuf {
    runtime_dir().join("state.json")
}

pub fn ensure_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    Ok(())
}

/// Write a file atomically with the given mode (temporary file + rename).
pub fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("cannot write {}: {e}", path.display()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Short press toggles, a held key is push-to-talk.
    #[default]
    Hybrid,
    /// Every press starts or stops.
    Toggle,
    /// Record only while the key is held.
    Hold,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Hybrid => "Hybrid (tap to toggle, hold to talk)",
            Mode::Toggle => "Toggle (press to start, press again to stop)",
            Mode::Hold => "Push-to-talk (record while held)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// Put the text on the clipboard, paste it, restore the old clipboard.
    #[default]
    Paste,
    /// Type the text with synthetic key presses.
    Type,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    /// `sv` or `en`.
    pub language: String,
    /// Play the NIRUX dictation sounds (start, stop, done, error).
    pub sounds: bool,
    /// Start the background service with the desktop session.
    pub autostart: bool,
    /// `auto` follows NIRUX (or Omarchy outside NIRU), or a theme name.
    pub theme: String,
    /// Show the recording overlay.
    pub overlay: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: "sv".into(),
            sounds: true,
            autostart: true,
            theme: "auto".into(),
            overlay: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    /// Model id per language; empty means "the recommended installed one".
    pub sv: String,
    pub en: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Shortcut {
    pub key: String,
    pub mode: Mode,
    /// A press longer than this is push-to-talk in hybrid mode.
    pub hold_ms: u64,
    /// Stop after this much silence once speech was heard (toggle and the
    /// tapped hybrid mode). 0 turns it off.
    pub silence_seconds: f32,
    /// Hard limit for one dictation.
    pub max_seconds: u32,
}

impl Default for Shortcut {
    fn default() -> Self {
        Self {
            key: "ctrl+alt+space".into(),
            mode: Mode::Hybrid,
            hold_ms: 350,
            silence_seconds: 2.0,
            max_seconds: 120,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    pub method: Method,
    /// Put the previous clipboard back after pasting.
    pub restore_clipboard: bool,
    /// Add a space after the text so the next dictation does not run into it.
    pub trailing_space: bool,
    /// Start with a capital letter.
    pub capitalize: bool,
    /// Extra WM_CLASS names that paste with Ctrl+Shift+V.
    pub terminals: Vec<String>,
    /// Fall back to typing when nothing asked for the clipboard.
    pub type_fallback: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            method: Method::Paste,
            restore_clipboard: true,
            trailing_space: true,
            capitalize: true,
            terminals: Vec::new(),
            type_fallback: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vocabulary {
    /// Names and terms the model should prefer, passed as its initial prompt.
    pub words: Vec<String>,
}

impl Default for Vocabulary {
    fn default() -> Self {
        Self {
            words: vec![
                "NIRUX".into(),
                "NIRUWM".into(),
                "niruwm".into(),
                "nirubar".into(),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct History {
    pub enabled: bool,
    /// Entries kept; older ones are removed.
    pub keep: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            enabled: true,
            keep: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Advanced {
    /// PipeWire/Pulse source name; empty is the default microphone.
    pub microphone: String,
    /// Transcription threads; 0 picks one per physical core (at most 8).
    pub threads: u32,
    pub keep_model_loaded: bool,
    pub unload_after_minutes: u32,
    /// Niceness of the transcription threads (0..19).
    pub nice: i32,
}

impl Advanced {
    pub fn microphone_label(&self) -> &str {
        if self.microphone.is_empty() {
            "default microphone"
        } else {
            &self.microphone
        }
    }
}

impl Default for Advanced {
    fn default() -> Self {
        Self {
            microphone: String::new(),
            threads: 0,
            keep_model_loaded: false,
            unload_after_minutes: 10,
            nice: 10,
        }
    }
}

/// Agent mode: a second shortcut whose dictation goes to opencode instead of
/// the text cursor (see `agent.rs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    /// Grab the agent shortcut (only when opencode is installed).
    pub enabled: bool,
    pub key: String,
    /// The folder opencode works in; `~` is the home folder.
    pub directory: String,
    /// An opencode agent name; empty uses opencode's default agent.
    pub agent: String,
    /// The confirmation box sends by itself after this many seconds
    /// (Enter sends at once, Esc cancels). 0 sends without the box.
    pub confirm_seconds: f32,
    /// Two quick taps of the agent shortcut start a new conversation.
    pub double_tap_new: bool,
    /// `auto` (kitty, alacritty, xterm), or one of those names.
    pub terminal: String,
    /// Path to the opencode program; empty finds it on PATH and in
    /// ~/.opencode/bin and ~/.local/bin.
    pub opencode: String,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            enabled: true,
            key: "shift+alt+space".into(),
            directory: "~".into(),
            agent: String::new(),
            confirm_seconds: 2.0,
            double_tap_new: true,
            terminal: "auto".into(),
            opencode: String::new(),
        }
    }
}

pub const TERMINALS: &[&str] = &["auto", "kitty", "alacritty", "xterm"];

impl Agent {
    /// `directory` with `~` expanded.
    pub fn folder(&self) -> PathBuf {
        let d = self.directory.trim();
        if d.is_empty() || d == "~" {
            return home();
        }
        if let Some(rest) = d.strip_prefix("~/") {
            return home().join(rest);
        }
        PathBuf::from(d)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub general: General,
    pub models: Models,
    pub shortcut: Shortcut,
    pub output: Output,
    pub vocabulary: Vocabulary,
    pub history: History,
    pub advanced: Advanced,
    pub agent: Agent,
}

pub const LANGUAGES: &[(&str, &str)] = &[("sv", "Swedish"), ("en", "English")];

pub fn language_label(code: &str) -> &'static str {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, l)| *l)
        .unwrap_or("Unknown")
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !LANGUAGES.iter().any(|(c, _)| *c == self.general.language) {
            return Err(format!(
                "general.language must be sv or en, not {:?}",
                self.general.language
            ));
        }
        crate::hotkey::parse(&self.shortcut.key).map_err(|e| format!("shortcut.key: {e}"))?;
        if !(100..=2000).contains(&self.shortcut.hold_ms) {
            return Err("shortcut.hold_ms must be between 100 and 2000".into());
        }
        if !(0.0..=30.0).contains(&self.shortcut.silence_seconds) {
            return Err("shortcut.silence_seconds must be between 0 and 30".into());
        }
        if !(5..=600).contains(&self.shortcut.max_seconds) {
            return Err("shortcut.max_seconds must be between 5 and 600".into());
        }
        if self.history.keep > 10_000 {
            return Err("history.keep must be at most 10000".into());
        }
        if self.advanced.threads > 64 {
            return Err("advanced.threads must be at most 64".into());
        }
        if !(1..=1440).contains(&self.advanced.unload_after_minutes) {
            return Err("advanced.unload_after_minutes must be between 1 and 1440".into());
        }
        if !(0..=19).contains(&self.advanced.nice) {
            return Err("advanced.nice must be between 0 and 19".into());
        }
        let agent_key =
            crate::hotkey::parse(&self.agent.key).map_err(|e| format!("agent.key: {e}"))?;
        if crate::hotkey::parse(&self.shortcut.key).ok() == Some(agent_key) {
            return Err("agent.key must differ from shortcut.key".into());
        }
        if !(0.0..=10.0).contains(&self.agent.confirm_seconds) {
            return Err("agent.confirm_seconds must be between 0 and 10".into());
        }
        if !TERMINALS.contains(&self.agent.terminal.as_str()) {
            return Err(format!(
                "agent.terminal must be one of {}, not {:?}",
                TERMINALS.join(", "),
                self.agent.terminal
            ));
        }
        let folder = self.agent.directory.trim();
        if !(folder.is_empty() || folder.starts_with('~') || folder.starts_with('/')) {
            return Err("agent.directory must be an absolute path or start with ~".into());
        }
        if self
            .agent
            .agent
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("agent.agent must be a single opencode agent name".into());
        }
        Ok(())
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let cfg: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// The saved settings, or the defaults when there is no file yet.
    pub fn load() -> Result<Self, String> {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// `load`, falling back to the defaults with a warning on stderr.
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_else(|e| {
            eprintln!("nirudictate: {e}; using the defaults");
            Self::default()
        })
    }

    pub fn save(&self) -> Result<(), String> {
        self.validate()?;
        let body = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let text = format!(
            "# NIRUDICTATE settings. Edit here or with `nirudictate settings`.\n\
             # Reference: https://github.com/nirucon/nirudictate#configuration\n\n{body}"
        );
        atomic_write(&config_path(), text.as_bytes(), 0o600)
    }

    /// The model chosen for a language (may be empty).
    pub fn model_for(&self, language: &str) -> &str {
        match language {
            "en" => &self.models.en,
            _ => &self.models.sv,
        }
    }

    pub fn set_model_for(&mut self, language: &str, id: &str) {
        match language {
            "en" => self.models.en = id.to_string(),
            _ => self.models.sv = id.to_string(),
        }
    }

    pub fn threads(&self) -> u32 {
        if self.advanced.threads > 0 {
            return self.advanced.threads;
        }
        crate::hardware::default_threads()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_swedish() {
        let cfg = Config::default();
        cfg.validate().unwrap();
        assert_eq!(cfg.general.language, "sv");
        assert_eq!(cfg.shortcut.key, "ctrl+alt+space");
        assert_eq!(cfg.shortcut.mode, Mode::Hybrid);
        assert_eq!(cfg.advanced.unload_after_minutes, 10);
    }

    #[test]
    fn a_saved_file_round_trips() {
        let mut cfg = Config::default();
        cfg.general.language = "en".into();
        cfg.output.method = Method::Type;
        cfg.vocabulary.words = vec!["Åsa".into(), "Göteborg".into()];
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), cfg);
    }

    #[test]
    fn partial_files_keep_defaults() {
        let cfg = Config::parse("[shortcut]\nmode = \"hold\"\n").unwrap();
        assert_eq!(cfg.shortcut.mode, Mode::Hold);
        assert_eq!(cfg.shortcut.key, "ctrl+alt+space");
        assert!(cfg.output.restore_clipboard);
    }

    #[test]
    fn agent_mode_defaults_and_checks() {
        let cfg = Config::default();
        assert!(cfg.agent.enabled);
        assert_eq!(cfg.agent.key, "shift+alt+space");
        assert_eq!(cfg.agent.confirm_seconds, 2.0);
        assert_eq!(cfg.agent.folder(), home());
        let cfg = Config::parse("[agent]\ndirectory = \"~/src\"\nagent = \"plan\"\n").unwrap();
        assert_eq!(cfg.agent.folder(), home().join("src"));
        assert_eq!(cfg.agent.agent, "plan");
        // A 0.1 file without [agent] still loads.
        assert_eq!(
            Config::parse("[general]\nlanguage = \"en\"\n")
                .unwrap()
                .agent,
            Agent::default()
        );
        assert!(Config::parse("[agent]\nkey = \"ctrl+alt+space\"\n").is_err());
        assert!(Config::parse("[agent]\nconfirm_seconds = 30\n").is_err());
        assert!(Config::parse("[agent]\nterminal = \"gnome-terminal\"\n").is_err());
        assert!(Config::parse("[agent]\ndirectory = \"relative/dir\"\n").is_err());
        assert!(Config::parse("[agent]\nagent = \"two words\"\n").is_err());
        assert!(Config::parse("[agent]\nauto = true\n").is_err());
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(Config::parse("[general]\nlanguage = \"de\"\n").is_err());
        assert!(Config::parse("[shortcut]\nkey = \"ctrl+nosuchkey\"\n").is_err());
        assert!(Config::parse("[shortcut]\nhold_ms = 5\n").is_err());
        assert!(Config::parse("[advanced]\nnice = 40\n").is_err());
        assert!(Config::parse("[general]\nunknown = 1\n").is_err());
    }
}
