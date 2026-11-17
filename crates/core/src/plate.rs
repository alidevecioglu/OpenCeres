//! Plate solutions: where each pixel is on the sky, fitted to Gaia stars in
//! the frame itself, so reported positions do not inherit the header WCS's
//! errors (1.5–6″ on IASC's Pan-STARRS chips, see [`crate::wcs`]).
//!
//! The model is the classical one of photographic astrometry ("plate
//! constants", e.g. Green, *Spherical Astronomy*, ch. 13; König 1962): the
//! catalogue stars' standard coordinates (ξ, η) — their gnomonic projection
//! about a tangent point, Calabretta & Greisen 2002 eq. 54 — are a
//! polynomial in the pixel coordinates. Order 1 is the six-constant affine
//! plate; orders 2 and 3 take up optical distortion, as SIP does for FITS
//! (Shupe et al. 2005). Folding the CD matrix into the polynomial, as here,
//! describes the same family of mappings.
//!
//! The order is chosen per frame from the data by cross-validation: a
//! higher order is used only if it predicts *left-out* stars better, so
//! the fit never chases noise in a sparse field.
//!
//! Matching starts from the header WCS: the offset that brings the most
//! catalogue stars onto detections (a vote over all pairs) is found first,
//! falling back to triangle matching ([`crate::align`]) when the header is
//! rotated or scaled wrongly too; matches are then refined by fitting and
//! re-matching with a shrinking radius.

use crate::align;
use crate::detect::Star;
use crate::wcs::Wcs;

/// A catalogue star at the observation epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefStar {
    pub ra: f64,
    pub dec: f64,
    pub mag: f64,
}

/// Fewest matched stars for any solution: an order-1 plate has 3
/// constants per axis, and a few more are needed to see outliers.
pub const MIN_STARS: usize = 8;
/// Highest polynomial order tried.
pub const MAX_ORDER: usize = 3;
/// A solution with a larger per-axis RMS (arcseconds) is flagged as poor.
/// IASC's Pan-STARRS chips fit Gaia DR3 to 0.02–0.03″ with ~45 stars (an
/// order-2 plate); 0.4″ means something is wrong (bad matches, trailed
/// stars, bad seeing) and positions would be no better than a few tenths.
pub const MAX_RMS: f64 = 0.4;
/// Largest header-WCS error the offset vote searches, arcseconds.
const MAX_SHIFT_ARCSEC: f64 = 90.0;
/// Brightest detections and catalogue stars that vote on the offset.
const VOTERS: usize = 120;
/// Detections considered for matching, brightest first.
const MAX_DETECTIONS: usize = 600;
/// Outliers beyond this many sigmas (radial, 2-D) are left out of a fit.
const CLIP: f64 = 3.5;

#[derive(Debug, Clone, PartialEq)]
pub enum SolveError {
    /// The frame has no header WCS to start from.
    NoWcs,
    /// Too few catalogue stars fall on the frame (e.g. a cloudy frame or an
    /// empty field).
    TooFewReference { found: usize, needed: usize },
    /// No consistent offset/rotation between catalogue and detections.
    NoMatch,
    /// Matched, but fewer stars than a fit needs.
    TooFewStars { matched: usize, needed: usize },
    /// The least-squares system was singular (stars all on a line).
    Singular,
}

impl std::fmt::Display for SolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SolveError::NoWcs => write!(f, "no WCS in the header to start from"),
            SolveError::TooFewReference { found, needed } => write!(
                f,
                "only {found} Gaia stars fall on the frame ({needed} needed)"
            ),
            SolveError::NoMatch => write!(f, "the stars could not be matched to Gaia"),
            SolveError::TooFewStars { matched, needed } => {
                write!(f, "{matched} stars matched to Gaia, {needed} needed")
            }
            SolveError::Singular => write!(f, "the matched stars do not constrain a fit"),
        }
    }
}

/// One matched star: detection and catalogue entry, and whether the final
/// fit used it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matched {
    pub x: f64,
    pub y: f64,
    pub ra: f64,
    pub dec: f64,
    pub mag: f64,
    /// Fit residual (catalogue − fit) in ξ and η, arcseconds.
    pub residual: (f64, f64),
    pub used: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlateSolution {
    /// Tangent point (RA, Dec), degrees.
    pub tangent: (f64, f64),
    /// Pixel origin of the polynomial and its normalisation (pixels per
    /// unit), keeping the normal equations well conditioned.
    origin: (f64, f64),
    norm: f64,
    pub order: usize,
    /// Coefficients of ξ and η (arcseconds) for [`terms`] of `order`.
    xi: Vec<f64>,
    eta: Vec<f64>,
    /// (AᵀA)⁻¹ of the final fit: the coefficients' covariance per unit
    /// variance, for the model's uncertainty at a point.
    cov: Vec<f64>,
    /// Stars used in the final fit.
    pub stars: usize,
    /// RMS of the fit residuals in RA (×cos Dec) and Dec, arcseconds,
    /// corrected for the fitted degrees of freedom.
    pub rms: (f64, f64),
    /// Cross-validated RMS (arcseconds, both axes) of each order tried,
    /// index 0 = order 1; `None` where there were too few stars.
    pub cv_rms: [Option<f64>; MAX_ORDER],
    pub matched: Vec<Matched>,
}

