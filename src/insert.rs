//! Putting text into the focused X11 window.
//!
//! Paste (default): the previous clipboard is read in full (every target, so
//! images and rich text survive), the dictated text is offered as the new
//! clipboard, Ctrl+V (Ctrl+Shift+V in terminals, Shift+Insert in xterm-like
//! terminals) is sent with XTest, and once the application has fetched the
//! text the previous clipboard is offered again. Type: every character is sent
//! as a key press with XTest; characters missing from the keyboard layout are
//! typed through a temporarily remapped spare key, so å, ä, ö and anything
//! else work on every layout.
//!
//! This runs on its own thread with its own X connection, so the dictation
//! service keeps handling the shortcut while text is inserted.
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::connection::RequestConnection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    SelectionRequestEvent, Window, WindowClass, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::NONE;

/// WM_CLASS names that paste with Ctrl+Shift+V.
pub const TERMINALS: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "wezterm",
    "org.wezfurlong.wezterm",
    "st",
    "st-256color",
    "gnome-terminal",
    "gnome-terminal-server",
    "konsole",
    "xfce4-terminal",
    "tilix",
    "terminator",
    "lxterminal",
    "mate-terminal",
    "qterminal",
    "sakura",
    "terminology",
    "ghostty",
    "com.mitchellh.ghostty",
    "warp",
    "contour",
    "rio",
    "termite",
];

/// WM_CLASS names whose paste key is Shift+Insert (PRIMARY selection).
pub const PRIMARY_TERMINALS: &[&str] = &[
    "xterm",
    "uxterm",
    "urxvt",
    "rxvt",
    "urxvt-unicode",
    "rxvt-unicode",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteKey {
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
}

pub fn paste_key(classes: &[String], extra_terminals: &[String]) -> PasteKey {
    let lower: Vec<String> = classes.iter().map(|c| c.to_lowercase()).collect();
    if lower
        .iter()
        .any(|c| PRIMARY_TERMINALS.contains(&c.as_str()))
    {
        return PasteKey::ShiftInsert;
    }
    let extra: Vec<String> = extra_terminals.iter().map(|c| c.to_lowercase()).collect();
    if lower
        .iter()
        .any(|c| TERMINALS.contains(&c.as_str()) || extra.contains(c))
    {
        return PasteKey::CtrlShiftV;
    }
    PasteKey::CtrlV
}

#[derive(Clone, Debug)]
pub struct Insert {
    pub text: String,
    pub paste: bool,
    pub restore: bool,
    pub type_fallback: bool,
    pub terminals: Vec<String>,
    pub reply: Sender<Result<String, String>>,
}

pub struct Inserter {
    tx: Sender<Insert>,
    wake: std::os::fd::OwnedFd,
}

impl Inserter {
    pub fn spawn() -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| format!("X11: {e}"))?;
        let (tx, rx) = mpsc::channel();
        let (wake_read, wake) = crate::daemon::pipe().ok_or("cannot create a pipe")?;
        thread::Builder::new()
            .name("nirudictate-insert".into())
            .spawn(move || {
                let mut worker = match Worker::new(conn, screen) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("nirudictate: text insertion unavailable: {e}");
                        return;
                    }
                };
                worker.run(rx, wake_read);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { tx, wake })
    }

    pub fn insert(&self, job: Insert) {
        let _ = self.tx.send(job);
        let _ = nix_write(&self.wake);
    }
}

