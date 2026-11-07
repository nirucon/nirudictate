//! The dictation shortcut: parsing `ctrl+alt+space`, and the press/release
//! logic of the three modes. The X11 grab itself is in `daemon.rs`.
use crate::config::Mode;
use std::time::{Duration, Instant};

pub const SHIFT: u16 = 1;
pub const CONTROL: u16 = 4;
pub const MOD1: u16 = 8; // Alt
pub const MOD4: u16 = 64; // Super

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub mods: u16,
    pub keysym: u32,
}

fn keysym(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        "space" => Some(0x20),
        "return" | "enter" => Some(0xff0d),
        "tab" => Some(0xff09),
        "escape" | "esc" => Some(0xff1b),
        "backspace" => Some(0xff08),
        "insert" => Some(0xff63),
        "delete" => Some(0xffff),
        "home" => Some(0xff50),
        "end" => Some(0xff57),
        "pause" => Some(0xff13),
        "scroll_lock" => Some(0xff14),
        "print" => Some(0xff61),
        "menu" => Some(0xff67),
        "grave" => Some(0x60),
        "minus" => Some(0x2d),
        "equal" => Some(0x3d),
        "comma" => Some(0x2c),
        "period" => Some(0x2e),
        "slash" => Some(0x2f),
        "semicolon" => Some(0x3b),
        "apostrophe" => Some(0x27),
        "section" => Some(0xa7),
        "xf86audiomicmute" => Some(0x1008ffb2),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(n) = lower.strip_prefix('f') {
        if let Ok(n) = n.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0xffbe + n - 1);
            }
        }
    }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_lowercase() || c.is_ascii_digit() => Some(c as u32),
        _ => None,
    }
}

/// Parse `ctrl+alt+space`, `super+shift+d`, `f9` …
pub fn parse(spec: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = spec
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let Some((key, mods)) = parts.split_last() else {
        return Err("empty shortcut".into());
    };
    let mut mask = 0u16;
    for m in mods {
        mask |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => CONTROL,
            "alt" | "mod1" => MOD1,
            "shift" => SHIFT,
            "super" | "mod4" | "win" => MOD4,
            other => return Err(format!("unknown modifier {other:?}")),
        };
    }
    let keysym = keysym(key).ok_or_else(|| format!("unknown key {key:?}"))?;
    if mask == 0 && keysym < 0xff00 && keysym != 0xa7 {
        return Err("a plain character needs a modifier".into());
    }
    Ok(Shortcut { mods: mask, keysym })
}

