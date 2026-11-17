//! Reducing one frame for a report: plate solution against Gaia, the
//! photometric zero point, and precise positions and magnitudes of the
//! points a student marked.
//!
//! Each mark is re-measured on the frame's pixels with a windowed centroid
//! ([`crate::detect::centroid`]) started at the mark, and turned into RA/Dec
//! with the frame's own plate solution ([`crate::plate`]). Its uncertainty
//! combines the centroid's (from pixel noise) with the plate model's at
//! that point. Without a catalogue the header WCS is used, and the result
//! says so: such positions are a preview, several arcseconds off.

use crate::detect::{self, DetectOptions, Star};
use crate::fits::{Header, Image};
use crate::gaia::GaiaStar;
use crate::photometry::{self, Flux, ZeroPoint};
use crate::plate::{self, PlateSolution, RefStar, SolveError};
use crate::time;
use crate::wcs::Wcs;

/// Aperture radius in units of the frame's FWHM (near the SNR optimum for
/// faint sources; the same aperture for stars and object, so the aperture
/// correction cancels).
const APERTURE_FWHM: f64 = 1.0;
/// Sky annulus, in FWHM.
const SKY_INNER_FWHM: f64 = 3.0;
const SKY_OUTER_FWHM: f64 = 5.0;
/// Below this signal-to-noise a magnitude is flagged as unreliable.
pub const MIN_MAG_SNR: f64 = 5.0;

/// How a mark came out.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    /// Centroid, 0-based pixels, and its 1σ.
    pub x: f64,
    pub y: f64,
    pub sigma_px: (f64, f64),
    /// J2000 (ICRS) position, degrees.
    pub ra: f64,
    pub dec: f64,
    /// 1σ in RA ×cos Dec and Dec, arcseconds: centroid and plate model;
    /// `None` when the position comes from the header WCS.
    pub sigma: Option<(f64, f64)>,
    /// Position from a Gaia plate solution (not the header WCS).
    pub solved: bool,
    /// How far the centroid moved from the mark, pixels.
    pub moved: f64,
    /// Why the position should not be reported, if it shouldn't: the
    /// source touches a cell gap, dead cell, the edge or a saturated pixel,
    /// so its centroid is pulled (on an IASC frame, a 133105 (2003 OG10)
    /// partly in a gap came out 1.8″ off).
    pub problem: Option<String>,
    pub flux: Flux,
    /// Gaia-G-calibrated magnitude and its 1σ, if there is a zero point.
    pub mag: Option<(f64, f64)>,
    /// Why the magnitude should not be trusted, if it shouldn't.
    pub mag_problem: Option<String>,
}

impl Measurement {
    pub fn snr(&self) -> f64 {
        self.flux.snr()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarkError {
    /// No source found at the mark (too faint, or the click missed).
    NoSource,
    /// The mark is off the image.
    Outside,
}

impl std::fmt::Display for MarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkError::NoSource => write!(f, "no source could be measured at the mark"),
            MarkError::Outside => write!(f, "the mark is off the image"),
        }
    }
}

/// Everything reduced on one frame.
#[derive(Debug, Clone)]
pub struct Reduction {
    /// Mid-exposure UTC Julian date.
    pub mid_jd: Option<f64>,
    pub exposure: Option<f64>,
    /// Stars detected.
    pub detected: usize,
    /// Median star FWHM, pixels.
    pub fwhm: f64,
    /// Aperture radius, pixels.
    pub aperture: f64,
    /// `None` when no catalogue was given (preview).
    pub solution: Option<Result<PlateSolution, SolveError>>,
    pub zero_point: Option<ZeroPoint>,
    /// Pixel scale, arcseconds (solution's, else the header's).
    pub scale: Option<f64>,
    /// One per mark, in the order given.
    pub marks: Vec<Result<Measurement, MarkError>>,
}

impl Reduction {
    pub fn plate(&self) -> Option<&PlateSolution> {
        self.solution.as_ref()?.as_ref().ok()
    }
}

/// Saturation level (ADU) from the header: `SATURATE`, else 95 % of the
/// largest value 16-bit unsigned data can hold.
pub fn saturation(header: &Header) -> Option<f64> {
    header
        .float("SATURATE")
        .or_else(|| header.float("SATURATN"))
        .or_else(|| {
            (header.int("BITPIX") == Some(16) && header.float("BZERO") == Some(32768.0))
                .then_some(0.95 * 65535.0)
        })
}

