//! A client-side framebuffer (0x00RRGGBB per pixel) with the few anti-aliased
//! shapes the control panel needs. Everything is drawn here and sent to the
//! window in one copy (MIT-SHM when the server offers it).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn shrink(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.x + dx,
            self.y + dy,
            (self.w - 2.0 * dx).max(0.0),
            (self.h - 2.0 * dy).max(0.0),
        )
    }
    pub fn scale(&self, s: f32) -> Rect {
        Rect::new(self.x * s, self.y * s, self.w * s, self.h * s)
    }
}

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
    clip: (i32, i32, i32, i32),
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        let (w, h) = (w.max(1), h.max(1));
        Self {
            w,
            h,
            px: vec![0; (w * h) as usize],
            clip: (0, 0, w, h),
        }
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        let (w, h) = (w.max(1), h.max(1));
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.px = vec![0; (w * h) as usize];
        }
        self.reset_clip();
    }

    pub fn set_clip(&mut self, r: Rect) {
        let x0 = (r.x.floor() as i32).clamp(0, self.w);
        let y0 = (r.y.floor() as i32).clamp(0, self.h);
        let x1 = (r.right().ceil() as i32).clamp(0, self.w);
        let y1 = (r.bottom().ceil() as i32).clamp(0, self.h);
        self.clip = (x0, y0, x1.max(x0), y1.max(y0));
    }

    pub fn reset_clip(&mut self) {
        self.clip = (0, 0, self.w, self.h);
    }

    pub fn clear(&mut self, color: u32) {
        self.px.fill(color);
    }

    #[inline]
    fn blend_at(&mut self, i: usize, color: u32, a: u32) {
        if a == 0 {
            return;
        }
        if a >= 255 {
            self.px[i] = color;
            return;
        }
        let d = self.px[i];
        let inv = 255 - a;
        let ch = |s: u32, d: u32| (s * a + d * inv + 127) / 255;
        self.px[i] = (ch((color >> 16) & 0xff, (d >> 16) & 0xff) << 16)
            | (ch((color >> 8) & 0xff, (d >> 8) & 0xff) << 8)
            | ch(color & 0xff, d & 0xff);
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, color: u32, a: u32) {
        let (x0, y0, x1, y1) = self.clip;
        if x >= x0 && x < x1 && y >= y0 && y < y1 {
            self.blend_at((y * self.w + x) as usize, color, a);
        }
    }

    fn span(&mut self, y: i32, xa: i32, xb: i32, color: u32, a: u32) {
        let (x0, y0, x1, y1) = self.clip;
        if y < y0 || y >= y1 {
            return;
        }
        let (xa, xb) = (xa.max(x0), xb.min(x1));
        if xa >= xb {
            return;
        }
        let row = (y * self.w) as usize;
        if a >= 255 {
            self.px[row + xa as usize..row + xb as usize].fill(color);
        } else {
            for x in xa..xb {
                self.blend_at(row + x as usize, color, a);
            }
        }
    }

    /// An anti-aliased rounded rectangle in pixels; `alpha` 0..=1.
    pub fn round_rect(&mut self, r: Rect, radius: f32, color: u32, alpha: f32) {
        if r.w <= 0.0 || r.h <= 0.0 || alpha <= 0.0 {
            return;
        }
        let rad = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        let (cx, cy) = r.center();
        let (hw, hh) = (r.w / 2.0, r.h / 2.0);
        let amax = (alpha.clamp(0.0, 1.0) * 255.0).round();
        let (x0, y0, x1, y1) = self.clip;
        let ya = (r.y.floor() as i32).max(y0);
        let yb = (r.bottom().ceil() as i32).min(y1);
        let xa = (r.x.floor() as i32).max(x0);
        let xb = (r.right().ceil() as i32).min(x1);
        if xa >= xb || ya >= yb {
            return;
        }
        let sdf = |px: f32, py: f32| {
            let qx = (px - cx).abs() - (hw - rad);
            let qy = (py - cy).abs() - (hh - rad);
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad
        };
        for y in ya..yb {
            let py = y as f32 + 0.5;
            let in_corner_band = (py - cy).abs() > hh - rad - 0.5;
            if in_corner_band || r.w < 4.0 {
                for x in xa..xb {
                    let cov = (0.5 - sdf(x as f32 + 0.5, py)).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.put(x, y, color, (cov * amax) as u32);
                    }
                }
                continue;
            }
            // Straight band: anti-alias the two ends, fill the middle.
            let vcov = (hh - (py - cy).abs() + 0.5).clamp(0.0, 1.0);
            let left = r.x;
            let right = r.right();
            let fl = left.ceil() as i32;
            let fr = right.floor() as i32;
            if fl > fr {
                let cov = (right - left).clamp(0.0, 1.0) * vcov;
                self.put(left.floor() as i32, y, color, (cov * amax) as u32);
                continue;
            }
            if (fl as f32) > left {
                let cov = (fl as f32 - left) * vcov;
                self.put(fl - 1, y, color, (cov * amax) as u32);
            }
            if right > fr as f32 {
                let cov = (right - fr as f32) * vcov;
                self.put(fr, y, color, (cov * amax) as u32);
            }
            self.span(y, fl, fr, color, (vcov * amax) as u32);
        }
    }

    /// A thick anti-aliased line with round caps.
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: u32) {
        let hw = width / 2.0;
        let (x0, y0, x1, y1) = self.clip;
        let xa = ((a.0.min(b.0) - hw - 1.0).floor() as i32).max(x0);
        let xb = ((a.0.max(b.0) + hw + 1.0).ceil() as i32).min(x1);
        let ya = ((a.1.min(b.1) - hw - 1.0).floor() as i32).max(y0);
        let yb = ((a.1.max(b.1) + hw + 1.0).ceil() as i32).min(y1);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in ya..yb {
            for x in xa..xb {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0);
                let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
                let d = (qx * qx + qy * qy).sqrt() - hw;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(x, y, color, (cov * 255.0) as u32);
                }
            }
        }
    }

    /// Draw a coverage mask (one byte per pixel) in `color` at (x, y).
    pub fn mask(&mut self, x: i32, y: i32, w: i32, h: i32, coverage: &[u8], color: u32) {
        let (cx0, cy0, cx1, cy1) = self.clip;
        let xa = x.max(cx0);
        let xb = (x + w).min(cx1);
        let ya = y.max(cy0);
        let yb = (y + h).min(cy1);
        for py in ya..yb {
            let src = ((py - y) * w) as usize;
            let dst = (py * self.w) as usize;
            for px in xa..xb {
                let a = coverage[src + (px - x) as usize] as u32;
                self.blend_at(dst + px as usize, color, a);
            }
        }
    }

    /// Premultiplied-free RGBA pixels (straight alpha), e.g. the window icon.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: u32 has no padding; the slice covers exactly the pixels.
        unsafe { std::slice::from_raw_parts(self.px.as_ptr() as *const u8, self.px.len() * 4) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rounded_rect_fills_its_middle_and_leaves_corners() {
        let mut c = Canvas::new(40, 20);
        c.round_rect(Rect::new(0.0, 0.0, 40.0, 20.0), 8.0, 0xffffff, 1.0);
        assert_eq!(c.px[10 * 40 + 20], 0xffffff);
        assert_eq!(c.px[0], 0);
        assert_eq!(c.px[10 * 40], 0xffffff);
    }

    #[test]
    fn the_clip_limits_drawing() {
        let mut c = Canvas::new(10, 10);
        c.set_clip(Rect::new(5.0, 5.0, 5.0, 5.0));
        c.round_rect(Rect::new(0.0, 0.0, 10.0, 10.0), 0.0, 0x123456, 1.0);
        assert_eq!(c.px[4 * 10 + 4], 0);
        assert_eq!(c.px[5 * 10 + 5], 0x123456);
    }

    #[test]
    fn half_alpha_blends_halfway() {
        let mut c = Canvas::new(4, 4);
        c.round_rect(Rect::new(0.0, 0.0, 4.0, 4.0), 0.0, 0xffffff, 0.5);
        let v = c.px[5] & 0xff;
        assert!((125..=130).contains(&v), "{v}");
    }

    #[test]
    fn lines_cover_their_path() {
        let mut c = Canvas::new(20, 20);
        c.line((2.0, 10.0), (18.0, 10.0), 2.0, 0xffffff);
        assert_eq!(c.px[10 * 20 + 10] & 0xff, 0xff);
        assert_eq!(c.px[2 * 20 + 10], 0);
    }
}
