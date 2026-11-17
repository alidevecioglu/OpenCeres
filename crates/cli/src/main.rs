//! `openceres` command-line tool, mainly for checking the core against real
//! data before the app exists.

use std::process::ExitCode;

use openceres_core::detect::{self, DetectOptions};
use openceres_core::fits::{self, Image, Value};
use openceres_core::{align, gaia, known, plate, reduce, time, wcs::Wcs};

const USAGE: &str = "\
Usage:
  openceres info <image.fits>...           image size, key header values, sky level
  openceres stars <image.fits> [options]   detected stars with pixel and sky positions
  openceres known <image.fits> [--mpcorb <file>]
                                           known asteroids/comets in the field, with the nearest
                                           detection: from SkyBoT (needs internet and curl), or
                                           offline from an uncompressed MPCORB.DAT (`-` = stdin)
  openceres align <image.fits>...          how each frame maps onto the first by matching stars,
                                           compared with what the frames' WCS headers say
  openceres solve <image.fits>... [--gaia <file.tsv>] [--mark <x>,<y>]...
                                           plate solution of each frame against Gaia DR3 (fetched
                                           from VizieR with curl, or read from/saved to the file),
                                           with residuals, the photometric zero point, and the
                                           measured position and magnitude at each mark

Options for `stars`:
  --sigma <n>    detection threshold in noise sigmas (default 5)
  --limit <n>    print at most n stars, brightest first (default 50)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") if args.len() > 1 => info(&args[1..]),
        Some("stars") if args.len() > 1 => stars(&args[1..]),
        Some("align") if args.len() > 2 => align(&args[1..]),
        Some("solve") if args.len() > 1 => solve(&args[1..]),
        Some("known") if args.len() == 2 => known_objects(&args[1], None),
        Some("known") if args.len() == 4 && args[2] == "--mpcorb" => {
            known_objects(&args[1], Some(&args[3]))
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn load(path: &str) -> Result<Image> {
    fits::read(path).map_err(|e| format!("{path}: {e}").into())
}

fn info(paths: &[String]) -> Result<()> {
    for path in paths {
        let image = load(path)?;
        let bg = detect::background(&image);
        println!("{path}");
        println!("  size        {} x {}", image.width, image.height);
        for key in [
            "DATE-OBS", "MJD-OBS", "EXPTIME", "FILTER", "OBJECT", "TELESCOP",
        ] {
            if let Some(v) = image.header.get(key) {
                println!("  {key:<11} {}", show(v));
            }
        }
        println!("  sky         {:.2} ± {:.2}", bg.level, bg.sigma);
        match Wcs::from_header(&image.header) {
            Some(wcs) => {
                let (ra, dec) =
                    wcs.pixel_to_sky(image.width as f64 / 2.0, image.height as f64 / 2.0);
                println!("  centre      RA {ra:.6}°  Dec {dec:+.6}°");
                println!("  scale       {:.3}″/px", wcs.pixel_scale());
            }
            None => println!("  wcs         none (no TAN projection in header)"),
        }
    }
    Ok(())
}

fn stars(args: &[String]) -> Result<()> {
    let path = &args[0];
    let mut options = DetectOptions::default();
    let mut limit = 50usize;
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--sigma" => options.threshold_sigma = value.parse()?,
            "--limit" => limit = value.parse()?,
            _ => return Err(format!("unknown option {flag}\n\n{USAGE}").into()),
        }
    }

    let image = load(path)?;
    let found = detect::detect(&image, &options);
    let wcs = Wcs::from_header(&image.header);
    println!(
        "{} stars in {path} ({} shown)",
        found.len(),
        found.len().min(limit)
    );
    println!(
        "{:>9} {:>9} {:>11} {:>6} {:>12} {:>12}",
        "x", "y", "flux", "fwhm", "ra_deg", "dec_deg"
    );
    for s in found.iter().take(limit) {
        let (ra, dec) = wcs
            .as_ref()
            .map(|w| w.pixel_to_sky(s.x, s.y))
            .map_or((String::from("-"), String::from("-")), |(r, d)| {
                (format!("{r:.6}"), format!("{d:+.6}"))
            });
        let flag = if s.saturated { " sat" } else { "" };
        println!(
            "{:>9.3} {:>9.3} {:>11.1} {:>6.2} {ra:>12} {dec:>12}{flag}",
            s.x, s.y, s.flux, s.fwhm
        );
    }
    Ok(())
}

