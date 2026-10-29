//! Developer-only window screenshots, for reviewing the UI without
//! capturing the rest of the screen.
//!
//! ```sh
//! OPENCERES_SCREENSHOT=/tmp/out.png \
//! OPENCERES_SCREENSHOT_STATE=dark,grid,mark=2x1,panel,paused,size=1280x820 \
//!   openceres-desktop data/set.zip
//! ```
//!
//! With `OPENCERES_SCREENSHOT` set, the app starts with default preferences
//! (and never saves them), opens the paths given on the command line, waits
//! until they have loaded and a few frames have been drawn, saves a PNG of
//! its own window and exits. `OPENCERES_SCREENSHOT_STATE` is a comma list of:
//! `light`, `dark`, `stars`, `invert`, `grid`, `panel`, `settings`, `help`,
//! `paused`, `frame=N` (1-based), `mark=RxC` (row × column, repeatable),
//! `gridsize=N`, `known` (look up known objects and wait for the answer),
//! `focus=LABEL` (centre and zoom on that known object) and `size=WxH`
//! (window size in points).

use eframe::egui;

#[derive(Debug, Default, PartialEq)]
pub struct State {
    pub theme: Option<egui::Theme>,
    pub stars: bool,
    pub invert: bool,
    pub grid: bool,
    pub panel: bool,
    pub settings: bool,
    pub help: bool,
    pub paused: bool,
    pub known: bool,
    pub focus: Option<String>,
    pub frame: Option<usize>,
    pub grid_size: Option<usize>,
    /// Report boxes as (row, column).
    pub marks: Vec<(usize, usize)>,
    pub size: Option<[f32; 2]>,
}

pub struct Capture {
    pub path: String,
    pub state: State,
    /// Frames drawn since the image set (if any) finished loading.
    pub frames: u32,
    /// Input time when the state was applied; fade-in animations must end
    /// before the shot.
    pub applied_at: Option<f64>,
    pub requested: bool,
    /// Known object to centre on once the lookup has finished.
    pub focus: Option<String>,
}

impl Capture {
    pub fn from_env() -> Option<Capture> {
        let path = std::env::var("OPENCERES_SCREENSHOT")
            .ok()
            .filter(|p| !p.is_empty())?;
        let state = State::parse(&std::env::var("OPENCERES_SCREENSHOT_STATE").unwrap_or_default());
        Some(Capture {
            path,
            state,
            frames: 0,
            applied_at: None,
            requested: false,
            focus: None,
        })
    }
}

impl State {
    pub fn parse(list: &str) -> State {
        let mut s = State::default();
        let pair = |v: &str| {
            let (a, b) = v.split_once('x')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        };
        for item in list.split(',').map(str::trim) {
            let (key, value) = item.split_once('=').unwrap_or((item, ""));
            match key {
                "light" => s.theme = Some(egui::Theme::Light),
                "dark" => s.theme = Some(egui::Theme::Dark),
                "stars" => s.stars = true,
                "invert" => s.invert = true,
                "grid" => s.grid = true,
                "panel" => s.panel = true,
                "settings" => s.settings = true,
                "help" => s.help = true,
                "paused" => s.paused = true,
                "known" => s.known = true,
                "focus" => s.focus = Some(value.to_owned()).filter(|v| !v.is_empty()),
                "frame" => s.frame = value.parse().ok(),
                "gridsize" => s.grid_size = value.parse().ok(),
                "mark" => s.marks.extend(pair(value)),
                "size" => {
                    s.size = pair(value).map(|(w, h)| [w as f32, h as f32]);
                }
                _ => {}
            }
        }
        s
    }
}

/// Writes an 8-bit RGBA PNG: one IDAT chunk, no filtering. Enough for
/// screenshots; not meant to be small.
pub fn write_png(path: &str, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    std::fs::write(path, encode_png(w, h, &rgba(image)))
}

fn rgba(image: &egui::ColorImage) -> Vec<u8> {
    image
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect()
}

pub fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 4 + 1) * height);
    for row in rgba.chunks_exact(width * 4).take(height) {
        raw.push(0); // filter type: none
        raw.extend_from_slice(row);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, adaptive, no interlace
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(
        &mut out,
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_structure() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[idat - 4..idat].try_into().unwrap()) as usize;
        let raw =
            miniz_oxide::inflate::decompress_to_vec_zlib(&png[idat + 4..idat + 4 + len]).unwrap();
        assert_eq!(raw, [0, 255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn parses_state() {
        let s = State::parse(
            "dark, grid,mark=2x1,mark=3x4,size=900x600,frame=2,known,focus=2026 RA34,bogus",
        );
        assert_eq!(s.theme, Some(egui::Theme::Dark));
        assert!(s.grid && !s.panel);
        assert_eq!(s.marks, [(2, 1), (3, 4)]);
        assert_eq!(s.size, Some([900.0, 600.0]));
        assert_eq!(s.frame, Some(2));
        assert!(s.known);
        assert_eq!(s.focus.as_deref(), Some("2026 RA34"));
    }
}
