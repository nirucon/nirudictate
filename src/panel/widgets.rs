//! Small drawing helpers that give the panel its look: cards, chips, a level
//! meter, key caps and the navigation list. Colours always come from the
//! resolved NIRUX palette, so every theme (light or dark) looks right.
use crate::theme::Palette;
use egui::{Color32, Response, RichText, Sense, Stroke, Ui, Vec2};

pub fn red(p: &Palette) -> Color32 {
    if p.light {
        Color32::from_rgb(196, 43, 43)
    } else {
        Color32::from_rgb(235, 87, 87)
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Text colour that reads on `bg`.
fn on(bg: Color32) -> Color32 {
    let l = 0.2126 * bg.r() as f32 + 0.7152 * bg.g() as f32 + 0.0722 * bg.b() as f32;
    if l > 150.0 {
        Color32::from_rgb(20, 20, 22)
    } else {
        Color32::from_rgb(250, 250, 250)
    }
}

/// Rounder widgets and roomier spacing on top of the shared NIRUX style.
pub fn tune_style(ctx: &egui::Context, p: &Palette) {
    let mut style = (*ctx.style()).clone();
    let r = egui::Rounding::same(7.0);
    for w in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
        &mut style.visuals.widgets.noninteractive,
    ] {
        w.rounding = r;
    }
    style.visuals.widgets.inactive.weak_bg_fill = p.surface_hi;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    style.visuals.window_rounding = egui::Rounding::same(12.0);
    style.visuals.slider_trailing_fill = true;
    style.visuals.selection.bg_fill = mix(p.surface, p.accent, 0.55);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.slider_width = 220.0;
    style.spacing.interact_size.y = 26.0;
    ctx.set_style(style);
}

pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(p.surface)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(12.0)
        .inner_margin(egui::Margin::symmetric(18.0, 14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn section_title(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).strong().size(15.0).color(p.foreground));
}

pub fn chip(ui: &mut Ui, p: &Palette, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(mix(p.surface, color, if p.light { 0.16 } else { 0.22 }))
        .stroke(Stroke::new(1.0_f32, mix(p.surface, color, 0.55)))
        .rounding(20.0)
        .inner_margin(egui::Margin::symmetric(9.0, 2.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                status_dot(ui, color, 7.0);
                ui.label(RichText::new(text).size(12.0).color(p.foreground));
            });
        });
}

pub fn status_dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let fill = p.accent;
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(on(fill)))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, mix(fill, p.foreground, 0.15)))
            .rounding(8.0)
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

pub fn segment(ui: &mut Ui, p: &Palette, text: &str, selected: bool) -> Response {
    let fill = if selected { p.accent } else { p.surface_hi };
    let color = if selected { on(fill) } else { p.foreground };
    ui.add(
        egui::Button::new(RichText::new(text).color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, if selected { fill } else { p.border }))
            .rounding(20.0),
    )
}

pub fn nav_item(ui: &mut Ui, p: &Palette, icon: &str, text: &str, selected: bool) -> Response {
    let height = 34.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(
            rect,
            8.0,
            mix(p.panel, p.accent, if p.light { 0.16 } else { 0.20 }),
        );
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, 8.0),
            egui::vec2(3.0, height - 16.0),
        );
        painter.rect_filled(bar, 2.0, p.accent);
    } else if response.hovered() {
        painter.rect_filled(rect, 8.0, mix(p.panel, p.foreground, 0.06));
    }
    let color = if selected {
        p.foreground
    } else {
        mix(p.muted, p.foreground, 0.3)
    };
    painter.text(
        rect.left_center() + egui::vec2(14.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        if selected { p.accent } else { p.muted },
    );
    painter.text(
        rect.left_center() + egui::vec2(40.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(14.5),
        color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn level_meter(ui: &mut Ui, p: &Palette, levels: &[f32], size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, p.inset);
    let bars = 32usize;
    let gap = 2.0;
    let w = (rect.width() - 8.0 - gap * (bars as f32 - 1.0)) / bars as f32;
    for i in 0..bars {
        let v = levels
            .len()
            .checked_sub(bars - i)
            .and_then(|j| levels.get(j))
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let h = (rect.height() - 8.0) * v.max(0.06);
        let x = rect.left() + 4.0 + i as f32 * (w + gap);
        let bar =
            egui::Rect::from_min_size(egui::pos2(x, rect.center().y - h / 2.0), egui::vec2(w, h));
        let color = if v > 0.92 {
            red(p)
        } else {
            mix(p.surface, p.accent, 0.45 + 0.55 * v)
        };
        painter.rect_filled(bar, w / 2.0, color);
    }
}

pub fn keycaps(ui: &mut Ui, p: &Palette, spec: &str) {
    let display = crate::hotkey::display(spec);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, part) in display.split(" + ").enumerate() {
            if i > 0 {
                ui.label(RichText::new("+").color(p.muted));
            }
            egui::Frame::none()
                .fill(p.surface_hi)
                .stroke(Stroke::new(1.0_f32, p.border_hi))
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(part).monospace().color(p.foreground));
                });
        }
    });
}

