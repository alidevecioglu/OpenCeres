//! Aperture photometry calibrated against Gaia G.
//!
//! The object and the Gaia stars in the same frame are measured through the
//! same circular aperture (radius tied to the frame's seeing) with the sky
//! from a surrounding annulus, so the aperture correction cancels. The
//! zero point is the clipped median of `G − instrumental` over unsaturated,
//! well-exposed stars (Howell, *Handbook of CCD Astronomy*, ch. 5).
//!
//! The band reported is `G`: the MPC lists `G` as Gaia G in its accepted
//! bands for the 80-column format and in the ADES band list. No colour term
//! is applied: asteroids are redder than the average field star and IASC
//! frames are taken in Pan-STARRS w, so expect a few tenths of a magnitude
//! of systematic error, comparable to the 0.1 mag precision MPC reports
//! normally carry and much less than asteroid rotation light curves.

use crate::fits::Image;

/// Local sky from an annulus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// Clipped median, ADU.
    pub level: f64,
    /// Robust (MAD) noise per pixel, ADU.
    pub sigma: f64,
    pub pixels: usize,
}

/// A pixel fill value (survey cell gaps, dead cells): equal to all four
/// neighbours, which real noise practically never is.
fn is_fill(image: &Image, x: usize, y: usize) -> bool {
    let (w, h) = (image.width, image.height);
    if x == 0 || y == 0 || x + 1 >= w || y + 1 >= h {
        return false;
    }
    let v = image.at(x, y);
    image.at(x - 1, y) == v
        && image.at(x + 1, y) == v
        && image.at(x, y - 1) == v
        && image.at(x, y + 1) == v
}

/// Sky in the annulus `inner`..`outer` (pixels) around (x, y): median and
/// MAD-scaled sigma, clipped at 3σ a few times so stars in the annulus do
/// not bias it. `None` with fewer than 20 usable pixels.
pub fn local_sky(image: &Image, x: f64, y: f64, inner: f64, outer: f64) -> Option<Sky> {
    let mut values = Vec::new();
    let (x0, x1) = (
        (x - outer).floor().max(0.0) as usize,
        (x + outer).ceil().min(image.width as f64 - 1.0),
    );
    let (y0, y1) = (
        (y - outer).floor().max(0.0) as usize,
        (y + outer).ceil().min(image.height as f64 - 1.0),
    );
    if x1 < 0.0 || y1 < 0.0 {
        return None;
    }
    for py in y0..=y1 as usize {
        for px in x0..=x1 as usize {
            let r = (px as f64 - x).hypot(py as f64 - y);
            let v = image.at(px, py);
            if r >= inner && r <= outer && v.is_finite() && !is_fill(image, px, py) {
                values.push(f64::from(v));
            }
        }
    }
    let (mut level, mut sigma) = median_mad(&mut values)?;
    for _ in 0..4 {
        values.retain(|v| (v - level).abs() <= 3.0 * sigma.max(1e-9));
        let Some((l, s)) = median_mad(&mut values) else {
            break;
        };
        (level, sigma) = (l, s);
    }
    (values.len() >= 20).then_some(Sky {
        level,
        sigma,
        pixels: values.len(),
    })
}

fn median_mad(values: &mut [f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mid = values.len() / 2;
    let m = *values.select_nth_unstable_by(mid, f64::total_cmp).1;
    let mut dev: Vec<f64> = values.iter().map(|v| (v - m).abs()).collect();
    let d = *dev.select_nth_unstable_by(mid, f64::total_cmp).1;
    Some((m, 1.4826 * d))
}

/// Background-subtracted flux through a circular aperture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flux {
    pub flux: f64,
    /// 1σ: sky noise in the aperture, the sky level's own error and (with
    /// a gain) the source's photon noise.
    pub error: f64,
    /// False if the aperture touches the edge, a gap/dead cell, a NaN or a
    /// saturated pixel: the flux is then not to be trusted.
    pub clean: bool,
}

impl Flux {
    pub fn snr(&self) -> f64 {
        if self.error > 0.0 {
            self.flux / self.error
        } else {
            0.0
        }
    }
}

/// Subdivisions per pixel side where a pixel straddles the aperture edge.
const SUBPIXELS: usize = 5;

