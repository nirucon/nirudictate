//! The recording overlay: a small pill at the bottom centre of the screen with
//! a pulsing dot, a level meter and the elapsed time while recording, and a
//! quiet animation while transcribing. It exists only while it is visible.
//!
//! It is drawn in software into a 32-bit window (with a compositor the corners
//! are smooth and slightly transparent; without one the window is shaped), so
//! the service needs no GPU context and stays a few megabytes in size. Clicks
//! pass through it, and it never takes focus.
use crate::theme::Palette;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use egui::Color32;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ColormapAlloc, ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux,
    EventMask, ImageFormat, PropMode, Rectangle, StackMode, VisualClass, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Recording {
        seconds: f32,
        levels: Vec<f32>,
        language: String,
        holding: bool,
        /// Agent mode: `AI` (and `NEW` for a new conversation) by the language.
        agent: bool,
        fresh: bool,
        /// Command mode: `CMD` by the language.
        command: bool,
    },
    Transcribing {
        phase: f32,
    },
    /// A box that waits for Enter or Esc: the agent's transcript (sent by
    /// itself when `left` reaches 0) or a dangerous voice command.
    Confirm {
        title: String,
        text: String,
        hint: String,
        /// Share of the waiting time left, 1 → 0; None draws no countdown.
        left: Option<f32>,
        /// A warning (red dot): the command closes or powers something off.
        warning: bool,
    },
    /// Agent mode: handing the message to opencode.
    Sending {
        phase: f32,
    },
    Message {
        text: String,
        error: bool,
    },
}

/// Break `text` into at most `max` lines of `width` pixels; the last line
/// ends with an ellipsis when text is left over.
pub fn wrap(font: &FontArc, size: f32, text: &str, width: f32, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut words = text.split_whitespace().peekable();
    let mut cut = false;
    while let Some(word) = words.next() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if text_width(font, size, &candidate) <= width || line.is_empty() {
            line = candidate;
            continue;
        }
        lines.push(std::mem::take(&mut line));
        if lines.len() == max {
            cut = true;
            break;
        }
        line = word.to_string();
        if words.peek().is_none() {
            break;
        }
    }
    if !cut && !line.is_empty() {
        if lines.len() == max {
            cut = true;
        } else {
            lines.push(line);
        }
    }
    // A single word longer than the line, or text left over: shorten.
    for (i, l) in lines.iter_mut().enumerate() {
        let last = i + 1 == max;
        if text_width(font, size, l) > width || (last && cut) {
            let mut t: String = l.clone();
            while !t.is_empty() && text_width(font, size, &format!("{t}…")) > width {
                t.pop();
            }
            *l = format!("{}…", t.trim_end());
        }
    }
    lines
}

struct Colors {
    bg: [f32; 4],
    border: [f32; 4],
    fg: [f32; 4],
    muted: [f32; 4],
    accent: [f32; 4],
    red: [f32; 4],
}

fn rgba(c: Color32, a: f32) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        a,
    ]
}

impl Colors {
    fn from(p: &Palette, translucent: bool) -> Self {
        let red = if p.light {
            Color32::from_rgb(196, 43, 43)
        } else {
            Color32::from_rgb(235, 87, 87)
        };
        Self {
            bg: rgba(p.panel, if translucent { 0.94 } else { 1.0 }),
            border: rgba(p.border_hi, 1.0),
            fg: rgba(p.foreground, 1.0),
            muted: rgba(p.muted, 1.0),
            accent: rgba(p.accent, 1.0),
            red: rgba(red, 1.0),
        }
    }
}

