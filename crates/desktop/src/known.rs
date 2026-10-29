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

/// SkyBoT intermittently leaves objects out of a reply (in testing, about
/// one identical request in six came back with 4 of 6 objects, missing the
/// unnumbered ones). So every lookup sends the same request twice at once,
/// as two `curl` processes, and keeps the union of both replies; it fails
/// only if both requests fail.
fn skybot(field: &Field) -> Result<Vec<KnownObject>, String> {
    let url = known::skybot_url(
        field.ra,
        field.dec,
        field.radius,
        field.jd,
        field.observatory.as_deref(),
    );
    let ask = || -> Result<Vec<KnownObject>, String> {
        let out = curl()
            .args(["-sSf", "--max-time", "60", &url])
            .stdin(Stdio::null())
            .output()
            .map_err(|_| NO_CURL.to_owned())?;
        if !out.status.success() {
            return Err(OFFLINE.into());
        }
        known::parse_skybot(&String::from_utf8_lossy(&out.stdout))
    };
    let (first, second) = std::thread::scope(|s| {
        let other = s.spawn(ask);
        let first = ask();
        let second = other
            .join()
            .unwrap_or_else(|_| Err("The lookup stopped unexpectedly.".into()));
        (first, second)
    });
    merge_replies(first, second)
}

/// Union of two replies by designation, keeping the first reply's entry
/// (and order) for objects in both.
fn merge_replies(
    first: Result<Vec<KnownObject>, String>,
    second: Result<Vec<KnownObject>, String>,
) -> Result<Vec<KnownObject>, String> {
    match (first, second) {
        (Ok(mut a), Ok(b)) => {
            for o in b {
                if !a.iter().any(|x| x.number == o.number && x.name == o.name) {
                    a.push(o);
                }
            }
            Ok(a)
        }
        (Ok(a), Err(_)) | (Err(_), Ok(a)) => Ok(a),
        (Err(e), Err(_)) => Err(e),
    }
}

/// `curl` without a console window popping up on Windows.
pub fn curl() -> Command {
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
    crate::format::unix_clock(crate::format::unix_now())
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
    match local_version(&path) {
        Some(version) => Catalogue::Present {
            date: catalogue_date(&path),
            bytes: version.length.unwrap_or(0),
        },
        None => Catalogue::Missing,
    }
}

/// What tells one version of the catalogue on the MPC server from another:
/// its `ETag`, `Last-Modified` (as Unix time) and `Content-Length`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Version {
    pub etag: Option<String>,
    pub modified: Option<u64>,
    pub length: Option<u64>,
}

impl Version {
    /// Headers of the last response (after redirects) in `curl -I` / `-D` output.
    fn from_headers(head: &str) -> Version {
        let last = head
            .split("\n")
            .collect::<Vec<_>>()
            .rsplit(|l| l.starts_with("HTTP/"))
            .next()
            .map(|lines| lines.join("\n"))
            .unwrap_or_default();
        let mut v = Version::default();
        for (key, value) in last.lines().filter_map(|l| l.split_once(':')) {
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "etag" if !value.is_empty() => v.etag = Some(value.to_owned()),
                "last-modified" => v.modified = crate::format::parse_http_date(value),
                "content-length" => v.length = value.parse().ok(),
                _ => {}
            }
        }
        v
    }

    fn parse(text: &str) -> Version {
        let mut v = Version::default();
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            let value = value.trim();
            match key.trim() {
                "etag" if !value.is_empty() => v.etag = Some(value.to_owned()),
                "modified" => v.modified = value.parse().ok(),
                "length" => v.length = value.parse().ok(),
                _ => {}
            }
        }
        v
    }

    fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(e) = &self.etag {
            out += &format!("etag={e}\n");
        }
        if let Some(m) = self.modified {
            out += &format!("modified={m}\n");
        }
        if let Some(l) = self.length {
            out += &format!("length={l}\n");
        }
        out
    }

    /// True if the server's `self` differs from the `local` copy. Compares
    /// the ETag first, then the modification time, then the size; when
    /// nothing can be compared, assumes it is newer.
    pub fn is_newer_than(&self, local: &Version) -> bool {
        if let (Some(a), Some(b)) = (&self.etag, &local.etag) {
            return a != b;
        }
        // A local time is the server's Last-Modified, or (for catalogues
        // downloaded before versions were recorded) the file's own time,
        // which is later than the version it holds.
        if let (Some(a), Some(b)) = (self.modified, local.modified) {
            return a > b;
        }
        if let (Some(a), Some(b)) = (self.length, local.length) {
            return a != b;
        }
        true
    }
}

