//! Known objects in the field: one lookup per image set, from IMCCE's SkyBoT
//! service (online) or the MPC's MPCORB catalogue (offline, downloaded once
//! into the app data folder).
//!
//! Network access goes through the system `curl` (shipped with macOS,
//! Windows 10/11 and practically every Linux), so the app links no HTTP or
//! TLS code of its own. Everything here runs on background threads.

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui;
use openceres_core::known::{self, FieldQuery, KnownObject, mpcorb_date, mpcorb_search};

use crate::gz::GzReader;
use crate::load::Frame;
use crate::prefs::{self, KnownSource};

const MPCORB_URL: &str = "https://www.minorplanetcenter.net/iau/MPCORB/MPCORB.DAT.gz";
const MPCORB_FILE: &str = "MPCORB.DAT.gz";
const OFFLINE: &str = "Couldn't reach SkyBoT — check the internet connection, or switch to \
                       the offline MPCORB catalogue in Settings.";
const NO_CURL: &str = "Couldn't run curl, which OpenCeres uses to go online. Install curl, \
                       or switch to the offline MPCORB catalogue in Settings.";
/// Read buffer for the compressed catalogue.
const GZ_BUFFER: usize = 256 * 1024;

/// The sky area of an image set and the time to look it up for.
pub struct Field {
    pub ra: f64,
    pub dec: f64,
    /// Search radius, degrees.
    pub radius: f64,
    /// Query epoch: the middle frame's mid-exposure time (UTC JD).
    pub jd: f64,
    pub observatory: Option<String>,
}

/// Field of the middle frame: its image centre, a radius 10 % beyond the
/// corners, and its mid-exposure time.
pub fn field(frames: &[Frame]) -> Result<Field, String> {
    let f = frames
        .get(frames.len().saturating_sub(1) / 2)
        .ok_or("No image set is open.")?;
    let wcs = f.wcs.as_ref().ok_or(
        "This image set has no sky coordinates (WCS), so known objects can't be placed on it.",
    )?;
    let jd = f
        .mid_jd
        .ok_or("This image set has no observation times, so known objects can't be looked up.")?;
    let (w, h) = (f.width as f64, f.height as f64);
    let (ra, dec) = wcs.pixel_to_sky((w - 1.0) / 2.0, (h - 1.0) / 2.0);
    let radius = 1.1 * (w.hypot(h) / 2.0) * wcs.pixel_scale() / 3600.0;
    Ok(Field {
        ra,
        dec,
        radius,
        jd,
        observatory: f.observatory.map(str::to_owned),
    })
}

pub enum LookupError {
    /// The offline source is chosen but the catalogue is not downloaded.
    NoCatalogue,
    Message(String),
}

pub struct Lookup {
    pub objects: Vec<KnownObject>,
    /// Epoch the objects' positions refer to.
    pub epoch: f64,
    pub source: KnownSource,
    /// `14:05 UTC`, when the lookup finished.
    pub at: String,
    /// Catalogue date, for MPCORB.
    pub catalogue: Option<String>,
}

/// Starts a lookup on a background thread; the result arrives on the
/// returned channel and the UI is woken up for it.
pub fn start(
    ctx: &egui::Context,
    source: KnownSource,
    field: Field,
) -> Receiver<Result<Lookup, LookupError>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(lookup(source, &field));
        ctx.request_repaint();
    });
    rx
}

fn lookup(source: KnownSource, field: &Field) -> Result<Lookup, LookupError> {
    let (objects, catalogue) = match source {
        KnownSource::SkyBot => (skybot(field).map_err(LookupError::Message)?, None),
        KnownSource::Mpcorb => {
            let path = catalogue_path()
                .filter(|p| p.is_file())
                .ok_or(LookupError::NoCatalogue)?;
            let query = FieldQuery {
                ra: field.ra,
                dec: field.dec,
                radius: field.radius,
                jd: field.jd,
                observatory: field.observatory.clone(),
            };
            let objects = open_catalogue(&path)
                .and_then(|r| mpcorb_search(BufReader::new(r), &query))
                .map_err(LookupError::Message)?;
            (objects, catalogue_date(&path))
        }
    };
    Ok(Lookup {
        objects,
        epoch: field.jd,
        source,
        at: utc_clock(),
        catalogue,
    })
}