/// Premultiplied ARGB canvas.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
        }
    }

    fn blend(&mut self, x: usize, y: usize, c: [f32; 4], coverage: f32) {
        if x >= self.w || y >= self.h {
            return;
        }
        let a = (c[3] * coverage).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let d = &mut self.px[y * self.w + x];
        for i in 0..3 {
            d[i] = c[i] * a + d[i] * (1.0 - a);
        }
        d[3] = a + d[3] * (1.0 - a);
    }

    /// Anti-aliased rounded rectangle (signed distance per pixel).
    fn rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: [f32; 4]) {
        let (x0, y0) = (x.floor().max(0.0) as usize, y.floor().max(0.0) as usize);
        let (x1, y1) = (
            ((x + w).ceil() as usize).min(self.w),
            ((y + h).ceil() as usize).min(self.h),
        );
        let r = r.min(w / 2.0).min(h / 2.0);
        for py in y0..y1 {
            for px in x0..x1 {
                let cx = px as f32 + 0.5;
                let cy = py as f32 + 0.5;
                let qx = (cx - (x + w / 2.0)).abs() - (w / 2.0 - r);
                let qy = (cy - (y + h / 2.0)).abs() - (h / 2.0 - r);
                let outside =
                    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                let coverage = (0.5 - outside).clamp(0.0, 1.0);
                self.blend(px, py, c, coverage);
            }
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, c: [f32; 4]) {
        self.rounded_rect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, c);
    }

    fn text(
        &mut self,
        font: &FontArc,
        size: f32,
        x: f32,
        baseline: f32,
        text: &str,
        c: [f32; 4],
    ) -> f32 {
        let scaled = font.as_scaled(PxScale::from(size));
        let mut pen = x;
        let mut last = None;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(prev) = last {
                pen += scaled.kern(prev, id);
            }
            let glyph = id.with_scale_and_position(size, ab_glyph::point(pen, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    let px = b.min.x as i32 + gx as i32;
                    let py = b.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 {
                        self.blend(px as usize, py as usize, c, cov);
                    }
                });
            }
            pen += scaled.h_advance(id);
            last = Some(id);
        }
        pen - x
    }

    /// Bytes for PutImage: BGRA little-endian, premultiplied.
    fn bytes(&self, opaque: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = if opaque { 1.0 } else { p[3] };
            let (r, g, b) = if opaque && p[3] > 0.0 {
                (p[0] / p[3], p[1] / p[3], p[2] / p[3])
            } else {
                (p[0], p[1], p[2])
            };
            out.extend_from_slice(&[
                (b.clamp(0.0, 1.0) * 255.0) as u8,
                (g.clamp(0.0, 1.0) * 255.0) as u8,
                (r.clamp(0.0, 1.0) * 255.0) as u8,
                (a.clamp(0.0, 1.0) * 255.0) as u8,
            ]);
        }
        out
    }
}

pub fn text_width(font: &FontArc, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut w = 0.0;
    let mut last = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(prev) = last {
            w += scaled.kern(prev, id);
        }
        w += scaled.h_advance(id);
        last = Some(id);
    }
    w
}