fn meta_path(catalogue: &Path) -> PathBuf {
    catalogue.with_extension("gz.meta")
}

/// The local catalogue's version: from the sidecar file written after the
/// download, else its file time and size. `None` if there is no catalogue.
fn local_version(path: &Path) -> Option<Version> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let mut v = std::fs::read_to_string(meta_path(path))
        .map(|t| Version::parse(&t))
        .unwrap_or_default();
    if v.etag.is_none() && v.modified.is_none() {
        v.modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
    }
    v.length = v.length.or(Some(meta.len()));
    Some(v)
}

/// The catalogue's version on the MPC server (a HEAD request, a few hundred bytes).
fn remote_version() -> Result<Version, String> {
    let out = curl()
        .args(["-sSfLI", "--max-time", "30", MPCORB_URL])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| NO_CURL.to_owned())?;
    if !out.status.success() {
        return Err(
            "Couldn't reach the Minor Planet Center — check the internet connection.".into(),
        );
    }
    Ok(Version::from_headers(&String::from_utf8_lossy(&out.stdout)))
}

/// Whether the server has a newer catalogue than the local one.
pub enum Freshness {
    /// The local catalogue is the server's current one.
    Current(Version),
    /// The server has a different (newer) catalogue, or there is none here.
    Newer(Version),
}

/// Asks the server (HEAD only) whether the local catalogue is current, on
/// a background thread.
pub fn start_check(ctx: &egui::Context) -> Receiver<Result<Freshness, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = remote_version().map(|remote| {
            match catalogue_path().as_deref().and_then(local_version) {
                Some(local) if !remote.is_newer_than(&local) => Freshness::Current(remote),
                _ => Freshness::Newer(remote),
            }
        });
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

/// Seconds since the local catalogue's version was published (or
/// downloaded), if there is one.
pub fn catalogue_age() -> Option<u64> {
    let v = local_version(&catalogue_path()?)?;
    Some(crate::format::unix_now().saturating_sub(v.modified?))
}

/// How a download ended.
pub enum Fetched {
    Downloaded,
    /// The server's catalogue is the one already here; nothing was fetched.
    AlreadyCurrent,
}

/// A catalogue download in progress. Dropping it does not stop it; use
/// [`Download::cancel`].
pub struct Download {
    pub received: Arc<AtomicU64>,
    /// Expected size in bytes, 0 while unknown.
    pub total: Arc<AtomicU64>,
    pub checking: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    pub rx: Receiver<Result<Fetched, String>>,
}

