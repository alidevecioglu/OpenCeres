//! `openceres` command-line tool, mainly for checking the core against real
//! data before the app exists.

use std::process::ExitCode;

use openceres_core::detect::{self, DetectOptions};
use openceres_core::fits::{self, Image, Value};
use openceres_core::{known, time, wcs::Wcs};

const USAGE: &str = "\
Usage:
  openceres info <image.fits>...           image size, key header values, sky level
  openceres stars <image.fits> [options]   detected stars with pixel and sky positions
  openceres known <image.fits> [--mpcorb <file>]
                                           known asteroids/comets in the field, with the nearest
                                           detection: from SkyBoT (needs internet and curl), or
                                           offline from an uncompressed MPCORB.DAT (`-` = stdin)

Options for `stars`:
  --sigma <n>    detection threshold in noise sigmas (default 5)
  --limit <n>    print at most n stars, brightest first (default 50)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") if args.len() > 1 => info(&args[1..]),
        Some("stars") if args.len() > 1 => stars(&args[1..]),
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
