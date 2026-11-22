//! A small immediate-mode UI for the control panel, drawn on the software
//! canvas. Every frame the panel describes itself from its state; the layout
//! records drawing commands (in logical pixels) and hit regions, and input of
//! the event that caused the frame is answered while laying out, like egui.
//!
//! Frames run only for an event (input, a wake-up from a worker thread, or a
//! one-shot timer an animation asked for), so an idle panel uses no CPU.
use super::canvas::{Canvas, Rect};
use super::text::Text;
use super::x::Cursor;
use crate::color::Color32;
use crate::fonts::Face;
use crate::theme::Palette;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const SPACING_X: f32 = 10.0;
pub const SPACING_Y: f32 = 8.0;
pub const INTERACT_H: f32 = 26.0;

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Text colour that reads on `bg`.
pub fn on(bg: Color32) -> Color32 {
    let l = 0.2126 * bg.r() as f32 + 0.7152 * bg.g() as f32 + 0.0722 * bg.b() as f32;
    if l > 150.0 {
        Color32::from_rgb(20, 20, 22)
    } else {
        Color32::from_rgb(250, 250, 250)
    }
}

/// Styled text, like egui's RichText.
#[derive(Clone, Debug)]
pub struct Rt {
    pub text: String,
    pub size: Option<f32>,
    pub color: Option<Color32>,
    pub face: Face,
}

impl Rt {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            size: None,
            color: None,
            face: Face::Sans,
        }
    }
    pub fn size(mut self, s: f32) -> Self {
        self.size = Some(s);
        self
    }
    pub fn color(mut self, c: Color32) -> Self {
        self.color = Some(c);
        self
    }
    pub fn strong(mut self) -> Self {
        self.face = Face::Bold;
        self
    }
    pub fn monospace(mut self) -> Self {
        self.face = Face::Mono;
        self
    }
}

impl From<&str> for Rt {
    fn from(s: &str) -> Self {
        Rt::new(s)
    }
}
impl From<String> for Rt {
    fn from(s: String) -> Self {
        Rt::new(s)
    }
}
impl From<&String> for Rt {
    fn from(s: &String) -> Self {
        Rt::new(s.clone())
    }
}

#[derive(Clone, Debug)]
pub enum Cmd {
    Rect {
        r: Rect,
        radius: f32,
        color: u32,
        alpha: f32,
    },
    Text {
        x: f32,
        y: f32,
        size: f32,
        face: Face,
        color: u32,
        text: String,
    },
    Line {
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
        color: u32,
    },
    Clip(Option<Rect>),
}