/// Reduces `image`: detects stars, solves the plate against `catalog` (if
/// given), calibrates magnitudes and measures each of `marks` (0-based
/// pixels).
pub fn reduce(image: &Image, catalog: Option<&[GaiaStar]>, marks: &[(f64, f64)]) -> Reduction {
    let header = &image.header;
    let saturation = saturation(header);
    let gain = header.float("GAIN").filter(|g| *g > 0.0);
    let mid_jd = time::mid_exposure_jd(header);
    let guess = Wcs::from_header(header);
    let options = DetectOptions {
        saturation: saturation.map(|s| s as f32),
        ..DetectOptions::default()
    };
    let stars = detect::detect(image, &options);
    let fwhm = median_fwhm(&stars);

    let solution = catalog.map(|cat| {
        let guess = guess.as_ref().ok_or(SolveError::NoWcs)?;
        let jd = mid_jd.unwrap_or(crate::gaia::EPOCH_JD);
        let refs: Vec<RefStar> = cat
            .iter()
            .map(|s| {
                let (ra, dec) = s.position_at(jd);
                RefStar { ra, dec, mag: s.g }
            })
            .collect();
        plate::solve(&stars, image.width, image.height, &refs, guess)
    });
    let plate = solution.as_ref().and_then(|s| s.as_ref().ok());
    let scale = plate
        .map(PlateSolution::pixel_scale)
        .or_else(|| guess.as_ref().map(Wcs::pixel_scale));

    let aperture = (APERTURE_FWHM * fwhm).max(2.5);
    let (inner, outer) = (
        (SKY_INNER_FWHM * fwhm).max(8.0),
        (SKY_OUTER_FWHM * fwhm).max(14.0),
    );
    let measure_flux = |x: f64, y: f64| -> Option<Flux> {
        let sky = photometry::local_sky(image, x, y, inner, outer)?;
        Some(photometry::aperture(
            image, x, y, aperture, &sky, gain, saturation,
        ))
    };

    let zero_point = plate.and_then(|p| {
        let samples: Vec<(Flux, f64)> = p
            .matched
            .iter()
            .filter(|m| m.used)
            .filter_map(|m| Some((measure_flux(m.x, m.y)?, m.mag)))
            .collect();
        photometry::zero_point(&samples)
    });

    let psf_sigma = fwhm / 2.354_820_045;
    let marks = marks
        .iter()
        .map(|&(mx, my)| {
            let inside =
                (0.0..image.width as f64).contains(&mx) && (0.0..image.height as f64).contains(&my);
            if !inside {
                return Err(MarkError::Outside);
            }
            let sky =
                photometry::local_sky(image, mx, my, inner, outer).ok_or(MarkError::NoSource)?;
            let c = detect::centroid(image, (mx, my), psf_sigma, sky.level, sky.sigma, gain)
                .ok_or(MarkError::NoSource)?;
            let flux = photometry::aperture(image, c.x, c.y, aperture, &sky, gain, saturation);
            if flux.flux <= 0.0 {
                return Err(MarkError::NoSource);
            }
            let (ra, dec, sigma, solved) = match (plate, &guess) {
                (Some(p), _) => {
                    let (ra, dec) = p.pixel_to_sky(c.x, c.y);
                    let s = p.pixel_scale();
                    let centroid = (c.sigma_x.powi(2) + c.sigma_y.powi(2)).sqrt()
                        / std::f64::consts::SQRT_2
                        * s;
                    let (mx, my) = p.model_sigma(c.x, c.y);
                    (
                        ra,
                        dec,
                        Some((centroid.hypot(mx), centroid.hypot(my))),
                        true,
                    )
                }
                (None, Some(w)) => {
                    let (ra, dec) = w.pixel_to_sky(c.x, c.y);
                    (ra, dec, None, false)
                }
                (None, None) => return Err(MarkError::Outside),
            };
            let mag = zero_point.as_ref().and_then(|z| z.magnitude(&flux));
            let mag_problem = match &zero_point {
                None => Some("no magnitude calibration".to_owned()),
                Some(z) => z.problem().or_else(|| {
                    if !flux.clean {
                        Some("it touches a gap between cells, the edge or a saturated pixel".into())
                    } else if flux.snr() < MIN_MAG_SNR {
                        Some(format!("too faint to measure well (S/N {:.1})", flux.snr()))
                    } else {
                        None
                    }
                }),
            };
            Ok(Measurement {
                x: c.x,
                y: c.y,
                sigma_px: (c.sigma_x, c.sigma_y),
                ra,
                dec,
                sigma,
                solved,
                moved: (c.x - mx).hypot(c.y - my),
                problem: (!flux.clean).then(|| {
                    "it touches a gap between cells, the edge or a saturated pixel".to_owned()
                }),
                flux,
                mag,
                mag_problem,
            })
        })
        .collect();

    Reduction {
        mid_jd,
        exposure: header.float("EXPTIME"),
        detected: stars.len(),
        fwhm,
        aperture,
        solution,
        zero_point,
        scale,
        marks,
    }
}