/// Flux within `radius` pixels of (x, y). `gain` in electrons per ADU (adds
/// the source's photon noise), `saturation` in ADU.
pub fn aperture(
    image: &Image,
    x: f64,
    y: f64,
    radius: f64,
    sky: &Sky,
    gain: Option<f64>,
    saturation: Option<f64>,
) -> Flux {
    let (w, h) = (image.width as isize, image.height as isize);
    let mut clean = true;
    let (mut sum, mut area) = (0.0, 0.0);
    let reach = radius.ceil() as isize + 1;
    let (cx, cy) = (x.round() as isize, y.round() as isize);
    for py in cy - reach..=cy + reach {
        for px in cx - reach..=cx + reach {
            let r = (px as f64 - x).hypot(py as f64 - y);
            // Fraction of the pixel inside the circle.
            let frac = if r <= radius - 0.75 {
                1.0
            } else if r >= radius + 0.75 {
                0.0
            } else {
                let n = SUBPIXELS as f64;
                let mut inside = 0;
                for j in 0..SUBPIXELS {
                    for i in 0..SUBPIXELS {
                        let sx = px as f64 - 0.5 + (i as f64 + 0.5) / n;
                        let sy = py as f64 - 0.5 + (j as f64 + 0.5) / n;
                        if (sx - x).hypot(sy - y) <= radius {
                            inside += 1;
                        }
                    }
                }
                f64::from(inside) / (n * n)
            };
            if frac == 0.0 {
                continue;
            }
            if px < 0 || py < 0 || px >= w || py >= h {
                clean = false;
                continue;
            }
            let v = image.at(px as usize, py as usize);
            if !v.is_finite() {
                clean = false;
                continue;
            }
            if is_fill(image, px as usize, py as usize)
                || saturation.is_some_and(|s| f64::from(v) >= s)
            {
                clean = false;
            }
            sum += frac * f64::from(v);
            area += frac;
        }
    }
    let flux = sum - area * sky.level;
    let sky_var = area * sky.sigma * sky.sigma;
    // The median's variance is π/2 times the mean's.
    let level_var = area * area * sky.sigma * sky.sigma * std::f64::consts::FRAC_PI_2
        / sky.pixels.max(1) as f64;
    let photon_var = gain.filter(|g| *g > 0.0).map_or(0.0, |g| flux.max(0.0) / g);
    Flux {
        flux,
        error: (sky_var + level_var + photon_var).sqrt(),
        clean,
    }
}

/// Magnitude zero point of a frame: `G = zp − 2.5 log₁₀ flux`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZeroPoint {
    pub zp: f64,
    /// Star-to-star scatter (robust sigma), magnitudes.
    pub scatter: f64,
    /// Uncertainty of `zp` itself.
    pub error: f64,
    pub stars: usize,
}

/// Stars needed for a zero point.
pub const MIN_ZP_STARS: usize = 5;
/// A larger star-to-star scatter (magnitudes) makes the zero point suspect.
pub const MAX_ZP_SCATTER: f64 = 0.15;
/// Stars measured less precisely than this (magnitudes) don't vote.
const MAX_STAR_ERROR: f64 = 0.03;

/// Zero point from (flux, catalogue G) pairs of clean, unsaturated stars.
pub fn zero_point(stars: &[(Flux, f64)]) -> Option<ZeroPoint> {
    let mut d: Vec<f64> = stars
        .iter()
        .filter(|(f, _)| f.clean && f.flux > 0.0 && 1.0857 / f.snr() < MAX_STAR_ERROR)
        .map(|(f, g)| g + 2.5 * f.flux.log10())
        .collect();
    let (mut zp, mut scatter) = median_mad(&mut d)?;
    for _ in 0..5 {
        d.retain(|v| (v - zp).abs() <= 3.0 * scatter.max(0.01));
        let Some((z, s)) = median_mad(&mut d) else {
            break;
        };
        (zp, scatter) = (z, s);
    }
    // Fewer than MIN_ZP_STARS still gives a value, which `problem` flags.
    if d.len() < 2 {
        return None;
    }
    let n = d.len();
    Some(ZeroPoint {
        zp,
        scatter,
        error: 1.2533 * scatter / (n as f64).sqrt(),
        stars: n,
    })
}

impl ZeroPoint {
    pub fn problem(&self) -> Option<String> {
        if self.stars < MIN_ZP_STARS {
            Some(format!("only {} stars calibrate the magnitude", self.stars))
        } else if self.scatter > MAX_ZP_SCATTER {
            Some(format!(
                "comparison stars scatter by {:.2} mag",
                self.scatter
            ))
        } else {
            None
        }
    }