impl Cmd {
    fn shift(&mut self, dy: f32) {
        match self {
            Cmd::Rect { r, .. } => r.y += dy,
            Cmd::Text { y, .. } => *y += dy,
            Cmd::Line { a, b, .. } => {
                a.1 += dy;
                b.1 += dy;
            }
            Cmd::Clip(_) => {}
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Key {
    pub sym: u32,
    /// The unshifted keysym of the key.
    pub base: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub sup: bool,
    pub text: Option<char>,
}

/// What happened since the last frame (logical pixels).
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub pos: Option<(f32, f32)>,
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    pub shift_click: bool,
    pub wheel: f32,
    pub keys: Vec<Key>,
    pub paste: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct Edit {
    id: u64,
    cursor: usize,
    anchor: usize,
    hscroll: f32,
    /// Remembered x when moving up and down.
    want_x: Option<f32>,
}

/// Persistent UI state plus the output of the current frame.
pub struct Ctx {
    pub text: Text,
    pub scale: f32,
    pub p: Palette,
    pub body: f32,
    pub input: Input,
    pub size: (f32, f32),
    active: Option<u64>,
    press_pos: Option<(f32, f32)>,
    drag_value: f64,
    focus: Option<u64>,
    edit: Edit,
    num_buf: String,
    popup: Option<u64>,
    popup_rect: Option<Rect>,
    popup_owner: Option<Rect>,
    popup_scroll: f32,
    pub cmds: Vec<Cmd>,
    pub over: Vec<Cmd>,
    hits: Vec<(u64, Rect, Cursor)>,
    ids: HashMap<u64, u32>,
    clip: Option<Rect>,
    pub tooltip: Option<String>,
    /// Whether the pointer is over the slider laid out last.
    pub slider_hovered: bool,
    pub repaint_at: Option<Instant>,
    /// Something was clicked or changed: the caller lays out once more.
    pub changed: bool,
    pub copy: Option<String>,
    pub want_paste: bool,
    pub cursor: Cursor,
    blocked: bool,
    started: Instant,
    /// Tests: every widget's label and rectangle of the last frame
    /// (`NIRUDICTATE_HITS=FILE`).
    pub debug: Option<Vec<(String, Rect)>>,
}

impl Ctx {
    pub fn new(text: Text, scale: f32, p: Palette, body: f32) -> Self {
        Self {
            text,
            scale,
            p,
            body,
            input: Input::default(),
            size: (0.0, 0.0),
            active: None,
            press_pos: None,
            drag_value: 0.0,
            focus: None,
            edit: Edit::default(),
            num_buf: String::new(),
            popup: None,
            popup_rect: None,
            popup_owner: None,
            popup_scroll: 0.0,
            cmds: Vec::new(),
            over: Vec::new(),
            hits: Vec::new(),
            ids: HashMap::new(),
            clip: None,
            tooltip: None,
            slider_hovered: false,
            repaint_at: None,
            changed: false,
            copy: None,
            want_paste: false,
            cursor: Cursor::Arrow,
            blocked: false,
            started: Instant::now(),
            debug: std::env::var_os("NIRUDICTATE_HITS").map(|_| Vec::new()),
        }
    }

    /// Start a frame with `input`.
    pub fn begin(&mut self, input: Input, size: (f32, f32)) {
        self.input = input;
        self.size = size;
        self.cmds.clear();
        self.over.clear();
        if let Some(d) = self.debug.as_mut() {
            d.clear();
        }
        self.hits.clear();
        self.ids.clear();
        self.clip = None;
        self.tooltip = None;
        self.changed = false;
        self.cursor = Cursor::Arrow;
        self.blocked = match (self.popup_rect, self.input.pos) {
            (Some(r), Some((x, y))) => self.popup.is_some() && r.contains(x, y),
            _ => false,
        };
        if self.input.pressed && !self.blocked {
            self.active = None;
        }
        if self.input.pressed {
            self.press_pos = self.input.pos;
        }
        // Escape closes an open list first.
        if self.popup.is_some() && self.take_key(0xff1b) {
            self.popup = None;
            self.changed = true;
        }
    }

    /// End a frame: a press outside every widget clears the focus.
    pub fn end(&mut self) {
        if self.input.released {
            self.active = None;
        }
        if self.popup.is_none() {
            self.popup_rect = None;
        }
        if self.input.pressed && self.popup.is_some() {
            // A press outside the open list (and its button) closes it.
            let keep = self.input.pos.is_some_and(|(x, y)| {
                self.popup_rect.is_some_and(|r| r.contains(x, y))
                    || self.popup_owner.is_some_and(|r| r.contains(x, y))
            });
            if !keep {
                self.popup = None;
                self.changed = true;
            }
        }
        if let Some((x, y)) = self.input.pos {
            if let Some((_, _, c)) = self.hits.iter().rev().find(|(_, r, _)| r.contains(x, y)) {
                self.cursor = *c;
            }
        }
    }

    /// The id of the widget under the pointer in the last frame.
    pub fn hover_id(&self, pos: (f32, f32)) -> Option<u64> {
        if self.popup.is_some() && self.popup_rect.is_some_and(|r| r.contains(pos.0, pos.1)) {
            return Some(1);
        }
        self.hits
            .iter()
            .rev()
            .find(|(_, r, _)| r.contains(pos.0, pos.1))
            .map(|(id, _, _)| *id)
    }

    /// The cursor the widget under `pos` asked for in the last frame.
    pub fn cursor_at(&self, pos: (f32, f32)) -> Option<Cursor> {
        if self.popup.is_some() && self.popup_rect.is_some_and(|r| r.contains(pos.0, pos.1)) {
            return Some(Cursor::Arrow);
        }
        self.hits
            .iter()
            .rev()
            .find(|(_, r, _)| r.contains(pos.0, pos.1))
            .map(|(_, _, c)| *c)
    }

    pub fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    pub fn dragging(&self) -> bool {
        self.active.is_some()
    }

    pub fn request_repaint_after(&mut self, d: Duration) {
        let at = Instant::now() + d;
        self.repaint_at = Some(self.repaint_at.map_or(at, |t| t.min(at)));
    }

    pub fn seconds(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// Take a key press with keysym `sym` (no Ctrl/Alt) from this frame.
    pub fn take_key(&mut self, sym: u32) -> bool {
        if let Some(i) = self
            .input
            .keys
            .iter()
            .position(|k| k.sym == sym && !k.ctrl && !k.alt)
        {
            self.input.keys.remove(i);
            return true;
        }
        false
    }

    fn id(&mut self, salt: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        salt.hash(&mut h);
        let base = h.finish();
        let n = self.ids.entry(base).or_insert(0);
        *n += 1;
        base.wrapping_add((*n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 2
    }

    fn visible(&self, x: f32, y: f32) -> bool {
        self.clip.is_none_or(|c| c.contains(x, y))
    }

    /// Hover/press/click for a widget at `r`.
    fn interact(&mut self, id: u64, r: Rect, cursor: Cursor, label: &str) -> Resp {
        self.hits.push((id, r, cursor));
        if let Some(d) = self.debug.as_mut() {
            d.push((label.to_string(), r));
        }
        let inside = !self.blocked
            && self
                .input
                .pos
                .is_some_and(|(x, y)| r.contains(x, y) && self.visible(x, y));
        let mut resp = Resp {
            rect: r,
            hovered: inside && (self.active.is_none() || self.active == Some(id)),
            ..Resp::default()
        };
        if inside && self.input.pressed {
            self.active = Some(id);
            resp.pressed = true;
        }
        resp.down = self.active == Some(id) && self.input.down;
        if self.input.released && self.active == Some(id) && inside {
            resp.clicked = true;
            self.changed = true;
        }
        resp
    }

    pub fn tw(&mut self, face: Face, size: f32, s: &str) -> f32 {
        self.text.width(face, size * self.scale, s) / self.scale
    }

    pub fn line_h(&mut self, face: Face, size: f32) -> f32 {
        (self.text.line_height(face, size * self.scale) / self.scale).ceil()
    }

    pub fn push(&mut self, c: Cmd) {
        self.cmds.push(c);
    }

    pub fn rect(&mut self, r: Rect, radius: f32, color: Color32) {
        self.cmds.push(Cmd::Rect {
            r,
            radius,
            color: color.rgb(),
            alpha: 1.0,
        });
    }

    /// A filled rounded rectangle with a one pixel border.
    pub fn framed(&mut self, r: Rect, radius: f32, fill: Color32, stroke: Option<Color32>) {
        match stroke {
            Some(s) => {
                self.rect(r, radius, s);
                let px = 1.0 / self.scale.max(1.0);
                self.rect(r.shrink(px, px), (radius - px).max(0.0), fill);
            }
            None => self.rect(r, radius, fill),
        }
    }

    pub fn text_at(&mut self, x: f32, y: f32, size: f32, face: Face, color: Color32, s: &str) {
        self.cmds.push(Cmd::Text {
            x,
            y,
            size,
            face,
            color: color.rgb(),
            text: s.to_string(),
        });
    }

    /// Draw the frame's commands into `cv` (pixels = logical × scale).
    pub fn render(&mut self, cv: &mut Canvas) {
        cv.reset_clip();
        cv.clear(self.p.background.rgb());
        let s = self.scale;
        let cmds = std::mem::take(&mut self.cmds);
        let over = std::mem::take(&mut self.over);
        for c in cmds.iter().chain(over.iter()) {
            match c {
                Cmd::Rect {
                    r,
                    radius,
                    color,
                    alpha,
                } => cv.round_rect(r.scale(s), radius * s, *color, *alpha),
                Cmd::Text {
                    x,
                    y,
                    size,
                    face,
                    color,
                    text,
                } => {
                    self.text
                        .draw(cv, *face, size * s, x * s, y * s, text, *color);
                }
                Cmd::Line { a, b, width, color } => {
                    cv.line((a.0 * s, a.1 * s), (b.0 * s, b.1 * s), width * s, *color)
                }
                Cmd::Clip(Some(r)) => cv.set_clip(r.scale(s)),
                Cmd::Clip(None) => cv.reset_clip(),
            }
        }
        self.cmds = cmds;
        self.over = over;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Resp {
    pub rect: Rect,
    pub hovered: bool,
    pub pressed: bool,
    pub down: bool,
    pub clicked: bool,
    pub changed: bool,
    pub lost_focus: bool,
}

impl Resp {
    pub fn on_hover_text(self, ui: &mut Ui, text: &str) -> Self {
        if self.hovered {
            ui.cx.tooltip = Some(text.to_string());
        }
        self
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dir {
    Vertical,
    Row { wrap: bool },
    Rtl,
}

pub struct Ui<'a> {
    pub cx: &'a mut Ctx,
    left: f32,
    right: f32,
    top: f32,
    x: f32,
    y: f32,
    row_h: f32,
    dir: Dir,
    max_x: f32,
    min_x: f32,
    bottom: f32,
    /// Items of the current row: (first command, height) for centring.
    row_items: Vec<(usize, f32)>,
}

impl<'a> Ui<'a> {
    pub fn new(cx: &'a mut Ctx, area: Rect) -> Self {
        Self {
            cx,
            left: area.x,
            right: area.right(),
            top: area.y,
            x: area.x,
            y: area.y,
            row_h: 0.0,
            dir: Dir::Vertical,
            max_x: area.x,
            min_x: area.right(),
            bottom: area.y,
            row_items: Vec::new(),
        }
    }

    pub fn p(&self) -> Palette {
        self.cx.p
    }

    pub fn available_width(&self) -> f32 {
        match self.dir {
            Dir::Vertical => self.right - self.left,
            Dir::Row { .. } => self.right - self.x,
            Dir::Rtl => self.x - self.left,
        }
    }

    /// Height used so far (without trailing spacing).
    pub fn used_height(&self) -> f32 {
        (self
            .bottom
            .max(self.y + if self.row_h > 0.0 { self.row_h } else { 0.0 })
            - self.top)
            .max(0.0)
    }

    pub fn cursor_y(&self) -> f32 {
        self.y
    }

    fn flush_row(&mut self) {
        if self.row_items.is_empty() {
            return;
        }
        let items = std::mem::take(&mut self.row_items);
        let end = self.cx.cmds.len();
        for (i, (start, h)) in items.iter().enumerate() {
            let stop = items.get(i + 1).map_or(end, |n| n.0);
            let dy = ((self.row_h - h) / 2.0).floor();
            if dy > 0.0 {
                for c in &mut self.cx.cmds[*start..stop] {
                    c.shift(dy);
                }
            }
        }
    }

    fn newline(&mut self) {
        self.flush_row();
        if self.row_h > 0.0 {
            self.y += self.row_h + SPACING_Y;
        }
        self.x = self.left;
        self.row_h = 0.0;
    }

    /// Reserve space for an item of `w` × `h`; `wraps` lets a row break.
    fn alloc(&mut self, w: f32, h: f32) -> Rect {
        let r = match self.dir {
            Dir::Vertical => {
                let r = Rect::new(self.left, self.y, w, h);
                self.y += h + SPACING_Y;
                r
            }
            Dir::Row { wrap } => {
                if wrap && self.x > self.left && self.x + w > self.right {
                    // The item moves to the next row with its centring entry.
                    let last = self.row_items.pop();
                    self.newline();
                    self.row_items.extend(last);
                }
                let r = Rect::new(self.x, self.y, w, h);
                self.x += w + SPACING_X;
                self.row_h = self.row_h.max(h);
                r
            }
            Dir::Rtl => {
                let r = Rect::new(self.x - w, self.y, w, h);
                self.x -= w + SPACING_X;
                self.row_h = self.row_h.max(h);
                r
            }
        };
        self.max_x = self.max_x.max(r.right());
        self.min_x = self.min_x.min(r.x);
        self.bottom = self.bottom.max(r.bottom());
        r
    }

    /// Mark the start of a row item (its commands follow).
    fn item(&mut self, h: f32) {
        if self.dir != Dir::Vertical {
            self.row_items.push((self.cx.cmds.len(), h));
        }
    }

    pub fn add_space(&mut self, v: f32) {
        match self.dir {
            Dir::Vertical => self.y += v,
            Dir::Row { .. } => self.x += v,
            Dir::Rtl => self.x -= v,
        }
    }

    fn child_area(&mut self) -> Rect {
        match self.dir {
            Dir::Vertical => Rect::new(self.left, self.y, self.right - self.left, 0.0),
            Dir::Row { .. } => Rect::new(self.x, self.y, self.right - self.x, 0.0),
            Dir::Rtl => Rect::new(self.left, self.y, self.x - self.left, 0.0),
        }
    }

    fn run_child<R>(&mut self, dir: Dir, f: impl FnOnce(&mut Ui) -> R) -> R {
        let area = self.child_area();
        let start = self.cx.cmds.len();
        let (r, w, h) = {
            let mut child = Ui::new(self.cx, area);
            child.dir = dir;
            if dir == Dir::Rtl {
                child.x = area.right();
            }
            let r = f(&mut child);
            child.flush_row();
            let h = child.used_height();
            let w = if dir == Dir::Rtl {
                area.right() - child.min_x.min(area.right())
            } else {
                (child.max_x - area.x).max(0.0)
            };
            (r, w, h)
        };
        if self.dir != Dir::Vertical {
            self.row_items.push((start, h));
        }
        if dir == Dir::Rtl {
            // The rest of the row belongs to it.
            self.bottom = self.bottom.max(self.y + h);
            self.row_h = self.row_h.max(h);
            if self.dir == Dir::Vertical {
                self.y += h + SPACING_Y;
            } else {
                self.x = self.right;
            }
            return r;
        }
        match self.dir {
            Dir::Vertical => {
                self.y += h + SPACING_Y;
                self.max_x = self.max_x.max(area.x + w);
                self.bottom = self.bottom.max(area.y + h);
            }
            _ => {
                self.x += w + SPACING_X;
                self.row_h = self.row_h.max(h);
                self.max_x = self.max_x.max(area.x + w);
                self.bottom = self.bottom.max(area.y + h);
            }
        }
        r
    }

    pub fn horizontal<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.run_child(Dir::Row { wrap: false }, f)
    }

    pub fn horizontal_wrapped<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.run_child(Dir::Row { wrap: true }, f)
    }

    pub fn vertical<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.run_child(Dir::Vertical, f)
    }

    /// Lay out the rest of the row from the right edge.
    pub fn right_to_left<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.run_child(Dir::Rtl, f)
    }

    /// A full-width block with a background: cards, quotes.
    pub fn frame<R>(
        &mut self,
        fill: Color32,
        stroke: Option<Color32>,
        radius: f32,
        margin: (f32, f32),
        f: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let area = self.child_area();
        let slot = self.cx.cmds.len();
        self.cx.cmds.push(Cmd::Clip(None));
        self.cx.cmds.push(Cmd::Clip(None));
        let inner = Rect::new(
            area.x + margin.0,
            area.y + margin.1,
            (area.w - 2.0 * margin.0).max(10.0),
            0.0,
        );
        let (r, h) = {
            let mut child = Ui::new(self.cx, inner);
            let r = f(&mut child);
            child.flush_row();
            (r, child.used_height())
        };
        let outer = Rect::new(area.x, area.y, area.w, h + 2.0 * margin.1);
        let px = 1.0 / self.cx.scale.max(1.0);
        let rect = |r: Rect, radius: f32, c: Color32, alpha: f32| Cmd::Rect {
            r,
            radius,
            color: c.rgb(),
            alpha,
        };
        let (a, b) = match stroke {
            Some(s) => (
                rect(outer, radius, s, 1.0),
                rect(outer.shrink(px, px), (radius - px).max(0.0), fill, 1.0),
            ),
            None => (
                rect(Rect::default(), 0.0, fill, 0.0),
                rect(outer, radius, fill, 1.0),
            ),
        };
        self.cx.cmds[slot] = a;
        self.cx.cmds[slot + 1] = b;
        self.item(outer.h);
        self.alloc(outer.w, outer.h);
        r
    }

    pub fn set_clip(&mut self, r: Option<Rect>) {
        self.cx.clip = r;
        self.cx.cmds.push(Cmd::Clip(r));
    }

    // ── text ───────────────────────────────────────────────────────────

    fn style(&self, rt: &Rt) -> (f32, Face, Color32) {
        (
            rt.size.unwrap_or(self.cx.body),
            rt.face,
            rt.color.unwrap_or(self.cx.p.foreground),
        )
    }

    pub fn label(&mut self, rt: impl Into<Rt>) -> Resp {
        let rt = rt.into();
        let (size, face, color) = self.style(&rt);
        let lh = self.cx.line_h(face, size);
        let w = self.cx.tw(face, size, &rt.text);
        let full = self.right - self.left;
        let lines: Vec<String> = match self.dir {
            Dir::Vertical => {
                if w <= full && !rt.text.contains('\n') {
                    vec![rt.text.clone()]
                } else {
                    let s = self.cx.scale;
                    self.cx.text.wrap(face, size * s, &rt.text, full * s)
                }
            }
            Dir::Row { wrap } => {
                if wrap && self.x > self.left && self.x + w > self.right {
                    self.newline();
                }
                if wrap && w > self.right - self.x {
                    let s = self.cx.scale;
                    self.cx
                        .text
                        .wrap(face, size * s, &rt.text, (self.right - self.x) * s)
                } else {
                    vec![rt.text.clone()]
                }
            }
            Dir::Rtl => vec![rt.text.clone()],
        };
        let lw = if lines.len() > 1 {
            lines
                .iter()
                .map(|l| self.cx.tw(face, size, l))
                .fold(0.0, f32::max)
        } else {
            w
        };
        let h = lh * lines.len().max(1) as f32;
        self.item(h);
        let r = self.alloc(lw.ceil(), h);
        for (i, l) in lines.iter().enumerate() {
            self.cx
                .text_at(r.x, r.y + i as f32 * lh, size, face, color, l);
        }
        let hovered = self
            .cx
            .input
            .pos
            .is_some_and(|(x, y)| r.contains(x, y) && !self.cx.blocked);
        Resp {
            rect: r,
            hovered,
            ..Resp::default()
        }
    }

    pub fn separator(&mut self) {
        let h = 6.0;
        self.item(h);
        let w = self.right - self.left;
        let r = self.alloc(w, h);
        let c = self.cx.p.border;
        self.cx.rect(Rect::new(r.x, r.y + 2.5, r.w, 1.0), 0.0, c);
    }

    // ── buttons ────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn button_with(
        &mut self,
        rt: Rt,
        default_size: f32,
        pad: (f32, f32),
        min_h: f32,
        radius: f32,
        fill: Option<Color32>,
        stroke: Option<Color32>,
        cursor: Cursor,
    ) -> Resp {
        let size = rt.size.unwrap_or(default_size);
        let lh = self.cx.line_h(rt.face, size);
        let tw = self.cx.tw(rt.face, size, &rt.text);
        let w = (tw + 2.0 * pad.0).ceil();
        let h = (lh + 2.0 * pad.1).max(min_h).ceil();
        self.item(h);
        let r = self.alloc(w, h);
        let id = self.cx.id(&rt.text);
        let resp = self.cx.interact(id, r, cursor, &rt.text);
        let p = self.cx.p;
        let base_fill = fill.unwrap_or(p.surface_hi);
        let f = if resp.down {
            if fill.is_some() {
                mix(base_fill, p.foreground, 0.15)
            } else {
                p.selected
            }
        } else if resp.hovered {
            if fill.is_some() {
                mix(base_fill, p.foreground, 0.08)
            } else {
                mix(p.surface_hi, p.focus, 0.18)
            }
        } else {
            base_fill
        };
        let s = if resp.hovered && fill.is_none() {
            p.focus
        } else {
            stroke.unwrap_or(p.border)
        };
        self.cx.framed(r, radius, f, Some(s));
        let color = rt.color.unwrap_or(p.foreground);
        self.cx.text_at(
            r.x + (w - tw) / 2.0,
            r.y + (h - lh) / 2.0,
            size,
            rt.face,
            color,
            &rt.text,
        );
        resp
    }

    pub fn button(&mut self, rt: impl Into<Rt>) -> Resp {
        let body = self.cx.body;
        self.button_with(
            rt.into(),
            body,
            (12.0, 6.0),
            INTERACT_H,
            7.0,
            None,
            None,
            Cursor::Arrow,
        )
    }

    pub fn small_button(&mut self, rt: impl Into<Rt>) -> Resp {
        let small = (self.cx.body * 0.8).round();
        self.button_with(
            rt.into(),
            small,
            (8.0, 1.0),
            0.0,
            7.0,
            None,
            None,
            Cursor::Arrow,
        )
    }

    pub fn primary_button(&mut self, text: &str) -> Resp {
        let p = self.cx.p;
        let fill = p.accent;
        let body = self.cx.body;
        self.button_with(
            Rt::new(text).strong().color(on(fill)),
            body,
            (12.0, 6.0),
            30.0,
            8.0,
            Some(fill),
            Some(mix(fill, p.foreground, 0.15)),
            Cursor::Arrow,
        )
    }

    pub fn segment(&mut self, text: &str, selected: bool) -> Resp {
        let p = self.cx.p;
        let fill = if selected { p.accent } else { p.surface_hi };
        let color = if selected { on(fill) } else { p.foreground };
        let body = self.cx.body;
        self.button_with(
            Rt::new(text).color(color),
            body,
            (12.0, 6.0),
            INTERACT_H,
            20.0,
            if selected { Some(fill) } else { None },
            Some(if selected { fill } else { p.border }),
            Cursor::Arrow,
        )
    }

    /// A full-width entry of the navigation list.
    pub fn nav_item(&mut self, text: &str, selected: bool) -> Resp {
        let height = 34.0;
        let w = self.right - self.left;
        self.item(height);
        let r = self.alloc(w, height);
        let id = self.cx.id(text);
        let resp = self.cx.interact(id, r, Cursor::Hand, text);
        let p = self.cx.p;
        if selected {
            self.cx.rect(
                r,
                8.0,
                mix(p.panel, p.accent, if p.light { 0.16 } else { 0.20 }),
            );
            self.cx
                .rect(Rect::new(r.x, r.y + 8.0, 3.0, height - 16.0), 2.0, p.accent);
        } else if resp.hovered {
            self.cx.rect(r, 8.0, mix(p.panel, p.foreground, 0.06));
        }
        let color = if selected {
            p.foreground
        } else {
            mix(p.muted, p.foreground, 0.3)
        };
        let lh = self.cx.line_h(Face::Sans, 14.5);
        self.cx.text_at(
            r.x + 16.0,
            r.y + (height - lh) / 2.0,
            14.5,
            Face::Sans,
            color,
            text,
        );
        resp
    }

    // ── small shapes ───────────────────────────────────────────────────

    pub fn status_dot(&mut self, color: Color32, size: f32) {
        self.item(size);
        let r = self.alloc(size, size);
        self.cx.rect(r, size / 2.0, color);
    }

    pub fn chip(&mut self, text: &str, color: Color32) {
        let p = self.cx.p;
        let lh = self.cx.line_h(Face::Sans, 12.0);
        let tw = self.cx.tw(Face::Sans, 12.0, text);
        let (w, h) = ((9.0 + 7.0 + 6.0 + tw + 9.0).ceil(), (lh + 4.0).ceil());
        self.item(h);
        let r = self.alloc(w, h);
        self.cx.framed(
            r,
            20.0,
            mix(p.surface, color, if p.light { 0.16 } else { 0.22 }),
            Some(mix(p.surface, color, 0.55)),
        );
        self.cx.rect(
            Rect::new(r.x + 9.0, r.y + (h - 7.0) / 2.0, 7.0, 7.0),
            3.5,
            color,
        );
        self.cx.text_at(
            r.x + 22.0,
            r.y + (h - lh) / 2.0,
            12.0,
            Face::Sans,
            p.foreground,
            text,
        );
    }

    pub fn keycaps(&mut self, spec: &str) {
        let p = self.cx.p;
        let display = crate::hotkey::display(spec);
        let size = self.cx.body;
        let lh = self.cx.line_h(Face::Mono, size);
        let plus = self.cx.tw(Face::Sans, size, "+");
        let parts: Vec<&str> = display.split(" + ").collect();
        let widths: Vec<f32> = parts
            .iter()
            .map(|s| self.cx.tw(Face::Mono, size, s) + 16.0)
            .collect();
        let gap = 4.0;
        let total: f32 = widths.iter().sum::<f32>()
            + (parts.len().saturating_sub(1)) as f32 * (plus + 2.0 * gap);
        let h = (lh + 6.0).ceil();
        self.item(h);
        let r = self.alloc(total.ceil(), h);
        let mut x = r.x;
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                x += gap;
                self.cx
                    .text_at(x, r.y + 3.0, size, Face::Sans, p.muted, "+");
                x += plus + gap;
            }
            let cap = Rect::new(x, r.y, widths[i], h);
            self.cx.framed(cap, 6.0, p.surface_hi, Some(p.border_hi));
            self.cx
                .text_at(x + 8.0, r.y + 3.0, size, Face::Mono, p.foreground, part);
            x += widths[i];
        }
    }

    pub fn quote(&mut self, text: &str) {
        let p = self.cx.p;
        let t = text.to_string();
        self.frame(p.inset, None, 8.0, (12.0, 10.0), |ui| {
            ui.label(Rt::new(t).size(15.0).color(p.foreground));
        });
    }

    pub fn level_meter(&mut self, levels: &[f32], w: f32, h: f32) {
        let p = self.cx.p;
        self.item(h);
        let rect = self.alloc(w, h);
        self.cx.rect(rect, 6.0, p.inset);
        let bars = 32usize;
        let gap = 2.0;
        let bw = (rect.w - 8.0 - gap * (bars as f32 - 1.0)) / bars as f32;
        for i in 0..bars {
            let v = levels
                .len()
                .checked_sub(bars - i)
                .and_then(|j| levels.get(j))
                .copied()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let bh = (rect.h - 8.0) * v.max(0.06);
            let x = rect.x + 4.0 + i as f32 * (bw + gap);
            let color = if v > 0.92 {
                red(&p)
            } else {
                mix(p.surface, p.accent, 0.45 + 0.55 * v)
            };
            self.cx.rect(
                Rect::new(x, rect.y + rect.h / 2.0 - bh / 2.0, bw, bh),
                bw / 2.0,
                color,
            );
        }
    }

    /// The microphone mark.
    pub fn logo(&mut self, size: f32) {
        let p = self.cx.p;
        self.item(size);
        let r = self.alloc(size, size);
        let (cx, cy) = r.center();
        self.cx.rect(r, size / 2.0, p.accent);
        let fg = on(p.accent);
        let (bw, bh) = (size * 0.24, size * 0.40);
        self.cx.rect(
            Rect::new(cx - bw / 2.0, cy - size * 0.08 - bh / 2.0, bw, bh),
            size * 0.12,
            fg,
        );
        let width = size * 0.06;
        let rad = size * 0.21;
        let mut prev = None;
        for i in 0..=16 {
            let a = std::f32::consts::PI * (i as f32 / 16.0);
            let pt = (cx + rad * a.cos(), cy + rad * a.sin() - size * 0.02);
            if let Some(q) = prev {
                self.cx.push(Cmd::Line {
                    a: q,
                    b: pt,
                    width,
                    color: fg.rgb(),
                });
            }
            prev = Some(pt);
        }
        self.cx.push(Cmd::Line {
            a: (cx, cy + size * 0.19),
            b: (cx, cy + size * 0.30),
            width,
            color: fg.rgb(),
        });
    }

    pub fn spinner(&mut self) {
        let p = self.cx.p;
        let size = 18.0;
        self.item(size);
        let r = self.alloc(size, size);
        let (cx, cy) = r.center();
        let t = self.cx.seconds() * 5.0;
        let rad = size / 2.0 - 2.0;
        let mut prev = None;
        for i in 0..=14 {
            let a = t + i as f32 / 14.0 * std::f32::consts::PI * 1.4;
            let pt = (cx + rad * a.cos(), cy + rad * a.sin());
            if let Some(q) = prev {
                self.cx.push(Cmd::Line {
                    a: q,
                    b: pt,
                    width: 2.0,
                    color: p.foreground.rgb(),
                });
            }
            prev = Some(pt);
        }
        self.cx.request_repaint_after(Duration::from_millis(100));
    }

    pub fn progress_bar(&mut self, frac: f32, width: f32, text: &str) {
        let p = self.cx.p;
        let h = 20.0;
        self.item(h);
        let r = self.alloc(width, h);
        self.cx.rect(r, h / 2.0, p.inset);
        let fw = (r.w * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { h } else { 0.0 });
        if fw > 0.0 {
            self.cx
                .rect(Rect::new(r.x, r.y, fw.min(r.w), h), h / 2.0, p.selected);
        }
        let size = self.cx.body * 0.85;
        let lh = self.cx.line_h(Face::Sans, size);
        let tw = self.cx.tw(Face::Sans, size, text);
        self.cx.text_at(
            r.x + ((r.w - tw) / 2.0).max(6.0),
            r.y + (h - lh) / 2.0,
            size,
            Face::Sans,
            p.foreground,
            text,
        );
    }

    // ── choices ────────────────────────────────────────────────────────

    fn toggle_row(&mut self, text: &str, round: bool, on_state: bool) -> Resp {
        let p = self.cx.p;
        let size = self.cx.body;
        let lh = self.cx.line_h(Face::Sans, size);
        let tw = if text.is_empty() {
            0.0
        } else {
            self.cx.tw(Face::Sans, size, text) + 8.0
        };
        let bx = 18.0;
        let h = lh.max(bx);
        let w = (bx + tw).ceil();
        self.item(h);
        let r = self.alloc(w, h);
        let id = self
            .cx
            .id(&format!("{}{text}", if round { "r:" } else { "c:" }));
        let resp = self.cx.interact(id, r, Cursor::Arrow, text);
        let b = Rect::new(r.x, r.y + (h - bx) / 2.0, bx, bx);
        let stroke = if resp.hovered { p.focus } else { p.border };
        if round {
            self.cx.framed(b, bx / 2.0, p.surface_hi, Some(stroke));
            if on_state {
                let (cx, cy) = b.center();
                self.cx
                    .rect(Rect::new(cx - 4.5, cy - 4.5, 9.0, 9.0), 4.5, p.foreground);
            }
        } else {
            let fill = if on_state {
                mix(p.surface_hi, p.accent, 0.35)
            } else {
                p.surface_hi
            };
            self.cx.framed(b, 4.0, fill, Some(stroke));
            if on_state {
                let c = p.foreground.rgb();
                let (x, y) = (b.x, b.y);
                self.cx.push(Cmd::Line {
                    a: (x + 4.5, y + 9.5),
                    b: (x + 7.5, y + 12.5),
                    width: 2.0,
                    color: c,
                });
                self.cx.push(Cmd::Line {
                    a: (x + 7.5, y + 12.5),
                    b: (x + 13.5, y + 5.5),
                    width: 2.0,
                    color: c,
                });
            }
        }
        if !text.is_empty() {
            self.cx.text_at(
                r.x + bx + 8.0,
                r.y + (h - lh) / 2.0,
                size,
                Face::Sans,
                p.foreground,
                text,
            );
        }
        resp
    }

    pub fn checkbox(&mut self, value: &mut bool, text: &str) -> Resp {
        let mut resp = self.toggle_row(text, false, *value);
        if resp.clicked {
            *value = !*value;
            resp.changed = true;
        }
        resp
    }

    pub fn radio(&mut self, selected: bool, text: &str) -> Resp {
        self.toggle_row(text, true, selected)
    }

    pub fn radio_value<T: PartialEq + Copy>(&mut self, value: &mut T, v: T, text: &str) -> Resp {
        let mut resp = self.radio(*value == v, text);
        if resp.clicked && *value != v {
            *value = v;
            resp.changed = true;
        }
        resp
    }

    /// A slider over `min..=max` in `step`s, the value and a trailing text.
    #[allow(clippy::too_many_arguments)]
    pub fn slider(
        &mut self,
        salt: &str,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        decimals: usize,
        suffix: &str,
        text: &str,
    ) -> Option<f64> {
        let p = self.cx.p;
        let (w, h) = (220.0, INTERACT_H);
        self.item(h);
        let r = self.alloc(w, h);
        let id = self.cx.id(&format!("slider:{salt}"));
        let resp = self
            .cx
            .interact(id, r, Cursor::Hand, &format!("slider:{salt}"));
        self.cx.slider_hovered = resp.hovered;
        let thumb_r = 8.0;
        let track = Rect::new(r.x + thumb_r, r.y + h / 2.0 - 3.0, w - 2.0 * thumb_r, 6.0);
        let snap = |v: f64| {
            let v = ((v - min) / step).round() * step + min;
            v.clamp(min, max)
        };
        let mut out = None;
        if resp.down || resp.pressed {
            if let Some((x, _)) = self.cx.input.pos {
                let t = ((x - track.x) / track.w).clamp(0.0, 1.0) as f64;
                let v = snap(min + t * (max - min));
                if (v - value).abs() > f64::EPSILON {
                    out = Some(v);
                }
            }
        }
        if resp.hovered && self.cx.input.wheel != 0.0 {
            let dir = if self.cx.input.wheel > 0.0 { -1.0 } else { 1.0 };
            let v = snap(value + dir * step);
            if (v - value).abs() > f64::EPSILON {
                out = Some(v);
            }
            self.cx.input.wheel = 0.0;
        }
        if out.is_some() {
            self.cx.changed = true;
        }
        let shown = out.unwrap_or(value);
        let t = if max > min {
            ((shown - min) / (max - min)) as f32
        } else {
            0.0
        };
        self.cx.rect(track, 3.0, p.inset);
        self.cx.rect(
            Rect::new(track.x, track.y, track.w * t, track.h),
            3.0,
            mix(p.surface, p.accent, 0.55),
        );
        let tx = track.x + track.w * t;
        let tc = if resp.hovered || resp.down {
            p.foreground
        } else {
            mix(p.foreground, p.surface_hi, 0.2)
        };
        self.cx.framed(
            Rect::new(
                tx - thumb_r,
                r.y + h / 2.0 - thumb_r,
                2.0 * thumb_r,
                2.0 * thumb_r,
            ),
            thumb_r,
            tc,
            Some(p.border_hi),
        );
        let v = format!("{shown:.decimals$}{suffix}");
        self.value_box(&v);
        if !text.is_empty() {
            self.label(text);
        }
        out
    }

    fn value_box(&mut self, v: &str) {
        let p = self.cx.p;
        let size = self.cx.body;
        let lh = self.cx.line_h(Face::Sans, size);
        let tw = self.cx.tw(Face::Sans, size, v);
        let (w, h) = ((tw + 16.0).ceil(), INTERACT_H - 2.0);
        self.item(h);
        let r = self.alloc(w, h);
        self.cx.framed(r, 6.0, p.surface_hi, Some(p.border));
        self.cx.text_at(
            r.x + 8.0,
            r.y + (h - lh) / 2.0,
            size,
            Face::Sans,
            p.foreground,
            v,
        );
    }

    /// A number you drag sideways, scroll, or click and type.
    pub fn drag_value(
        &mut self,
        salt: &str,
        value: &mut u32,
        min: u32,
        max: u32,
        speed: f32,
        suffix: &str,
    ) -> Resp {
        let p = self.cx.p;
        let id = self.cx.id(&format!("drag:{salt}"));
        let size = self.cx.body;
        let lh = self.cx.line_h(Face::Sans, size);
        let editing = self.cx.focus == Some(id);
        let shown = if editing {
            self.cx.num_buf.clone()
        } else {
            format!("{value}{suffix}")
        };
        let tw = self.cx.tw(Face::Sans, size, &shown).max(24.0);
        let (w, h) = ((tw + 16.0).ceil(), INTERACT_H - 2.0);
        self.item(h);
        let r = self.alloc(w, h);
        let mut resp = self
            .cx
            .interact(id, r, Cursor::Hand, &format!("drag:{salt}"));
        if resp.pressed {
            self.cx.drag_value = *value as f64;
        }
        if resp.down {
            if let (Some((x, _)), Some((px, _))) = (self.cx.input.pos, self.cx.press_pos) {
                let v = (self.cx.drag_value + ((x - px) * speed) as f64)
                    .round()
                    .clamp(min as f64, max as f64) as u32;
                if v != *value && (x - px).abs() > 2.0 {
                    *value = v;
                    resp.changed = true;
                }
            }
        }
        if resp.hovered && self.cx.input.wheel != 0.0 && !editing {
            let step = (speed.round() as i64).max(1);
            let dir = if self.cx.input.wheel > 0.0 { -1 } else { 1 };
            let v = (*value as i64 + dir * step).clamp(min as i64, max as i64) as u32;
            if v != *value {
                *value = v;
                resp.changed = true;
            }
            self.cx.input.wheel = 0.0;
        }
        let moved = match (self.cx.input.pos, self.cx.press_pos) {
            (Some((x, _)), Some((px, _))) => (x - px).abs() > 2.0,
            _ => false,
        };
        if resp.clicked && !moved && !editing {
            self.cx.focus = Some(id);
            self.cx.num_buf = value.to_string();
        }
        if editing {
            let mut commit = false;
            let keys = std::mem::take(&mut self.cx.input.keys);
            for k in &keys {
                match k.sym {
                    0xff0d | 0xff8d | 0xff09 => commit = true,
                    0xff1b => {
                        self.cx.focus = None;
                    }
                    0xff08 => {
                        self.cx.num_buf.pop();
                    }
                    _ => {
                        if let Some(c) = k.text.filter(|c| c.is_ascii_digit()) {
                            if self.cx.num_buf.len() < 9 {
                                self.cx.num_buf.push(c);
                            }
                        }
                    }
                }
            }
            if self.cx.input.pressed && !resp.pressed {
                commit = true;
            }
            if commit {
                if let Ok(v) = self.cx.num_buf.parse::<u32>() {
                    let v = v.clamp(min, max);
                    if v != *value {
                        *value = v;
                        resp.changed = true;
                    }
                }
                self.cx.focus = None;
                self.cx.changed = true;
            }
        }
        if resp.changed {
            self.cx.changed = true;
        }
        let editing = self.cx.focus == Some(id);
        let stroke = if editing || resp.hovered {
            p.focus
        } else {
            p.border
        };
        self.cx.framed(
            r,
            6.0,
            if editing { p.inset } else { p.surface_hi },
            Some(stroke),
        );
        let shown = if editing {
            self.cx.num_buf.clone()
        } else {
            format!("{value}{suffix}")
        };
        self.cx.text_at(
            r.x + 8.0,
            r.y + (h - lh) / 2.0,
            size,
            Face::Sans,
            p.foreground,
            &shown,
        );
        if editing {
            let cw = self.cx.tw(Face::Sans, size, &shown);
            self.cx.rect(
                Rect::new(r.x + 8.0 + cw + 1.0, r.y + 4.0, 1.5, h - 8.0),
                0.0,
                p.foreground,
            );
        }
        resp
    }

    /// A drop-down list. Returns the index picked in this frame.
    pub fn combo(
        &mut self,
        salt: &str,
        selected: &str,
        width: Option<f32>,
        items: &[String],
    ) -> Option<usize> {
        let p = self.cx.p;
        let size = self.cx.body;
        let lh = self.cx.line_h(Face::Sans, size);
        let id = self.cx.id(&format!("combo:{salt}"));
        let natural = self.cx.tw(Face::Sans, size, selected) + 44.0;
        let w = width.unwrap_or(natural.max(140.0)).ceil();
        let h = INTERACT_H;
        self.item(h);
        let r = self.alloc(w, h);
        let resp = self
            .cx
            .interact(id, r, Cursor::Arrow, &format!("combo:{salt}"));
        let open = self.cx.popup == Some(id);
        if open {
            self.cx.popup_owner = Some(r);
        }
        if resp.clicked {
            self.cx.popup = if open { None } else { Some(id) };
            self.cx.popup_owner = Some(r);
            self.cx.popup_scroll = 0.0;
        }
        let stroke = if resp.hovered || open {
            p.focus
        } else {
            p.border
        };
        self.cx.framed(r, 7.0, p.surface_hi, Some(stroke));
        let text = {
            let s = self.cx.scale;
            self.cx
                .text
                .ellipsize(Face::Sans, size * s, selected, (w - 36.0) * s)
        };
        self.cx.text_at(
            r.x + 10.0,
            r.y + (h - lh) / 2.0,
            size,
            Face::Sans,
            p.foreground,
            &text,
        );
        // The arrow.
        let (ax, ay) = (r.right() - 16.0, r.y + h / 2.0);
        let c = p.muted.rgb();
        self.cx.push(Cmd::Line {
            a: (ax - 4.0, ay - 2.0),
            b: (ax, ay + 2.0),
            width: 1.6,
            color: c,
        });
        self.cx.push(Cmd::Line {
            a: (ax, ay + 2.0),
            b: (ax + 4.0, ay - 2.0),
            width: 1.6,
            color: c,
        });
        if self.cx.popup != Some(id) {
            return None;
        }
        // The list, on top of everything.
        let row = (lh + 8.0).ceil();
        let pw = items
            .iter()
            .map(|i| self.cx.tw(Face::Sans, size, i) + 24.0)
            .fold(w, f32::max)
            .min(self.cx.size.0 - 8.0);
        let full = row * items.len() as f32 + 8.0;
        let below = self.cx.size.1 - r.bottom() - 6.0;
        let above = r.y - 6.0;
        let (py, ph) = if full <= below || below >= above {
            (r.bottom() + 2.0, full.min(below))
        } else {
            let ph = full.min(above);
            (r.y - 2.0 - ph, ph)
        };
        let px = r.x.min(self.cx.size.0 - pw - 4.0).max(4.0);
        let pr = Rect::new(px, py, pw, ph);
        self.cx.popup_rect = Some(pr);
        let max_scroll = (full - ph).max(0.0);
        let inside = self.cx.input.pos.is_some_and(|(x, y)| pr.contains(x, y));
        if inside && self.cx.input.wheel != 0.0 {
            self.cx.popup_scroll =
                (self.cx.popup_scroll + self.cx.input.wheel).clamp(0.0, max_scroll);
            self.cx.input.wheel = 0.0;
        }
        let mut picked = None;
        let mut over = Vec::new();
        let shadow = mix(p.background, Color32::BLACK, 0.5);
        over.push(Cmd::Rect {
            r: Rect::new(pr.x + 2.0, pr.y + 3.0, pr.w, pr.h),
            radius: 8.0,
            color: shadow.rgb(),
            alpha: 0.35,
        });
        over.push(Cmd::Rect {
            r: pr,
            radius: 8.0,
            color: p.border_hi.rgb(),
            alpha: 1.0,
        });
        over.push(Cmd::Rect {
            r: pr.shrink(1.0, 1.0),
            radius: 7.0,
            color: p.panel.rgb(),
            alpha: 1.0,
        });
        over.push(Cmd::Clip(Some(pr.shrink(1.0, 1.0))));
        for (i, item) in items.iter().enumerate() {
            let ir = Rect::new(
                pr.x + 4.0,
                pr.y + 4.0 + i as f32 * row - self.cx.popup_scroll,
                pr.w - 8.0,
                row,
            );
            let hot = inside && self.cx.input.pos.is_some_and(|(x, y)| ir.contains(x, y));
            let is_sel = item == selected;
            if hot || is_sel {
                over.push(Cmd::Rect {
                    r: ir,
                    radius: 6.0,
                    color: if hot {
                        mix(p.panel, p.foreground, 0.10).rgb()
                    } else {
                        p.selected.rgb()
                    },
                    alpha: 1.0,
                });
            }
            over.push(Cmd::Text {
                x: ir.x + 8.0,
                y: ir.y + (row - lh) / 2.0,
                size,
                face: Face::Sans,
                color: p.foreground.rgb(),
                text: item.clone(),
            });
            if hot && self.cx.input.released {
                picked = Some(i);
            }
            if let Some(d) = self.cx.debug.as_mut() {
                d.push((format!("item:{item}"), ir));
            }
        }
        over.push(Cmd::Clip(self.cx.clip));
        self.cx.over.extend(over);
        if inside {
            self.cx.cursor = Cursor::Arrow;
        }
        if picked.is_some() {
            self.cx.popup = None;
            self.cx.changed = true;
        }
        picked
    }

    // ── text entry ─────────────────────────────────────────────────────

    /// A text field; `rows` > 0 makes it multi-line with at least that many
    /// rows. `width` None fills the line.
    #[allow(clippy::too_many_arguments)]
    pub fn text_edit(
        &mut self,
        salt: &str,
        value: &mut String,
        hint: &str,
        width: Option<f32>,
        rows: usize,
        mono: bool,
    ) -> Resp {
        let p = self.cx.p;
        let face = if mono { Face::Mono } else { Face::Sans };
        let size = self.cx.body;
        let lh = self.cx.line_h(face, size);
        let id = self.cx.id(&format!("edit:{salt}"));
        let multiline = rows > 0;
        let lines_n = value.split('\n').count().max(rows.max(1));
        let w = width.unwrap_or(self.available_width()).ceil();
        let h = if multiline {
            (lines_n as f32 * lh + 12.0).ceil()
        } else {
            INTERACT_H.max(lh + 8.0)
        };
        self.item(h);
        let r = self.alloc(w, h);
        let mut resp = self
            .cx
            .interact(id, r, Cursor::Text, &format!("edit:{salt}"));
        let pad = (8.0, if multiline { 6.0 } else { (h - lh) / 2.0 });
        let was_focused = self.cx.focus == Some(id);
        let chars: Vec<char> = value.chars().collect();
        // Map a pointer position to a character index.
        let hscroll = if was_focused {
            self.cx.edit.hscroll
        } else {
            0.0
        };
        let index_at = |cx: &mut Ctx, x: f32, y: f32| -> usize {
            let line = if multiline {
                (((y - r.y - pad.1) / lh).floor().max(0.0)) as usize
            } else {
                0
            };
            let mut start = 0usize;
            let mut ln = 0usize;
            for (i, c) in chars.iter().enumerate() {
                if ln == line {
                    break;
                }
                if *c == '\n' {
                    ln += 1;
                    start = i + 1;
                }
            }
            if ln < line {
                return chars.len();
            }
            let end = chars[start..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(chars.len(), |e| start + e);
            let s: String = chars[start..end].iter().collect();
            let offs = cx.text.offsets(face, size * cx.scale, &s);
            let rel = (x - r.x - pad.0 + hscroll) * cx.scale;
            let mut best = 0;
            for (i, o) in offs.iter().enumerate() {
                if (*o - rel).abs() < (offs[best] - rel).abs() {
                    best = i;
                }
            }
            start + best
        };
        if resp.pressed {
            let (x, y) = self.cx.input.pos.unwrap_or((r.x, r.y));
            let i = index_at(self.cx, x, y);
            if !was_focused {
                self.cx.edit = Edit {
                    id,
                    ..Edit::default()
                };
            }
            self.cx.focus = Some(id);
            self.cx.edit.cursor = i;
            if !self.cx.input.shift_click {
                self.cx.edit.anchor = i;
            }
            self.cx.edit.want_x = None;
            self.cx.changed = true;
        } else if resp.down && was_focused {
            if let Some((x, y)) = self.cx.input.pos {
                self.cx.edit.cursor = index_at(self.cx, x, y);
            }
        } else if was_focused && self.cx.input.pressed {
            // A press anywhere else ends the editing.
            self.cx.focus = None;
            resp.lost_focus = true;
        }
        let focused = self.cx.focus == Some(id);
        if focused {
            if self.cx.edit.id != id {
                self.cx.edit = Edit {
                    id,
                    cursor: chars.len(),
                    anchor: chars.len(),
                    ..Edit::default()
                };
            }
            let (changed, lost) = self.edit_keys(value, multiline, face, size);
            resp.changed |= changed;
            if lost {
                self.cx.focus = None;
                resp.lost_focus = true;
            }
        }
        if resp.changed || resp.lost_focus {
            self.cx.changed = true;
        }
        let focused = self.cx.focus == Some(id);
        // Draw.
        let stroke = if focused {
            p.focus
        } else if resp.hovered {
            mix(p.border, p.focus, 0.6)
        } else {
            p.border
        };
        self.cx.framed(r, 6.0, p.inset, Some(stroke));
        let inner = r.shrink(4.0, 2.0);
        let old_clip = self.cx.clip;
        let clip = old_clip.map_or(inner, |c| c.intersect(&inner));
        self.cx.push(Cmd::Clip(Some(clip)));
        let chars: Vec<char> = value.chars().collect();
        let len = chars.len();
        if focused {
            self.cx.edit.cursor = self.cx.edit.cursor.min(len);
            self.cx.edit.anchor = self.cx.edit.anchor.min(len);
        }
        if value.is_empty() && !focused {
            self.cx
                .text_at(r.x + pad.0, r.y + pad.1, size, face, p.disabled, hint);
        }
        let (cur, anc) = (self.cx.edit.cursor, self.cx.edit.anchor);
        let (sa, sb) = (cur.min(anc), cur.max(anc));
        // Horizontal scroll keeps the cursor visible on one line.
        let mut hs = if focused { self.cx.edit.hscroll } else { 0.0 };
        if focused && !multiline {
            let cx_ = self
                .cx
                .tw(face, size, &chars[..cur].iter().collect::<String>());
            let room = w - 2.0 * pad.0;
            if cx_ - hs > room {
                hs = cx_ - room;
            }
            if cx_ - hs < 0.0 {
                hs = cx_;
            }
            self.cx.edit.hscroll = hs;
        }
        let mut start = 0usize;
        for (ln, line) in value.split('\n').enumerate() {
            let n = line.chars().count();
            let y = r.y + pad.1 + ln as f32 * lh;
            let x0 = r.x + pad.0 - hs;
            if focused && sa != sb {
                let a = sa.max(start).min(start + n);
                let b = sb.max(start).min(start + n);
                if b > a || (sb > start + n && sa <= start + n) {
                    let pre: String = chars[start..a].iter().collect();
                    let mid: String = chars[a..b].iter().collect();
                    let xa = self.cx.tw(face, size, &pre);
                    let extra = if sb > start + n { 4.0 } else { 0.0 };
                    let xb = xa + self.cx.tw(face, size, &mid) + extra;
                    self.cx.rect(
                        Rect::new(x0 + xa, y, xb - xa, lh),
                        2.0,
                        mix(p.surface, p.accent, 0.45),
                    );
                }
            }
            if !line.is_empty() {
                self.cx.text_at(x0, y, size, face, p.foreground, line);
            }
            if focused && cur >= start && cur <= start + n {
                let pre: String = chars[start..cur].iter().collect();
                let cxp = self.cx.tw(face, size, &pre);
                self.cx.rect(
                    Rect::new(x0 + cxp, y + 1.0, 1.5, lh - 2.0),
                    0.0,
                    p.foreground,
                );
            }
            start += n + 1;
        }
        self.cx.push(Cmd::Clip(old_clip));
        resp
    }

    /// Keys for the focused field. Returns (changed, focus lost).
    fn edit_keys(
        &mut self,
        value: &mut String,
        multiline: bool,
        face: Face,
        size: f32,
    ) -> (bool, bool) {
        let mut changed = false;
        let mut lost = false;
        let keys = std::mem::take(&mut self.cx.input.keys);
        let mut chars: Vec<char> = value.chars().collect();
        let mut e = self.cx.edit.clone();
        e.cursor = e.cursor.min(chars.len());
        e.anchor = e.anchor.min(chars.len());
        let line_start =
            |c: &[char], i: usize| c[..i].iter().rposition(|x| *x == '\n').map_or(0, |p| p + 1);
        let line_end = |c: &[char], i: usize| {
            c[i..]
                .iter()
                .position(|x| *x == '\n')
                .map_or(c.len(), |p| i + p)
        };
        let mut paste = self.cx.input.paste.take();
        for k in keys {
            let sel = (e.cursor.min(e.anchor), e.cursor.max(e.anchor));
            let has_sel = sel.0 != sel.1;
            let delete_sel = |chars: &mut Vec<char>, e: &mut Edit| {
                chars.drain(sel.0..sel.1);
                e.cursor = sel.0;
                e.anchor = sel.0;
            };
            let moving = |e: &mut Edit, to: usize, shift: bool| {
                e.cursor = to;
                if !shift {
                    e.anchor = to;
                }
            };
            if k.ctrl {
                match k.base {
                    0x61 => {
                        e.anchor = 0;
                        e.cursor = chars.len();
                    }
                    0x63 | 0x78 if has_sel => {
                        self.cx.copy = Some(chars[sel.0..sel.1].iter().collect());
                        if k.base == 0x78 {
                            delete_sel(&mut chars, &mut e);
                            changed = true;
                        }
                    }
                    0x76 => self.cx.want_paste = true,
                    0xff08 => {
                        // Delete the word before the cursor.
                        if has_sel {
                            delete_sel(&mut chars, &mut e);
                        } else {
                            let mut i = e.cursor;
                            while i > 0 && chars[i - 1] == ' ' {
                                i -= 1;
                            }
                            while i > 0 && chars[i - 1] != ' ' && chars[i - 1] != '\n' {
                                i -= 1;
                            }
                            chars.drain(i..e.cursor);
                            e.cursor = i;
                            e.anchor = i;
                        }
                        changed = true;
                    }
                    0xff51 | 0xff53 => {
                        let left = k.base == 0xff51;
                        let mut i = e.cursor;
                        if left {
                            while i > 0 && !chars[i - 1].is_alphanumeric() {
                                i -= 1;
                            }
                            while i > 0 && chars[i - 1].is_alphanumeric() {
                                i -= 1;
                            }
                        } else {
                            while i < chars.len() && !chars[i].is_alphanumeric() {
                                i += 1;
                            }
                            while i < chars.len() && chars[i].is_alphanumeric() {
                                i += 1;
                            }
                        }
                        moving(&mut e, i, k.shift);
                    }
                    0xff50 => moving(&mut e, 0, k.shift),
                    0xff57 => moving(&mut e, chars.len(), k.shift),
                    _ => {}
                }
                continue;
            }
            match k.sym {
                0xff1b => lost = true,
                0xff09 | 0xfe20 => lost = true,
                0xff0d | 0xff8d => {
                    if multiline {
                        if has_sel {
                            delete_sel(&mut chars, &mut e);
                        }
                        chars.insert(e.cursor, '\n');
                        e.cursor += 1;
                        e.anchor = e.cursor;
                        changed = true;
                    } else {
                        lost = true;
                    }
                }
                0xff08 => {
                    if has_sel {
                        delete_sel(&mut chars, &mut e);
                        changed = true;
                    } else if e.cursor > 0 {
                        chars.remove(e.cursor - 1);
                        e.cursor -= 1;
                        e.anchor = e.cursor;
                        changed = true;
                    }
                }
                0xffff | 0xff9f => {
                    if has_sel {
                        delete_sel(&mut chars, &mut e);
                        changed = true;
                    } else if e.cursor < chars.len() {
                        chars.remove(e.cursor);
                        changed = true;
                    }
                }
                0xff51 | 0xff96 => {
                    let to = if has_sel && !k.shift {
                        sel.0
                    } else {
                        e.cursor.saturating_sub(1)
                    };
                    moving(&mut e, to, k.shift);
                    e.want_x = None;
                }
                0xff53 | 0xff98 => {
                    let to = if has_sel && !k.shift {
                        sel.1
                    } else {
                        (e.cursor + 1).min(chars.len())
                    };
                    moving(&mut e, to, k.shift);
                    e.want_x = None;
                }
                0xff50 | 0xff95 => {
                    let to = line_start(&chars, e.cursor);
                    moving(&mut e, to, k.shift);
                }
                0xff57 | 0xff9c => {
                    let to = line_end(&chars, e.cursor);
                    moving(&mut e, to, k.shift);
                }
                0xff52 | 0xff54 | 0xff97 | 0xff99 if multiline => {
                    let up = k.sym == 0xff52 || k.sym == 0xff97;
                    let ls = line_start(&chars, e.cursor);
                    let pre: String = chars[ls..e.cursor].iter().collect();
                    let x = e.want_x.unwrap_or_else(|| self.cx.tw(face, size, &pre));
                    let target_start = if up {
                        if ls == 0 {
                            None
                        } else {
                            Some(line_start(&chars, ls - 1))
                        }
                    } else {
                        let le = line_end(&chars, e.cursor);
                        (le < chars.len()).then_some(le + 1)
                    };
                    let to = match target_start {
                        None => {
                            if up {
                                0
                            } else {
                                chars.len()
                            }
                        }
                        Some(ts) => {
                            let te = line_end(&chars, ts);
                            let s: String = chars[ts..te].iter().collect();
                            let sc = self.cx.scale;
                            let offs = self.cx.text.offsets(face, size * sc, &s);
                            let mut best = 0;
                            for (i, o) in offs.iter().enumerate() {
                                if (o / sc - x).abs() < (offs[best] / sc - x).abs() {
                                    best = i;
                                }
                            }
                            ts + best
                        }
                    };
                    moving(&mut e, to, k.shift);
                    e.want_x = Some(x);
                }
                _ => {
                    if let Some(c) = k.text {
                        if c == '\n' && !multiline {
                            continue;
                        }
                        if has_sel {
                            delete_sel(&mut chars, &mut e);
                        }
                        chars.insert(e.cursor, c);
                        e.cursor += 1;
                        e.anchor = e.cursor;
                        e.want_x = None;
                        changed = true;
                    }
                }
            }
        }
        if let Some(text) = paste.take() {
            let sel = (e.cursor.min(e.anchor), e.cursor.max(e.anchor));
            chars.drain(sel.0..sel.1);
            e.cursor = sel.0;
            let ins: Vec<char> = if multiline {
                text.replace("\r\n", "\n").chars().collect()
            } else {
                text.chars()
                    .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                    .collect()
            };
            let n = ins.len();
            chars.splice(e.cursor..e.cursor, ins);
            e.cursor += n;
            e.anchor = e.cursor;
            changed = true;
        }
        if changed {
            *value = chars.into_iter().collect();
        }
        self.cx.edit = e;
        (changed, lost)
    }
}

pub fn red(p: &Palette) -> Color32 {
    if p.light {
        Color32::from_rgb(196, 43, 43)
    } else {
        Color32::from_rgb(235, 87, 87)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        let p = crate::theme::resolve(&crate::theme::Preferences {
            theme: "classic".into(),
            ..Default::default()
        })
        .unwrap()
        .palette;
        Ctx::new(Text::new(None).expect("a system font"), 1.0, p, 15.0)
    }

    fn click_at(cx: &mut Ctx, pos: (f32, f32), f: &mut dyn FnMut(&mut Ui)) {
        for (pressed, released, down) in [(true, false, true), (false, true, false)] {
            cx.begin(
                Input {
                    pos: Some(pos),
                    pressed,
                    released,
                    down,
                    ..Input::default()
                },
                (400.0, 300.0),
            );
            let mut ui = Ui::new(cx, Rect::new(0.0, 0.0, 400.0, 300.0));
            f(&mut ui);
            cx.end();
        }
    }

    #[test]
    fn a_click_on_a_button_is_reported_once() {
        let mut cx = ctx();
        let mut clicks = 0;
        click_at(&mut cx, (10.0, 10.0), &mut |ui| {
            if ui.button("Press").clicked {
                clicks += 1;
            }
        });
        assert_eq!(clicks, 1);
        click_at(&mut cx, (300.0, 200.0), &mut |ui| {
            if ui.button("Press").clicked {
                clicks += 1;
            }
        });
        assert_eq!(clicks, 1);
    }

    #[test]
    fn checkboxes_toggle_and_text_fields_take_keys() {
        let mut cx = ctx();
        let mut on = false;
        click_at(&mut cx, (5.0, 8.0), &mut |ui| {
            ui.checkbox(&mut on, "Keep");
        });
        assert!(on);
        let mut text = String::from("ab");
        click_at(&mut cx, (380.0, 10.0), &mut |ui| {
            ui.text_edit("t", &mut text, "", None, 0, false);
        });
        cx.begin(
            Input {
                keys: vec![
                    Key {
                        sym: 0x63,
                        base: 0x63,
                        text: Some('c'),
                        ..Key::default()
                    },
                    Key {
                        sym: 0xe5,
                        base: 0xe5,
                        text: Some('å'),
                        ..Key::default()
                    },
                    Key {
                        sym: 0xff51,
                        base: 0xff51,
                        ..Key::default()
                    },
                    Key {
                        sym: 0xff08,
                        base: 0xff08,
                        ..Key::default()
                    },
                ],
                ..Input::default()
            },
            (400.0, 300.0),
        );
        let changed = {
            let mut ui = Ui::new(&mut cx, Rect::new(0.0, 0.0, 400.0, 300.0));
            ui.text_edit("t", &mut text, "", None, 0, false).changed
        };
        cx.end();
        assert!(changed);
        assert_eq!(text, "abå");
    }

    #[test]
    fn rows_wrap_and_cards_grow_with_their_content() {
        let mut cx = ctx();
        cx.begin(Input::default(), (200.0, 300.0));
        let h = {
            let mut ui = Ui::new(&mut cx, Rect::new(0.0, 0.0, 200.0, 300.0));
            let p = ui.p();
            ui.frame(p.surface, Some(p.border), 12.0, (18.0, 14.0), |ui| {
                ui.horizontal_wrapped(|ui| {
                    for i in 0..8 {
                        ui.button(format!("Button {i}"));
                    }
                });
            });
            ui.used_height()
        };
        assert!(h > 4.0 * INTERACT_H, "{h}");
    }

    #[test]
    fn a_combo_opens_and_picks() {
        let mut cx = ctx();
        let items: Vec<String> = ["One", "Two", "Three"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut picked = None;
        click_at(&mut cx, (20.0, 10.0), &mut |ui| {
            ui.combo("c", "One", Some(150.0), &items);
        });
        // The list is below the button: rows of about 26 px from y 30.
        click_at(&mut cx, (20.0, 30.0 + 26.0 + 12.0), &mut |ui| {
            if let Some(i) = ui.combo("c", "One", Some(150.0), &items) {
                picked = Some(i);
            }
        });
        assert_eq!(picked, Some(1));
    }
}