pub fn quote(ui: &mut Ui, p: &Palette, text: &str) {
    egui::Frame::none()
        .fill(p.inset)
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(15.0).color(p.foreground));
        });
}

/// The microphone mark.
pub fn logo(ui: &mut Ui, p: &Palette, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    painter.circle_filled(c, size / 2.0, p.accent);
    let fg = on(p.accent);
    let body = egui::Rect::from_center_size(
        c - egui::vec2(0.0, size * 0.08),
        egui::vec2(size * 0.24, size * 0.40),
    );
    painter.rect_filled(body, size * 0.12, fg);
    let stroke = Stroke::new(size * 0.06, fg);
    let r = size * 0.21;
    let points: Vec<egui::Pos2> = (0..=16)
        .map(|i| {
            let a = std::f32::consts::PI * (i as f32 / 16.0);
            c + egui::vec2(r * a.cos(), r * a.sin() - size * 0.02)
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
    painter.line_segment(
        [
            c + egui::vec2(0.0, size * 0.19),
            c + egui::vec2(0.0, size * 0.30),
        ],
        stroke,
    );
}

/// The window icon: an accent disc with the microphone.
pub fn icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let accent = [214.0, 168.0, 95.0];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
            let disc = (31.0 - (fx * fx + fy * fy).sqrt()).clamp(0.0, 1.0);
            // Microphone body: a capsule.
            let by = (fy + 5.0).abs() - 7.0;
            let capsule =
                (7.5 - ((fx.abs().max(0.0)).powi(2) + by.max(0.0).powi(2)).sqrt()).clamp(0.0, 1.0);
            let ring_r = (fx * fx + (fy + 2.0) * (fy + 2.0)).sqrt();
            let ring = if fy > -2.0 {
                (1.8 - (ring_r - 13.0).abs()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let stem = if fx.abs() < 1.8 && fy > 11.0 && fy < 19.0 {
                1.0
            } else {
                0.0
            };
            let mark = capsule.max(ring).max(stem);
            let i = (y * n + x) * 4;
            for k in 0..3 {
                rgba[i + k] = (accent[k] * (1.0 - mark) + 250.0 * mark) as u8;
            }
            rgba[i + 3] = (disc * 255.0) as u8;
        }
    }
    egui::IconData {
        rgba,
        width: n as u32,
        height: n as u32,
    }
}

/// The key combination pressed in this frame, as `ctrl+alt+d`.
pub fn capture_shortcut(ctx: &egui::Context) -> Option<String> {
    ctx.input(|i| {
        for e in &i.events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = e
            {
                use egui::Key;
                let name = match key {
                    Key::Escape => return Some("escape".to_string()),
                    Key::Space => "space".to_string(),
                    Key::Enter => "return".to_string(),
                    Key::Tab => "tab".to_string(),
                    Key::Backspace => "backspace".to_string(),
                    Key::Insert => "insert".to_string(),
                    Key::Delete => "delete".to_string(),
                    Key::Home => "home".to_string(),
                    Key::End => "end".to_string(),
                    Key::Minus => "minus".to_string(),
                    Key::Equals => "equal".to_string(),
                    Key::Comma => "comma".to_string(),
                    Key::Period => "period".to_string(),
                    Key::Slash => "slash".to_string(),
                    Key::Semicolon => "semicolon".to_string(),
                    Key::Backtick => "grave".to_string(),
                    other => {
                        let n = other.name().to_ascii_lowercase();
                        if n.len() == 1 || (n.starts_with('f') && n[1..].parse::<u32>().is_ok()) {
                            n
                        } else {
                            continue;
                        }
                    }
                };
                let mut parts = Vec::new();
                if modifiers.ctrl {
                    parts.push("ctrl".to_string());
                }
                if modifiers.alt {
                    parts.push("alt".to_string());
                }
                if modifiers.shift {
                    parts.push("shift".to_string());
                }
                parts.push(name);
                return Some(parts.join("+"));
            }
        }
        None
    })
}

/// Local date and time for a Unix timestamp.
pub fn when(time: u64) -> String {
    let t = time as libc::time_t;
    // SAFETY: localtime_r writes into our zeroed struct.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}
