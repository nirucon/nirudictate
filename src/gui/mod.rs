//! The control panel's window toolkit: x11rb, a software canvas and a small
//! immediate-mode UI. No GPU context, no GUI framework. The event loop sleeps
//! in `poll` on the X connection and an eventfd that worker threads write to,
//! with a timeout only while something animates.
pub mod canvas;
pub mod text;
pub mod ui;
pub mod x;

use canvas::Canvas;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use ui::{Ctx, Input, Key};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{KeyButMask, ModMask};
use x11rb::protocol::Event;

/// When the process started (set first thing in `main`).
pub static START: OnceLock<Instant> = OnceLock::new();

pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Something outside the window that the panel may want to react to.
pub enum Outside {
    /// A worker thread called the waker.
    Woken,
    /// `_NIRUDICTATE_STATE` changed on the root window.
    ServiceState,
    /// The window got the keyboard focus.
    FocusIn,
}

pub trait App {
    /// Describe the whole window for this frame.
    fn frame(&mut self, cx: &mut Ctx);
    fn outside(&mut self, _event: Outside) {}
    fn exit(&mut self) {}
}

fn eventfd() -> Result<OwnedFd, String> {
    // SAFETY: plain syscall; the descriptor is owned from here on.
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub struct Window {
    pub x: x::X,
    pub cx: Ctx,
    cv: Canvas,
    wake: Arc<OwnedFd>,
    size: (u16, u16),
    timing: bool,
    first: bool,
    dead: Option<u32>,
}

/// Open the window (not yet mapped). `logical` is the size in points.
pub fn open(
    title: &str,
    logical: (f32, f32),
    min: (f32, f32),
    icon: (u32, u32, Vec<u8>),
    font_path: Option<&std::path::Path>,
    palette: crate::theme::Palette,
    body: f32,
) -> Result<Window, String> {
    let (x, conn_scale) =
        x::X::open(logical, min, title).map_err(|e| format!("cannot open the window: {e}"))?;
    let _ = x.set_icon(icon.0, icon.1, &icon.2);
    let (w, h) = x.size;
    let text = text::Text::new(font_path)?;
    let cx = Ctx::new(text, conn_scale, palette, body);
    Ok(Window {
        x,
        cx,
        cv: Canvas::new(w as i32, h as i32),
        wake: Arc::new(eventfd()?),
        size: (w, h),
        timing: std::env::var_os("NIRUDICTATE_TIMING").is_some(),
        first: true,
        dead: None,
    })
}

impl Window {
    pub fn waker(&self) -> Waker {
        let fd = self.wake.clone();
        Arc::new(move || {
            let one: u64 = 1;
            // SAFETY: writing 8 bytes from a local to our eventfd.
            unsafe {
                libc::write(fd.as_raw_fd(), &one as *const u64 as *const libc::c_void, 8);
            }
        })
    }

    fn logical_size(&self) -> (f32, f32) {
        let s = self.cx.scale;
        (self.size.0 as f32 / s, self.size.1 as f32 / s)
    }

    fn run_frame(&mut self, app: &mut dyn App, input: Input) {
        let size = self.logical_size();
        let keep = Input {
            pos: input.pos,
            down: input.down,
            ..Input::default()
        };
        let mut next = input;
        for _ in 0..4 {
            self.cx.repaint_at = None;
            self.cx.begin(next, size);
            app.frame(&mut self.cx);
            self.cx.end();
            if !self.cx.changed {
                break;
            }
            next = keep.clone();
        }
        if let Some(text) = self.cx.copy.take() {
            self.x.copy(text);
        }
        if std::mem::take(&mut self.cx.want_paste) {
            self.x.request_paste();
        }
        self.x.set_cursor(self.cx.cursor);
        self.cv.resize(self.size.0 as i32, self.size.1 as i32);
        self.cx.render(&mut self.cv);
        let _ = self.x.present(&self.cv);
        if let (Some(d), Some(path)) = (&self.cx.debug, std::env::var_os("NIRUDICTATE_HITS")) {
            let mut out = String::new();
            for (label, r) in d {
                let (x, y) = r.center();
                let s = self.cx.scale;
                out.push_str(&format!(
                    "{label}\t{}\t{}\n",
                    (x * s) as i32,
                    (y * s) as i32
                ));
            }
            let _ = std::fs::write(path, out);
        }
        if self.first {
            self.first = false;
            if self.timing {
                if let Some(start) = START.get() {
                    eprintln!(
                        "nirudictate: first frame after {:.1} ms",
                        start.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
        }
    }

    fn key(&mut self, code: u8, state: u16) -> Option<Key> {
        let sym = self.x.keysym(code, state);
        let base = self.x.base_keysym(code);
        let ctrl = state & u16::from(KeyButMask::CONTROL) != 0;
        let alt = state & u16::from(KeyButMask::MOD1) != 0;
        let sup = state & u16::from(KeyButMask::MOD4) != 0;
        let shift = state & u16::from(KeyButMask::SHIFT) != 0;
        // Modifier keys alone.
        if (0xffe1..=0xffee).contains(&sym) || sym == 0xfe03 {
            return None;
        }
        if (0xfe50..=0xfe62).contains(&sym) {
            self.dead = Some(sym);
            return None;
        }
        let mut text = if ctrl || alt || sup {
            None
        } else {
            keysym_char(sym)
        };
        if let Some(d) = self.dead.take() {
            if let Some(c) = text {
                text = Some(compose(d, c).unwrap_or(c));
            }
        }
        Some(Key {
            sym,
            base,
            ctrl,
            shift,
            alt,
            sup,
            text,
        })
    }

    /// Show the window and run until it is closed.
    pub fn run(mut self, app: &mut dyn App) -> Result<(), String> {
        // The first frame is drawn on the first Expose, when it can be seen.
        self.x.map().map_err(|e| e.to_string())?;
        let xfd = self.x.c.stream().as_raw_fd();
        let wfd = self.wake.as_raw_fd();
        let mut pos: Option<(f32, f32)> = None;
        let mut down = false;
        let mut hover: Option<u64> = None;
        // Events that arrived while waiting for a reply are already read
        // from the socket: they must be handled before sleeping in poll.
        let mut pending: Vec<Event> = Vec::new();
        loop {
            let mut queue: Vec<Input> = Vec::new();
            let mut dirty = false;
            let mut keys: Vec<Key> = Vec::new();
            let mut wheel = 0.0f32;
            let mut moved = false;
            let mut paste = None;
            let mut quit = false;
            let mut events = std::mem::take(&mut pending);
            while let Ok(Some(ev)) = self.x.c.poll_for_event() {
                events.push(ev);
            }
            for ev in events {
                let s = self.cx.scale;
                match ev {
                    Event::Expose(e) if e.count == 0 => dirty = true,
                    Event::ConfigureNotify(e) if e.window == self.x.win => {
                        if (e.width, e.height) != self.size {
                            self.size = (e.width, e.height);
                            dirty = true;
                        }
                    }
                    Event::MotionNotify(e) => {
                        pos = Some((e.event_x as f32 / s, e.event_y as f32 / s));
                        moved = true;
                    }
                    Event::LeaveNotify(_) if !down => {
                        pos = None;
                        moved = true;
                    }
                    Event::ButtonPress(e) => {
                        pos = Some((e.event_x as f32 / s, e.event_y as f32 / s));
                        match e.detail {
                            1 => {
                                down = true;
                                queue.push(Input {
                                    pos,
                                    pressed: true,
                                    down: true,
                                    shift_click: u16::from(e.state) & u16::from(ModMask::SHIFT)
                                        != 0,
                                    ..Input::default()
                                });
                            }
                            4 => wheel -= 48.0,
                            5 => wheel += 48.0,
                            _ => {}
                        }
                    }
                    Event::ButtonRelease(e) if e.detail == 1 => {
                        pos = Some((e.event_x as f32 / s, e.event_y as f32 / s));
                        down = false;
                        queue.push(Input {
                            pos,
                            released: true,
                            ..Input::default()
                        });
                    }
                    Event::KeyPress(e) => {
                        if let Some(k) = self.key(e.detail, u16::from(e.state)) {
                            keys.push(k);
                        }
                    }
                    Event::MappingNotify(_) => {
                        let _ = self.x.load_keymap();
                    }
                    Event::ClientMessage(e) => {
                        if e.format == 32 && e.data.as_data32()[0] == self.x.atoms.wm_delete {
                            quit = true;
                        }
                    }
                    Event::SelectionRequest(e) => self.x.answer(&e),
                    Event::SelectionClear(e) if e.selection == self.x.atoms.clipboard => {
                        self.x.clip_text = None;
                    }
                    Event::SelectionNotify(e) => {
                        paste = self.x.take_paste(&e);
                    }
                    Event::PropertyNotify(e) if e.atom == self.x.atoms.dictate_state => {
                        app.outside(Outside::ServiceState);
                        dirty = true;
                    }
                    Event::FocusIn(_) => {
                        app.outside(Outside::FocusIn);
                        dirty = true;
                    }
                    _ => {}
                }
            }
            if quit {
                app.exit();
                return Ok(());
            }
            if moved {
                let now = pos.and_then(|p| self.cx.hover_id(p));
                if down || self.cx.dragging() || now != hover {
                    dirty = true;
                }
                hover = now;
                if !dirty {
                    // Same widget: only the cursor may change.
                    self.x
                        .set_cursor(match pos.and_then(|p| self.cx.cursor_at(p)) {
                            Some(c) => c,
                            None => x::Cursor::Arrow,
                        });
                    let _ = self.x.c.flush();
                }
            }
            for mut input in queue {
                input.keys = std::mem::take(&mut keys);
                input.wheel = std::mem::take(&mut wheel);
                self.run_frame(app, input);
                dirty = false;
            }
            if !keys.is_empty() || wheel != 0.0 || paste.is_some() {
                self.run_frame(
                    app,
                    Input {
                        pos,
                        down,
                        keys: std::mem::take(&mut keys),
                        wheel,
                        paste: paste.take(),
                        ..Input::default()
                    },
                );
                dirty = false;
            }
            let timer_due = self.cx.repaint_at.is_some_and(|t| t <= Instant::now());
            if dirty || timer_due {
                self.run_frame(
                    app,
                    Input {
                        pos,
                        down,
                        ..Input::default()
                    },
                );
            }
            let _ = self.x.c.flush();
            while let Ok(Some(ev)) = self.x.c.poll_for_event() {
                pending.push(ev);
            }
            if !pending.is_empty() {
                continue;
            }
            // Sleep until an event, a wake-up or the next animation frame.
            let timeout = match self.cx.repaint_at {
                Some(t) => t
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(60_000) as i32,
                None => -1,
            };
            let mut fds = [
                libc::pollfd {
                    fd: xfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: two valid pollfd entries.
            unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if fds[1].revents != 0 {
                let mut buf = [0u8; 8];
                // SAFETY: draining our own eventfd into a local buffer.
                unsafe { libc::read(wfd, buf.as_mut_ptr() as *mut libc::c_void, 8) };
                app.outside(Outside::Woken);
                self.run_frame(
                    app,
                    Input {
                        pos,
                        down,
                        ..Input::default()
                    },
                );
            }
            if fds[0].revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                app.exit();
                return Err("the X connection was closed".into());
            }
        }
    }
}

/// The character a keysym types, if any.
pub fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        0x0100_0000..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
        0xffaa..=0xffb9 => char::from_u32(sym - 0xffaa + 0x2a),
        0xffbd => Some('='),
        0xff80 => Some(' '),
        0x20ac => Some('€'),
        _ => None,
    }
}

/// A dead key followed by a letter.
pub fn compose(dead: u32, c: char) -> Option<char> {
    let (from, to) = match dead {
        0xfe50 => ("aeiouAEIOU", "àèìòùÀÈÌÒÙ"),
        0xfe51 => ("aeiouyAEIOUY", "áéíóúýÁÉÍÓÚÝ"),
        0xfe52 => ("aeiouAEIOU", "âêîôûÂÊÎÔÛ"),
        0xfe53 => ("anoANO", "ãñõÃÑÕ"),
        0xfe57 => ("aeiouyAEIOU", "äëïöüÿÄËÏÖÜ"),
        0xfe58 => ("aA", "åÅ"),
        0xfe5b => ("cC", "çÇ"),
        _ => return None,
    };
    let i = from.chars().position(|x| x == c)?;
    to.chars().nth(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_type_characters_and_dead_keys_compose() {
        assert_eq!(keysym_char(0x61), Some('a'));
        assert_eq!(keysym_char(0xe5), Some('å'));
        assert_eq!(keysym_char(0x0100_20ac), Some('€'));
        assert_eq!(keysym_char(0xff0d), None);
        assert_eq!(compose(0xfe51, 'e'), Some('é'));
        assert_eq!(compose(0xfe57, 'U'), Some('Ü'));
        assert_eq!(compose(0xfe51, 'x'), None);
    }
}