pub fn format_time(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Draw one frame of the pill. Pure, so it is unit tested and screenshot-able.
pub fn render(
    view: &View,
    palette: &Palette,
    font: &FontArc,
    scale: f32,
    translucent: bool,
) -> Canvas {
    let colors = Colors::from(palette, translucent);
    let s = scale;
    if let View::Confirm {
        title,
        text,
        hint,
        left,
        warning,
    } = view
    {
        return render_confirm(title, text, hint, *left, *warning, &colors, font, s);
    }
    let h = (44.0 * s).round();
    let w = match view {
        View::Message { text, .. } => {
            (text_width(font, 15.0 * s, text) + 56.0 * s).clamp(160.0 * s, 560.0 * s)
        }
        _ => 268.0 * s,
    }
    .round();
    let mut c = Canvas::new(w as usize, h as usize);
    // Border then body.
    c.rounded_rect(0.0, 0.0, w, h, h / 2.0, colors.border);
    c.rounded_rect(
        1.0 * s,
        1.0 * s,
        w - 2.0 * s,
        h - 2.0 * s,
        h / 2.0 - s,
        colors.bg,
    );
    let mid = h / 2.0;
    match view {
        View::Recording {
            seconds,
            levels,
            language,
            holding,
            agent,
            fresh,
            command,
        } => {
            // Pulsing red dot.
            let pulse = 0.65 + 0.35 * (seconds * std::f32::consts::TAU * 0.8).cos().abs();
            let mut red = colors.red;
            red[3] = pulse;
            c.circle(22.0 * s, mid, 6.5 * s, red);
            // Level meter: newest on the right (shorter when the agent label
            // needs the room).
            let bars = if *agent || *command { 18usize } else { 26 };
            let left = 40.0 * s;
            let bar_w = 3.0 * s;
            let gap = 2.4 * s;
            let max_h = h - 18.0 * s;
            for i in 0..bars {
                let v = levels
                    .len()
                    .checked_sub(bars - i)
                    .and_then(|j| levels.get(j))
                    .copied()
                    .unwrap_or(0.0);
                let bh = (3.0 * s).max(max_h * v.clamp(0.0, 1.0));
                let x = left + i as f32 * (bar_w + gap);
                let mut col = colors.accent;
                col[3] = 0.55 + 0.45 * v.clamp(0.0, 1.0);
                c.rounded_rect(x, mid - bh / 2.0, bar_w, bh, bar_w / 2.0, col);
            }
            let time = format_time(*seconds);
            let size = 15.0 * s;
            let tw = text_width(font, size, &time);
            let lang = language.to_uppercase();
            let lsize = 11.0 * s;
            let mut lw = text_width(font, lsize, &lang);
            let right = w - 18.0 * s;
            c.text(font, lsize, right - lw, mid + 4.0 * s, &lang, colors.muted);
            if *agent || *command {
                let tag = match (*command, *fresh) {
                    (true, _) => "CMD",
                    (false, true) => "NEW AI",
                    (false, false) => "AI",
                };
                let tw = text_width(font, lsize, tag);
                c.text(
                    font,
                    lsize,
                    right - lw - 6.0 * s - tw,
                    mid + 4.0 * s,
                    tag,
                    colors.accent,
                );
                lw += 6.0 * s + tw;
            }
            c.text(
                font,
                size,
                right - lw - 8.0 * s - tw,
                mid + 5.5 * s,
                &time,
                colors.fg,
            );
            if *holding {
                // A thin underline marks push-to-talk.
                let mut col = colors.red;
                col[3] = 0.8;
                c.rounded_rect(16.0 * s, h - 6.0 * s, 12.0 * s, 2.0 * s, 1.0 * s, col);
            }
        }
        View::Transcribing { phase } | View::Sending { phase } => {
            for i in 0..3 {
                let t = (phase * 2.2 - i as f32 * 0.22).rem_euclid(1.0);
                let lift = (t * std::f32::consts::PI).sin().max(0.0);
                let mut col = colors.accent;
                col[3] = 0.45 + 0.55 * lift;
                c.circle(
                    20.0 * s + i as f32 * 11.0 * s,
                    mid - 3.0 * s * lift,
                    3.6 * s,
                    col,
                );
            }
            let label = if matches!(view, View::Sending { .. }) {
                "Sending to opencode…"
            } else {
                "Transcribing…"
            };
            c.text(font, 15.0 * s, 62.0 * s, mid + 5.5 * s, label, colors.fg);
        }
        View::Confirm { .. } => {}
        View::Message { text, error } => {
            let dot = if *error { colors.red } else { colors.accent };
            c.circle(22.0 * s, mid, 5.0 * s, dot);
            c.text(font, 15.0 * s, 38.0 * s, mid + 5.5 * s, text, colors.fg);
        }
    }
    c
}

/// The confirmation box: what it is about, the text (up to four lines), the
/// keys, and for the agent a bar that runs out when it sends by itself.
#[allow(clippy::too_many_arguments)]
fn render_confirm(
    title: &str,
    text: &str,
    hint: &str,
    left: Option<f32>,
    warning: bool,
    colors: &Colors,
    font: &FontArc,
    s: f32,
) -> Canvas {
    let w = (460.0 * s).round();
    let pad = 18.0 * s;
    let size = 15.0 * s;
    let line_h = 21.0 * s;
    let lines = wrap(font, size, text, w - 2.0 * pad, 4);
    let h =
        (pad + 16.0 * s + 8.0 * s + line_h * lines.len().max(1) as f32 + 12.0 * s + 14.0 * s + pad)
            .round();
    let mut c = Canvas::new(w as usize, h as usize);
    let r = 14.0 * s;
    c.rounded_rect(0.0, 0.0, w, h, r, colors.border);
    c.rounded_rect(1.0 * s, 1.0 * s, w - 2.0 * s, h - 2.0 * s, r - s, colors.bg);
    let small = 12.0 * s;
    let mut y = pad + 11.0 * s;
    let dot = if warning { colors.red } else { colors.accent };
    c.circle(pad + 4.0 * s, y - 4.0 * s, 4.0 * s, dot);
    c.text(font, small, pad + 14.0 * s, y, title, colors.muted);
    y += 8.0 * s;
    for line in &lines {
        y += line_h;
        c.text(font, size, pad, y - 5.0 * s, line, colors.fg);
    }
    y += 12.0 * s;
    c.text(font, small, pad, y + 6.0 * s, hint, colors.muted);
    // The countdown: a thin bar along the bottom edge.
    if let Some(left) = left {
        let bar_w = (w - 2.0 * pad) * left.clamp(0.0, 1.0);
        let mut track = colors.muted;
        track[3] = 0.25;
        c.rounded_rect(pad, h - 9.0 * s, w - 2.0 * pad, 3.0 * s, 1.5 * s, track);
        if bar_w > 0.5 {
            c.rounded_rect(pad, h - 9.0 * s, bar_w, 3.0 * s, 1.5 * s, colors.accent);
        }
    }
    c
}

pub fn font(appearance_font: Option<&std::path::Path>) -> FontArc {
    if let Some(path) = appearance_font {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(f) = FontArc::try_from_vec(bytes) {
                return f;
            }
        }
    }
    let defs = egui::FontDefinitions::default();
    let data = defs
        .font_data
        .get("Ubuntu-Light")
        .or_else(|| defs.font_data.values().next())
        .expect("egui ships default fonts");
    FontArc::try_from_vec(data.font.to_vec()).expect("valid default font")
}

