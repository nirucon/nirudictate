//! An opaque sRGB colour: the palette type shared by the theme, the overlay
//! and the control panel.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color32([u8; 3]);

impl Color32 {
    pub const WHITE: Color32 = Color32([255, 255, 255]);
    pub const BLACK: Color32 = Color32([0, 0, 0]);

    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b])
    }
    pub const fn r(self) -> u8 {
        self.0[0]
    }
    pub const fn g(self) -> u8 {
        self.0[1]
    }
    pub const fn b(self) -> u8 {
        self.0[2]
    }
    /// 0x00RRGGBB, the canvas pixel format.
    pub const fn rgb(self) -> u32 {
        ((self.0[0] as u32) << 16) | ((self.0[1] as u32) << 8) | self.0[2] as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_as_rgb() {
        let c = Color32::from_rgb(0x12, 0x34, 0x56);
        assert_eq!((c.r(), c.g(), c.b()), (0x12, 0x34, 0x56));
        assert_eq!(c.rgb(), 0x123456);
    }
}
