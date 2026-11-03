//! Opening an image set: expanding dropped or chosen paths (FITS files or a
//! zip of them), reading and measuring every frame in parallel, and putting
//! the frames in time order.

use std::path::Path;

use openceres_core::align::{self, Affine};
use openceres_core::{detect, fits, known, render, time, wcs::Wcs};

use crate::zip;

pub const MAX_IMAGES: usize = 4;
const FITS_EXTENSIONS: [&str; 3] = ["fits", "fit", "fts"];
pub const OPENABLE_EXTENSIONS: [&str; 4] = ["fits", "fit", "fts", "zip"];
pub const NOT_OPENABLE: &str =
    "Only FITS images (.fits, .fit, .fts) or a .zip of them can be opened.";

/// A detected star, as much as the viewer needs. Positions are 0-based FITS
/// pixels (y = 0 is the bottom row).
pub struct StarMark {
    pub x: f32,
    pub y: f32,
    pub fwhm: f32,
}

/// One processed frame. The raw pixel data is not kept: only the display
/// bytes (until they are uploaded to the GPU), the stars, the WCS and the
/// few header values the app needs.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// 8-bit display pixels, width × height, top row first.
    pub gray: Vec<u8>,
    pub stars: Vec<StarMark>,
    pub wcs: Option<Wcs>,
    pub date_obs: Option<String>,
    pub mjd_obs: Option<f64>,
    /// Julian Date of the exposure start, for ordering and the time span.
    pub jd: Option<f64>,
    /// Julian Date of the middle of the exposure, for known-object positions.
    pub mid_jd: Option<f64>,
    /// MPC observatory code, if the telescope is recognised.
    pub observatory: Option<&'static str>,
    pub filter: Option<String>,
    /// Maps this frame's pixels onto the first frame's, found by matching
    /// stars (the identity for the first frame). `None` if too few stars
    /// matched; the frame is then shown as it is.
    pub to_reference: Option<Affine>,
    /// File name, without the folder.
    pub name: String,
}

pub struct ImageSet {
    /// Short name for reports, e.g. `XY57_p01`: the folder inside a zip, the
    /// zip's name, or the folder holding the chosen files.
    pub name: String,
    pub frames: Vec<Frame>,
}

pub fn has_extension(path: &str, exts: &[&str]) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// A FITS file to load: its display name and either a path or its bytes (from a zip).
enum Source {
    File(String),
    Bytes(String, Vec<u8>),
}

/// Expands dropped/selected paths into FITS sources and a set name.
fn collect(paths: &[String]) -> Result<(String, Vec<Source>), String> {
    let mut sources = Vec::new();
    let mut set_name = None;
    for path in paths {
        if has_extension(path, &["zip"]) {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", file_name(path)))?;
            let entries = zip::entries(&bytes).map_err(|e| format!("{}: {e}", file_name(path)))?;
            let fits: Vec<_> = entries
                .iter()
                .filter(|e| has_extension(&e.name, &FITS_EXTENSIONS))
                // macOS adds resource-fork copies (`__MACOSX/`, `._name`) to zips.
                .filter(|e| {
                    !e.name.starts_with("__MACOSX/") && !file_name(&e.name).starts_with("._")
                })
                .collect();
            if fits.is_empty() {
                return Err(format!("{}: no FITS images inside", file_name(path)));
            }
            if sources.len() + fits.len() > MAX_IMAGES {
                return Err(too_many(sources.len() + fits.len()));
            }
            let folder = Path::new(&fits[0].name)
                .parent()
                .and_then(|p| p.file_name());
            let stem = Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned());
            set_name.get_or_insert(
                folder
                    .map(|f| f.to_string_lossy().into_owned())
                    .or(stem)
                    .unwrap_or_default(),
            );
            for entry in fits {
                sources.push(Source::Bytes(
                    file_name(&entry.name),
                    zip::extract(&bytes, entry)?,
                ));
            }
        } else if has_extension(path, &FITS_EXTENSIONS) {
            let folder = Path::new(path).parent().and_then(|p| p.file_name());
            set_name.get_or_insert(
                folder.map_or_else(String::new, |f| f.to_string_lossy().into_owned()),
            );
            sources.push(Source::File(path.clone()));
        }
    }
    if sources.is_empty() {
        return Err(NOT_OPENABLE.into());
    }
    if sources.len() > MAX_IMAGES {
        return Err(too_many(sources.len()));
    }
    Ok((set_name.unwrap_or_default(), sources))
}

