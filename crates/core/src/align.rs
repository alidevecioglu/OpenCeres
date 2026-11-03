//! Matching the stars of two frames of the same field, so a blink can show
//! them on top of each other even when the telescope drifted between
//! exposures (poor guiding, re-pointing, a little field rotation).
//!
//! The method is triangle matching (Groth 1986, AJ 91, 1244; Valdes et al.
//! 1995, PASP 107, 1119): the shape of a triangle of stars — its side
//! ratios — does not change when the frame is shifted, rotated or scaled, so
//! triangles of bright stars with the same shape in both frames suggest
//! which stars correspond. Each suggestion gives a transform; the one that
//! brings the most stars together wins, and is then refined by a
//! least-squares fit to every matched star with outliers (moving objects,
//! blends, noise) clipped.
//!
//! No sky coordinates are needed, so this works on frames without a WCS or
//! with a header WCS that does not follow the guiding errors.

/// An affine map of pixel coordinates:
/// `x' = m[0] x + m[1] y + m[2]`, `y' = m[3] x + m[4] y + m[5]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub m: [f64; 6],
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    };

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.m;
        (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5])
    }

    /// `None` if the map is singular.
    pub fn inverse(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.m;
        let det = a * e - b * d;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let (ia, ib, id, ie) = (e / det, -b / det, -d / det, a / det);
        Some(Affine {
            m: [ia, ib, -(ia * c + ib * f), id, ie, -(id * c + ie * f)],
        })
    }

    /// Mean scale factor (square root of the area change).
    pub fn scale(&self) -> f64 {
        (self.m[0] * self.m[4] - self.m[1] * self.m[3]).abs().sqrt()
    }

    /// Rotation in degrees (counter-clockwise in pixel coordinates).
    pub fn rotation(&self) -> f64 {
        (self.m[3] - self.m[1])
            .atan2(self.m[0] + self.m[4])
            .to_degrees()
    }
}

/// How one frame maps onto the reference frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// Target pixel → reference pixel.
    pub transform: Affine,
    /// Stars used in the final fit.
    pub matched: usize,
    /// RMS distance (pixels) between matched stars after the fit.
    pub rms: f64,
}

/// Brightest stars used to build triangles: C(30, 3) = 4060 triangles.
const TRIANGLE_STARS: usize = 30;
/// Brightest stars used to score a candidate transform.
const SCORE_STARS: usize = 60;
/// Brightest stars used in the final fit. Fainter ones have centroids too
/// noisy to help: on Pan-STARRS frames the median match error grows from
/// 0.25 px for the brightest 50 stars to over 1 px past the 200th.
const FIT_STARS: usize = 150;
/// Side-ratio tolerance for two triangles to count as the same shape.
const SHAPE_TOLERANCE: f64 = 0.015;
/// Triangles longer than this many times their shortest side are too
/// sensitive to centroid errors.
const MAX_ELONGATION: f64 = 8.0;
/// Shortest side (pixels) of a usable triangle.
const MIN_SIDE: f64 = 8.0;
/// Distance (pixels) within which a star counts as matched while scoring.
const MATCH_RADIUS: f64 = 3.0;
/// Fewest matched stars for an alignment to be trusted.
const MIN_MATCHED: usize = 6;
/// Frames of one set may come from slightly different setups, but not from
/// a different pixel scale by more than this factor.
const MAX_SCALE_CHANGE: f64 = 1.25;

