//! Sky background estimation and star (source) detection.

use crate::fits::Image;

/// Sky level and noise of an image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Background {
    pub level: f32,
    pub sigma: f32,
}

/// Sigma-clipped median and robust sigma (scaled MAD) of the whole image.
///
/// Uses at most ~250k evenly spaced pixels, which is plenty for a stable
/// estimate and keeps this fast on large survey frames.
///
/// Survey images often fill cell gaps and dead cells with one constant value
/// (in IASC's Pan-STARRS frames, about a quarter of all pixels). Those carry
/// no noise and would drag the sigma estimate down, so pixels equal to all
/// four neighbours are left out; with real noise that almost never happens.
pub fn background(image: &Image) -> Background {
    background_of(&image.data, image.width, image.height)
}

fn background_of(data: &[f32], w: usize, h: usize) -> Background {
    let is_fill = |i: usize| {
        let (x, y) = (i % w, i / w);
        let v = data[i];
        x > 0
            && x + 1 < w
            && y > 0
            && y + 1 < h
            && data[i - 1] == v
            && data[i + 1] == v
            && data[i - w] == v
            && data[i + w] == v
    };
    let step = (data.len() / 250_000).max(1);
    let mut sample: Vec<f32> = (0..data.len())
        .step_by(step)
        .filter(|&i| data[i].is_finite() && !is_fill(i))
        .map(|i| data[i])
        .collect();
    if sample.is_empty() {
        return Background {
            level: 0.0,
            sigma: 0.0,
        };
    }

    let (mut level, mut sigma) = median_and_sigma(&mut sample);
    for _ in 0..5 {
        let (lo, hi) = (level - 3.0 * sigma, level + 3.0 * sigma);
        sample.retain(|v| (lo..=hi).contains(v));
        if sample.is_empty() {
            break;
        }
        let (l, s) = median_and_sigma(&mut sample);
        let converged = (l - level).abs() <= 1e-3 * sigma.max(f32::EPSILON);
        (level, sigma) = (l, s);
        if converged {
            break;
        }
    }
    Background { level, sigma }
}

fn median(values: &mut [f32]) -> f32 {
    let mid = values.len() / 2;
    *values.select_nth_unstable_by(mid, f32::total_cmp).1
}

