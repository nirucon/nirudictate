//! Text: a sans-serif face (with a bold variant) and a monospace face, a
//! glyph coverage cache per (face, character, size), measuring, word wrap and
//! ellipsis. Sizes are in pixels.
use super::canvas::Canvas;
use crate::fonts::{self, Face};
use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use std::collections::HashMap;

/// Text set at size 15 in the 0.3 panel (egui and its Ubuntu font) was this
/// wide; every font is scaled so the panel keeps its proportions.
const REFERENCE: &str = "Speak, and the text appears where you type. Nothing leaves this computer.";
const REFERENCE_WIDTH_PER_PX: f32 = 504.0 / 15.0;

/// The size multiplier that makes `f` as wide as the reference.
fn calibrate(f: &FontArc) -> f32 {
    let s = f.as_scaled(PxScale::from(100.0));
    let w: f32 = REFERENCE.chars().map(|c| s.h_advance(f.glyph_id(c))).sum();
    if w <= 0.0 {
        return 1.0;
    }
    (REFERENCE_WIDTH_PER_PX * 100.0 / w).clamp(0.6, 2.0)
}

struct Mask {
    w: i32,
    h: i32,
    x: i32,
    y: i32,
    coverage: Vec<u8>,
}

pub struct Text {
    sans: FontArc,
    mult: f32,
    bold: Option<Option<FontArc>>,
    mono: Option<Option<FontArc>>,
    fallback: Option<Option<FontArc>>,
    masks: HashMap<(Face, char, u32), Option<Mask>>,
    advances: HashMap<(Face, char, u32), f32>,
}

impl Text {
    /// `font_path`: the NIRUX appearance font, used for every face.
    pub fn new(font_path: Option<&std::path::Path>) -> Result<Self, String> {
        let custom = font_path.and_then(fonts::read);
        let sans = match custom.clone() {
            Some(f) => f,
            None => fonts::load(Face::Sans).ok_or("no usable font found (install fonts-dejavu)")?,
        };
        Ok(Self {
            mult: calibrate(&sans),
            // A custom NIRUX font is used for every face, like before.
            bold: custom.clone().map(Some),
            mono: custom.clone().map(Some),
            sans,
            fallback: None,
            masks: HashMap::new(),
            advances: HashMap::new(),
        })
    }

    fn face(&mut self, face: Face) -> FontArc {
        match face {
            Face::Sans => self.sans.clone(),
            Face::Bold => self
                .bold
                .get_or_insert_with(|| fonts::load(Face::Bold))
                .clone()
                .unwrap_or_else(|| self.sans.clone()),
            Face::Mono => self
                .mono
                .get_or_insert_with(|| fonts::load(Face::Mono))
                .clone()
                .unwrap_or_else(|| self.sans.clone()),
        }
    }

    fn font_for(&mut self, face: Face, c: char) -> FontArc {
        let f = self.face(face);
        if f.glyph_id(c).0 != 0 || c.is_ascii() {
            return f;
        }
        let fb = self
            .fallback
            .get_or_insert_with(|| {
                fonts::read(std::path::Path::new(
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                ))
                .or_else(|| {
                    fonts::read(std::path::Path::new("/usr/share/fonts/TTF/DejaVuSans.ttf"))
                })
            })
            .clone();
        match fb {
            Some(fb) if fb.glyph_id(c).0 != 0 => fb,
            _ => f,
        }
    }

    pub fn ascent(&mut self, face: Face, px: f32) -> f32 {
        let m = self.mult;
        self.face(face).as_scaled(PxScale::from(px * m)).ascent()
    }

    /// The line height for `px`.
    pub fn line_height(&mut self, face: Face, px: f32) -> f32 {
        let f = self.face(face);
        let s = f.as_scaled(PxScale::from(px * self.mult));
        (s.ascent() - s.descent() + s.line_gap()).max(px)
    }

    pub fn advance(&mut self, face: Face, c: char, px: f32) -> f32 {
        let key = (face, c, px.to_bits());
        if let Some(a) = self.advances.get(&key) {
            return *a;
        }
        let f = self.font_for(face, c);
        let a = f
            .as_scaled(PxScale::from(px * self.mult))
            .h_advance(f.glyph_id(c));
        self.advances.insert(key, a);
        a
    }

    pub fn width(&mut self, face: Face, px: f32, s: &str) -> f32 {
        s.chars().map(|c| self.advance(face, c, px)).sum()
    }