fn skybot(field: &Field) -> Result<Vec<KnownObject>, String> {
    let url = known::skybot_url(
        field.ra,
        field.dec,
        field.radius,
        field.jd,
        field.observatory.as_deref(),
    );
    let out = curl()
        .args(["-sSf", "--max-time", "60", &url])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| NO_CURL.to_owned())?;
    if !out.status.success() {
        return Err(OFFLINE.into());
    }
    known::parse_skybot(&String::from_utf8_lossy(&out.stdout))
}

/// `curl` without a console window popping up on Windows.
fn curl() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new("curl");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn utc_clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86_400;
    format!("{:02}:{:02} UTC", day / 3600, day % 3600 / 60)
}

// ---------------------------------------------------------------------------
// The offline catalogue

pub fn catalogue_path() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join(MPCORB_FILE))
}

/// The catalogue, decompressed on the fly.
fn open_catalogue(path: &Path) -> Result<GzReader<BufReader<std::fs::File>>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("MPCORB catalogue: {e}"))?;
    GzReader::new(BufReader::with_capacity(GZ_BUFFER, file))
        .map_err(|e| format!("MPCORB catalogue: {e}"))
}

/// The catalogue's date (from its header), if it can be read. Only the
/// first few kilobytes are decompressed.
pub fn catalogue_date(path: &Path) -> Option<String> {
    mpcorb_date(BufReader::new(open_catalogue(path).ok()?))
}

/// What Settings shows about the catalogue; read once, not every frame.
pub enum Catalogue {
    Missing,
    Present { date: Option<String>, bytes: u64 },
}

pub fn catalogue_info() -> Catalogue {
    let Some(path) = catalogue_path() else {
        return Catalogue::Missing;
    };
    match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => Catalogue::Present {
            date: catalogue_date(&path),
            bytes: m.len(),
        },
        _ => Catalogue::Missing,
    }
}

/// A catalogue download in progress. Dropping it does not stop it; use
/// [`Download::cancel`].
pub struct Download {
    pub received: Arc<AtomicU64>,
    /// Expected size in bytes, 0 while unknown.
    pub total: Arc<AtomicU64>,
    pub checking: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    pub rx: Receiver<Result<(), String>>,
}

impl Download {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Downloads the catalogue next to the preferences: into a `.part` file,
/// which is checked (a complete gzip with an MPCORB header) and only then
/// renamed over the old catalogue, so a failed or cancelled download never
/// replaces a good one.
pub fn start_download(ctx: &egui::Context) -> Download {
    let (tx, rx) = std::sync::mpsc::channel();
    let d = Download {
        received: Arc::default(),
        total: Arc::default(),
        checking: Arc::default(),
        cancel: Arc::default(),
        rx,
    };
    let (received, total, checking, cancel) = (
        d.received.clone(),
        d.total.clone(),
        d.checking.clone(),
        d.cancel.clone(),
    );
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = download(&ctx, &received, &total, &checking, &cancel);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    d
}

fn download(
    ctx: &egui::Context,
    received: &AtomicU64,
    total: &AtomicU64,
    checking: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let path = catalogue_path().ok_or("Couldn't find the app's data folder.")?;
    let dir = path
        .parent()
        .ok_or("Couldn't find the app's data folder.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = path.with_extension("gz.part");
    let _ = std::fs::remove_file(&part);

    // The size, for progress; the download works without it.
    if let Ok(head) = curl()
        .args(["-sSfLI", "--max-time", "30", MPCORB_URL])
        .stdin(Stdio::null())
        .output()
    {
        let text = String::from_utf8_lossy(&head.stdout);
        if let Some(n) = content_length(&text) {
            total.store(n, Ordering::Relaxed);
        }
    }

    let mut child = curl()
        .args(["-sSfL", "--connect-timeout", "30"])
        // Give up when the transfer stalls (< 1 kB/s for a minute).
        .args(["--speed-limit", "1024", "--speed-time", "60", "-o"])
        .arg(&part)
        .arg(MPCORB_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| NO_CURL.to_owned())?;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&part);
            return Err("Download cancelled.".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        let size = std::fs::metadata(&part).map_or(0, |m| m.len());
        if size != received.swap(size, Ordering::Relaxed) {
            ctx.request_repaint();
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(
            "Couldn't download the MPCORB catalogue — check the internet connection \
                    and try again."
                .into(),
        );
    }
    checking.store(true, Ordering::Relaxed);
    ctx.request_repaint();
    if let Err(e) = verify(&part, cancel) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("Couldn't save the MPCORB catalogue: {e}")
    })
}

