//! Converting raw image data to 8-bit grayscale pixels for display.

use crate::detect::Background;
use crate::fits::Image;

/// Display stretch: maps `black..white` to 0..255 with an asinh curve, which
/// keeps faint sky detail visible without blowing out bright stars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    pub black: f32,
    pub white: f32,
    /// 0 is linear; larger values brighten faint pixels more.
    pub softening: f32,
}

impl Stretch {
    /// A good default for asteroid searching: black just below the sky,
    /// white well above the noise so faint movers stand out.
    pub fn auto(bg: Background) -> Stretch {
        let sigma = bg.sigma.max(f32::EPSILON);
        Stretch {
            black: bg.level - 2.0 * sigma,
            white: bg.level + 40.0 * sigma,
            softening: 8.0,
        }
    }

    fn map(&self, v: f32) -> u8 {
        let t = ((v - self.black) / (self.white - self.black).max(f32::EPSILON)).clamp(0.0, 1.0);
        let t = if self.softening > 0.0 {
            (t * self.softening).asinh() / self.softening.asinh()
        } else {
            t
        };
        (t * 255.0).round() as u8
    }
}

/// Renders the image as 8-bit grayscale, one byte per pixel.
///
/// Rows are flipped so the first output row is the *top* of the image
/// (FITS stores the bottom row first), ready for a display canvas.
pub fn grayscale(image: &Image, stretch: Stretch) -> Vec<u8> {
    let mut out = Vec::with_capacity(image.data.len());
    for row in image.data.chunks_exact(image.width).rev() {
        out.extend(row.iter().map(|&v| stretch.map(v)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fits::Header;

    #[test]
    fn maps_range_and_flips_rows() {
        let image = Image {
            width: 2,
            height: 2,
            data: vec![0.0, 100.0, 50.0, 1000.0], // bottom row first
            header: Header::default(),
        };
        let stretch = Stretch {
            black: 0.0,
            white: 100.0,
            softening: 0.0,
        };
        assert_eq!(grayscale(&image, stretch), vec![128, 255, 0, 255]);
    }

    #[test]
    fn asinh_brightens_faint_pixels() {
        let linear = Stretch {
            black: 0.0,
            white: 100.0,
            softening: 0.0,
        };
        let soft = Stretch {
            softening: 8.0,
            ..linear
        };
        assert!(soft.map(10.0) > linear.map(10.0));
        assert_eq!(soft.map(100.0), 255);
        assert_eq!(soft.map(-5.0), 0);
    }
}