/// Finds the transform taking `target` pixel positions onto `reference`
/// ones. Both lists should be brightest first (as [`crate::detect::detect`]
/// returns them). `None` if no reliable match was found.
pub fn align(reference: &[(f64, f64)], target: &[(f64, f64)]) -> Option<Alignment> {
    let ref_tris = triangles(&reference[..reference.len().min(TRIANGLE_STARS)]);
    let mut tgt_tris = triangles(&target[..target.len().min(TRIANGLE_STARS)]);
    if ref_tris.is_empty() || tgt_tris.is_empty() {
        return None;
    }
    tgt_tris.sort_by(|a, b| a.shape[0].total_cmp(&b.shape[0]));

    let index = SpatialIndex::new(&reference[..reference.len().min(SCORE_STARS)]);
    let scored = &target[..target.len().min(SCORE_STARS)];
    // Chance coincidences put at most a few stars within the match radius,
    // so a transform that brings half of them together is the answer and
    // the search can stop.
    let convincing = (scored.len().min(SCORE_STARS) / 2).max(2 * MIN_MATCHED);
    let mut best: Option<(usize, Affine)> = None;
    'search: for r in &ref_tris {
        let lo = tgt_tris.partition_point(|t| t.shape[0] < r.shape[0] - SHAPE_TOLERANCE);
        for t in tgt_tris[lo..]
            .iter()
            .take_while(|t| t.shape[0] <= r.shape[0] + SHAPE_TOLERANCE)
        {
            if (t.shape[1] - r.shape[1]).abs() > SHAPE_TOLERANCE {
                continue;
            }
            let pairs =
                t.v.map(|i| target[i])
                    .into_iter()
                    .zip(r.v.map(|i| reference[i]));
            let Some(transform) = fit_similarity(pairs) else {
                continue;
            };
            let s = transform.scale();
            if !(1.0 / MAX_SCALE_CHANGE..=MAX_SCALE_CHANGE).contains(&s) {
                continue;
            }
            let to_beat = best.map_or(0, |(b, _)| b);
            let mut score = 0;
            for (i, &(x, y)) in scored.iter().enumerate() {
                if score + (scored.len() - i) <= to_beat {
                    break; // cannot win any more
                }
                let (x, y) = transform.apply(x, y);
                score += usize::from(index.nearest(x, y, MATCH_RADIUS).is_some());
            }
            if score > to_beat {
                best = Some((score, transform));
                if score >= convincing {
                    break 'search;
                }
            }
        }
    }
    let (score, transform) = best?;
    if score < MIN_MATCHED {
        return None;
    }
    refine(reference, target, transform)
}

/// Re-matches up to [`FIT_STARS`] stars, fits, and clips outliers until
/// the set of matches settles.
fn refine(reference: &[(f64, f64)], target: &[(f64, f64)], start: Affine) -> Option<Alignment> {
    let reference = &reference[..reference.len().min(FIT_STARS)];
    let target = &target[..target.len().min(FIT_STARS)];
    let index = SpatialIndex::new(reference);
    let mut transform = start;
    let mut radius = MATCH_RADIUS;
    let mut result = None;
    for _ in 0..6 {
        let pairs = mutual_matches(reference, target, &index, &transform, radius);
        if pairs.len() < MIN_MATCHED {
            break;
        }
        // An affine fit needs more stars to be better than a similarity.
        let fitted = if pairs.len() >= 12 {
            fit_affine(&pairs)
        } else {
            fit_similarity(pairs.iter().copied())
        };
        let Some(fitted) = fitted else { break };
        let mut dist: Vec<f64> = pairs
            .iter()
            .map(|&((x, y), (rx, ry))| {
                let (px, py) = fitted.apply(x, y);
                (px - rx).hypot(py - ry)
            })
            .collect();
        let rms = (dist.iter().map(|d| d * d).sum::<f64>() / dist.len() as f64).sqrt();
        transform = fitted;
        result = Some(Alignment {
            transform,
            matched: pairs.len(),
            rms,
        });
        // Clip on a robust scale: the median distance of a 2-D Gaussian is
        // 1.1774 σ, and 3.5 σ keeps 99.8 % of good matches. The RMS would be
        // inflated by the very outliers (bloated saturated stars, blends)
        // that should go. A floor keeps sharp, well-measured stars from
        // being clipped by chance.
        let mid = dist.len() / 2;
        let median = *dist.select_nth_unstable_by(mid, f64::total_cmp).1;
        let next = (3.5 * median / 1.1774).clamp(0.5, MATCH_RADIUS);
        if (next - radius).abs() < 0.01 {
            break;
        }
        radius = next;
    }
    result
}

/// A matched star: (target position, reference position).
type Pair = ((f64, f64), (f64, f64));