impl PlateSolution {
    /// 0-based pixel to (RA, Dec) in degrees.
    pub fn pixel_to_sky(&self, x: f64, y: f64) -> (f64, f64) {
        let (xi, eta) = self.standard(x, y);
        deproject(self.tangent, xi, eta)
    }

    /// (RA, Dec) in degrees to 0-based pixel, by Newton's method on the
    /// polynomial; `None` on the far side of the sky or if it diverges.
    pub fn sky_to_pixel(&self, ra: f64, dec: f64) -> Option<(f64, f64)> {
        let (xi, eta) = project(self.tangent, ra, dec)?;
        let (mut u, mut v) = (0.0, 0.0);
        for _ in 0..30 {
            let t = terms(self.order, u, v);
            let (du_t, dv_t) = term_derivatives(self.order, u, v);
            let f = (dot(&self.xi, &t) - xi, dot(&self.eta, &t) - eta);
            let (a, b) = (dot(&self.xi, &du_t), dot(&self.xi, &dv_t));
            let (c, d) = (dot(&self.eta, &du_t), dot(&self.eta, &dv_t));
            let det = a * d - b * c;
            if det == 0.0 || !det.is_finite() {
                return None;
            }
            let (su, sv) = ((d * f.0 - b * f.1) / det, (-c * f.0 + a * f.1) / det);
            u -= su;
            v -= sv;
            if su.hypot(sv) * self.norm < 1e-9 {
                return Some((self.origin.0 + u * self.norm, self.origin.1 + v * self.norm));
            }
            if u.abs() > 50.0 || v.abs() > 50.0 {
                return None;
            }
        }
        None
    }

    /// Standard coordinates (ξ, η) of a pixel, arcseconds.
    pub fn standard(&self, x: f64, y: f64) -> (f64, f64) {
        let t = terms(
            self.order,
            (x - self.origin.0) / self.norm,
            (y - self.origin.1) / self.norm,
        );
        (dot(&self.xi, &t), dot(&self.eta, &t))
    }

    /// 1σ uncertainty (RA ×cos Dec, Dec; arcseconds) of the fitted mapping
    /// itself at pixel (x, y): what the reference stars' scatter leaves
    /// uncertain in the plate constants, √(tᵀ (AᵀA)⁻¹ t) times the RMS.
    pub fn model_sigma(&self, x: f64, y: f64) -> (f64, f64) {
        let t = terms(
            self.order,
            (x - self.origin.0) / self.norm,
            (y - self.origin.1) / self.norm,
        );
        let p = t.len();
        let mut q = 0.0;
        for i in 0..p {
            for j in 0..p {
                q += t[i] * self.cov[i * p + j] * t[j];
            }
        }
        let s = q.max(0.0).sqrt();
        (self.rms.0 * s, self.rms.1 * s)
    }

    /// Mean pixel scale, arcseconds per pixel.
    pub fn pixel_scale(&self) -> f64 {
        let (a, b, c, d) = (self.xi[1], self.xi[2], self.eta[1], self.eta[2]);
        (a * d - b * c).abs().sqrt() / self.norm
    }

    /// Why the solution should not be trusted for a report, if it shouldn't.
    pub fn problem(&self) -> Option<String> {
        let worst = self.rms.0.max(self.rms.1);
        if self.stars < MIN_STARS {
            Some(format!("only {} stars in the fit", self.stars))
        } else if worst > MAX_RMS {
            Some(format!(
                "the fit to Gaia is poor: {worst:.2}″ RMS (at most {MAX_RMS}″ expected)"
            ))
        } else {
            None
        }
    }
}

/// Gnomonic projection of (`ra`, `dec`) about `tangent` (degrees), giving
/// standard coordinates in arcseconds; `None` beyond 90° from the tangent.
pub fn project(tangent: (f64, f64), ra: f64, dec: f64) -> Option<(f64, f64)> {
    let (ra0, dec0) = (tangent.0.to_radians(), tangent.1.to_radians());
    let (ra, dec) = (ra.to_radians(), dec.to_radians());
    let dra = ra - ra0;
    let cos_c = dec0.sin() * dec.sin() + dec0.cos() * dec.cos() * dra.cos();
    if cos_c <= 0.0 {
        return None;
    }
    let xi = dec.cos() * dra.sin() / cos_c;
    let eta = (dec0.cos() * dec.sin() - dec0.sin() * dec.cos() * dra.cos()) / cos_c;
    let to_arcsec = 180.0 / std::f64::consts::PI * 3600.0;
    Some((xi * to_arcsec, eta * to_arcsec))
}