/// The X11 window that shows the canvas.
pub struct Overlay {
    win: Window,
    gc: u32,
    depth: u8,
    translucent: bool,
    shown: bool,
    size: (u16, u16),
    pos: (i16, i16),
}

fn argb_visual(conn: &RustConnection, screen: usize) -> Option<u32> {
    conn.setup().roots[screen]
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.class == VisualClass::TRUE_COLOR)
        .map(|v| v.visual_id)
}

fn compositor_running(conn: &RustConnection, screen: usize) -> bool {
    let name = format!("_NET_WM_CM_S{screen}");
    conn.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| conn.get_selection_owner(r.atom).ok()?.reply().ok())
        .map(|r| r.owner != x11rb::NONE)
        .unwrap_or(false)
}

impl Overlay {
    pub fn new(conn: &RustConnection, screen: usize) -> Result<Self, String> {
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id().map_err(|e| e.to_string())?;
        let gc = conn.generate_id().map_err(|e| e.to_string())?;
        let translucent = compositor_running(conn, screen);
        let visual = argb_visual(conn, screen);
        let (depth, visual_id, aux) = match visual {
            Some(v) => {
                let cmap = conn.generate_id().map_err(|e| e.to_string())?;
                conn.create_colormap(ColormapAlloc::NONE, cmap, root, v)
                    .map_err(|e| e.to_string())?;
                (
                    32u8,
                    v,
                    CreateWindowAux::new()
                        .background_pixel(0)
                        .border_pixel(0)
                        .colormap(cmap),
                )
            }
            None => (
                conn.setup().roots[screen].root_depth,
                conn.setup().roots[screen].root_visual,
                CreateWindowAux::new().background_pixel(0),
            ),
        };
        let aux = aux.override_redirect(1).event_mask(EventMask::EXPOSURE);
        conn.create_window(
            depth,
            win,
            root,
            0,
            0,
            268,
            44,
            0,
            WindowClass::INPUT_OUTPUT,
            visual_id,
            &aux,
        )
        .map_err(|e| e.to_string())?;
        conn.create_gc(gc, win, &CreateGCAux::new())
            .map_err(|e| e.to_string())?;
        // Clicks pass through to the window below.
        let _ = conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            x11rb::protocol::xproto::ClipOrdering::UNSORTED,
            win,
            0,
            0,
            &[],
        );
        let _ = conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"NIRUDICTATE",
        );
        let _ = conn.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirudictate-overlay\0nirudictate\0",
        );
        if let Some(ty) = intern(conn, "_NET_WM_WINDOW_TYPE") {
            if let Some(notification) = intern(conn, "_NET_WM_WINDOW_TYPE_NOTIFICATION") {
                let _ = conn.change_property32(
                    PropMode::REPLACE,
                    win,
                    ty,
                    AtomEnum::ATOM,
                    &[notification],
                );
            }
        }
        Ok(Self {
            win,
            gc,
            depth,
            translucent: translucent && depth == 32,
            shown: false,
            size: (268, 44),
            pos: (0, 0),
        })
    }

    pub fn translucent(&self) -> bool {
        self.translucent
    }

    /// Bottom centre of the monitor the pointer is on.
    fn place(conn: &RustConnection, screen: usize, w: u16, h: u16) -> (i16, i16) {
        let root = conn.setup().roots[screen].root;
        let s = &conn.setup().roots[screen];
        let pointer = conn
            .query_pointer(root)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|p| (p.root_x, p.root_y))
            .unwrap_or((0, 0));
        let mut area = (0i16, 0i16, s.width_in_pixels, s.height_in_pixels);
        if let Ok(Ok(monitors)) = conn.randr_get_monitors(root, true).map(|c| c.reply()) {
            let mut best = None;
            for m in &monitors.monitors {
                let inside = pointer.0 >= m.x
                    && pointer.0 < m.x + m.width as i16
                    && pointer.1 >= m.y
                    && pointer.1 < m.y + m.height as i16;
                if inside {
                    best = Some((m.x, m.y, m.width, m.height));
                    break;
                }
                if m.primary && best.is_none() {
                    best = Some((m.x, m.y, m.width, m.height));
                }
            }
            if let Some(b) = best {
                area = b;
            }
        }
        let x = area.0 as i32 + (area.2 as i32 - w as i32) / 2;
        let y = area.1 as i32 + area.3 as i32 - h as i32 - (area.3 as i32 / 14).clamp(40, 120);
        (x as i16, y as i16)
    }

    pub fn show(&mut self, conn: &RustConnection, screen: usize, canvas: &Canvas) {
        let (w, h) = (canvas.w as u16, canvas.h as u16);
        if !self.shown || self.size != (w, h) {
            let pos = Self::place(conn, screen, w, h);
            let _ = conn.configure_window(
                self.win,
                &ConfigureWindowAux::new()
                    .x(pos.0 as i32)
                    .y(pos.1 as i32)
                    .width(w as u32)
                    .height(h as u32)
                    .stack_mode(StackMode::ABOVE),
            );
            self.size = (w, h);
            self.pos = pos;
            if !self.translucent {
                self.shape(conn, canvas);
            }
        }
        if !self.shown {
            let _ = conn.map_window(self.win);
            self.shown = true;
        }
        self.draw(conn, canvas);
        let _ = conn.flush();
    }

    fn shape(&self, conn: &RustConnection, canvas: &Canvas) {
        let mut rects = Vec::new();
        for y in 0..canvas.h {
            let row = &canvas.px[y * canvas.w..(y + 1) * canvas.w];
            let first = row.iter().position(|p| p[3] >= 0.5);
            let last = row.iter().rposition(|p| p[3] >= 0.5);
            if let (Some(a), Some(b)) = (first, last) {
                rects.push(Rectangle {
                    x: a as i16,
                    y: y as i16,
                    width: (b - a + 1) as u16,
                    height: 1,
                });
            }
        }
        let _ = conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::BOUNDING,
            x11rb::protocol::xproto::ClipOrdering::UNSORTED,
            self.win,
            0,
            0,
            &rects,
        );
    }

    fn draw(&self, conn: &RustConnection, canvas: &Canvas) {
        let data = canvas.bytes(!self.translucent);
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.win,
            self.gc,
            canvas.w as u16,
            canvas.h as u16,
            0,
            0,
            0,
            self.depth,
            &data,
        );
    }

    pub fn hide(&mut self, conn: &RustConnection) {
        if self.shown {
            let _ = conn.unmap_window(self.win);
            let _ = conn.flush();
            self.shown = false;
        }
    }
}

