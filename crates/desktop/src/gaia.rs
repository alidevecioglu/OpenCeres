//! Fetching Gaia DR3 reference stars from VizieR for the plate solution:
//! the mirrors (try in order, remember the last one that worked), a manual
//! choice and per-mirror tests for Settings, and a cache of star lists per
//! field in the data folder so re-opening a set does not fetch again.
//!
//! The core builds the URLs and parses the replies
//! ([`openceres_core::gaia`]); this module only runs `curl`, in the
//! background, like the known-object lookup.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui;
use openceres_core::gaia::{self, GaiaStar, MIRRORS};

use crate::known::curl;
use crate::load::Frame;
use crate::prefs;

/// Seconds one mirror gets before the next is tried.
const MIRROR_TIMEOUT: u64 = 30;
/// A test that takes longer than this is shown as slow.
pub const SLOW: Duration = Duration::from_secs(5);

/// Sky area to fetch: covering every frame of the set (they drift a little).
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub ra: f64,
    pub dec: f64,
    /// Degrees.
    pub radius: f64,
}

impl Field {
    /// Cache file name: the centre to 0.001° and the radius to 0.001°, so
    /// the same set always gives the same name.
    fn key(&self) -> String {
        format!(
            "gaia3_{:08.3}_{:+07.3}_{:.3}.tsv",
            self.ra, self.dec, self.radius
        )
    }
}

/// The field of `frames`: the middle frame's centre, and a radius reaching
/// every frame's corners (by their header WCS) plus 1′ for its errors.
pub fn field(frames: &[Frame]) -> Option<Field> {
    let mid = frames.get(frames.len().saturating_sub(1) / 2)?;
    let wcs = mid.wcs.as_ref()?;
    let centre = wcs.pixel_to_sky(
        (mid.width as f64 - 1.0) / 2.0,
        (mid.height as f64 - 1.0) / 2.0,
    );
    let mut radius: f64 = 0.0;
    for f in frames {
        let Some(w) = &f.wcs else { continue };
        let (x1, y1) = (f.width as f64 - 1.0, f.height as f64 - 1.0);
        for (x, y) in [(0.0, 0.0), (x1, 0.0), (0.0, y1), (x1, y1)] {
            let (ra, dec) = w.pixel_to_sky(x, y);
            radius = radius.max(separation(centre, (ra, dec)));
        }
    }
    // Rounded up so tiny differences between opens give the same cache file.
    Some(Field {
        ra: (centre.0 * 1000.0).round() / 1000.0,
        dec: (centre.1 * 1000.0).round() / 1000.0,
        radius: ((radius + 1.0 / 60.0) * 100.0).ceil() / 100.0,
    })
}

fn separation(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (ra1, de1, ra2, de2) = (
        a.0.to_radians(),
        a.1.to_radians(),
        b.0.to_radians(),
        b.1.to_radians(),
    );
    let h = ((de2 - de1) / 2.0).sin().powi(2)
        + de1.cos() * de2.cos() * ((ra2 - ra1) / 2.0).sin().powi(2);
    (2.0 * h.sqrt().min(1.0).asin()).to_degrees()
}

fn cache_dir() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("Gaia"))
}

/// Where a field's stars come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    Cache,
    Mirror(usize),
}

pub struct Fetched {
    pub stars: Arc<Vec<GaiaStar>>,
    pub origin: Origin,
}

/// A fetch in progress: which mirror it is asking, and how to stop it.
pub struct Fetch {
    pub rx: Receiver<Result<Fetched, String>>,
    pub mirror: Arc<AtomicUsize>,
    pub started: Instant,
    cancel: Arc<AtomicBool>,
}

impl Fetch {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn mirror_name(&self) -> &'static str {
        MIRRORS
            .get(self.mirror.load(Ordering::Relaxed))
            .map_or("cache", |m| m.name)
    }
}

/// The order mirrors are asked in: the chosen one alone, or (automatic)
/// the last one that worked first, then the others in list order.
pub fn order(choice: Option<usize>, last_good: usize) -> Vec<usize> {
    match choice {
        Some(i) if i < MIRRORS.len() => vec![i],
        _ => {
            let first = last_good.min(MIRRORS.len() - 1);
            std::iter::once(first)
                .chain((0..MIRRORS.len()).filter(|&i| i != first))
                .collect()
        }
    }
}