fn median_and_sigma(values: &mut [f32]) -> (f32, f32) {
    let m = median(values);
    let mut dev: Vec<f32> = values.iter().map(|v| (v - m).abs()).collect();
    (m, 1.4826 * median(&mut dev))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Star {
    /// Flux-weighted centroid in 0-based pixel coordinates.
    pub x: f64,
    pub y: f64,
    /// Background-subtracted sum over the detected pixels.
    pub flux: f64,
    /// Highest pixel value (not background-subtracted).
    pub peak: f32,
    pub pixels: usize,
    /// Full width at half maximum from second moments, in pixels.
    pub fwhm: f64,
    pub saturated: bool,
}

#[derive(Debug, Clone)]
pub struct DetectOptions {
    /// Detection threshold, in noise sigmas of the star-matched (smoothed) image.
    pub threshold_sigma: f32,
    /// Expected star width (seeing) in pixels; sets the smoothing kernel.
    /// 0 disables smoothing and thresholds raw pixels.
    pub fwhm: f64,
    /// Smaller groups are treated as noise or hot pixels.
    pub min_pixels: usize,
    /// Pixel value at which the detector saturates, if known.
    pub saturation: Option<f32>,
}

impl Default for DetectOptions {
    fn default() -> Self {
        DetectOptions {
            threshold_sigma: 5.0,
            // ~1.1″ seeing at Pan-STARRS's 0.26″/px
            fwhm: 4.0,
            min_pixels: 5,
            saturation: None,
        }
    }
}

/// Finds sources and measures them. Results are sorted brightest first.
///
/// Detection runs on the image smoothed with a Gaussian the size of a star
/// (a matched filter): it averages away pixel noise while keeping point
/// sources, so stars and asteroids several times fainter than any single
/// pixel's noise are found. Sources are 8-connected groups of smoothed pixels
/// above the threshold; positions and fluxes are measured on the original
/// pixels.
pub fn detect(image: &Image, options: &DetectOptions) -> Vec<Star> {
    detect_with_background(image, background(image), options)
}

/// [`detect`] for callers that already computed the image's background
/// (e.g. for the display stretch), saving a second pass over the image.
pub fn detect_with_background(image: &Image, bg: Background, options: &DetectOptions) -> Vec<Star> {
    let (w, h) = (image.width, image.height);
    let smoothed =
        (options.fwhm > 0.0).then(|| gaussian_smooth(image, options.fwhm / 2.354_820_045));
    let (map, detection_bg) = match &smoothed {
        Some(s) => (s.as_slice(), background_of(s, w, h)),
        None => (image.data.as_slice(), bg),
    };
    let threshold =
        detection_bg.level + options.threshold_sigma * detection_bg.sigma.max(f32::EPSILON);

    let mut seen = vec![false; map.len()];
    let mut stack = Vec::new();
    let mut members = Vec::new();
    let mut stars = Vec::new();

    for start in 0..map.len() {
        let value = map[start];
        if seen[start] || value.is_nan() || value <= threshold {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        members.clear();
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = (i % w, i / w);
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let n = ny * w + nx;
                    if !seen[n] && map[n] > threshold {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        if members.len() >= options.min_pixels {
            if let Some(star) = measure(image, &members, bg.level, options.saturation) {
                stars.push(star);
            }
        }
    }

    stars.sort_by(|a, b| b.flux.total_cmp(&a.flux));
    stars
}

/// Separable Gaussian convolution; near the edges the kernel is renormalised
/// over the pixels that exist, so edges don't darken. Uses one image-sized
/// buffer: rows are smoothed into it, then each column is smoothed in place
/// through a small column buffer.
fn gaussian_smooth(image: &Image, sigma: f64) -> Vec<f32> {
    let radius = (3.0 * sigma).ceil() as usize;
    let kernel: Vec<f32> = (0..=2 * radius)
        .map(|i| {
            let d = i as f64 - radius as f64;
            (-d * d / (2.0 * sigma * sigma)).exp() as f32
        })
        .collect();
    // Smooths `src` (one line of `len` values) into `dst`.
    let line = |src: &[f32], dst: &mut [f32]| {
        let len = src.len();
        for (i, out) in dst.iter_mut().enumerate() {
            let (lo, hi) = (i.saturating_sub(radius), (i + radius).min(len - 1));
            let (mut sum, mut norm) = (0.0f32, 0.0f32);
            for (j, &v) in src.iter().enumerate().take(hi + 1).skip(lo) {
                let k = kernel[j + radius - i];
                sum += k * v;
                norm += k;
            }
            *out = sum / norm;
        }
    };
    let (w, h) = (image.width, image.height);

    let mut out = vec![0.0; image.data.len()];
    for (src, dst) in image.data.chunks_exact(w).zip(out.chunks_exact_mut(w)) {
        line(src, dst);
    }
    let (mut column, mut smoothed) = (vec![0.0; h], vec![0.0; h]);
    for x in 0..w {
        for (y, c) in column.iter_mut().enumerate() {
            *c = out[y * w + x];
        }
        line(&column, &mut smoothed);
        for (y, &v) in smoothed.iter().enumerate() {
            out[y * w + x] = v;
        }
    }
    out
}

fn measure(image: &Image, members: &[usize], sky: f32, saturation: Option<f32>) -> Option<Star> {
    let w = image.width;
    let (mut sum, mut sx, mut sy, mut peak) = (0.0f64, 0.0f64, 0.0f64, f32::MIN);
    for &i in members {
        let v = image.data[i];
        let weight = (v - sky).max(0.0) as f64;
        sum += weight;
        sx += weight * (i % w) as f64;
        sy += weight * (i / w) as f64;
        peak = peak.max(v);
    }
    if sum <= 0.0 {
        return None;
    }
    let (cx, cy) = (sx / sum, sy / sum);

    let mut second = 0.0;
    for &i in members {
        let weight = (image.data[i] - sky).max(0.0) as f64;
        let (dx, dy) = ((i % w) as f64 - cx, (i / w) as f64 - cy);
        second += weight * (dx * dx + dy * dy);
    }
    // For a circular Gaussian, <r²> = 2σ² and FWHM = 2√(2 ln 2) σ.
    let sigma = (second / sum / 2.0).sqrt();
    let (x, y) = windowed_centroid(image, sky, (cx, cy), sigma.max(0.5)).unwrap_or((cx, cy));

    Some(Star {
        x,
        y,
        flux: sum,
        peak,
        pixels: members.len(),
        fwhm: 2.354_820_045 * sigma,
        saturated: saturation.is_some_and(|s| peak >= s),
    })
}

/// Refines a centroid with an iterative Gaussian-weighted window (as in
/// SExtractor's XWIN). The thresholded first moment is biased by noise for
/// faint stars; the window uses every pixel near the star, unclipped, which
/// gives an unbiased and much more precise position.
fn windowed_centroid(image: &Image, sky: f32, start: (f64, f64), sigma: f64) -> Option<(f64, f64)> {
    let (w, h) = (image.width as isize, image.height as isize);
    let radius = (4.0 * sigma).ceil() as isize;
    let inv_2s2 = 1.0 / (2.0 * sigma * sigma);
    let (mut x, mut y) = start;

    for _ in 0..16 {
        let (x0, y0) = (x.round() as isize, y.round() as isize);
        let (mut sw, mut sdx, mut sdy) = (0.0, 0.0, 0.0);
        for py in (y0 - radius).max(0)..=(y0 + radius).min(h - 1) {
            for px in (x0 - radius).max(0)..=(x0 + radius).min(w - 1) {
                let (dx, dy) = (px as f64 - x, py as f64 - y);
                let r2 = dx * dx + dy * dy;
                if r2 > (radius * radius) as f64 {
                    continue;
                }
                let weight =
                    (image.data[(py * w + px) as usize] - sky) as f64 * (-r2 * inv_2s2).exp();
                sw += weight;
                sdx += weight * dx;
                sdy += weight * dy;
            }
        }
        if sw <= 0.0 {
            return None;
        }
        // The factor 2 makes the step exact when the window matches the star's width.
        let (step_x, step_y) = (2.0 * sdx / sw, 2.0 * sdy / sw);
        x += step_x;
        y += step_y;
        if (x - start.0).hypot(y - start.1) > 2.0 {
            return None; // wandered off, e.g. towards a neighbour
        }
        if step_x.hypot(step_y) < 1e-5 {
            break;
        }
    }
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fits::Header;

    /// Deterministic pseudo-random noise so tests are reproducible.
    fn noise(seed: &mut u64) -> f32 {
        let mut sum = 0.0;
        for _ in 0..12 {
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            sum += (*seed >> 40) as f32 / (1u64 << 24) as f32;
        }
        sum - 6.0 // ~ N(0, 1)
    }

    fn synthetic(stars: &[(f64, f64, f64)], sigma_px: f64) -> Image {
        let (w, h) = (128, 96);
        let mut seed = 42;
        let mut data = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut v = 1000.0 + 10.0 * noise(&mut seed) as f64;
                for &(sx, sy, amp) in stars {
                    let r2 = (x as f64 - sx).powi(2) + (y as f64 - sy).powi(2);
                    v += amp * (-r2 / (2.0 * sigma_px * sigma_px)).exp();
                }
                data[y * w + x] = v as f32;
            }
        }
        Image {
            width: w,
            height: h,
            data,
            header: Header::default(),
        }
    }

    #[test]
    fn background_ignores_stars() {
        let img = synthetic(&[(40.0, 30.0, 5000.0), (90.0, 60.0, 3000.0)], 1.5);
        let bg = background(&img);
        assert!((bg.level - 1000.0).abs() < 1.0, "level {}", bg.level);
        assert!((bg.sigma - 10.0).abs() < 1.0, "sigma {}", bg.sigma);
    }

    #[test]
    fn background_ignores_constant_fill_regions() {
        // A dead cell and a gap filled with the sky value, ~30% of the image.
        let mut img = synthetic(&[], 1.5);
        for y in 0..img.height {
            for x in 0..img.width {
                if x < 30 || (60..70).contains(&x) {
                    img.data[y * img.width + x] = 1000.0;
                }
            }
        }
        let bg = background(&img);
        assert!((bg.level - 1000.0).abs() < 1.0, "level {}", bg.level);
        assert!((bg.sigma - 10.0).abs() < 0.5, "sigma {}", bg.sigma);
    }

    #[test]
    fn finds_stars_with_subpixel_centroids() {
        let truth = [
            (40.3, 30.7, 5000.0),
            (90.6, 60.2, 2000.0),
            (20.0, 75.5, 800.0),
        ];
        let img = synthetic(&truth, 1.5);
        let stars = detect(&img, &DetectOptions::default());
        assert_eq!(stars.len(), 3);
        // Brightest first, matching the order of `truth`.
        for (star, &(x, y, _)) in stars.iter().zip(&truth) {
            // ~5× the noise-limited scatter of the faintest star (see centroid_precision).
            assert!((star.x - x).abs() < 0.05, "x {} vs {x}", star.x);
            assert!((star.y - y).abs() < 0.05, "y {} vs {y}", star.y);
        }
        // Thresholded moments underestimate width; the bright star is close to 2.355σ.
        assert!(
            (stars[0].fwhm - 2.355 * 1.5).abs() < 0.6,
            "fwhm {}",
            stars[0].fwhm
        );
    }

    #[test]
    fn smoothing_matches_direct_convolution() {
        let img = synthetic(&[(20.0, 30.0, 500.0), (100.0, 10.0, 900.0)], 1.5);
        let sigma = 1.7f64;
        let fast = gaussian_smooth(&img, sigma);
        let r = (3.0 * sigma).ceil() as isize;
        let g = |d: isize| (-(d * d) as f64 / (2.0 * sigma * sigma)).exp();
        let (w, h) = (img.width as isize, img.height as isize);
        for &(x, y) in &[(0, 0), (20, 30), (127, 95), (64, 1), (100, 10)] {
            // Separable and edge-renormalised: weight = g(dx)·g(dy) over pixels inside.
            let (mut sum, mut norm) = (0.0, 0.0);
            for dy in -r..=r {
                for dx in -r..=r {
                    let (px, py) = (x + dx, y + dy);
                    if (0..w).contains(&px) && (0..h).contains(&py) {
                        sum += g(dx) * g(dy) * img.data[(py * w + px) as usize] as f64;
                        norm += g(dx) * g(dy);
                    }
                }
            }
            let got = fast[(y * w + x) as usize] as f64;
            assert!(
                (got - sum / norm).abs() < 1e-3,
                "({x},{y}): {got} vs {}",
                sum / norm
            );
        }
    }

    #[test]
    fn detect_with_background_matches_detect() {
        let img = synthetic(&[(40.3, 30.7, 5000.0), (90.6, 60.2, 2000.0)], 1.5);
        let options = DetectOptions::default();
        assert_eq!(
            detect(&img, &options),
            detect_with_background(&img, background(&img), &options)
        );
    }

    #[test]
    fn matched_filter_finds_stars_below_single_pixel_noise() {
        // Peak of 3σ: no pixel reaches a 5σ per-pixel threshold, but the
        // star's total signal is ~25σ.
        let img = synthetic(&[(64.3, 48.6, 30.0)], 1.7);
        let raw = DetectOptions {
            fwhm: 0.0,
            ..DetectOptions::default()
        };
        assert!(detect(&img, &raw).is_empty());
        let stars = detect(&img, &DetectOptions::default());
        assert_eq!(stars.len(), 1);
        assert!((stars[0].x - 64.3).abs() < 0.3 && (stars[0].y - 48.6).abs() < 0.3);
    }

    #[test]
    fn flags_saturated_stars() {
        let img = synthetic(&[(64.0, 48.0, 70000.0)], 1.5);
        let options = DetectOptions {
            saturation: Some(60000.0),
            ..DetectOptions::default()
        };
        assert!(detect(&img, &options)[0].saturated);
    }

    #[test]
    fn empty_sky_has_no_detections() {
        let img = synthetic(&[], 1.5);
        assert!(detect(&img, &DetectOptions::default()).is_empty());
    }

    /// Centroids must be unbiased, with scatter close to the noise limit,
    /// over many noise realisations and sub-pixel positions.
    #[test]
    fn centroid_precision() {
        for &(amplitude, max_rms) in &[(5000.0, 0.003), (800.0, 0.015)] {
            let (mut sum_dx, mut sum_dy, mut sum_sq, mut n) = (0.0, 0.0, 0.0, 0.0);
            for seed in 0..300u64 {
                let mut rng = seed * 7919 + 1;
                let (tx, ty) = (
                    24.0 + (seed % 10) as f64 * 0.1,
                    24.0 + (seed % 7) as f64 * 0.13,
                );
                let (w, h) = (48, 48);
                let data = (0..w * h)
                    .map(|i| {
                        let r2 = ((i % w) as f64 - tx).powi(2) + ((i / w) as f64 - ty).powi(2);
                        (1000.0 + 10.0 * noise(&mut rng) as f64 + amplitude * (-r2 / 4.5).exp())
                            as f32
                    })
                    .collect();
                let image = Image {
                    width: w,
                    height: h,
                    data,
                    header: Header::default(),
                };
                let star = &detect(&image, &DetectOptions::default())[0];
                let (dx, dy) = (star.x - tx, star.y - ty);
                sum_dx += dx;
                sum_dy += dy;
                sum_sq += dx * dx + dy * dy;
                n += 1.0;
            }
            let rms = (sum_sq / (2.0 * n)).sqrt();
            assert!(
                (sum_dx / n).abs() < 0.002 && (sum_dy / n).abs() < 0.002,
                "biased"
            );
            assert!(rms < max_rms, "rms {rms} for amplitude {amplitude}");
        }
    }
}