/// Pairs (target, reference) whose nearest neighbours under `t` are each
/// other, within `radius`.
fn mutual_matches(
    reference: &[(f64, f64)],
    target: &[(f64, f64)],
    index: &SpatialIndex,
    t: &Affine,
    radius: f64,
) -> Vec<Pair> {
    let mut claim: Vec<Option<(usize, f64)>> = vec![None; reference.len()];
    for (i, &(x, y)) in target.iter().enumerate() {
        let (px, py) = t.apply(x, y);
        if let Some((j, d)) = index.nearest(px, py, radius)
            && claim[j].is_none_or(|(_, best)| d < best)
        {
            claim[j] = Some((i, d));
        }
    }
    claim
        .iter()
        .enumerate()
        .filter_map(|(j, c)| c.map(|(i, _)| (target[i], reference[j])))
        .collect()
}

struct Triangle {
    /// Middle and shortest side over the longest side.
    shape: [f64; 2],
    /// Vertices opposite the longest, middle and shortest side.
    v: [usize; 3],
}

/// Every usable triangle of `stars`. Mirror images get different shapes
/// (the orientation is folded into the sign of the second ratio), so a
/// flipped frame is not matched by mistake.
fn triangles(stars: &[(f64, f64)]) -> Vec<Triangle> {
    let n = stars.len();
    let mut out = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                if let Some(t) = triangle(stars, [i, j, k]) {
                    out.push(t);
                }
            }
        }
    }
    out
}

fn triangle(stars: &[(f64, f64)], v: [usize; 3]) -> Option<Triangle> {
    let p = v.map(|i| stars[i]);
    let dist = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
    // Side opposite vertex i.
    let mut sides = [
        (dist(p[1], p[2]), v[0]),
        (dist(p[0], p[2]), v[1]),
        (dist(p[0], p[1]), v[2]),
    ];
    sides.sort_by(|a, b| b.0.total_cmp(&a.0));
    let [(a, va), (b, vb), (c, vc)] = sides;
    if c < MIN_SIDE || a > MAX_ELONGATION * c {
        return None;
    }
    let (pa, pb, pc) = (stars[va], stars[vb], stars[vc]);
    let cross = (pb.0 - pa.0) * (pc.1 - pa.1) - (pb.1 - pa.1) * (pc.0 - pa.0);
    let orientation = if cross >= 0.0 { 1.0 } else { -1.0 };
    Some(Triangle {
        shape: [b / a, orientation * c / a],
        v: [va, vb, vc],
    })
}

/// Least-squares shift, rotation and scale taking each `.0` onto its `.1`.
fn fit_similarity(pairs: impl Iterator<Item = Pair> + Clone) -> Option<Affine> {
    let n = pairs.clone().count() as f64;
    if n < 2.0 {
        return None;
    }
    let (mut sx, mut sy, mut rx, mut ry) = (0.0, 0.0, 0.0, 0.0);
    for ((x, y), (u, v)) in pairs.clone() {
        (sx, sy, rx, ry) = (sx + x, sy + y, rx + u, ry + v);
    }
    let (sx, sy, rx, ry) = (sx / n, sy / n, rx / n, ry / n);
    let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
    for ((x, y), (u, v)) in pairs {
        let (x, y, u, v) = (x - sx, y - sy, u - rx, v - ry);
        num_a += x * u + y * v;
        num_b += x * v - y * u;
        den += x * x + y * y;
    }
    if den <= 0.0 {
        return None;
    }
    let (a, b) = (num_a / den, num_b / den);
    Some(Affine {
        m: [a, -b, rx - a * sx + b * sy, b, a, ry - b * sx - a * sy],
    })
}