impl Download {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Downloads the catalogue next to the preferences: into a `.part` file,
/// which is checked (a complete gzip with an MPCORB header) and only then
/// renamed over the old catalogue, so a failed or cancelled download never
/// replaces a good one. A HEAD request first skips the download when the
/// server's catalogue is the one already here.
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
) -> Result<Fetched, String> {
    let path = catalogue_path().ok_or("Couldn't find the app's data folder.")?;
    let dir = path
        .parent()
        .ok_or("Couldn't find the app's data folder.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = path.with_extension("gz.part");
    let headers = path.with_extension("gz.headers");
    let _ = std::fs::remove_file(&part);

    // The server's version: skips an unchanged catalogue and gives the size
    // for progress. The download works without it.
    let remote = remote_version().ok();
    if let Some(remote) = &remote {
        if let Some(local) = local_version(&path)
            && !remote.is_newer_than(&local)
        {
            return Ok(Fetched::AlreadyCurrent);
        }
        if let Some(n) = remote.length {
            total.store(n, Ordering::Relaxed);
        }
    }

    let mut child = curl()
        .args(["-sSfL", "--connect-timeout", "30"])
        // Give up when the transfer stalls (< 1 kB/s for a minute).
        .args(["--speed-limit", "1024", "--speed-time", "60", "-D"])
        .arg(&headers)
        .arg("-o")
        .arg(&part)
        .arg(MPCORB_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| NO_CURL.to_owned())?;
    let cleanup = || {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&headers);
    };
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            cleanup();
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
        cleanup();
        return Err(
            "Couldn't download the MPCORB catalogue — check the internet connection \
                    and try again."
                .into(),
        );
    }
    checking.store(true, Ordering::Relaxed);
    ctx.request_repaint();
    if let Err(e) = verify(&part, cancel) {
        cleanup();
        return Err(e);
    }
    // The version of what was actually downloaded, from its own headers.
    let got = std::fs::read_to_string(&headers)
        .map(|h| Version::from_headers(&h))
        .ok()
        .filter(|v| v.etag.is_some() || v.modified.is_some())
        .or(remote);
    let _ = std::fs::remove_file(&headers);
    let meta = meta_path(&path);
    let _ = std::fs::remove_file(&meta);
    std::fs::rename(&part, &path).map_err(|e| {
        cleanup();
        format!("Couldn't save the MPCORB catalogue: {e}")
    })?;
    if let Some(v) = got {
        let _ = std::fs::write(&meta, v.to_text());
    }
    Ok(Fetched::Downloaded)
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
    fn version_from_last_response_after_redirect() {
        let head = "HTTP/1.1 301 Moved\r\nContent-Length: 162\r\nLocation: x\r\nETag: \"old\"\r\n\r\n\
                    HTTP/1.1 200 OK\r\ncontent-length: 94068293\r\n\
                    Last-Modified: Thu, 29 Oct 2026 13:04:33 GMT\r\n\
                    ETag: \"59b5e45-65efa522a4343\"\r\nAccept-Ranges: bytes\r\n\r\n";
        let v = Version::from_headers(head);
        assert_eq!(v.length, Some(94_068_293));
        assert_eq!(v.etag.as_deref(), Some("\"59b5e45-65efa522a4343\""));
        assert_eq!(
            v.modified,
            crate::format::parse_http_date("Thu, 29 Oct 2026 13:04:33 GMT")
        );
        assert_eq!(Version::parse(&v.to_text()), v);
        assert_eq!(Version::from_headers("HTTP/2 200\r\n"), Version::default());
    }

    #[test]
    fn compares_etag_then_time_then_size() {
        let v = |etag: Option<&str>, modified: Option<u64>, length: Option<u64>| Version {
            etag: etag.map(str::to_owned),
            modified,
            length,
        };
        let server = v(Some("b"), Some(200), Some(10));
        // ETag decides when both have one, even if the times say otherwise.
        assert!(!server.is_newer_than(&v(Some("b"), Some(900), Some(5))));
        assert!(server.is_newer_than(&v(Some("a"), Some(900), Some(10))));
        // No local ETag (old download): file time, then size.
        assert!(!server.is_newer_than(&v(None, Some(300), Some(1))));
        assert!(server.is_newer_than(&v(None, Some(100), Some(10))));
        assert!(!server.is_newer_than(&v(None, None, Some(10))));
        assert!(server.is_newer_than(&v(None, None, Some(11))));
        assert!(server.is_newer_than(&Version::default()));
    }

    #[test]
    fn merges_skybot_replies() {
        let o = |number: Option<&str>, name: &str, mag: f64| KnownObject {
            number: number.map(str::to_owned),
            name: name.to_owned(),
            ra: 0.0,
            dec: 0.0,
            class: String::new(),
            magnitude: Some(mag),
            uncertainty: 0.0,
            motion_ra: 0.0,
            motion_dec: 0.0,
        };
        let a = vec![o(Some("133105"), "2000 AA", 19.0)];
        let b = vec![
            o(Some("133105"), "2000 AA", 99.0),
            o(None, "2026 RA34", 21.0),
        ];
        let merged = merge_replies(Ok(a.clone()), Ok(b)).unwrap();
        let labels: Vec<_> = merged.iter().map(|o| (o.label(), o.magnitude)).collect();
        assert_eq!(labels, [("133105", Some(19.0)), ("2026 RA34", Some(21.0))]);
        assert_eq!(
            merge_replies(Err("x".into()), Ok(a.clone())).unwrap().len(),
            1
        );
        assert_eq!(merge_replies(Ok(a), Err("x".into())).unwrap().len(), 1);
        assert_eq!(
            merge_replies(Err("first".into()), Err("second".into())).unwrap_err(),
            "first"
        );
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
        let _ = d.rx.recv().unwrap().unwrap();
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