fn nix_write(fd: &std::os::fd::OwnedFd) -> std::io::Result<()> {
    // SAFETY: one byte from a local buffer to a descriptor we own.
    let n = unsafe { libc::write(fd.as_raw_fd(), [1u8].as_ptr() as *const libc::c_void, 1) };
    if n == 1 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// One selection's saved content: (target, type, format, bytes).
#[derive(Clone, Debug, Default)]
struct Saved {
    items: Vec<(Atom, Atom, u8, Vec<u8>)>,
}

struct Atoms {
    clipboard: Atom,
    primary: Atom,
    targets: Atom,
    timestamp: Atom,
    multiple: Atom,
    utf8: Atom,
    text: Atom,
    text_plain: Atom,
    text_plain_utf8: Atom,
    incr: Atom,
    prop: Atom,
    active: Atom,
    save_targets: Atom,
}

fn intern(conn: &RustConnection, name: &str) -> Result<Atom, String> {
    conn.intern_atom(false, name.as_bytes())
        .map_err(|e| e.to_string())?
        .reply()
        .map(|r| r.atom)
        .map_err(|e| e.to_string())
}

/// What we currently offer for a selection.
enum Offer {
    Text(String),
    Saved(Saved),
}

struct Worker {
    conn: RustConnection,
    root: Window,
    win: Window,
    atoms: Atoms,
    offers: HashMap<Atom, Offer>,
    /// Requests for the dictated text: (any text request, from the focused client).
    served: (bool, bool),
    focus_base: Option<u32>,
    resource_mask: u32,
}

impl Worker {
    fn new(conn: RustConnection, screen: usize) -> Result<Self, String> {
        let root = conn.setup().roots[screen].root;
        let resource_mask = conn.setup().resource_id_mask;
        let win = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            0,
            win,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
        let atoms = Atoms {
            clipboard: intern(&conn, "CLIPBOARD")?,
            primary: AtomEnum::PRIMARY.into(),
            targets: intern(&conn, "TARGETS")?,
            timestamp: intern(&conn, "TIMESTAMP")?,
            multiple: intern(&conn, "MULTIPLE")?,
            utf8: intern(&conn, "UTF8_STRING")?,
            text: intern(&conn, "TEXT")?,
            text_plain: intern(&conn, "text/plain")?,
            text_plain_utf8: intern(&conn, "text/plain;charset=utf-8")?,
            incr: intern(&conn, "INCR")?,
            prop: intern(&conn, "_NIRUDICTATE_SEL")?,
            active: intern(&conn, "_NET_ACTIVE_WINDOW")?,
            save_targets: intern(&conn, "SAVE_TARGETS")?,
        };
        conn.flush().map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            root,
            win,
            atoms,
            offers: HashMap::new(),
            served: (false, false),
            focus_base: None,
            resource_mask,
        })
    }

    fn run(&mut self, rx: Receiver<Insert>, wake: std::os::fd::OwnedFd) {
        loop {
            // Handle X events (selection requests for what we offer).
            while let Ok(Some(event)) = self.conn.poll_for_event() {
                self.handle(event);
            }
            let _ = self.conn.flush();
            while let Ok(job) = rx.try_recv() {
                let result = self.insert(&job);
                let _ = job.reply.send(result);
            }
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
            ];
            // SAFETY: two valid pollfds.
            unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
            if fds[1].revents != 0 {
                let mut buf = [0u8; 64];
                // SAFETY: reading into a local buffer from our own pipe.
                unsafe {
                    libc::read(
                        wake.as_raw_fd(),
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
            }
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::SelectionRequest(e) => self.serve(e),
            Event::SelectionClear(e) => {
                self.offers.remove(&e.selection);
            }
            _ => {}
        }
    }

    fn text_targets(&self) -> [Atom; 5] {
        [
            self.atoms.utf8,
            AtomEnum::STRING.into(),
            self.atoms.text,
            self.atoms.text_plain,
            self.atoms.text_plain_utf8,
        ]
    }

    fn serve(&mut self, e: SelectionRequestEvent) {
        let property = if e.property == NONE {
            e.target
        } else {
            e.property
        };
        let mut ok = false;
        if let Some(offer) = self.offers.get(&e.selection) {
            if e.target == self.atoms.targets {
                let mut list = vec![self.atoms.targets, self.atoms.timestamp];
                match offer {
                    Offer::Text(_) => list.extend(self.text_targets()),
                    Offer::Saved(s) => list.extend(s.items.iter().map(|i| i.0)),
                }
                ok = self
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        property,
                        AtomEnum::ATOM,
                        &list,
                    )
                    .is_ok();
            } else if e.target == self.atoms.timestamp {
                ok = self
                    .conn
                    .change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        property,
                        AtomEnum::INTEGER,
                        &[0],
                    )
                    .is_ok();
            } else if e.target != self.atoms.multiple {
                match offer {
                    Offer::Text(text) if self.text_targets().contains(&e.target) => {
                        let (ty, data) = if e.target == u32::from(AtomEnum::STRING) {
                            (AtomEnum::STRING.into(), latin1(text))
                        } else {
                            (self.atoms.utf8, text.as_bytes().to_vec())
                        };
                        let max = self.conn.maximum_request_bytes().saturating_sub(1024);
                        if data.len() <= max {
                            ok = self
                                .conn
                                .change_property8(
                                    PropMode::REPLACE,
                                    e.requestor,
                                    property,
                                    ty,
                                    &data,
                                )
                                .is_ok();
                        }
                        if ok && e.selection == self.wanted_selection() {
                            self.served.0 = true;
                            if self.focus_base == Some(e.requestor & !self.resource_mask) {
                                self.served.1 = true;
                            }
                        }
                    }
                    Offer::Saved(s) => {
                        if let Some((_, ty, format, data)) =
                            s.items.iter().find(|i| i.0 == e.target)
                        {
                            let max = self.conn.maximum_request_bytes().saturating_sub(1024);
                            if data.len() <= max {
                                let unit = (*format as usize / 8).max(1);
                                ok = self
                                    .conn
                                    .change_property(
                                        PropMode::REPLACE,
                                        e.requestor,
                                        property,
                                        *ty,
                                        *format,
                                        (data.len() / unit) as u32,
                                        data,
                                    )
                                    .is_ok();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if ok { property } else { NONE },
        };
        let _ = self
            .conn
            .send_event(false, e.requestor, EventMask::NO_EVENT, notify);
        let _ = self.conn.flush();
    }

    /// The selection the current paste reads (set during an insert).
    fn wanted_selection(&self) -> Atom {
        if self.offers.contains_key(&self.atoms.primary)
            && !self.offers.contains_key(&self.atoms.clipboard)
        {
            self.atoms.primary
        } else {
            self.atoms.clipboard
        }
    }

    /// Wait for one event matching `pred` (other events are handled normally).
    fn wait_event<T>(
        &mut self,
        timeout: Duration,
        mut pred: impl FnMut(&Event) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        loop {
            while let Ok(Some(event)) = self.conn.poll_for_event() {
                if let Some(v) = pred(&event) {
                    return Some(v);
                }
                self.handle(event);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let _ = self.conn.flush();
            let mut fd = libc::pollfd {
                fd: self.conn.stream().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            unsafe { libc::poll(&mut fd, 1, left.as_millis().clamp(1, 1000) as i32) };
        }
    }

    /// Fetch one target of a selection (handles INCR transfers).
    fn fetch(&mut self, selection: Atom, target: Atom) -> Option<(Atom, u8, Vec<u8>)> {
        let prop = self.atoms.prop;
        self.conn
            .convert_selection(self.win, selection, target, prop, x11rb::CURRENT_TIME)
            .ok()?;
        self.conn.flush().ok()?;
        let win = self.win;
        let got = self.wait_event(Duration::from_millis(400), |e| match e {
            Event::SelectionNotify(n) if n.requestor == win && n.selection == selection => {
                Some(n.property != NONE)
            }
            _ => None,
        })?;
        if !got {
            return None;
        }
        let reply = self
            .conn
            .get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4)
            .ok()?
            .reply()
            .ok()?;
        if reply.type_ != self.atoms.incr {
            return Some((reply.type_, reply.format, reply.value));
        }
        // INCR: read chunks until an empty one, at most 64 MB.
        let mut data = Vec::new();
        let mut ty = NONE;
        let mut format = 8;
        loop {
            let atom = prop;
            let new_value = self.wait_event(Duration::from_millis(1000), |e| match e {
                Event::PropertyNotify(p)
                    if p.window == win
                        && p.atom == atom
                        && p.state == x11rb::protocol::xproto::Property::NEW_VALUE =>
                {
                    Some(())
                }
                _ => None,
            });
            new_value?;
            let chunk = self
                .conn
                .get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4)
                .ok()?
                .reply()
                .ok()?;
            if chunk.value.is_empty() {
                break;
            }
            ty = chunk.type_;
            format = chunk.format;
            data.extend_from_slice(&chunk.value);
            if data.len() > 64 << 20 {
                return None;
            }
        }
        Some((ty, format, data))
    }

    /// Read everything a selection offers, so it can be offered again.
    fn save(&mut self, selection: Atom) -> Option<Saved> {
        let owner = self
            .conn
            .get_selection_owner(selection)
            .ok()?
            .reply()
            .ok()?
            .owner;
        if owner == NONE {
            return None;
        }
        if owner == self.win {
            return match self.offers.get(&selection) {
                Some(Offer::Saved(s)) => Some(s.clone()),
                _ => None,
            };
        }
        let started = Instant::now();
        let mut saved = Saved::default();
        let targets: Vec<Atom> = match self.fetch(selection, self.atoms.targets) {
            Some((_, 32, bytes)) => bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_ne_bytes(*c))
                .collect(),
            _ => vec![self.atoms.utf8, AtomEnum::STRING.into()],
        };
        let skip = [
            self.atoms.targets,
            self.atoms.timestamp,
            self.atoms.multiple,
            self.atoms.save_targets,
            NONE,
        ];
        let mut total = 0usize;
        for target in targets {
            if skip.contains(&target) || saved.items.iter().any(|i| i.0 == target) {
                continue;
            }
            if started.elapsed() > Duration::from_millis(1500) || total > 48 << 20 {
                break;
            }
            if let Some((ty, format, data)) = self.fetch(selection, target) {
                if ty == NONE || ty == self.atoms.incr {
                    continue;
                }
                total += data.len();
                saved.items.push((target, ty, format, data));
            }
        }
        if saved.items.is_empty() {
            None
        } else {
            Some(saved)
        }
    }

    fn own(&mut self, selection: Atom, offer: Offer) -> bool {
        self.offers.insert(selection, offer);
        let _ = self
            .conn
            .set_selection_owner(self.win, selection, x11rb::CURRENT_TIME);
        let _ = self.conn.flush();
        self.conn
            .get_selection_owner(selection)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.owner == self.win)
            .unwrap_or(false)
    }

    fn release(&mut self, selection: Atom) {
        self.offers.remove(&selection);
        let owner = self
            .conn
            .get_selection_owner(selection)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.owner);
        if owner == Some(self.win) {
            let _ = self
                .conn
                .set_selection_owner(NONE, selection, x11rb::CURRENT_TIME);
        }
        let _ = self.conn.flush();
    }

    fn active_window(&self) -> Option<Window> {
        let reply = self
            .conn
            .get_property(false, self.root, self.atoms.active, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let w = reply.value32()?.next().filter(|w| *w != NONE);
        w
    }

    fn focused_classes(&self) -> (Option<Window>, Vec<String>) {
        let focus = self.active_window().or_else(|| {
            self.conn
                .get_input_focus()
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.focus)
                .filter(|w| *w > 1)
        });
        let Some(win) = focus else {
            return (None, Vec::new());
        };
        let classes = self
            .conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| {
                r.value
                    .split(|b| *b == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect()
            })
            .unwrap_or_default();
        (Some(win), classes)
    }

    fn insert(&mut self, job: &Insert) -> Result<String, String> {
        let (focus, classes) = self.focused_classes();
        self.focus_base = focus.map(|w| w & !self.resource_mask);
        let mut keys = Keyboard::load(&self.conn)?;
        keys.wait_for_modifiers_released(&self.conn, Duration::from_millis(1500));
        if !job.paste {
            keys.type_text(&self.conn, &job.text)?;
            return Ok("typed".into());
        }
        let key = paste_key(&classes, &job.terminals);
        let selection = if key == PasteKey::ShiftInsert {
            self.atoms.primary
        } else {
            self.atoms.clipboard
        };
        let previous = if job.restore {
            self.save(selection)
        } else {
            None
        };
        self.served = (false, false);
        if !self.own(selection, Offer::Text(job.text.clone())) {
            if job.type_fallback {
                keys.type_text(&self.conn, &job.text)?;
                return Ok("typed (the clipboard could not be taken)".into());
            }
            return Err("could not take the clipboard".into());
        }
        match key {
            PasteKey::CtrlV => keys.chord(&self.conn, &[0xffe3], 0x76)?,
            PasteKey::CtrlShiftV => keys.chord(&self.conn, &[0xffe3, 0xffe1], 0x76)?,
            PasteKey::ShiftInsert => keys.chord(&self.conn, &[0xffe1], 0xff63)?,
        }
        // Wait until the focused application has fetched the text, then a
        // little longer for the follow-up request some toolkits make.
        let deadline = Instant::now() + Duration::from_millis(1200);
        let mut settled: Option<Instant> = None;
        while Instant::now() < deadline {
            let _ = self.wait_event(Duration::from_millis(30), |_| None::<()>);
            if self.served.1 && settled.is_none() {
                settled = Some(Instant::now());
            }
            if settled.is_some_and(|t| t.elapsed() > Duration::from_millis(250)) {
                break;
            }
        }
        // Only a request from the focused client proves the paste: clipboard
        // managers poll the clipboard too.
        let pasted = self.served.1;
        // Give the old clipboard back (or clear it if there was none).
        if job.restore {
            match previous {
                Some(saved) => {
                    self.own(selection, Offer::Saved(saved));
                }
                None => self.release(selection),
            }
        }
        if pasted {
            let how = match key {
                PasteKey::CtrlV => "pasted (Ctrl+V)",
                PasteKey::CtrlShiftV => "pasted (Ctrl+Shift+V)",
                PasteKey::ShiftInsert => "pasted (Shift+Insert)",
            };
            return Ok(how.into());
        }
        if job.type_fallback {
            keys.type_text(&self.conn, &job.text)?;
            return Ok("typed (the window did not paste)".into());
        }
        Err("the focused window did not paste the text".into())
    }
}

fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect()
}

/// The keyboard mapping, for XTest key events.
struct Keyboard {
    min: u8,
    per: usize,
    syms: Vec<u32>,
    /// A keycode with no symbols, used to type characters the layout lacks.
    spare: Option<u8>,
}

/// Unicode character to X keysym.
pub fn keysym_for(c: char) -> u32 {
    match c {
        '\n' => 0xff0d,
        '\t' => 0xff09,
        c if (c as u32) >= 0x20 && (c as u32) <= 0x7e => c as u32,
        c if (c as u32) >= 0xa0 && (c as u32) <= 0xff => c as u32,
        c => 0x0100_0000 + c as u32,
    }
}

impl Keyboard {
    fn load(conn: &RustConnection) -> Result<Self, String> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let per = map.keysyms_per_keycode as usize;
        let syms = map.keysyms;
        let spare = (0..syms.len() / per.max(1))
            .rev()
            .find(|i| syms[i * per..(i + 1) * per].iter().all(|s| *s == 0))
            .map(|i| min + i as u8);
        Ok(Self {
            min,
            per,
            syms,
            spare,
        })
    }

    /// (keycode, needs shift) for a keysym on the current layout's first two levels.
    fn find(&self, sym: u32) -> Option<(u8, bool)> {
        let per = self.per.max(1);
        for (i, chunk) in self.syms.chunks(per).enumerate() {
            let code = self.min + i as u8;
            if chunk.first() == Some(&sym) {
                return Some((code, false));
            }
            if chunk.get(1) == Some(&sym) {
                return Some((code, true));
            }
        }
        None
    }

    fn code(&self, sym: u32) -> Option<u8> {
        self.find(sym).map(|(c, _)| c)
    }

    fn fake(&self, conn: &RustConnection, press: bool, code: u8) -> Result<(), String> {
        let ty = if press { 2 } else { 3 };
        conn.xtest_fake_input(ty, code, x11rb::CURRENT_TIME, x11rb::NONE, 0, 0, 0)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn sync(conn: &RustConnection) {
        if let Ok(c) = conn.get_input_focus() {
            let _ = c.reply();
        }
    }

    /// Held modifier keycodes (Shift, Ctrl, Alt, Super, AltGr).
    fn held_modifiers(&self, conn: &RustConnection) -> Vec<u8> {
        let Ok(keys) = conn.query_keymap().map(|c| c.reply()) else {
            return Vec::new();
        };
        let Ok(keys) = keys else {
            return Vec::new();
        };
        let mods = [
            0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xfe03, 0xffe7, 0xffe8,
        ];
        let mut held = Vec::new();
        for sym in mods {
            for (i, chunk) in self.syms.chunks(self.per.max(1)).enumerate() {
                if chunk.first() == Some(&sym) {
                    let code = self.min as usize + i;
                    if keys.keys[code / 8] & (1 << (code % 8)) != 0 && !held.contains(&(code as u8))
                    {
                        held.push(code as u8);
                    }
                }
            }
        }
        held
    }

    /// Push-to-talk ends while Ctrl+Alt may still be held. Wait for the user
    /// to let go; after the timeout release them synthetically.
    fn wait_for_modifiers_released(&mut self, conn: &RustConnection, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let held = self.held_modifiers(conn);
            if held.is_empty() {
                return;
            }
            if Instant::now() >= deadline {
                for code in held {
                    let _ = self.fake(conn, false, code);
                }
                let _ = conn.flush();
                Self::sync(conn);
                return;
            }
            thread::sleep(Duration::from_millis(15));
        }
    }

    fn chord(&self, conn: &RustConnection, mods: &[u32], key: u32) -> Result<(), String> {
        let codes: Vec<u8> = mods
            .iter()
            .map(|m| self.code(*m).ok_or("modifier key not on this keyboard"))
            .collect::<Result<_, _>>()?;
        let key = self.code(key).ok_or("paste key not on this keyboard")?;
        for c in &codes {
            self.fake(conn, true, *c)?;
        }
        self.fake(conn, true, key)?;
        self.fake(conn, false, key)?;
        for c in codes.iter().rev() {
            self.fake(conn, false, *c)?;
        }
        conn.flush().map_err(|e| e.to_string())?;
        Self::sync(conn);
        Ok(())
    }

    fn type_text(&mut self, conn: &RustConnection, text: &str) -> Result<(), String> {
        let shift = self.code(0xffe1).ok_or("no Shift key")?;
        let mut remapped = false;
        for c in text.chars() {
            let sym = keysym_for(c);
            let (code, needs_shift) = match self.find(sym) {
                Some(found) => found,
                None => {
                    let spare = self
                        .spare
                        .ok_or("no spare key to type special characters")?;
                    let mut row = vec![0u32; self.per.max(1)];
                    row[0] = sym;
                    if self.per > 1 {
                        row[1] = sym;
                    }
                    conn.change_keyboard_mapping(1, spare, self.per as u8, &row)
                        .map_err(|e| e.to_string())?;
                    conn.flush().map_err(|e| e.to_string())?;
                    Self::sync(conn);
                    // Clients refetch the mapping after MappingNotify.
                    thread::sleep(Duration::from_millis(25));
                    remapped = true;
                    (spare, false)
                }
            };
            if needs_shift {
                self.fake(conn, true, shift)?;
            }
            self.fake(conn, true, code)?;
            self.fake(conn, false, code)?;
            if needs_shift {
                self.fake(conn, false, shift)?;
            }
            conn.flush().map_err(|e| e.to_string())?;
            Self::sync(conn);
            thread::sleep(Duration::from_millis(if remapped { 12 } else { 4 }));
        }
        if remapped {
            if let Some(spare) = self.spare {
                thread::sleep(Duration::from_millis(40));
                let row = vec![0u32; self.per.max(1)];
                let _ = conn.change_keyboard_mapping(1, spare, self.per as u8, &row);
                let _ = conn.flush();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_get_their_paste_key() {
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            paste_key(&c(&["kitty", "kitty"]), &[]),
            PasteKey::CtrlShiftV
        );
        assert_eq!(
            paste_key(&c(&["Alacritty", "Alacritty"]), &[]),
            PasteKey::CtrlShiftV
        );
        assert_eq!(
            paste_key(&c(&["xterm", "XTerm"]), &[]),
            PasteKey::ShiftInsert
        );
        assert_eq!(
            paste_key(&c(&["Navigator", "firefox"]), &[]),
            PasteKey::CtrlV
        );
        assert_eq!(
            paste_key(&c(&["myterm", "MyTerm"]), &["MyTerm".into()]),
            PasteKey::CtrlShiftV
        );
        assert_eq!(paste_key(&[], &[]), PasteKey::CtrlV);
    }

    #[test]
    fn keysyms_cover_swedish_letters_and_unicode() {
        assert_eq!(keysym_for('a'), 0x61);
        assert_eq!(keysym_for('å'), 0xe5);
        assert_eq!(keysym_for('Ö'), 0xd6);
        assert_eq!(keysym_for('€'), 0x0100_20ac);
        assert_eq!(keysym_for('\n'), 0xff0d);
    }
}