/// Least-squares affine map taking each `.0` onto its `.1`. Coordinates are
/// centred first so the normal equations stay well conditioned.
fn fit_affine(pairs: &[Pair]) -> Option<Affine> {
    let n = pairs.len() as f64;
    let (cx, cy) = pairs
        .iter()
        .fold((0.0, 0.0), |(a, b), &((x, y), _)| (a + x / n, b + y / n));
    let mut ata = [[0.0; 3]; 3];
    let (mut bu, mut bv) = ([0.0; 3], [0.0; 3]);
    for &((x, y), (u, v)) in pairs {
        let row = [x - cx, y - cy, 1.0];
        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += row[i] * row[j];
            }
            bu[i] += row[i] * u;
            bv[i] += row[i] * v;
        }
    }
    let [a, b, c0] = solve3(ata, bu)?;
    let [d, e, f0] = solve3(ata, bv)?;
    // Undo the centring: x' = a (x - cx) + b (y - cy) + c0.
    Some(Affine {
        m: [a, b, c0 - a * cx - b * cy, d, e, f0 - d * cx - e * cy],
    })
}

/// Cramer's rule for a symmetric 3 × 3 system.
fn solve3(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-9 || !d.is_finite() {
        return None;
    }
    let mut out = [0.0; 3];
    for (col, o) in out.iter_mut().enumerate() {
        let mut mc = m;
        for row in 0..3 {
            mc[row][col] = r[row];
        }
        *o = det(mc) / d;
    }
    Some(out)
}

/// Stars sorted by x for nearest-neighbour lookups within a small radius.
struct SpatialIndex {
    /// (x, y, index into the original list), by increasing x.
    sorted: Vec<(f64, f64, usize)>,
}