fn known_objects(path: &str, mpcorb: Option<&str>) -> Result<()> {
    let image = load(path)?;
    let wcs = Wcs::from_header(&image.header).ok_or("image has no WCS")?;
    let jd = time::mid_exposure_jd(&image.header).ok_or("image has no DATE-OBS or MJD-OBS")?;
    let (w, h) = (image.width as f64, image.height as f64);
    let (ra, dec) = wcs.pixel_to_sky(w / 2.0, h / 2.0);
    let radius = 1.1 * w.hypot(h) / 2.0 * wcs.pixel_scale() / 3600.0;
    let observatory = known::observatory_code(&image.header);
    let objects = match mpcorb {
        Some(file) => {
            let query = known::FieldQuery {
                ra,
                dec,
                radius,
                jd,
                observatory: observatory.map(String::from),
            };
            let started = std::time::Instant::now();
            let objects = if file == "-" {
                known::mpcorb_search(std::io::stdin().lock(), &query)?
            } else {
                let open = || std::fs::File::open(file).map(std::io::BufReader::new);
                let date = known::mpcorb_date(open()?).unwrap_or_else(|| "?".into());
                eprintln!("MPCORB {file}, updated {date}");
                known::mpcorb_search(open()?, &query)?
            };
            eprintln!(
                "MPCORB search took {:.2} s",
                started.elapsed().as_secs_f64()
            );
            objects
        }
        None => {
            let url = known::skybot_url(ra, dec, radius, jd, observatory);
            let reply = std::process::Command::new("curl")
                .args(["-sSf", "--max-time", "60", &url])
                .output()
                .map_err(|e| format!("cannot run curl: {e}"))?;
            if !reply.status.success() {
                return Err(format!(
                    "SkyBoT request failed: {}",
                    String::from_utf8_lossy(&reply.stderr).trim()
                )
                .into());
            }
            known::parse_skybot(&String::from_utf8_lossy(&reply.stdout))?
        }
    };
    let found = detect::detect(&image, &DetectOptions::default());

    println!("{} known objects in {path}", objects.len());
    println!(
        "{:<8} {:<12} {:>13} {:>13} {:>5} {:>8} {:>8} {:>9} {:>9}  nearest detection",
        "number", "name", "ra", "dec", "mag", "dra/h", "ddec/h", "x", "y"
    );
    for o in &objects {
        let Some((x, y)) = wcs.sky_to_pixel(o.ra, o.dec) else {
            continue;
        };
        let nearest = found
            .iter()
            .map(|s| ((s.x - x).hypot(s.y - y), s))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map_or(String::from("-"), |(d, s)| {
                format!(
                    "{d:.1} px ({:.2}″) at {:.1},{:.1}",
                    d * wcs.pixel_scale(),
                    s.x,
                    s.y
                )
            });
        let inside = (0.0..w).contains(&x) && (0.0..h).contains(&y);
        println!(
            "{:<8} {:<12} {:>13} {:>13} {:>5} {:>8.2} {:>8.2} {x:>9.1} {y:>9.1}  {}",
            o.number.as_deref().unwrap_or("-"),
            o.name,
            sexagesimal(o.ra / 15.0, false),
            sexagesimal(o.dec, true),
            o.magnitude.map_or("-".into(), |m| format!("{m:.1}")),
            o.motion_ra,
            o.motion_dec,
            if inside {
                nearest
            } else {
                "(outside the image)".into()
            }
        );
    }
    Ok(())
}

/// `01 24 57.877` (hours) or `+04 08 08.73` (degrees, `signed`).
fn sexagesimal(value: f64, signed: bool) -> String {
    let decimals = if signed { 100.0 } else { 1000.0 };
    let units = (value.abs() * 3600.0 * decimals).round() as i64;
    let (whole, frac) = (units / decimals as i64, units % decimals as i64);
    let (h, m, sec) = (whole / 3600, whole / 60 % 60, whole % 60);
    let sign = if !signed {
        ""
    } else if value < 0.0 {
        "-"
    } else {
        "+"
    };
    let width = if signed { 2 } else { 3 };
    format!("{sign}{h:02} {m:02} {sec:02}.{frac:0width$}")
}