fn too_many(n: usize) -> String {
    format!("An image set has at most {MAX_IMAGES} images (got {n}).")
}

fn read(source: Source) -> Result<(String, fits::Image), String> {
    let (name, image) = match source {
        Source::File(path) => (file_name(&path), fits::read(&path)),
        // The zip entry's bytes are dropped as soon as they are parsed.
        Source::Bytes(name, bytes) => {
            let image = fits::parse(&bytes);
            (name, image)
        }
    };
    let image = image.map_err(|e| format!("{name}: {e}"))?;
    if image.width == 0 || image.height == 0 {
        return Err(format!("{name}: image is empty"));
    }
    Ok((name, image))
}

fn load(source: Source) -> Result<Frame, String> {
    let (name, image) = read(source)?;
    let bg = detect::background(&image);
    let gray = render::grayscale(&image, render::Stretch::auto(bg));
    let wcs = Wcs::from_header(&image.header);
    let stars = detect::detect_with_background(&image, bg, &detect::DetectOptions::default())
        .into_iter()
        .map(|s| StarMark {
            x: s.x as f32,
            y: s.y as f32,
            fwhm: s.fwhm as f32,
        })
        .collect();

    let h = &image.header;
    let date_obs = h.text("DATE-OBS").map(str::to_owned);
    let mjd_obs = h.float("MJD-OBS");
    let jd = time::start_jd(h);

    Ok(Frame {
        width: image.width,
        height: image.height,
        gray,
        stars,
        wcs,
        date_obs,
        mjd_obs,
        jd,
        mid_jd: time::mid_exposure_jd(h),
        observatory: known::observatory_code(h),
        filter: h.text("FILTER").map(str::to_owned),
        to_reference: None,
        name,
    })
}

/// Frames processed at the same time. Each needs ~60 MB of scratch memory for
/// star detection on a Pan-STARRS chip, so this bounds the loading peak.
const PARALLEL: usize = 2;

/// Loads an image set, [`PARALLEL`] frames at a time, and sorts the frames by
/// observation time (or by file name if any frame lacks a time).
pub fn open(paths: &[String]) -> Result<ImageSet, String> {
    if paths.is_empty() {
        return Err("No images selected.".into());
    }
    let (name, sources) = collect(paths)?;
    let mut results = Vec::with_capacity(sources.len());
    let mut pending = sources.into_iter().peekable();
    while pending.peek().is_some() {
        let batch: Vec<_> = pending.by_ref().take(PARALLEL).collect();
        std::thread::scope(|s| {
            let jobs: Vec<_> = batch
                .into_iter()
                .map(|src| s.spawn(move || load(src)))
                .collect();
            results.extend(jobs.into_iter().map(|j| {
                j.join()
                    .unwrap_or_else(|_| Err("image loader crashed".into()))
            }));
        });
    }
    let frames: Result<Vec<_>, String> = results.into_iter().collect();
    release_freed_memory();
    let mut frames = frames?;
    if frames.iter().all(|f| f.jd.is_some()) {
        frames.sort_by(|a, b| a.jd.unwrap_or(0.0).total_cmp(&b.jd.unwrap_or(0.0)));
    } else {
        frames.sort_by(|a, b| a.name.cmp(&b.name));
    }
    align_frames(&mut frames);
    Ok(ImageSet { name, frames })
}

/// Matches every frame's stars against the first frame's (in parallel; a
/// few tens of milliseconds each), so the blink can hold the stars still.
fn align_frames(frames: &mut [Frame]) {
    let positions = |f: &Frame| -> Vec<(f64, f64)> {
        f.stars
            .iter()
            .map(|s| (f64::from(s.x), f64::from(s.y)))
            .collect()
    };
    let Some((first, rest)) = frames.split_first_mut() else {
        return;
    };
    first.to_reference = Some(Affine::IDENTITY);
    let reference = positions(first);
    std::thread::scope(|s| {
        for frame in rest {
            let reference = &reference;
            s.spawn(move || {
                frame.to_reference = align::align(reference, &positions(frame))
                    .map(|a| a.transform)
                    .filter(|t| t.inverse().is_some());
            });
        }
    });
}