impl SpatialIndex {
    fn new(stars: &[(f64, f64)]) -> Self {
        let mut sorted: Vec<_> = stars
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| (x, y, i))
            .collect();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        SpatialIndex { sorted }
    }

    /// Nearest star within `radius` and its distance.
    fn nearest(&self, x: f64, y: f64, radius: f64) -> Option<(usize, f64)> {
        let lo = self.sorted.partition_point(|s| s.0 < x - radius);
        let mut best: Option<(usize, f64)> = None;
        for &(sx, sy, i) in self.sorted[lo..].iter().take_while(|s| s.0 <= x + radius) {
            let d = (sx - x).hypot(sy - y);
            if d <= radius && best.is_none_or(|(_, b)| d < b) {
                best = Some((i, d));
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator (xorshift64*), so tests need no crates.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        }
        fn gauss(&mut self) -> f64 {
            let (u, v) = (self.next().max(1e-300), self.next());
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
        }
    }

    const SIZE: f64 = 2400.0;

    fn field(rng: &mut Rng, n: usize) -> Vec<(f64, f64)> {
        (0..n)
            .map(|_| (rng.next() * SIZE, rng.next() * SIZE))
            .collect()
    }

    /// The same sky seen through `truth` (reference → target), with
    /// centroid noise, stars lost off the edge, some missing (noise, brightness
    /// order changes) and some extra sources (hot pixels, moving objects).
    fn observe(
        rng: &mut Rng,
        sky: &[(f64, f64)],
        truth: &Affine,
        noise: f64,
        drop: f64,
        extra: usize,
    ) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for &(x, y) in sky {
            let (x, y) = truth.apply(x, y);
            let inside = (0.0..SIZE).contains(&x) && (0.0..SIZE).contains(&y);
            if inside && rng.next() >= drop {
                out.push((x + noise * rng.gauss(), y + noise * rng.gauss()));
            }
        }
        for _ in 0..extra {
            let at = (rng.next() * out.len() as f64) as usize;
            out.insert(at, (rng.next() * SIZE, rng.next() * SIZE));
        }
        out
    }

    fn rotation(deg: f64, scale: f64, tx: f64, ty: f64) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        Affine {
            m: [scale * c, -scale * s, tx, scale * s, scale * c, ty],
        }
    }

    /// Distances between where `found` and the truth put the image's four
    /// corners and its centre.
    fn errors(found: &Affine, truth_inverse: &Affine) -> [f64; 5] {
        [
            (0.0, 0.0),
            (SIZE, 0.0),
            (0.0, SIZE),
            (SIZE, SIZE),
            (1200.0, 1200.0),
        ]
        .map(|(x, y)| {
            let (a, b) = found.apply(x, y);
            let (c, d) = truth_inverse.apply(x, y);
            (a - c).hypot(b - d)
        })
    }

    fn worst_error(found: &Affine, truth_inverse: &Affine) -> f64 {
        errors(found, truth_inverse).into_iter().fold(0.0, f64::max)
    }

    #[test]
    fn affine_inverse_round_trips() {
        let t = rotation(17.0, 1.03, 40.0, -12.5);
        let inv = t.inverse().unwrap();
        let (x, y) = t.apply(321.0, 1234.0);
        let (u, v) = inv.apply(x, y);
        assert!((u - 321.0).abs() < 1e-9 && (v - 1234.0).abs() < 1e-9);
        assert!((t.rotation() - 17.0).abs() < 1e-9);
        assert!((t.scale() - 1.03).abs() < 1e-9);
    }

    /// Guiding drift: shifts of tens of pixels and a small rotation, typical
    /// centroid noise, a third of the stars missing and spurious sources.
    /// Over many random fields the transform must be found every time.
    ///
    /// Accuracy: with σ = 0.15 px per axis in each frame, a matched pair
    /// differs by σ√2 ≈ 0.21 px per axis. An affine fit to n stars spread
    /// over the image is off at its centre by about 0.21·√(1/n) per axis and
    /// at a corner by 0.21·√(7/n) (a corner lies √3 standard deviations of
    /// the star positions out on each axis). Here n ≈ 95 of the
    /// [`FIT_STARS`] brightest are matched, so the expected RMS 2-D error
    /// over the four corners and the centre is ≈ 0.074 px.
    #[test]
    fn recovers_guiding_drift_over_many_fields() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let (mut sum_sq, mut count) = (0.0, 0);
        for trial in 0..60 {
            let sky = field(&mut rng, 350);
            let reference = observe(&mut rng, &sky, &Affine::IDENTITY, 0.15, 0.15, 5);
            let truth = rotation(
                (rng.next() - 0.5) * 1.0,
                1.0,
                (rng.next() - 0.5) * 120.0,
                (rng.next() - 0.5) * 120.0,
            );
            let target = observe(&mut rng, &sky, &truth, 0.15, 0.3, 10);
            let found =
                align(&reference, &target).unwrap_or_else(|| panic!("trial {trial}: no alignment"));
            assert!(found.matched >= 60, "trial {trial}: {found:?}");
            for e in errors(&found.transform, &truth.inverse().unwrap()) {
                sum_sq += e * e;
                count += 1;
            }
        }
        let rms = (sum_sq / f64::from(count)).sqrt();
        assert!(rms < 0.09, "rms error {rms:.3} px");
    }

    /// Large rotations (a meridian flip turns the field by 180°) and a slight
    /// scale change are matched too.
    #[test]
    fn recovers_large_rotation() {
        let mut rng = Rng(42);
        for deg in [30.0, 90.0, 180.0, -135.0] {
            let sky = field(&mut rng, 300);
            let reference = observe(&mut rng, &sky, &Affine::IDENTITY, 0.1, 0.1, 0);
            // Rotate about the centre so most stars stay on the image.
            let r = rotation(deg, 1.01, 0.0, 0.0);
            let (cx, cy) = r.apply(1200.0, 1200.0);
            let truth = Affine {
                m: [r.m[0], r.m[1], 1200.0 - cx, r.m[3], r.m[4], 1200.0 - cy],
            };
            let target = observe(&mut rng, &sky, &truth, 0.1, 0.1, 0);
            let found = align(&reference, &target).expect("aligned");
            let err = worst_error(&found.transform, &truth.inverse().unwrap());
            assert!(err < 0.1, "{deg}°: error {err:.3} px");
        }
    }

    /// Unrelated fields must not be "aligned".
    #[test]
    fn rejects_different_fields() {
        let mut rng = Rng(7);
        for _ in 0..20 {
            let a = field(&mut rng, 300);
            let b = field(&mut rng, 300);
            assert!(align(&a, &b).is_none());
        }
    }

    #[test]
    fn too_few_stars_is_none() {
        let few = [(10.0, 10.0), (500.0, 40.0)];
        assert!(align(&few, &few).is_none());
    }
}