    /// The x offset of each character boundary (len + 1 values).
    pub fn offsets(&mut self, face: Face, px: f32, s: &str) -> Vec<f32> {
        let mut out = Vec::with_capacity(s.len() + 1);
        let mut x = 0.0;
        out.push(0.0);
        for c in s.chars() {
            x += self.advance(face, c, px);
            out.push(x);
        }
        out
    }

    /// The longest prefix of `s` that fits `max`, with "…" when cut.
    pub fn ellipsize(&mut self, face: Face, px: f32, s: &str, max: f32) -> String {
        if self.width(face, px, s) <= max {
            return s.to_string();
        }
        let dots = self.advance(face, '…', px);
        let mut out = String::new();
        let mut w = 0.0;
        for c in s.chars() {
            let a = self.advance(face, c, px);
            if w + a + dots > max {
                break;
            }
            w += a;
            out.push(c);
        }
        out.push('…');
        out
    }

    /// Break `s` into lines of at most `max` pixels at spaces (explicit
    /// newlines are kept); a word longer than a line is broken anywhere.
    pub fn wrap(&mut self, face: Face, px: f32, s: &str, max: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let space = self.advance(face, ' ', px);
        for para in s.split('\n') {
            let mut line = String::new();
            let mut w = 0.0;
            for word in para.split(' ') {
                let ww = self.width(face, px, word);
                if !line.is_empty() && w + space + ww > max {
                    lines.push(std::mem::take(&mut line));
                    w = 0.0;
                }
                if ww > max {
                    // Break the long word.
                    for c in word.chars() {
                        let a = self.advance(face, c, px);
                        if !line.is_empty() && w + a > max {
                            lines.push(std::mem::take(&mut line));
                            w = 0.0;
                        }
                        line.push(c);
                        w += a;
                    }
                    continue;
                }
                if !line.is_empty() {
                    line.push(' ');
                    w += space;
                }
                line.push_str(word);
                w += ww;
            }
            lines.push(line);
        }
        lines
    }

    /// Draw `s` with its top at `y` (pixels). Returns the width.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        cv: &mut Canvas,
        face: Face,
        px: f32,
        x: f32,
        y: f32,
        s: &str,
        color: u32,
    ) -> f32 {
        let base = (y + self.ascent(face, px)).round() as i32;
        let mut pen = x;
        for c in s.chars() {
            let key = (face, c, px.to_bits());
            if !self.masks.contains_key(&key) {
                let m = self.rasterise(face, c, px);
                self.masks.insert(key, m);
            }
            if let Some(Some(m)) = self.masks.get(&key) {
                cv.mask(
                    pen.round() as i32 + m.x,
                    base + m.y,
                    m.w,
                    m.h,
                    &m.coverage,
                    color,
                );
            }
            pen += self.advance(face, c, px);
        }
        pen - x
    }

    fn rasterise(&mut self, face: Face, c: char, px: f32) -> Option<Mask> {
        if c == ' ' {
            return None;
        }
        let f = self.font_for(face, c);
        let glyph = f
            .glyph_id(c)
            .with_scale_and_position(PxScale::from(px * self.mult), point(0.0, 0.0));
        let outline = f.outline_glyph(glyph)?;
        let b = outline.px_bounds();
        let (w, h) = (b.width().ceil() as i32, b.height().ceil() as i32);
        if w <= 0 || h <= 0 {
            return None;
        }
        let mut coverage = vec![0u8; (w * h) as usize];
        outline.draw(|gx, gy, v| {
            let i = (gy as i32 * w + gx as i32) as usize;
            if i < coverage.len() {
                coverage[i] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        });
        Some(Mask {
            w,
            h,
            x: b.min.x.floor() as i32,
            y: b.min.y.floor() as i32,
            coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_keeps_newlines() {
        let mut t = Text::new(None).expect("a system font");
        let lines = t.wrap(Face::Sans, 15.0, "one two three four five six", 80.0);
        assert!(lines.len() > 1);
        assert!(lines
            .iter()
            .all(|l| t.width(Face::Sans, 15.0, l) <= 80.0 || !l.contains(' ')));
        assert_eq!(t.wrap(Face::Sans, 15.0, "a\nb", 500.0), ["a", "b"]);
        let e = t.ellipsize(Face::Sans, 15.0, &"x".repeat(100), 60.0);
        assert!(e.ends_with('…') && t.width(Face::Sans, 15.0, &e) <= 60.0);
    }
}
