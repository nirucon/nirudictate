//! The X11 side of the control panel: one window, the keyboard map, the
//! clipboard (text), cursors, and presenting the canvas with MIT-SHM (one
//! shared-memory copy per frame) or, without it, `PutImage` in slices.
use super::canvas::Canvas;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::properties::WmSizeHints;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub struct Atoms {
    pub wm_protocols: Atom,
    pub wm_delete: Atom,
    pub net_wm_name: Atom,
    pub net_wm_icon: Atom,
    pub net_wm_pid: Atom,
    pub utf8: Atom,
    pub clipboard: Atom,
    pub targets: Atom,
    pub text_plain: Atom,
    pub sel_prop: Atom,
    pub dictate_state: Atom,
}

struct Shm {
    seg: u32,
    addr: *mut u8,
    size: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    Arrow,
    Hand,
    Text,
}

pub struct X {
    pub c: RustConnection,
    pub win: Window,
    pub depth: u8,
    gc: Gcontext,
    pub atoms: Atoms,
    shm: Option<Shm>,
    shm_ok: bool,
    min_keycode: u8,
    per: u8,
    keysyms: Vec<u32>,
    cursors: [u32; 3],
    cursor: Cursor,
    /// The text this window offers on CLIPBOARD.
    pub clip_text: Option<String>,
    /// The size the window was created with, in pixels.
    pub size: (u16, u16),
}