fn show(v: &Value) -> String {
    match v {
        Value::Logical(b) => if *b { "T" } else { "F" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Text(s) => s.clone(),
    }
}

fn align(paths: &[String]) -> Result<()> {
    let frames = paths
        .iter()
        .map(|p| {
            let image = load(p)?;
            let stars: Vec<(f64, f64)> = detect::detect(&image, &DetectOptions::default())
                .iter()
                .map(|s| (s.x, s.y))
                .collect();
            Ok((image, stars))
        })
        .collect::<Result<Vec<_>>>()?;
    let (ref_image, ref_stars) = &frames[0];
    let ref_wcs = Wcs::from_header(&ref_image.header);
    let (cx, cy) = (ref_image.width as f64 / 2.0, ref_image.height as f64 / 2.0);
    println!("reference {}: {} stars", paths[0], ref_stars.len());
    for (path, (image, stars)) in paths.iter().zip(&frames).skip(1) {
        let start = std::time::Instant::now();
        let found = align::align(ref_stars, stars);
        let ms = start.elapsed().as_secs_f64() * 1e3;
        println!("{path}: {} stars, matched in {ms:.0} ms", stars.len());
        let Some(a) = found else {
            println!("  no alignment found");
            continue;
        };
        let to_target = a.transform.inverse().ok_or("singular transform")?;
        let (tx, ty) = to_target.apply(cx, cy);
        println!(
            "  centre moved by ({:+.3}, {:+.3}) px, rotation {:+.4}°, scale {:.5}, {} stars, rms {:.3} px",
            tx - cx,
            ty - cy,
            -a.transform.rotation(),
            1.0 / a.transform.scale(),
            a.matched,
            a.rms
        );
        if let (Some(rw), Some(tw)) = (&ref_wcs, Wcs::from_header(&image.header)) {
            let (ra, dec) = rw.pixel_to_sky(cx, cy);
            if let Some((wx, wy)) = tw.sky_to_pixel(ra, dec) {
                println!(
                    "  WCS headers say ({:+.3}, {:+.3}) px; stars and WCS differ by {:.3} px",
                    wx - cx,
                    wy - cy,
                    (wx - tx).hypot(wy - ty)
                );
            }
        }
    }
    Ok(())
}

/// Gaia DR3 stars around a frame: from `file` if it exists, else from the
/// first VizieR mirror that answers with stars (then saved to `file`).
fn gaia_stars(image: &Image, file: Option<&str>) -> Result<Vec<gaia::GaiaStar>> {
    if let Some(f) = file
        && let Ok(text) = std::fs::read_to_string(f)
    {
        return Ok(gaia::parse_tsv(&text)?);
    }
    let wcs = Wcs::from_header(&image.header).ok_or("image has no WCS")?;
    let (w, h) = (image.width as f64, image.height as f64);
    let (ra, dec) = wcs.pixel_to_sky((w - 1.0) / 2.0, (h - 1.0) / 2.0);
    let radius = w.hypot(h) / 2.0 * wcs.pixel_scale() / 3600.0 + 0.02;
    let mut errors = Vec::new();
    for mirror in &gaia::MIRRORS {
        let url = gaia::query_url(mirror, ra, dec, radius);
        let started = std::time::Instant::now();
        let reply = std::process::Command::new("curl")
            .args(["-sS", "--max-time", "30", &url])
            .output()
            .map_err(|e| format!("cannot run curl: {e}"))?;
        let text = String::from_utf8_lossy(&reply.stdout);
        match gaia::parse_tsv(&text) {
            Ok(stars) => {
                eprintln!(
                    "Gaia DR3: {} stars from {} in {:.1} s",
                    stars.len(),
                    mirror.name,
                    started.elapsed().as_secs_f64()
                );
                if let Some(f) = file {
                    std::fs::write(f, text.as_bytes())?;
                }
                return Ok(stars);
            }
            Err(e) => errors.push(format!("{}: {e}", mirror.name)),
        }
    }
    Err(format!("no VizieR mirror answered: {}", errors.join("; ")).into())
}

fn solve(args: &[String]) -> Result<()> {
    let mut paths = Vec::new();
    let mut gaia_file = None;
    let mut marks = Vec::new();
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--gaia" => gaia_file = Some(rest.next().ok_or("--gaia needs a file")?.clone()),
            "--mark" => {
                let v = rest.next().ok_or("--mark needs x,y")?;
                let (x, y) = v.split_once(',').ok_or("--mark needs x,y")?;
                marks.push((x.trim().parse::<f64>()?, y.trim().parse::<f64>()?));
            }
            _ => paths.push(a.clone()),
        }
    }
    let mut catalog: Option<Vec<gaia::GaiaStar>> = None;
    for path in &paths {
        let image = load(path)?;
        if catalog.is_none() {
            catalog = Some(gaia_stars(&image, gaia_file.as_deref())?);
        }
        let cat = catalog.as_deref().unwrap_or_default();
        let started = std::time::Instant::now();
        let red = reduce::reduce(&image, Some(cat), &marks);
        println!("{path}");
        println!(
            "  {} stars detected, FWHM {:.2} px, reduced in {:.2} s",
            red.detected,
            red.fwhm,
            started.elapsed().as_secs_f64()
        );
        match red.solution.as_ref().ok_or("no solution")? {
            Err(e) => println!("  plate solution failed: {e}"),
            Ok(sol) => {
                println!(
                    "  plate: order {}, {} of {} matched stars used, RMS {:.3}″ (RA) {:.3}″ (Dec), scale {:.4}″/px",
                    sol.order,
                    sol.stars,
                    sol.matched.len(),
                    sol.rms.0,
                    sol.rms.1,
                    sol.pixel_scale()
                );
                let cv: Vec<String> = sol
                    .cv_rms
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        c.map_or(format!("order {}: -", i + 1), |c| {
                            format!("order {}: {c:.3}″", i + 1)
                        })
                    })
                    .collect();
                println!("  cross-validated RMS  {}", cv.join(", "));
                if let Some(p) = sol.problem() {
                    println!("  PROBLEM: {p}");
                }
                // How far the header WCS is from Gaia on the same stars.
                if let Some(wcs) = Wcs::from_header(&image.header) {
                    let mut d: Vec<f64> = sol
                        .matched
                        .iter()
                        .filter(|m| m.used)
                        .map(|m| {
                            let (ra, dec) = wcs.pixel_to_sky(m.x, m.y);
                            ((ra - m.ra) * m.dec.to_radians().cos()).hypot(dec - m.dec) * 3600.0
                        })
                        .collect();
                    d.sort_by(f64::total_cmp);
                    if let (Some(lo), Some(hi)) = (d.first(), d.last()) {
                        println!(
                            "  header WCS vs Gaia: median {:.2}″, range {lo:.2}–{hi:.2}″",
                            d[d.len() / 2]
                        );
                    }
                }
                // Residuals by magnitude.
                for (lo, hi) in [(0.0, 16.0), (16.0, 18.0), (18.0, 19.5), (19.5, 25.0)] {
                    let r: Vec<f64> = sol
                        .matched
                        .iter()
                        .filter(|m| m.used && (lo..hi).contains(&m.mag))
                        .map(|m| m.residual.0.hypot(m.residual.1))
                        .collect();
                    if !r.is_empty() {
                        let rms =
                            (r.iter().map(|v| v * v).sum::<f64>() / (2.0 * r.len() as f64)).sqrt();
                        println!(
                            "    G {lo:>4.1}–{hi:<4.1} {:>4} stars, per-axis RMS {rms:.3}″",
                            r.len()
                        );
                    }
                }
            }
        }
        match &red.zero_point {
            Some(z) => println!(
                "  zero point G = {:.3} ± {:.3} − 2.5 log flux, {} stars, scatter {:.3} mag{}",
                z.zp,
                z.error,
                z.stars,
                z.scatter,
                z.problem()
                    .map_or(String::new(), |p| format!("  PROBLEM: {p}"))
            ),
            None => println!("  no zero point"),
        }
        for (mark, m) in marks.iter().zip(&red.marks) {
            match m {
                Ok(m) => println!(
                    "  mark {:.1},{:.1} -> {:.2},{:.2}  RA {} Dec {} ± {}  S/N {:.1}  G {}",
                    mark.0,
                    mark.1,
                    m.x,
                    m.y,
                    sexagesimal(m.ra / 15.0, false),
                    sexagesimal(m.dec, true),
                    m.sigma
                        .map_or("-".into(), |s| format!("{:.3}″/{:.3}″", s.0, s.1)),
                    m.snr(),
                    m.mag
                        .map_or("-".into(), |(v, e)| format!("{v:.2} ± {e:.2}"))
                        + &m.mag_problem
                            .as_ref()
                            .map_or(String::new(), |p| format!(" ({p})"))
                ),
                Err(e) => println!("  mark {:.1},{:.1}: {e}", mark.0, mark.1),
            }
        }
        let _ = plate::MAX_RMS;
    }
    Ok(())
}
