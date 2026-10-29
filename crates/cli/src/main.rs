//! `openceres` command-line tool, mainly for checking the core against real
//! data before the app exists.

use std::process::ExitCode;

use openceres_core::detect::{self, DetectOptions};
use openceres_core::fits::{self, Image, Value};
use openceres_core::wcs::Wcs;

const USAGE: &str = "\
Usage:
  openceres info <image.fits>...           image size, key header values, sky level
  openceres stars <image.fits> [options]   detected stars with pixel and sky positions

Options for `stars`:
  --sigma <n>    detection threshold in noise sigmas (default 5)
  --limit <n>    print at most n stars, brightest first (default 50)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("info") if args.len() > 1 => info(&args[1..]),
        Some("stars") if args.len() > 1 => stars(&args[1..]),
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

fn show(v: &Value) -> String {
    match v {
        Value::Logical(b) => if *b { "T" } else { "F" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Text(s) => s.clone(),
    }
}