/// A square to cut from one frame for a detailed report.
pub struct CutoutRequest {
    /// The frame's file name ([`Frame::name`]).
    pub file: String,
    /// Centre in that frame's 0-based FITS pixels.
    pub x: f64,
    pub y: f64,
    /// Side in pixels (odd, so the centre pixel is in the middle).
    pub size: usize,
}

/// Cuts the requested squares from the frames exactly as they are shown
/// (same stretch), reading the set's files again one frame at a time: the
/// app keeps no pixels after uploading them, and this runs rarely. Returns
/// a grayscale square per request (top row first; black outside the
/// frame), `None` where the frame was not found.
pub fn cutouts(
    paths: &[String],
    requests: &[CutoutRequest],
) -> Result<Vec<Option<Vec<u8>>>, String> {
    let (_, sources) = collect(paths)?;
    let mut out: Vec<Option<Vec<u8>>> = requests.iter().map(|_| None).collect();
    for source in sources {
        let name = match &source {
            Source::File(p) => file_name(p),
            Source::Bytes(n, _) => n.clone(),
        };
        if !requests.iter().any(|r| r.file == name) {
            continue;
        }
        let (_, image) = read(source)?;
        let gray = render::grayscale(&image, render::Stretch::auto(detect::background(&image)));
        for (slot, r) in out.iter_mut().zip(requests).filter(|(_, r)| r.file == name) {
            *slot = Some(cut(&gray, image.width, image.height, r));
        }
    }
    release_freed_memory();
    Ok(out)
}

fn cut(gray: &[u8], width: usize, height: usize, r: &CutoutRequest) -> Vec<u8> {
    let half = (r.size / 2) as i64;
    let (cx, cy) = (r.x.round() as i64, r.y.round() as i64);
    let mut out = vec![0u8; r.size * r.size];
    for j in 0..r.size as i64 {
        // Top row of the cutout = highest y; `gray` stores the top row first.
        let y = cy + half - j;
        if y < 0 || y >= height as i64 {
            continue;
        }
        let row = (height as i64 - 1 - y) as usize * width;
        for i in 0..r.size as i64 {
            let x = cx - half + i;
            if x >= 0 && x < width as i64 {
                out[(j * r.size as i64 + i) as usize] = gray[row + x as usize];
            }
        }
    }
    out
}

/// Loading needs several hundred MB of temporary images. The allocator keeps
/// freed large blocks around for reuse (macOS returns them lazily, glibc
/// raises its mmap threshold), so ask it to give them back to the system now.
fn release_freed_memory() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: libSystem call without preconditions; a null zone means
        // all zones and a goal of 0 means release as much as possible.
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: glibc call without preconditions.
        unsafe { malloc_trim(0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutouts_follow_fits_orientation() {
        // 4 × 3 image, top row first in `gray`: FITS (x, y) holds 10 y + x.
        let (w, h) = (4, 3);
        let gray: Vec<u8> = (0..h)
            .rev()
            .flat_map(|y| (0..w).map(move |x| (10 * y + x) as u8))
            .collect();
        let r = CutoutRequest {
            file: String::new(),
            x: 1.0,
            y: 1.0,
            size: 3,
        };
        // Top row of the cutout is y = 2.
        assert_eq!(cut(&gray, w, h, &r), [20, 21, 22, 10, 11, 12, 0, 1, 2]);
        let corner = CutoutRequest {
            x: 0.0,
            y: 0.0,
            ..r
        };
        assert_eq!(cut(&gray, w, h, &corner), [0, 10, 11, 0, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn rejects_other_files_and_large_sets() {
        assert_eq!(
            open(&["notes.txt".into()]).err().as_deref(),
            Some(NOT_OPENABLE)
        );
        let five: Vec<String> = (0..5).map(|i| format!("/x/set/{i}.fits")).collect();
        assert!(open(&five).err().unwrap_or_default().contains("at most 4"));
    }
}