impl X {
    /// Open a window of `logical` points (scaled by Xft.dpi); returns the
    /// connection and the scale factor.
    pub fn open(logical: (f32, f32), min: (f32, f32), title: &str) -> Res<(Self, f32)> {
        let (c, screen) = x11rb::connect(None)?;
        let s = &c.setup().roots[screen];
        let (root, depth, visual) = (s.root, s.root_depth, s.root_visual);
        let bg = s.black_pixel;
        let scale = scale_factor(&c, root);
        let px = |v: f32| (v * scale).round().clamp(1.0, 16000.0) as u16;
        let (w, h) = (px(logical.0), px(logical.1));
        let min = (px(min.0), px(min.1));
        // Interning in one batch: one round trip instead of eleven.
        let names = [
            "WM_PROTOCOLS",
            "WM_DELETE_WINDOW",
            "_NET_WM_NAME",
            "_NET_WM_ICON",
            "_NET_WM_PID",
            "UTF8_STRING",
            "CLIPBOARD",
            "TARGETS",
            "text/plain;charset=utf-8",
            "_NIRUDICTATE_SELECTION",
            "_NIRUDICTATE_STATE",
        ];
        let cookies: Vec<_> = names
            .iter()
            .map(|n| c.intern_atom(false, n.as_bytes()))
            .collect::<Result<_, _>>()?;
        let mut a = Vec::with_capacity(names.len());
        for ck in cookies {
            a.push(ck.reply()?.atom);
        }
        let atoms = Atoms {
            wm_protocols: a[0],
            wm_delete: a[1],
            net_wm_name: a[2],
            net_wm_icon: a[3],
            net_wm_pid: a[4],
            utf8: a[5],
            clipboard: a[6],
            targets: a[7],
            text_plain: a[8],
            sel_prop: a[9],
            dictate_state: a[10],
        };
        let win = c.generate_id()?;
        c.create_window(
            depth,
            win,
            root,
            0,
            0,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(bg)
                .bit_gravity(Gravity::NORTH_WEST)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::LEAVE_WINDOW
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::FOCUS_CHANGE,
                ),
        )?;
        // The service publishes its state on the root window.
        c.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirudictate\0nirudictate\0",
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.wm_protocols,
            AtomEnum::ATOM,
            &[atoms.wm_delete],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.net_wm_pid,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        let hints = WmSizeHints {
            min_size: Some((min.0 as i32, min.1 as i32)),
            size: Some((
                x11rb::properties::WmSizeHintsSpecification::ProgramSpecified,
                w as i32,
                h as i32,
            )),
            ..Default::default()
        };
        hints.set_normal_hints(&c, win)?;
        let gc = c.generate_id()?;
        c.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))?;
        let shm_ok = c
            .extension_information(x11rb::protocol::shm::X11_EXTENSION_NAME)?
            .is_some()
            && std::env::var_os("NIRUDICTATE_NO_SHM").is_none();
        let mut x = X {
            c,
            win,
            depth,
            gc,
            atoms,
            shm: None,
            shm_ok,
            min_keycode: 0,
            per: 0,
            keysyms: Vec::new(),
            cursors: [0; 3],
            cursor: Cursor::Arrow,
            clip_text: None,
            size: (w, h),
        };
        x.load_keymap()?;
        x.load_cursors();
        x.set_title(title)?;
        Ok((x, scale))
    }

    pub fn load_keymap(&mut self) -> Res<()> {
        let setup = self.c.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.c.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        self.min_keycode = min;
        self.per = map.keysyms_per_keycode;
        self.keysyms = map.keysyms;
        Ok(())
    }

    fn load_cursors(&mut self) {
        let Ok(font) = self.c.generate_id() else {
            return;
        };
        if self.c.open_font(font, b"cursor").is_err() {
            return;
        }
        // Glyphs of the standard cursor font: left_ptr, hand2, xterm.
        for (i, glyph) in [68u16, 60, 152].iter().enumerate() {
            if let Ok(id) = self.c.generate_id() {
                if self
                    .c
                    .create_glyph_cursor(
                        id,
                        font,
                        font,
                        *glyph,
                        glyph + 1,
                        0,
                        0,
                        0,
                        0xffff,
                        0xffff,
                        0xffff,
                    )
                    .is_ok()
                {
                    self.cursors[i] = id;
                }
            }
        }
        let _ = self.c.close_font(font);
    }

    pub fn set_cursor(&mut self, cursor: Cursor) {
        if cursor == self.cursor {
            return;
        }
        self.cursor = cursor;
        let id = self.cursors[cursor as usize];
        let _ = self.c.change_window_attributes(
            self.win,
            &ChangeWindowAttributesAux::new().cursor(if id == 0 { x11rb::NONE } else { id }),
        );
    }

    pub fn set_title(&self, title: &str) -> Res<()> {
        self.c.change_property8(
            PropMode::REPLACE,
            self.win,
            self.atoms.net_wm_name,
            self.atoms.utf8,
            title.as_bytes(),
        )?;
        self.c.change_property8(
            PropMode::REPLACE,
            self.win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        )?;
        Ok(())
    }

    /// `_NET_WM_ICON` from straight RGBA pixels.
    pub fn set_icon(&self, w: u32, h: u32, rgba: &[u8]) -> Res<()> {
        let mut data = vec![w, h];
        for p in rgba.as_chunks::<4>().0 {
            data.push(
                ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32,
            );
        }
        self.c.change_property32(
            PropMode::REPLACE,
            self.win,
            self.atoms.net_wm_icon,
            AtomEnum::CARDINAL,
            &data,
        )?;
        Ok(())
    }

    pub fn map(&self) -> Res<()> {
        self.c.map_window(self.win)?;
        self.c.flush()?;
        Ok(())
    }

    fn keysym_at(&self, keycode: u8, col: usize) -> u32 {
        let idx = (keycode.saturating_sub(self.min_keycode)) as usize * self.per as usize;
        if col >= self.per as usize {
            return 0;
        }
        self.keysyms.get(idx + col).copied().unwrap_or(0)
    }

    /// The keysym of a key with Shift/Lock, the XKB group (bits 13-14) and
    /// AltGr (level 3, columns 4/5).
    pub fn keysym(&self, keycode: u8, state: u16) -> u32 {
        let shift = (state & u16::from(KeyButMask::SHIFT) != 0)
            ^ (state & u16::from(KeyButMask::LOCK) != 0);
        let altgr = state & u16::from(KeyButMask::MOD5) != 0;
        let group = ((state >> 13) & 3) as usize;
        let base_col = if altgr {
            4
        } else if group > 0 {
            2
        } else {
            0
        };
        let k = self.keysym_at(keycode, base_col + usize::from(shift));
        if k != 0 {
            return k;
        }
        let base = self.keysym_at(keycode, base_col);
        if base != 0 {
            if shift && (0x61..=0x7a).contains(&base) {
                return base - 0x20;
            }
            return base;
        }
        self.keysym_at(keycode, usize::from(shift && altgr))
            .max(self.keysym_at(keycode, 0))
    }

    /// The unshifted keysym (for recording a shortcut).
    pub fn base_keysym(&self, keycode: u8) -> u32 {
        self.keysym_at(keycode, 0)
    }

    fn ensure_shm(&mut self, bytes: usize) -> bool {
        if !self.shm_ok {
            return false;
        }
        if self.shm.as_ref().is_some_and(|s| s.size >= bytes) {
            return true;
        }
        self.drop_shm();
        let size = bytes.next_multiple_of(1 << 20);
        // SAFETY: a private segment we attach, hand to the server and mark
        // for removal at once, so it cannot outlive both processes.
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if id < 0 {
                self.shm_ok = false;
                return false;
            }
            let addr = libc::shmat(id, std::ptr::null(), 0);
            if addr as isize == -1 {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                self.shm_ok = false;
                return false;
            }
            let Ok(seg) = self.c.generate_id() else {
                return false;
            };
            let attached = self
                .c
                .shm_attach(seg, id as u32, false)
                .ok()
                .and_then(|ck| ck.check().ok())
                .is_some();
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            if !attached {
                libc::shmdt(addr);
                self.shm_ok = false;
                return false;
            }
            self.shm = Some(Shm {
                seg,
                addr: addr as *mut u8,
                size,
            });
        }
        true
    }

    fn drop_shm(&mut self) {
        if let Some(s) = self.shm.take() {
            let _ = self.c.shm_detach(s.seg);
            // SAFETY: detaching the segment we attached.
            unsafe { libc::shmdt(s.addr as *const libc::c_void) };
        }
    }

    /// Copy the canvas to the window and wait until the server has read it.
    pub fn present(&mut self, cv: &Canvas) -> Res<()> {
        let bytes = cv.as_bytes();
        let (w, h) = (cv.w as u16, cv.h as u16);
        if self.depth == 24 && self.ensure_shm(bytes.len()) {
            let s = self.shm.as_ref().expect("shm");
            // SAFETY: the segment is at least `bytes.len()` long.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), s.addr, bytes.len()) };
            self.c.shm_put_image(
                self.win,
                self.gc,
                w,
                h,
                0,
                0,
                w,
                h,
                0,
                0,
                self.depth,
                ImageFormat::Z_PIXMAP.into(),
                false,
                s.seg,
                0,
            )?;
        } else {
            let max = self.c.maximum_request_bytes().saturating_sub(64).max(4096);
            let row_bytes = cv.w as usize * 4;
            let rows = (max / row_bytes).max(1);
            let mut y = 0usize;
            while y < cv.h as usize {
                let n = rows.min(cv.h as usize - y);
                self.c.put_image(
                    ImageFormat::Z_PIXMAP,
                    self.win,
                    self.gc,
                    w,
                    n as u16,
                    0,
                    y as i16,
                    0,
                    self.depth,
                    &bytes[y * row_bytes..(y + n) * row_bytes],
                )?;
                y += n;
            }
        }
        // A round trip: the next frame may overwrite the shared buffer.
        self.c.get_input_focus()?.reply()?;
        Ok(())
    }

    // ── clipboard ──────────────────────────────────────────────────────

    /// Offer `text` on CLIPBOARD.
    pub fn copy(&mut self, text: String) {
        self.clip_text = Some(text);
        let _ = self
            .c
            .set_selection_owner(self.win, self.atoms.clipboard, x11rb::CURRENT_TIME);
        let _ = self.c.flush();
    }

    /// Ask the CLIPBOARD owner for text; the answer is a SelectionNotify.
    pub fn request_paste(&self) {
        let _ = self.c.convert_selection(
            self.win,
            self.atoms.clipboard,
            self.atoms.utf8,
            self.atoms.sel_prop,
            x11rb::CURRENT_TIME,
        );
        let _ = self.c.flush();
    }

    pub fn take_paste(&self, e: &SelectionNotifyEvent) -> Option<String> {
        if e.property == x11rb::NONE {
            return None;
        }
        let r = self
            .c
            .get_property(true, self.win, e.property, AtomEnum::ANY, 0, 1 << 22)
            .ok()?
            .reply()
            .ok()?;
        String::from_utf8(r.value).ok()
    }

    /// Answer another program's paste request with our text.
    pub fn answer(&self, e: &SelectionRequestEvent) {
        let a = &self.atoms;
        let mut prop = if e.property == x11rb::NONE {
            e.target
        } else {
            e.property
        };
        let text = self.clip_text.as_deref().unwrap_or("");
        let string = u32::from(AtomEnum::STRING);
        if e.selection != a.clipboard || self.clip_text.is_none() {
            prop = x11rb::NONE;
        } else if e.target == a.targets {
            let t = [a.targets, a.utf8, a.text_plain, string];
            let _ =
                self.c
                    .change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &t);
        } else if e.target == a.utf8 || e.target == a.text_plain || e.target == string {
            let _ = self.c.change_property8(
                PropMode::REPLACE,
                e.requestor,
                prop,
                e.target,
                text.as_bytes(),
            );
        } else {
            prop = x11rb::NONE;
        }
        let ev = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: prop,
        };
        let _ = self
            .c
            .send_event(false, e.requestor, EventMask::NO_EVENT, ev);
        let _ = self.c.flush();
    }
}

/// Xft.dpi / 96 from the resource database, like other toolkits.
pub fn scale_factor(c: &RustConnection, root: Window) -> f32 {
    if let Some(v) = std::env::var("NIRUDICTATE_SCALE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
    {
        return v.clamp(0.5, 4.0);
    }
    let r = c
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            1 << 16,
        )
        .ok()
        .and_then(|c| c.reply().ok());
    let Some(r) = r else { return 1.0 };
    let text = String::from_utf8_lossy(&r.value);
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Xft.dpi:") {
            if let Ok(dpi) = v.trim().parse::<f32>() {
                return (dpi / 96.0).clamp(0.5, 4.0);
            }
        }
    }
    1.0
}

impl Drop for X {
    fn drop(&mut self) {
        self.drop_shm();
        let _ = self.c.flush();
    }
}