/// Human form for the panel: `Ctrl + Alt + Space`.
pub fn display(spec: &str) -> String {
    spec.split('+')
        .map(|p| {
            let p = p.trim();
            match p.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => "Ctrl".to_string(),
                "alt" | "mod1" => "Alt".to_string(),
                "shift" => "Shift".to_string(),
                "super" | "mod4" | "win" => "Super".to_string(),
                other => {
                    let mut c = other.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// What a key event means for the recorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Start,
    Stop,
    /// A second tap right after a tap that started recording: keep
    /// recording, for a new agent conversation.
    DoubleTap,
}

/// Two taps closer together than this are a double tap.
pub const DOUBLE_TAP: Duration = Duration::from_millis(450);

/// Press/release interpretation. Pure, so every mode is unit tested.
#[derive(Debug)]
pub struct Controller {
    pub mode: Mode,
    pub hold: Duration,
    recording: bool,
    pressed_at: Option<Instant>,
    /// When the recording started with a press, for double taps.
    started_at: Option<Instant>,
    /// The release that belongs to a stopping press.
    swallow_release: bool,
}

impl Controller {
    pub fn new(mode: Mode, hold: Duration) -> Self {
        Self {
            mode,
            hold,
            recording: false,
            pressed_at: None,
            started_at: None,
            swallow_release: false,
        }
    }

    /// Recording ended for another reason (silence, CLI, limit).
    pub fn reset(&mut self) {
        self.recording = false;
        self.pressed_at = None;
        self.started_at = None;
    }

    /// A recording started outside the key (CLI `start`): a later press stops it.
    pub fn started_elsewhere(&mut self) {
        self.recording = true;
        self.pressed_at = None;
        self.started_at = None;
    }

    /// `double`: a quick second tap means `DoubleTap` instead of stop
    /// (the agent shortcut in hybrid and toggle mode).
    pub fn press(&mut self, now: Instant, busy: bool, double: bool) -> Action {
        if self.recording {
            let quick = self
                .started_at
                .is_some_and(|t| now.saturating_duration_since(t) < DOUBLE_TAP);
            if double && quick && self.pressed_at.is_none() && self.mode != Mode::Hold {
                self.started_at = None;
                self.swallow_release = true;
                return Action::DoubleTap;
            }
            // Hold mode stops on release; a second press while held is noise.
            if self.mode == Mode::Hold && self.pressed_at.is_some() {
                return Action::None;
            }
            self.recording = false;
            self.pressed_at = None;
            self.swallow_release = true;
            return Action::Stop;
        }
        if busy {
            self.swallow_release = true;
            return Action::None;
        }
        self.recording = true;
        self.pressed_at = Some(now);
        self.started_at = Some(now);
        self.swallow_release = false;
        Action::Start
    }

    pub fn release(&mut self, now: Instant) -> Action {
        if self.swallow_release {
            self.swallow_release = false;
            return Action::None;
        }
        let Some(at) = self.pressed_at.take() else {
            return Action::None;
        };
        if !self.recording {
            return Action::None;
        }
        let held = now.saturating_duration_since(at) >= self.hold;
        match self.mode {
            Mode::Toggle => Action::None,
            Mode::Hold => {
                self.recording = false;
                Action::Stop
            }
            Mode::Hybrid if held => {
                self.recording = false;
                Action::Stop
            }
            // A tap: keep recording until the next press or silence.
            Mode::Hybrid => Action::None,
        }
    }

    /// Whether the key is currently held as push-to-talk (silence must not stop it).
    pub fn holding(&self) -> bool {
        self.recording && self.pressed_at.is_some() && self.mode != Mode::Toggle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_default_and_common_shortcuts() {
        assert_eq!(
            parse("ctrl+alt+space").unwrap(),
            Shortcut {
                mods: CONTROL | MOD1,
                keysym: 0x20
            }
        );
        assert_eq!(parse("Super+Shift+D").unwrap().keysym, 'd' as u32);
        assert_eq!(
            parse("f9").unwrap(),
            Shortcut {
                mods: 0,
                keysym: 0xffc6
            }
        );
        assert!(parse("ctrl+").is_err());
        assert!(parse("hyper+x").is_err());
        assert!(parse("a").is_err(), "a bare letter would steal typing");
        assert_eq!(display("ctrl+alt+space"), "Ctrl + Alt + Space");
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn hybrid_tap_toggles_and_hold_is_push_to_talk() {
        let t = Instant::now();
        let mut c = Controller::new(Mode::Hybrid, Duration::from_millis(350));
        // Tap: starts and keeps recording.
        assert_eq!(c.press(t, false, false), Action::Start);
        assert_eq!(c.release(at(t, 120)), Action::None);
        assert!(!c.holding());
        // Second press stops; its release is swallowed.
        assert_eq!(c.press(at(t, 3000), false, false), Action::Stop);
        assert_eq!(c.release(at(t, 3100)), Action::None);
        // Hold: stops on release.
        assert_eq!(c.press(at(t, 5000), false, false), Action::Start);
        assert!(c.holding());
        assert_eq!(c.release(at(t, 7000)), Action::Stop);
    }

    #[test]
    fn toggle_ignores_releases() {
        let t = Instant::now();
        let mut c = Controller::new(Mode::Toggle, Duration::from_millis(350));
        assert_eq!(c.press(t, false, false), Action::Start);
        assert_eq!(c.release(at(t, 2000)), Action::None);
        assert_eq!(c.press(at(t, 4000), false, false), Action::Stop);
        assert_eq!(c.release(at(t, 4100)), Action::None);
    }

    #[test]
    fn hold_mode_stops_on_release_only() {
        let t = Instant::now();
        let mut c = Controller::new(Mode::Hold, Duration::from_millis(350));
        assert_eq!(c.press(t, false, false), Action::Start);
        assert_eq!(c.press(at(t, 10), false, false), Action::None);
        assert_eq!(c.release(at(t, 50)), Action::Stop);
    }

    #[test]
    fn a_press_while_transcribing_does_nothing() {
        let t = Instant::now();
        let mut c = Controller::new(Mode::Hybrid, Duration::from_millis(350));
        assert_eq!(c.press(t, true, false), Action::None);
        assert_eq!(c.release(at(t, 900)), Action::None);
        assert_eq!(c.press(at(t, 1000), false, false), Action::Start);
    }

    #[test]
    fn silence_stop_resets_the_controller() {
        let t = Instant::now();
        let mut c = Controller::new(Mode::Hybrid, Duration::from_millis(350));
        assert_eq!(c.press(t, false, false), Action::Start);
        assert_eq!(c.release(at(t, 100)), Action::None);
        c.reset();
        assert_eq!(c.press(at(t, 5000), false, false), Action::Start);
    }

    #[test]
    fn a_quick_second_tap_is_a_double_tap_for_the_agent() {
        let t = Instant::now();
        let mut c = Controller::new(Mode::Hybrid, Duration::from_millis(350));
        assert_eq!(c.press(t, false, true), Action::Start);
        assert_eq!(c.release(at(t, 90)), Action::None);
        // Second tap within 450 ms: keep recording, new conversation.
        assert_eq!(c.press(at(t, 250), false, true), Action::DoubleTap);
        assert_eq!(c.release(at(t, 320)), Action::None);
        // A third tap stops as usual.
        assert_eq!(c.press(at(t, 400), false, true), Action::Stop);
        // A slow second tap stops.
        assert_eq!(c.press(at(t, 5000), false, true), Action::Start);
        assert_eq!(c.release(at(t, 5080)), Action::None);
        assert_eq!(c.press(at(t, 5600), false, true), Action::Stop);
        // Without `double` a quick second tap stops, as before.
        assert_eq!(c.press(at(t, 9000), false, false), Action::Start);
        assert_eq!(c.release(at(t, 9080)), Action::None);
        assert_eq!(c.press(at(t, 9200), false, false), Action::Stop);
        // Push-to-talk mode has no double tap.
        let mut h = Controller::new(Mode::Hold, Duration::from_millis(350));
        assert_eq!(h.press(t, false, true), Action::Start);
        assert_eq!(h.release(at(t, 90)), Action::Stop);
    }
}