/// Median FWHM of well-measured, unsaturated stars (the brightest 200,
/// leaving out galaxies and blends twice the typical width).
fn median_fwhm(stars: &[Star]) -> f64 {
    let mut w: Vec<f64> = stars
        .iter()
        .filter(|s| !s.saturated && s.pixels >= 9)
        .take(200)
        .map(|s| s.fwhm)
        .filter(|f| f.is_finite() && *f > 0.5)
        .collect();
    if w.is_empty() {
        return 4.0;
    }
    w.sort_by(f64::total_cmp);
    let m = w[w.len() / 2];
    w.retain(|f| *f < 2.0 * m);
    w[w.len() / 2]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fits::tests::make_fits;
    use crate::plate::tests::uniform;

    pub(crate) fn normal(seed: &mut u64) -> f64 {
        let (u1, u2) = (uniform(seed).max(1e-300), uniform(seed));
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub(crate) const W: usize = 900;
    /// Instrumental zero point of the synthetic frames: G = ZP − 2.5 log₁₀ flux.
    pub(crate) const ZP: f64 = 27.0;
    const PSF_SIGMA: f64 = 2.0;
    const SKY: f64 = 1000.0;
    const NOISE: f64 = 12.0;

    /// The true mapping of the synthetic frames: TAN about (21.28, 4.13)
    /// at 0.257″/px, rotated 0.4°, with a quadratic + cubic distortion of
    /// up to ~2 px at the corners.
    pub(crate) struct Sky {
        pub wcs: Wcs,
    }

    impl Sky {
        pub(crate) fn new() -> Sky {
            let (s, c) = 0.4f64.to_radians().sin_cos();
            let k = 0.257 / 3600.0;
            Sky {
                wcs: Wcs {
                    crpix: [450.5, 450.5],
                    crval: [21.28, 4.13],
                    cd: [[-k * c, k * s], [k * s, k * c]],
                },
            }
        }
        fn distortion(x: f64, y: f64) -> (f64, f64) {
            let (u, v) = ((x - 449.5) / 450.0, (y - 449.5) / 450.0);
            (
                1.2 * u * u - 0.8 * u * v + 0.9 * u * u * u,
                -v * v + 0.6 * u * u + 0.7 * v * v * v,
            )
        }
        pub(crate) fn pixel_to_sky(&self, x: f64, y: f64) -> (f64, f64) {
            let (dx, dy) = Sky::distortion(x, y);
            self.wcs.pixel_to_sky(x + dx, y + dy)
        }
        pub(crate) fn sky_to_pixel(&self, ra: f64, dec: f64) -> (f64, f64) {
            let (px, py) = self.wcs.sky_to_pixel(ra, dec).unwrap();
            let (mut x, mut y) = (px, py);
            for _ in 0..60 {
                let (dx, dy) = Sky::distortion(x, y);
                (x, y) = (px - dx, py - dy);
            }
            (x, y)
        }
        /// Header WCS as the frame carries it: no distortion, ~2″ off.
        fn header_cards(&self) -> Vec<String> {
            let w = &self.wcs;
            vec![
                "CTYPE1  = 'RA---TAN'".into(),
                "CTYPE2  = 'DEC--TAN'".into(),
                format!("CRPIX1  = {:>20.6}", w.crpix[0] + 5.0),
                format!("CRPIX2  = {:>20.6}", w.crpix[1] - 4.0),
                format!("CRVAL1  = {:>20.10}", w.crval[0]),
                format!("CRVAL2  = {:>20.10}", w.crval[1]),
                format!("CD1_1   = {:>20.12E}", w.cd[0][0]),
                format!("CD1_2   = {:>20.12E}", w.cd[0][1]),
                format!("CD2_1   = {:>20.12E}", w.cd[1][0]),
                format!("CD2_2   = {:>20.12E}", w.cd[1][1]),
            ]
        }
    }

    /// Gaia-like catalogue: 2016.0 positions with proper motions of up to
    /// ±60 mas/yr (0.6″ by 2026, so ignoring them would show), G 13–20.5.
    pub(crate) fn catalogue(sky: &Sky, seed: &mut u64) -> Vec<GaiaStar> {
        (0..170)
            .map(|i| {
                let (x, y) = (-40.0 + 980.0 * uniform(seed), -40.0 + 980.0 * uniform(seed));
                let (ra, dec) = sky.pixel_to_sky(x, y);
                GaiaStar {
                    source_id: i,
                    ra,
                    dec,
                    pm: Some((120.0 * uniform(seed) - 60.0, 120.0 * uniform(seed) - 60.0)),
                    g: 13.0 + 7.5 * uniform(seed),
                    bp_rp: Some(1.0),
                }
            })
            .collect()
    }

    fn add_star(data: &mut [f32], x: f64, y: f64, total: f64) {
        let r = (6.0 * PSF_SIGMA) as isize;
        let norm = total / (std::f64::consts::TAU * PSF_SIGMA * PSF_SIGMA);
        for py in (y as isize - r).max(0)..=(y as isize + r).min(W as isize - 1) {
            for px in (x as isize - r).max(0)..=(x as isize + r).min(W as isize - 1) {
                let d2 = (px as f64 - x).powi(2) + (py as f64 - y).powi(2);
                data[py as usize * W + px as usize] +=
                    (norm * (-d2 / (2.0 * PSF_SIGMA * PSF_SIGMA)).exp()) as f32;
            }
        }
    }

    /// One frame at `date_obs` (45 s): the catalogue's stars where they are
    /// at that time, plus `extra` sources (x, y, G).
    pub(crate) fn frame(
        sky: &Sky,
        cat: &[GaiaStar],
        date_obs: &str,
        extra: &[(f64, f64, f64)],
        seed: &mut u64,
    ) -> Image {
        let mut data: Vec<f32> = (0..W * W)
            .map(|_| (SKY + NOISE * normal(seed)) as f32)
            .collect();
        let jd = time::parse_iso(date_obs).unwrap() + 22.5 / 86_400.0;
        for s in cat {
            let (ra, dec) = s.position_at(jd);
            let (x, y) = sky.sky_to_pixel(ra, dec);
            add_star(&mut data, x, y, 10f64.powf(-0.4 * (s.g - ZP)));
        }
        for &(x, y, g) in extra {
            add_star(&mut data, x, y, 10f64.powf(-0.4 * (g - ZP)));
        }
        let mut cards = sky.header_cards();
        cards.push(format!("DATE-OBS= '{date_obs}'"));
        cards.push("EXPTIME =                  45.".into());
        let cards: Vec<&str> = cards.iter().map(String::as_str).collect();
        let mut image = crate::fits::parse(&make_fits(1, 1, &cards, &[0.0])).unwrap();
        image.width = W;
        image.height = W;
        image.data = data;
        image
    }

    /// A moving object with a known track: positions recovered over many
    /// noise realisations are unbiased, and their scatter is what the
    /// reported uncertainty says; magnitudes likewise.
    #[test]
    fn recovers_a_moving_object() {
        let sky = Sky::new();
        let times = [
            "2026-10-04T10:15:39.8",
            "2026-10-04T10:31:00.0",
            "2026-10-04T10:46:20.0",
        ];
        // The object: 0.9″/min towards the south-west, G = 19.
        let start = sky.pixel_to_sky(300.3, 610.7);
        let at = |t: &str| {
            let minutes =
                (time::parse_iso(t).unwrap() - time::parse_iso(times[0]).unwrap()) * 1440.0 + 0.375;
            let d = 0.9 * minutes / 3600.0 / 2f64.sqrt();
            (start.0 - d / start.1.to_radians().cos(), start.1 - d)
        };
        let (mut chi2, mut n, mut bias_ra, mut bias_dec) = (0.0, 0.0, 0.0, 0.0);
        let (mut mag_chi2, mut mag_bias) = (0.0, 0.0);
        let mut worst_rms: f64 = 0.0;
        for r in 0..12u64 {
            let mut seed = 77 + 1000 * r;
            // No star right on the track: a blend is a different test.
            let track: Vec<(f64, f64)> = times
                .iter()
                .map(|t| {
                    let (ra, dec) = at(t);
                    sky.sky_to_pixel(ra, dec)
                })
                .collect();
            let mut cat = catalogue(&sky, &mut seed);
            cat.retain(|s| {
                let (x, y) = sky.sky_to_pixel(s.ra, s.dec);
                track.iter().all(|p| (p.0 - x).hypot(p.1 - y) > 15.0)
            });
            for t in times {
                let (ra, dec) = at(t);
                let (x, y) = sky.sky_to_pixel(ra, dec);
                let image = frame(&sky, &cat, t, &[(x, y, 19.0)], &mut seed);
                // The student clicked a little off.
                let red = reduce(&image, Some(&cat), &[(x + 1.2, y - 0.9)]);
                let sol = red.plate().expect("solved");
                assert!(sol.problem().is_none(), "{:?}", sol.problem());
                worst_rms = worst_rms.max(sol.rms.0.max(sol.rms.1));
                let m = red.marks[0].as_ref().unwrap();
                assert!(m.solved);
                let (sr, sd) = m.sigma.unwrap();
                let er = (m.ra - ra) * dec.to_radians().cos() * 3600.0;
                let ed = (m.dec - dec) * 3600.0;
                chi2 += (er / sr).powi(2) + (ed / sd).powi(2);
                bias_ra += er;
                bias_dec += ed;
                n += 1.0;
                let (mag, mag_err) = m.mag.unwrap();
                assert!(m.mag_problem.is_none(), "{:?}", m.mag_problem);
                mag_chi2 += ((mag - 19.0) / mag_err).powi(2);
                mag_bias += mag - 19.0;
                // The aperture holds 1 − exp(−r²/2σ²) of a Gaussian star's light.
                let zp = red.zero_point.unwrap();
                let enclosed = 1.0 - (-red.aperture.powi(2) / (2.0 * PSF_SIGMA * PSF_SIGMA)).exp();
                let expected = ZP + 2.5 * enclosed.log10();
                assert!(
                    (zp.zp - expected).abs() < 0.02,
                    "zp {} expected {expected}",
                    zp.zp
                );
            }
        }
        let chi = (chi2 / (2.0 * n)).sqrt();
        let mag_chi = (mag_chi2 / n).sqrt();
        eprintln!(
            "moving object, {n} frames: position error/predicted {chi:.3}, bias {:+.4}″ {:+.4}″, \
             worst plate rms {worst_rms:.3}″; magnitude error/predicted {mag_chi:.3}, bias {:+.4}",
            bias_ra / n,
            bias_dec / n,
            mag_bias / n
        );
        assert!(
            (0.7..1.35).contains(&chi),
            "position error / predicted {chi}"
        );
        assert!(
            (bias_ra / n).abs() < 0.02 && (bias_dec / n).abs() < 0.02,
            "bias {} {}",
            bias_ra / n,
            bias_dec / n
        );
        assert!(worst_rms < 0.1, "plate rms {worst_rms}");
        assert!(
            (0.6..1.5).contains(&mag_chi),
            "magnitude error / predicted {mag_chi}"
        );
        assert!(
            (mag_bias / n).abs() < 0.03,
            "magnitude bias {}",
            mag_bias / n
        );
    }

    #[test]
    fn preview_without_catalogue_and_missed_marks() {
        let sky = Sky::new();
        let mut seed = 5;
        let cat = catalogue(&sky, &mut seed);
        let image = frame(
            &sky,
            &cat,
            "2026-10-04T10:15:39.8",
            &[(500.0, 500.0, 18.0)],
            &mut seed,
        );
        let red = reduce(&image, None, &[(500.4, 499.8), (100.0, 820.0), (-5.0, 3.0)]);
        assert!(red.solution.is_none() && red.zero_point.is_none());
        let m = red.marks[0].as_ref().unwrap();
        assert!(!m.solved && m.sigma.is_none() && m.mag.is_none());
        assert!(m.mag_problem.is_some());
        // Empty sky: the window finds nothing to converge on (or a noise
        // bump with no flux); either way no measurement.
        assert!(red.marks[1].is_err() || red.marks[1].as_ref().unwrap().snr() < 3.0);
        assert_eq!(red.marks[2], Err(MarkError::Outside));
    }
}