/// Inverse of [`project`]: standard coordinates (arcseconds) to (RA, Dec)
/// in degrees, RA in [0, 360).
pub fn deproject(tangent: (f64, f64), xi: f64, eta: f64) -> (f64, f64) {
    let to_rad = std::f64::consts::PI / 180.0 / 3600.0;
    let (xi, eta) = (xi * to_rad, eta * to_rad);
    let (ra0, dec0) = (tangent.0.to_radians(), tangent.1.to_radians());
    let denom = dec0.cos() - eta * dec0.sin();
    let ra = ra0 + xi.atan2(denom);
    let dec = (dec0.sin() + eta * dec0.cos()).atan2(xi.hypot(denom));
    (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
}

/// Polynomial terms uⁱvʲ with i + j ≤ order, by total degree:
/// 1, u, v, u², uv, v², u³, u²v, uv², v³.
fn terms(order: usize, u: f64, v: f64) -> Vec<f64> {
    let mut t = Vec::with_capacity(term_count(order));
    for degree in 0..=order {
        for j in 0..=degree {
            t.push(u.powi((degree - j) as i32) * v.powi(j as i32));
        }
    }
    t
}

/// ∂/∂u and ∂/∂v of [`terms`].
fn term_derivatives(order: usize, u: f64, v: f64) -> (Vec<f64>, Vec<f64>) {
    let (mut du, mut dv) = (Vec::new(), Vec::new());
    for degree in 0..=order {
        for j in 0..=degree {
            let i = degree - j;
            let pow = |b: f64, e: usize| {
                if e == 0 {
                    0.0
                } else {
                    e as f64 * b.powi(e as i32 - 1)
                }
            };
            du.push(pow(u, i) * v.powi(j as i32));
            dv.push(u.powi(i as i32) * pow(v, j));
        }
    }
    (du, dv)
}

fn term_count(order: usize) -> usize {
    (order + 1) * (order + 2) / 2
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Inverts a symmetric positive-definite `p`×`p` matrix (Gauss–Jordan with
/// partial pivoting); `None` if singular.
fn invert(mut m: Vec<f64>, p: usize) -> Option<Vec<f64>> {
    let mut inv = vec![0.0; p * p];
    for i in 0..p {
        inv[i * p + i] = 1.0;
    }
    let scale = (0..p).map(|i| m[i * p + i].abs()).fold(0.0, f64::max);
    for col in 0..p {
        let pivot =
            (col..p).max_by(|&a, &b| m[a * p + col].abs().total_cmp(&m[b * p + col].abs()))?;
        if m[pivot * p + col].abs() <= 1e-13 * scale {
            return None;
        }
        for k in 0..p {
            m.swap(col * p + k, pivot * p + k);
            inv.swap(col * p + k, pivot * p + k);
        }
        let d = m[col * p + col];
        for k in 0..p {
            m[col * p + k] /= d;
            inv[col * p + k] /= d;
        }
        for row in 0..p {
            if row != col {
                let f = m[row * p + col];
                if f != 0.0 {
                    for k in 0..p {
                        m[row * p + k] -= f * m[col * p + k];
                        inv[row * p + k] -= f * inv[col * p + k];
                    }
                }
            }
        }
    }
    Some(inv)
}

/// A catalogue star on the frame: where the header WCS puts it and its
/// standard coordinates (arcseconds) about the tangent point.
struct CatStar {
    star: RefStar,
    header: (f64, f64),
    standard: (f64, f64),
}

/// A point for the fit: normalised pixel (u, v) and standard coordinates.
#[derive(Clone, Copy)]
struct Pair {
    u: f64,
    v: f64,
    xi: f64,
    eta: f64,
}

struct Fit {
    xi: Vec<f64>,
    eta: Vec<f64>,
    cov: Vec<f64>,
}

impl Fit {
    fn residual(&self, order: usize, p: &Pair) -> (f64, f64) {
        let t = terms(order, p.u, p.v);
        (p.xi - dot(&self.xi, &t), p.eta - dot(&self.eta, &t))
    }
}

/// Unweighted least squares of ξ and η on the terms of `order`.
fn fit(order: usize, pairs: &[Pair]) -> Option<Fit> {
    let p = term_count(order);
    if pairs.len() < p {
        return None;
    }
    let mut n = vec![0.0; p * p];
    let (mut bx, mut by) = (vec![0.0; p], vec![0.0; p]);
    for pair in pairs {
        let t = terms(order, pair.u, pair.v);
        for i in 0..p {
            for j in 0..p {
                n[i * p + j] += t[i] * t[j];
            }
            bx[i] += t[i] * pair.xi;
            by[i] += t[i] * pair.eta;
        }
    }
    let cov = invert(n, p)?;
    let solve = |b: &[f64]| -> Vec<f64> {
        (0..p)
            .map(|i| (0..p).map(|j| cov[i * p + j] * b[j]).sum())
            .collect()
    };
    Some(Fit {
        xi: solve(&bx),
        eta: solve(&by),
        cov,
    })
}

/// Fits with iterative outlier rejection; returns the fit, which pairs it
/// used, and the per-axis RMS (corrected for degrees of freedom).
fn clipped_fit(order: usize, pairs: &[Pair]) -> Option<(Fit, Vec<bool>, (f64, f64))> {
    let p = term_count(order);
    let mut used = vec![true; pairs.len()];
    let mut result = None;
    for _ in 0..8 {
        let chosen: Vec<Pair> = pairs
            .iter()
            .zip(&used)
            .filter(|(_, u)| **u)
            .map(|(p, _)| *p)
            .collect();
        let f = fit(order, &chosen)?;
        let n = chosen.len();
        if n <= p {
            return None;
        }
        let (mut sx, mut sy) = (0.0, 0.0);
        for c in &chosen {
            let (rx, ry) = f.residual(order, c);
            sx += rx * rx;
            sy += ry * ry;
        }
        let dof = (n - p) as f64;
        let rms = ((sx / dof).sqrt(), (sy / dof).sqrt());
        // Radial residual against the combined sigma; a floor keeps a
        // near-perfect synthetic fit from clipping on rounding noise.
        let sigma = ((rms.0 * rms.0 + rms.1 * rms.1) / 2.0).sqrt().max(1e-4);
        let next: Vec<bool> = pairs
            .iter()
            .map(|q| {
                let (rx, ry) = f.residual(order, q);
                rx.hypot(ry) <= CLIP * sigma
            })
            .collect();
        let done = next == used;
        result = Some((f, used.clone(), rms));
        if done || next.iter().filter(|u| **u).count() <= p + 2 {
            break;
        }
        used = next;
    }
    result
}

/// Cross-validated RMS (arcseconds per axis, both axes pooled) of `order`
/// on the clipped pairs: each fifth of the stars is predicted from a fit
/// to the other four fifths.
fn cross_validate(order: usize, pairs: &[Pair]) -> Option<f64> {
    const FOLDS: usize = 5;
    let (mut sum, mut n) = (0.0, 0usize);
    for k in 0..FOLDS {
        let train: Vec<Pair> = pairs
            .iter()
            .enumerate()
            .filter(|(i, _)| i % FOLDS != k)
            .map(|(_, p)| *p)
            .collect();
        let f = fit(order, &train)?;
        for p in pairs.iter().skip(k).step_by(FOLDS) {
            let (rx, ry) = f.residual(order, p);
            sum += rx * rx + ry * ry;
            n += 2;
        }
    }
    (n > 0).then(|| (sum / n as f64).sqrt())
}

/// Highest order a fit to `n` stars may use: at least twice as many stars
/// as constants per axis, plus a few to spare for outliers.
fn max_order_for(n: usize) -> usize {
    (1..=MAX_ORDER)
        .rev()
        .find(|&o| n >= 2 * term_count(o) + 5)
        .unwrap_or(1)
}

/// Solves the plate of one frame.
///
/// `stars` are the frame's detections (brightest first, as
/// [`crate::detect::detect`] returns them); `refs` the catalogue stars at
/// the frame's epoch; `guess` the header WCS.
pub fn solve(
    stars: &[Star],
    width: usize,
    height: usize,
    refs: &[RefStar],
    guess: &Wcs,
) -> Result<PlateSolution, SolveError> {
    let (w, h) = (width as f64, height as f64);
    let centre = ((w - 1.0) / 2.0, (h - 1.0) / 2.0);
    let tangent = guess.pixel_to_sky(centre.0, centre.1);
    let scale = guess.pixel_scale();
    if !(scale.is_finite() && scale > 0.0) {
        return Err(SolveError::NoWcs);
    }

    // Detections fit for astrometry: unsaturated and star-shaped.
    let mut fwhms: Vec<f64> = stars
        .iter()
        .filter(|s| !s.saturated)
        .map(|s| s.fwhm)
        .collect();
    fwhms.sort_by(f64::total_cmp);
    let median_fwhm = fwhms.get(fwhms.len() / 2).copied().unwrap_or(0.0);
    let dets: Vec<&Star> = stars
        .iter()
        .filter(|s| !s.saturated && (median_fwhm == 0.0 || s.fwhm < 2.0 * median_fwhm))
        .take(MAX_DETECTIONS)
        .collect();

    // Catalogue stars on the frame (by the header WCS, with a margin for its error).
    let margin = MAX_SHIFT_ARCSEC / scale;
    let mut cat: Vec<CatStar> = refs
        .iter()
        .filter_map(|r| {
            let (x, y) = guess.sky_to_pixel(r.ra, r.dec)?;
            let inside = x > -margin && x < w + margin && y > -margin && y < h + margin;
            if !inside {
                return None;
            }
            Some(CatStar {
                star: *r,
                header: (x, y),
                standard: project(tangent, r.ra, r.dec)?,
            })
        })
        .collect();
    cat.sort_by(|a, b| a.star.mag.total_cmp(&b.star.mag));
    if cat.len() < MIN_STARS || dets.len() < MIN_STARS {
        return Err(SolveError::TooFewReference {
            found: cat.len().min(dets.len()),
            needed: MIN_STARS,
        });
    }

    // First guess at where each catalogue star is: header WCS plus an offset,
    // or the header WCS through a triangle-matched affine map.
    let header_px: Vec<(f64, f64)> = cat.iter().map(|c| c.header).collect();
    let initial: Vec<(f64, f64)> = match vote_offset(&dets, &header_px, margin) {
        Some((dx, dy)) => header_px.iter().map(|&(x, y)| (x + dx, y + dy)).collect(),
        None => {
            let det_xy: Vec<(f64, f64)> = dets.iter().map(|s| (s.x, s.y)).collect();
            let a = align::align(&det_xy, &header_px).ok_or(SolveError::NoMatch)?;
            header_px
                .iter()
                .map(|&(x, y)| a.transform.apply(x, y))
                .collect()
        }
    };

    let norm = w.max(h) / 2.0;
    let to_pair = |s: &Star, c: &CatStar| Pair {
        u: (s.x - centre.0) / norm,
        v: (s.y - centre.1) / norm,
        xi: c.standard.0,
        eta: c.standard.1,
    };

    // Refine: match within a radius, fit, predict again with the fit,
    // match within a smaller radius; raise the order as matches allow.
    let mut predicted = initial;
    let mut radius = (4.0 / scale).max(6.0); // 4″: the header's distortion after the offset
    let mut order = 1;
    let mut pairs_idx: Vec<(usize, usize)> = Vec::new();
    for round in 0..6 {
        pairs_idx = match_nearest(&dets, &predicted, radius);
        if pairs_idx.len() < MIN_STARS {
            return Err(SolveError::TooFewStars {
                matched: pairs_idx.len(),
                needed: MIN_STARS,
            });
        }
        let pairs: Vec<Pair> = pairs_idx
            .iter()
            .map(|&(d, c)| to_pair(dets[d], &cat[c]))
            .collect();
        order = order.min(max_order_for(pairs.len()));
        let (f, _, rms) = clipped_fit(order, &pairs).ok_or(SolveError::Singular)?;
        let sol = PlateSolution {
            tangent,
            origin: centre,
            norm,
            order,
            xi: f.xi,
            eta: f.eta,
            cov: f.cov,
            stars: 0,
            rms,
            cv_rms: [None; MAX_ORDER],
            matched: Vec::new(),
        };
        predicted = cat
            .iter()
            .map(|c| {
                sol.sky_to_pixel(c.star.ra, c.star.dec)
                    .unwrap_or((f64::NAN, f64::NAN))
            })
            .collect();
        let sigma_px = rms.0.hypot(rms.1) / scale;
        radius = (5.0 * sigma_px).clamp(1.5, radius);
        if round >= 1 {
            order = (order + 1).min(MAX_ORDER);
        }
    }

    // Final: choose the order by cross-validation on the matched stars.
    let pairs: Vec<Pair> = pairs_idx
        .iter()
        .map(|&(d, c)| to_pair(dets[d], &cat[c]))
        .collect();
    let mut cv_rms = [None; MAX_ORDER];
    let top = max_order_for(pairs.len());
    let mut best: Option<(usize, f64)> = None;
    for o in 1..=top {
        let Some((_, used, _)) = clipped_fit(o, &pairs) else {
            continue;
        };
        let kept: Vec<Pair> = pairs
            .iter()
            .zip(&used)
            .filter(|(_, u)| **u)
            .map(|(p, _)| *p)
            .collect();
        let Some(cv) = cross_validate(o, &kept) else {
            continue;
        };
        cv_rms[o - 1] = Some(cv);
        // A higher order must predict left-out stars clearly better.
        if best.is_none_or(|(_, b)| cv < 0.97 * b) {
            best = Some((o, cv));
        }
    }
    let order = best.map_or(1, |b| b.0);
    let (f, used, rms) = clipped_fit(order, &pairs).ok_or(SolveError::Singular)?;
    let matched: Vec<Matched> = pairs_idx
        .iter()
        .zip(&pairs)
        .zip(&used)
        .map(|((&(d, c), p), &u)| {
            let (rx, ry) = f.residual(order, p);
            Matched {
                x: dets[d].x,
                y: dets[d].y,
                ra: cat[c].star.ra,
                dec: cat[c].star.dec,
                mag: cat[c].star.mag,
                residual: (rx, ry),
                used: u,
            }
        })
        .collect();
    let stars = used.iter().filter(|u| **u).count();
    if stars < MIN_STARS {
        return Err(SolveError::TooFewStars {
            matched: stars,
            needed: MIN_STARS,
        });
    }
    Ok(PlateSolution {
        tangent,
        origin: centre,
        norm,
        order,
        xi: f.xi,
        eta: f.eta,
        cov: f.cov,
        stars,
        rms,
        cv_rms,
        matched,
    })
}

/// The pixel offset that brings the most bright catalogue stars onto bright
/// detections, within `max_shift` pixels; `None` if no offset stands out
/// from chance coincidences.
fn vote_offset(dets: &[&Star], cat: &[(f64, f64)], max_shift: f64) -> Option<(f64, f64)> {
    const BIN: f64 = 2.0;
    let bins = (2.0 * max_shift / BIN).ceil() as usize + 1;
    let mut votes = vec![0u32; bins * bins];
    let index = |d: f64| ((d + max_shift) / BIN).floor() as isize;
    for c in cat.iter().take(VOTERS) {
        for s in dets.iter().take(VOTERS) {
            let (dx, dy) = (s.x - c.0, s.y - c.1);
            if dx.abs() < max_shift && dy.abs() < max_shift {
                let (i, j) = (index(dx), index(dy));
                if (0..bins as isize).contains(&i) && (0..bins as isize).contains(&j) {
                    votes[j as usize * bins + i as usize] += 1;
                }
            }
        }
    }
    // Peak of the 2×2-bin sums (an offset on a bin edge splits its votes).
    let mut best = (0u32, 0usize, 0usize);
    let mut total = 0u64;
    for j in 0..bins - 1 {
        for i in 0..bins - 1 {
            let s = votes[j * bins + i]
                + votes[j * bins + i + 1]
                + votes[(j + 1) * bins + i]
                + votes[(j + 1) * bins + i + 1];
            total += u64::from(votes[j * bins + i]);
            if s > best.0 {
                best = (s, i, j);
            }
        }
    }
    // Chance pairs per 2×2 cell, on average; a real offset collects most
    // of the bright stars, so ask for well above that.
    let mean = total as f64 / ((bins - 1) * (bins - 1)) as f64 * 4.0;
    let needed = (6.0f64).max(mean + 5.0 * mean.sqrt());
    if f64::from(best.0) < needed {
        return None;
    }
    // Mean offset of the pairs in the peak.
    let (lo_x, lo_y) = (
        best.1 as f64 * BIN - max_shift,
        best.2 as f64 * BIN - max_shift,
    );
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for c in cat.iter().take(VOTERS) {
        for s in dets.iter().take(VOTERS) {
            let (dx, dy) = (s.x - c.0, s.y - c.1);
            if (lo_x..lo_x + 2.0 * BIN).contains(&dx) && (lo_y..lo_y + 2.0 * BIN).contains(&dy) {
                sx += dx;
                sy += dy;
                n += 1.0;
            }
        }
    }
    Some((sx / n, sy / n))
}

/// Pairs (detection, catalogue) that are each other's nearest within
/// `radius` pixels.
fn match_nearest(dets: &[&Star], cat: &[(f64, f64)], radius: f64) -> Vec<(usize, usize)> {
    let nearest_det = |&(x, y): &(f64, f64)| -> Option<usize> {
        dets.iter()
            .enumerate()
            .map(|(i, s)| (i, (s.x - x).hypot(s.y - y)))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let nearest_cat = |s: &Star| -> Option<usize> {
        cat.iter()
            .enumerate()
            .filter(|(_, c)| c.0.is_finite())
            .map(|(i, c)| (i, (s.x - c.0).hypot(s.y - c.1)))
            .filter(|(_, d)| *d <= radius)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    cat.iter()
        .enumerate()
        .filter(|(_, c)| c.0.is_finite())
        .filter_map(|(ci, c)| {
            let d = nearest_det(c)?;
            (nearest_cat(dets[d]) == Some(ci)).then_some((d, ci))
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Deterministic uniform numbers in [0, 1).
    pub(crate) fn uniform(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn gauss(seed: &mut u64) -> f64 {
        (0..12).map(|_| uniform(seed)).sum::<f64>() - 6.0
    }

    /// The true sky of a synthetic 2400 × 2400 frame: a TAN plate, slightly
    /// rotated, plus a smooth distortion of up to ~5″ at the corners (like
    /// a Pan-STARRS chip's header error).
    pub(crate) struct Truth {
        pub wcs: Wcs,
    }

    pub(crate) const SIZE: usize = 2400;

    impl Truth {
        pub(crate) fn new() -> Truth {
            let (s, c) = 0.3f64.to_radians().sin_cos();
            let scale = 0.257 / 3600.0;
            Truth {
                wcs: Wcs {
                    crpix: [1200.5, 1200.5],
                    crval: [21.28, 4.13],
                    cd: [[-scale * c, scale * s], [scale * s, scale * c]],
                },
            }
        }

        /// Distortion in pixels at 0-based (x, y): quadratic and cubic terms.
        pub(crate) fn distortion(x: f64, y: f64) -> (f64, f64) {
            let (u, v) = ((x - 1199.5) / 1200.0, (y - 1199.5) / 1200.0);
            (
                6.0 * u * u - 4.0 * u * v + 5.0 * u * u * u - 3.0 * u * v * v,
                -5.0 * v * v + 3.0 * u * u + 4.0 * v * v * v + 2.0 * u * u * v,
            )
        }

        pub(crate) fn pixel_to_sky(&self, x: f64, y: f64) -> (f64, f64) {
            let (dx, dy) = Truth::distortion(x, y);
            self.wcs.pixel_to_sky(x + dx, y + dy)
        }

        /// Inverse of [`Truth::pixel_to_sky`] by fixed-point iteration.
        pub(crate) fn sky_to_pixel(&self, ra: f64, dec: f64) -> (f64, f64) {
            let (px, py) = self.wcs.sky_to_pixel(ra, dec).unwrap();
            let (mut x, mut y) = (px, py);
            for _ in 0..50 {
                let (dx, dy) = Truth::distortion(x, y);
                (x, y) = (px - dx, py - dy);
            }
            (x, y)
        }

        /// The header WCS a frame would carry: the plate without the
        /// distortion, a few arcseconds off.
        pub(crate) fn header(&self) -> Wcs {
            let mut w = self.wcs.clone();
            w.crpix[0] += 9.0;
            w.crpix[1] -= 6.0;
            w
        }
    }

    /// Random catalogue stars over the frame and a margin, magnitudes 13–21.
    pub(crate) fn catalogue(truth: &Truth, n: usize, seed: &mut u64) -> Vec<RefStar> {
        (0..n)
            .map(|_| {
                let (x, y) = (
                    -100.0 + 2600.0 * uniform(seed),
                    -100.0 + 2600.0 * uniform(seed),
                );
                let (ra, dec) = truth.pixel_to_sky(x, y);
                RefStar {
                    ra,
                    dec,
                    mag: 13.0 + 8.0 * uniform(seed),
                }
            })
            .collect()
    }

    /// Detections of the catalogue stars with centroid noise `sigma_px`,
    /// some missing (gaps) and some extra (galaxies, asteroids).
    fn detections(truth: &Truth, refs: &[RefStar], sigma_px: f64, seed: &mut u64) -> Vec<Star> {
        let mut out: Vec<Star> = refs
            .iter()
            .filter_map(|r| {
                let (x, y) = truth.sky_to_pixel(r.ra, r.dec);
                let inside = (0.0..SIZE as f64).contains(&x) && (0.0..SIZE as f64).contains(&y);
                (inside && uniform(seed) > 0.1).then(|| Star {
                    x: x + sigma_px * gauss(seed),
                    y: y + sigma_px * gauss(seed),
                    flux: 10f64.powf(-0.4 * (r.mag - 25.0)),
                    peak: 0.0,
                    pixels: 20,
                    fwhm: 4.0,
                    saturated: false,
                })
            })
            .collect();
        for _ in 0..30 {
            out.push(Star {
                x: SIZE as f64 * uniform(seed),
                y: SIZE as f64 * uniform(seed),
                flux: 10f64.powf(-0.4 * (13.0 + 8.0 * uniform(seed) - 25.0)),
                peak: 0.0,
                pixels: 20,
                fwhm: 4.0,
                saturated: false,
            });
        }
        out.sort_by(|a, b| b.flux.total_cmp(&a.flux));
        out
    }

    /// Worst error (arcseconds) of a solution against the truth over a grid.
    fn worst_error(sol: &PlateSolution, truth: &Truth) -> f64 {
        let mut worst: f64 = 0.0;
        for i in 0..=10 {
            for j in 0..=10 {
                let (x, y) = (i as f64 * 239.9, j as f64 * 239.9);
                let (ra, dec) = sol.pixel_to_sky(x, y);
                let (tr, td) = truth.pixel_to_sky(x, y);
                let d = ((ra - tr) * td.to_radians().cos()).hypot(dec - td) * 3600.0;
                worst = worst.max(d);
            }
        }
        worst
    }

    #[test]
    fn projection_round_trips() {
        let t = (359.99, -30.0);
        for &(ra, dec) in &[(0.05, -29.9), (359.8, -30.1), (359.99, -30.0)] {
            let (xi, eta) = project(t, ra, dec).unwrap();
            let (r, d) = deproject(t, xi, eta);
            assert!(
                ((r - ra + 180.0).rem_euclid(360.0) - 180.0).abs() < 1e-11
                    && (d - dec).abs() < 1e-11
            );
        }
        assert!(project((0.0, 0.0), 180.0, 0.0).is_none());
    }

    #[test]
    fn header_without_distortion_is_several_arcseconds_off() {
        let truth = Truth::new();
        let header = truth.header();
        let (ra, dec) = header.pixel_to_sky(0.0, 0.0);
        let (tr, td) = truth.pixel_to_sky(0.0, 0.0);
        let err = ((ra - tr) * td.to_radians().cos()).hypot(dec - td) * 3600.0;
        assert!(err > 3.0, "{err}");
    }

    /// The solver recovers an injected distortion: with perfect centroids
    /// the solution matches the truth to milliarcseconds (the true
    /// distortion is a cubic, which order 3 represents exactly up to the
    /// TAN-vs-polynomial difference), and with realistic centroid noise
    /// to well inside the noise, over many realisations.
    #[test]
    fn recovers_distortion() {
        let truth = Truth::new();
        let header = truth.header();
        let mut seed = 7;
        let refs = catalogue(&truth, 250, &mut seed);
        let stars = detections(&truth, &refs, 0.0, &mut seed);
        let sol = solve(&stars, SIZE, SIZE, &refs, &header).unwrap();
        assert_eq!(sol.order, 3);
        let worst = worst_error(&sol, &truth);
        assert!(worst < 0.002, "worst {worst}″");
        assert!(sol.stars > 180, "{} stars", sol.stars);
        assert!(sol.problem().is_none());
        // The inverse is consistent.
        let (ra, dec) = sol.pixel_to_sky(100.0, 2300.0);
        let (x, y) = sol.sky_to_pixel(ra, dec).unwrap();
        assert!((x - 100.0).abs() < 1e-6 && (y - 2300.0).abs() < 1e-6);

        // 0.2 px (0.05″) centroid noise. The fit's RMS estimates the noise
        // (0.2 px × 0.257″), and the errors of the fitted mapping over the
        // whole frame, corners included, are as small as its own
        // uncertainty estimate says (a cubic fitted to ~120 stars is least
        // certain at the corners, ~0.1″ there).
        let (mut chi2, mut n, mut worst_all) = (0.0, 0.0, 0.0f64);
        for r in 0..40u64 {
            let mut seed = 1000 + r;
            let refs = catalogue(&truth, 160, &mut seed);
            let stars = detections(&truth, &refs, 0.2, &mut seed);
            let sol = solve(&stars, SIZE, SIZE, &refs, &header).unwrap();
            assert_eq!(sol.order, 3);
            assert!((sol.rms.0 - 0.0514).abs() < 0.015, "rms {:?}", sol.rms);
            for i in 0..=10 {
                for j in 0..=10 {
                    let (x, y) = (i as f64 * 239.9, j as f64 * 239.9);
                    let (ra, dec) = sol.pixel_to_sky(x, y);
                    let (tr, td) = truth.pixel_to_sky(x, y);
                    let e = (
                        ((ra - tr) * td.to_radians().cos()) * 3600.0,
                        (dec - td) * 3600.0,
                    );
                    let (sx, sy) = sol.model_sigma(x, y);
                    chi2 += (e.0 / sx).powi(2) + (e.1 / sy).powi(2);
                    n += 2.0;
                    worst_all = worst_all.max(e.0.hypot(e.1));
                }
            }
        }
        let chi = (chi2 / n).sqrt();
        assert!((0.8..1.25).contains(&chi), "error / predicted {chi}");
        assert!(
            worst_all < 0.3,
            "worst over grid and realisations {worst_all}″"
        );
    }

    /// The model's uncertainty estimate matches the actual error of the
    /// fitted mapping at the image centre over many realisations.
    #[test]
    fn model_sigma_is_calibrated() {
        let truth = Truth::new();
        let header = truth.header();
        let (mut sum_sq, mut sum_pred, mut n) = (0.0, 0.0, 0.0);
        for r in 0..60u64 {
            let mut seed = 5000 + r;
            let refs = catalogue(&truth, 160, &mut seed);
            let stars = detections(&truth, &refs, 0.4, &mut seed);
            let sol = solve(&stars, SIZE, SIZE, &refs, &header).unwrap();
            let (x, y) = (700.0, 1500.0);
            let (ra, _) = sol.pixel_to_sky(x, y);
            let (tr, td) = truth.pixel_to_sky(x, y);
            sum_sq += ((ra - tr) * td.to_radians().cos() * 3600.0).powi(2);
            sum_pred += sol.model_sigma(x, y).0.powi(2);
            n += 1.0;
        }
        let ratio = (sum_sq / sum_pred).sqrt();
        assert!((0.75..1.33).contains(&ratio), "actual/predicted {ratio}");
        let _ = n;
    }

    #[test]
    fn solves_when_the_header_is_rotated() {
        let truth = Truth::new();
        let mut header = truth.header();
        // 1° of rotation: 20–30 px at the edges (an offset alone matches only
        // the middle; the refinement and, failing it, triangles do the rest).
        // 0.1 px noise leaves ~0.05″ of model uncertainty at the corners.
        let (s, c) = 1f64.to_radians().sin_cos();
        let [[a, b], [cc, d]] = header.cd;
        header.cd = [
            [a * c - cc * s, b * c - d * s],
            [a * s + cc * c, b * s + d * c],
        ];
        let mut seed = 3;
        let refs = catalogue(&truth, 250, &mut seed);
        let stars = detections(&truth, &refs, 0.1, &mut seed);
        let sol = solve(&stars, SIZE, SIZE, &refs, &header).unwrap();
        let worst = worst_error(&sol, &truth);
        assert!(
            worst < 0.1,
            "worst {worst} order {} stars {} rms {:?}",
            sol.order,
            sol.stars,
            sol.rms
        );
    }

    /// A header 2′ off is beyond the offset vote; triangle matching finds it.
    #[test]
    fn falls_back_to_triangles_when_the_header_is_far_off() {
        let truth = Truth::new();
        let mut header = truth.header();
        header.crpix[0] += 470.0;
        let mut seed = 4;
        let refs = catalogue(&truth, 250, &mut seed);
        let stars = detections(&truth, &refs, 0.1, &mut seed);
        let sol = solve(&stars, SIZE, SIZE, &refs, &header).unwrap();
        let worst = worst_error(&sol, &truth);
        assert!(worst < 0.1, "worst {worst}");
    }

    #[test]
    fn reports_too_few_stars() {
        let truth = Truth::new();
        let mut seed = 9;
        let refs = catalogue(&truth, 5, &mut seed);
        let stars = detections(&truth, &refs, 0.1, &mut seed);
        assert!(matches!(
            solve(&stars, SIZE, SIZE, &refs, &truth.header()),
            Err(SolveError::TooFewReference { .. })
        ));
    }

    /// Order 1 when the plate has no distortion: higher orders must earn
    /// their place by predicting left-out stars better.
    #[test]
    fn chooses_low_order_without_distortion() {
        let truth = Truth::new();
        let mut seed = 11;
        let refs: Vec<RefStar> = (0..150)
            .map(|_| {
                let (x, y) = (2400.0 * uniform(&mut seed), 2400.0 * uniform(&mut seed));
                let (ra, dec) = truth.wcs.pixel_to_sky(x, y);
                RefStar { ra, dec, mag: 15.0 }
            })
            .collect();
        let stars: Vec<Star> = refs
            .iter()
            .map(|r| {
                let (x, y) = truth.wcs.sky_to_pixel(r.ra, r.dec).unwrap();
                Star {
                    x: x + 0.3 * gauss(&mut seed),
                    y: y + 0.3 * gauss(&mut seed),
                    flux: 1000.0,
                    peak: 0.0,
                    pixels: 20,
                    fwhm: 4.0,
                    saturated: false,
                }
            })
            .collect();
        let sol = solve(&stars, SIZE, SIZE, &refs, &truth.header()).unwrap();
        assert_eq!(sol.order, 1, "cv {:?}", sol.cv_rms);
    }
}
