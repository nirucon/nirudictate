//! Whether NIRUDICTATE runs in the NIRU desktop (taken from NIRUAMP).
//!
//! The NIRU session (NIRUWM on X11) always wins: there the theme always follows
//! NIRUX. Detection uses the
//! session environment NIRULOGIN and `niruwm-session` export, and only when
//! that says nothing (a terminal started outside the session, say) the X11
//! window manager's own name.
use std::sync::OnceLock;

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// `XDG_CURRENT_DESKTOP` / `DESKTOP_SESSION` say NIRU, or are silent.
fn niru_from_env() -> Option<bool> {
    let desktop = env("XDG_CURRENT_DESKTOP");
    let session = env("DESKTOP_SESSION");
    if desktop.is_none() && session.is_none() {
        return None;
    }
    let niru_desktop = desktop.as_deref().is_some_and(|d| {
        d.split(':')
            .any(|t| t.eq_ignore_ascii_case("NIRU") || t.eq_ignore_ascii_case("NIRUWM"))
    });
    let niru_session = session
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("niruwm") || s.ends_with("/niruwm"));
    Some(niru_desktop || niru_session)
}

/// `_NET_WM_NAME` of the EWMH supporting window starts with `NIRUWM`.
fn niru_from_x11() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
        let Ok((conn, screen)) = x11rb::connect(None) else {
            return false;
        };
        let root = conn.setup().roots[screen].root;
        let atom = |name: &[u8]| {
            conn.intern_atom(false, name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.atom)
        };
        let (Some(check), Some(name), Some(utf8)) = (
            atom(b"_NET_SUPPORTING_WM_CHECK"),
            atom(b"_NET_WM_NAME"),
            atom(b"UTF8_STRING"),
        ) else {
            return false;
        };
        let Some(wm) = conn
            .get_property(false, root, check, AtomEnum::WINDOW, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()))
        else {
            return false;
        };
        conn.get_property(false, wm, name, utf8, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.value.starts_with(b"NIRUWM"))
    })
}

/// True inside the NIRU (NIRUWM) desktop session.
pub fn niru() -> bool {
    match niru_from_env() {
        Some(answer) => answer,
        // Wayland is never NIRUWM; only ask X when it is the display.
        None => env("WAYLAND_DISPLAY").is_none() && env("DISPLAY").is_some() && niru_from_x11(),
    }
}

#[cfg(test)]
pub(crate) fn env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<_> = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in vars {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    const CLEAR: [(&str, Option<&str>); 4] = [
        ("XDG_CURRENT_DESKTOP", None),
        ("DESKTOP_SESSION", None),
        ("WAYLAND_DISPLAY", None),
        ("DISPLAY", None),
    ];

    fn vars<'a>(set: &[(&'a str, Option<&'a str>)]) -> Vec<(&'a str, Option<&'a str>)> {
        let mut v: Vec<_> = CLEAR.to_vec();
        for (k, val) in set {
            v.retain(|(x, _)| x != k);
            v.push((k, *val));
        }
        v
    }

    #[test]
    fn niru_session_env() {
        with_env(&vars(&[("XDG_CURRENT_DESKTOP", Some("NIRU"))]), || {
            assert!(niru());
        });
        with_env(&vars(&[("XDG_CURRENT_DESKTOP", Some("NIRUWM"))]), || {
            assert!(niru())
        });
        with_env(&vars(&[("DESKTOP_SESSION", Some("niruwm"))]), || {
            assert!(niru())
        });
    }

    #[test]
    fn plain_x11_without_niru_markers() {
        with_env(&vars(&[("XDG_CURRENT_DESKTOP", Some("XFCE"))]), || {
            assert!(!niru());
        });
    }
}