    /// Magnitude and its 1σ error for `flux`; `None` for a non-positive flux.
    pub fn magnitude(&self, flux: &Flux) -> Option<(f64, f64)> {
        (flux.flux > 0.0).then(|| {
            let m = self.zp - 2.5 * flux.flux.log10();
            let e = (1.0857 / flux.snr()).hypot(self.error);
            (m, e)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fits::Header;
    use crate::plate::tests::gauss;

    fn image(stars: &[(f64, f64, f64)], sky: f64, noise: f64, seed: u64) -> Image {
        let (w, h) = (96, 96);
        let mut seed = seed;
        let sigma: f64 = 1.8;
        let data = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                let mut v = sky + noise * gauss(&mut seed);
                for &(sx, sy, total) in stars {
                    let r2 = (x - sx).powi(2) + (y - sy).powi(2);
                    v += total / (2.0 * std::f64::consts::PI * sigma * sigma)
                        * (-r2 / (2.0 * sigma * sigma)).exp();
                }
                v as f32
            })
            .collect();
        Image {
            width: w,
            height: h,
            data,
            header: Header::default(),
        }
    }

    /// Fluxes are unbiased and their error estimate matches the scatter
    /// over many noise realisations.
    #[test]
    fn aperture_flux_and_error_are_calibrated() {
        let total = 5000.0;
        let (mut sum, mut sum_sq, mut pred, n) = (0.0, 0.0, 0.0, 300);
        for seed in 0..n {
            let x = 48.0 + (seed % 7) as f64 * 0.13;
            let img = image(&[(x, 47.6, total)], 1000.0, 10.0, seed as u64 * 31 + 1);
            let sky = local_sky(&img, x, 47.6, 15.0, 25.0).unwrap();
            // 3.5σ_psf aperture: 99.8 % of a Gaussian's light.
            let f = aperture(&img, x, 47.6, 3.5 * 1.8, &sky, None, None);
            assert!(f.clean);
            sum += f.flux;
            sum_sq += (f.flux - total * 0.9978).powi(2);
            pred += f.error * f.error;
        }
        let n = n as f64;
        let mean = sum / n;
        assert!((mean / (total * 0.9978) - 1.0).abs() < 0.01, "mean {mean}");
        let ratio = (sum_sq / pred).sqrt();
        assert!((0.85..1.15).contains(&ratio), "scatter/predicted {ratio}");
    }

    #[test]
    fn aperture_flags_edges_and_saturation() {
        let img = image(
            &[(3.0, 40.0, 5000.0), (50.0, 50.0, 500000.0)],
            1000.0,
            10.0,
            1,
        );
        let sky = local_sky(&img, 3.0, 40.0, 10.0, 20.0).unwrap();
        assert!(!aperture(&img, 3.0, 40.0, 6.0, &sky, None, None).clean);
        let sky = local_sky(&img, 50.0, 50.0, 10.0, 20.0).unwrap();
        assert!(!aperture(&img, 50.0, 50.0, 6.0, &sky, None, Some(20000.0)).clean);
        assert!(aperture(&img, 50.0, 50.0, 6.0, &sky, None, None).clean);
    }

    #[test]
    fn zero_point_recovers_the_offset_and_ignores_outliers() {
        let mut seed = 5;
        let mut stars: Vec<(Flux, f64)> = (0..40)
            .map(|i| {
                let g = 14.0 + i as f64 * 0.1;
                let flux = 10f64.powf(-0.4 * (g - 25.0)) * (1.0 + 0.02 * gauss(&mut seed));
                (
                    Flux {
                        flux,
                        error: flux / 200.0,
                        clean: true,
                    },
                    g,
                )
            })
            .collect();
        // A variable star and a blend.
        stars[3].1 += 1.0;
        stars[9].1 -= 0.8;
        // A saturated star is not used.
        stars[0].0.clean = false;
        stars[0].1 = 5.0;
        let zp = zero_point(&stars).unwrap();
        assert!((zp.zp - 25.0).abs() < 0.01, "{zp:?}");
        assert!(zp.scatter < 0.03 && zp.problem().is_none());
        let (m, e) = zp
            .magnitude(&Flux {
                flux: 10f64.powf(-0.4 * (20.0 - 25.0)),
                error: 10f64.powf(-0.4 * (20.0 - 25.0)) / 10.0,
                clean: true,
            })
            .unwrap();
        assert!(
            (m - 20.0).abs() < 0.01 && (e - 0.1086).abs() < 0.01,
            "{m} {e}"
        );
    }
}