fn intern(conn: &RustConnection, name: &str) -> Option<u32> {
    conn.intern_atom(false, name.as_bytes())
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        crate::theme::resolve(&crate::theme::Preferences {
            theme: "classic".into(),
            ..Default::default()
        })
        .unwrap()
        .palette
    }

    #[test]
    fn renders_every_view_inside_its_pill() {
        let font = font(None);
        let p = palette();
        for view in [
            View::Recording {
                seconds: 7.4,
                levels: vec![0.2, 0.9, 0.5],
                language: "sv".into(),
                holding: true,
                agent: false,
                fresh: false,
                command: false,
            },
            View::Recording {
                seconds: 3.0,
                levels: vec![0.4],
                language: "sv".into(),
                holding: false,
                agent: true,
                fresh: true,
                command: false,
            },
            View::Recording {
                seconds: 1.0,
                levels: vec![0.4],
                language: "sv".into(),
                holding: false,
                agent: false,
                fresh: false,
                command: true,
            },
            View::Transcribing { phase: 0.3 },
            View::Sending { phase: 0.6 },
            View::Message {
                text: "No Swedish model installed".into(),
                error: true,
            },
        ] {
            let c = render(&view, &p, &font, 1.0, true);
            assert_eq!(c.h, 44);
            // Corners are transparent, the middle is painted.
            assert!(c.px[0][3] < 0.05);
            assert!(c.px[c.w * (c.h / 2) + c.w / 2][3] > 0.9);
            assert_eq!(c.bytes(false).len(), c.w * c.h * 4);
        }
    }

    #[test]
    fn the_confirm_box_wraps_and_counts_down() {
        let font = font(None);
        let p = palette();
        let short = render(
            &View::Confirm {
                title: "To opencode".into(),
                text: "Hej".into(),
                hint: "Enter sends now · Esc cancels".into(),
                left: Some(1.0),
                warning: false,
            },
            &p,
            &font,
            1.0,
            true,
        );
        let long_text = "Kan du titta på varför bygget misslyckas i nirubar och föreslå en ändring som gör testerna gröna igen utan att ändra beteendet för användaren, och sedan skriva en kort sammanfattning av vad du gjorde och varför det behövdes.".repeat(2);
        let long = render(
            &View::Confirm {
                title: "To opencode · new conversation".into(),
                text: long_text.clone(),
                hint: "Enter sends now · Esc cancels".into(),
                left: Some(0.5),
                warning: false,
            },
            &p,
            &font,
            1.0,
            true,
        );
        assert_eq!(short.w, 460);
        assert!(long.h > short.h, "more lines, taller box");
        assert!(long.h < 260, "at most four lines");
        let lines = wrap(&font, 15.0, &long_text, 424.0, 4);
        assert_eq!(lines.len(), 4);
        assert!(lines[3].ends_with('…'));
        assert!(lines.iter().all(|l| text_width(&font, 15.0, l) <= 424.0));
        assert_eq!(wrap(&font, 15.0, "ett två", 424.0, 4), ["ett två"]);
        assert!(wrap(&font, 15.0, "", 424.0, 4).is_empty());
        let one = wrap(&font, 15.0, &"x".repeat(400), 100.0, 4);
        assert_eq!(one.len(), 1);
        assert!(one[0].ends_with('…'));
    }

    #[test]
    fn time_is_minutes_and_seconds() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(65.9), "1:05");
    }
}