/// `Content-Length` of the last response (after redirects) in `curl -I` output.
fn content_length(head: &str) -> Option<u64> {
    head.lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .filter_map(|(_, v)| v.trim().parse().ok())
        .next_back()
}

/// Reads the whole file through the gzip checks (CRC and length) and makes
/// sure it starts like the MPCORB catalogue.
fn verify(path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let bad = "The downloaded file is not the MPCORB catalogue. Try again later.";
    let mut reader = open_catalogue(path).map_err(|_| bad.to_owned())?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut head = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Download cancelled.".into());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("The download is incomplete or damaged ({e}). Try again."))?;
        if n == 0 {
            break;
        }
        if head.len() < 16 * 1024 {
            head.extend_from_slice(&buf[..n]);
            if head.len() >= 16 * 1024 && !looks_like_mpcorb(&head) {
                return Err(bad.into());
            }
        }
    }
    if looks_like_mpcorb(&head) {
        Ok(())
    } else {
        Err(bad.into())
    }
}

/// The catalogue opens with a text header naming it, then the column
/// heading line (`Des'n     H     G   Epoch …`).
fn looks_like_mpcorb(head: &[u8]) -> bool {
    let text = String::from_utf8_lossy(head);
    text.contains("MPCORB") || text.contains("Des'n     H     G")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_length_after_redirect() {
        let head = "HTTP/1.1 301 Moved\r\nContent-Length: 162\r\nLocation: x\r\n\r\n\
                    HTTP/1.1 200 OK\r\ncontent-length: 98765432\r\n\r\n";
        assert_eq!(content_length(head), Some(98_765_432));
        assert_eq!(content_length("HTTP/2 200\r\n"), None);
    }

    /// The Update MPCORB path end to end (downloads ~94 MB into the app
    /// data folder), then one offline lookup of the XY57 field:
    /// `cargo test -p openceres-desktop -- --ignored real_mpcorb`
    #[test]
    #[ignore]
    fn real_mpcorb_update_and_lookup() {
        let ctx = egui::Context::default();
        let d = start_download(&ctx);
        let started = std::time::Instant::now();
        d.rx.recv().unwrap().unwrap();
        println!(
            "downloaded {} of {} bytes in {:.0?}",
            d.received.load(Ordering::Relaxed),
            d.total.load(Ordering::Relaxed),
            started.elapsed()
        );
        let path = catalogue_path().unwrap();
        println!("catalogue of {:?}", catalogue_date(&path));
        let field = Field {
            ra: 21.285487,
            dec: 4.141814,
            radius: 0.134,
            jd: 2_461_317.940480,
            observatory: Some("F51".into()),
        };
        let started = std::time::Instant::now();
        let Ok(found) = lookup(KnownSource::Mpcorb, &field) else {
            panic!("lookup failed");
        };
        println!("lookup in {:.1?}", started.elapsed());
        for o in &found.objects {
            println!("{} {:.6} {:.6} {:?}", o.label(), o.ra, o.dec, o.magnitude);
        }
        assert!(found.objects.iter().any(|o| o.label() == "133105"));
    }

    #[test]
    fn recognises_catalogue_header() {
        assert!(looks_like_mpcorb(
            b"MINOR PLANET CENTER ORBIT DATABASE (MPCORB)\nDes'n     H     G   Epoch"
        ));
        assert!(!looks_like_mpcorb(b"<html><body>Not found</body></html>"));
    }
}