/// Starts fetching `field` (from the cache if it is there) on a background
/// thread.
pub fn start(ctx: &egui::Context, field: Field, mirrors: Vec<usize>) -> Fetch {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mirror = Arc::new(AtomicUsize::new(usize::MAX));
    let (c, m, ctx) = (cancel.clone(), mirror.clone(), ctx.clone());
    std::thread::spawn(move || {
        let _ = tx.send(fetch(&field, &mirrors, &c, &m));
        ctx.request_repaint();
    });
    Fetch {
        rx,
        mirror,
        started: Instant::now(),
        cancel,
    }
}

fn fetch(
    field: &Field,
    mirrors: &[usize],
    cancel: &AtomicBool,
    current: &AtomicUsize,
) -> Result<Fetched, String> {
    let cached = cache_dir().map(|d| d.join(field.key()));
    if let Some(stars) = cached
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| gaia::parse_tsv(&t).ok())
    {
        return Ok(Fetched {
            stars: Arc::new(stars),
            origin: Origin::Cache,
        });
    }
    let mut errors = Vec::new();
    for &i in mirrors {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        current.store(i, Ordering::Relaxed);
        let url = gaia::query_url(&MIRRORS[i], field.ra, field.dec, field.radius);
        match ask(&url, cancel).and_then(|text| Ok((gaia::parse_tsv(&text)?, text))) {
            Ok((stars, text)) => {
                if let (Some(dir), Some(path)) = (cache_dir(), &cached) {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(path, text);
                }
                return Ok(Fetched {
                    stars: Arc::new(stars),
                    origin: Origin::Mirror(i),
                });
            }
            Err(e) => errors.push(format!("{}: {e}", MIRRORS[i].name)),
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled.".into());
    }
    Err(format!(
        "Couldn't get Gaia stars from VizieR ({}). Check the internet connection, or choose \
         another mirror in Settings → Known objects.",
        errors.join("; ")
    ))
}

/// Runs curl on `url`, killing it if `cancel` is set; the reply's text.
fn ask(url: &str, cancel: &AtomicBool) -> Result<String, String> {
    let mut child = curl()
        .args(["-sS", "--max-time", &MIRROR_TIMEOUT.to_string(), url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "couldn't run curl".to_owned())?;
    let mut out = child.stdout.take().ok_or("no output")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        buf
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = reader.join().unwrap_or_default();
                if !status.success() {
                    return Err("no answer".into());
                }
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            Ok(None) if cancel.load(Ordering::Relaxed) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cancelled.".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return Err("curl failed".into()),
        }
    }
}

/// Result of testing one mirror: how long a small query took, or why it
/// failed.
pub type TestResult = Result<Duration, String>;

/// Tests every mirror at once with a small query (a 2′ cone near the
/// celestial equator, which always holds Gaia stars); results arrive one by
/// one as (mirror, result).
pub fn test_all(ctx: &egui::Context) -> Receiver<(usize, TestResult)> {
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, mirror) in MIRRORS.iter().enumerate() {
        let (tx, ctx) = (tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let url = gaia::query_url(mirror, 21.28, 4.13, 2.0 / 60.0);
            let never = AtomicBool::new(false);
            let result = ask(&url, &never)
                .and_then(|t| gaia::parse_tsv(&t))
                .map(|_| started.elapsed());
            let _ = tx.send((i, result));
            ctx.request_repaint();
        });
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_order_starts_with_the_last_good_mirror() {
        assert_eq!(order(None, 0), [0, 1, 2, 3, 4]);
        assert_eq!(order(None, 2), [2, 0, 1, 3, 4]);
        assert_eq!(order(None, 99), [4, 0, 1, 2, 3]);
        assert_eq!(order(Some(3), 0), [3]);
    }

    #[test]
    fn cache_key_is_stable() {
        let f = Field {
            ra: 21.285,
            dec: 4.142,
            radius: 0.15,
        };
        assert_eq!(f.key(), "gaia3_0021.285_+04.142_0.150.tsv");
        let south = Field { dec: -4.142, ..f };
        assert_eq!(south.key(), "gaia3_0021.285_-04.142_0.150.tsv");
    }
}
