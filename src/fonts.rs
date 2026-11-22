//! System fonts for the overlay and the control panel. Known paths are tried
//! first (no subprocess on the fast path), then fontconfig's `fc-match`.
use ab_glyph::FontArc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Sans,
    Bold,
    Mono,
}

fn candidates(face: Face) -> &'static [&'static str] {
    match face {
        Face::Sans => &[
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-L.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        ],
        Face::Bold => &[
            "/usr/share/fonts/truetype/ubuntu/Ubuntu-M.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/noto/NotoSans-SemiBold.ttf",
            "/usr/share/fonts/noto/NotoSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
        ],
        Face::Mono => &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
        ],
    }
}

fn pattern(face: Face) -> &'static str {
    match face {
        Face::Sans => "sans-serif",
        Face::Bold => "sans-serif:weight=bold",
        Face::Mono => "monospace",
    }
}

pub fn read(path: &std::path::Path) -> Option<FontArc> {
    let bytes = std::fs::read(path).ok()?;
    FontArc::try_from_vec(bytes).ok()
}

/// A font for `face`, or None when the system has no usable font at all.
pub fn load(face: Face) -> Option<FontArc> {
    for p in candidates(face) {
        if let Some(f) = read(std::path::Path::new(p)) {
            return Some(f);
        }
    }
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern(face)])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !path.is_empty() {
        return read(std::path::Path::new(&path));
    }
    None
}
