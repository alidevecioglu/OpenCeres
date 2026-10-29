//! Opening an image set: expanding dropped or chosen paths (FITS files or a
//! zip of them), reading and measuring every frame in parallel, and putting
//! the frames in time order.

use std::path::Path;

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
    name: String,
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

fn load(source: Source) -> Result<Frame, String> {
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
    let jd = mjd_obs
        .map(|m| m + time::MJD_OFFSET)
        .or_else(|| date_obs.as_deref().and_then(time::parse_iso));

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
    Ok(ImageSet { name, frames })
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
    fn rejects_other_files_and_large_sets() {
        assert_eq!(
            open(&["notes.txt".into()]).err().as_deref(),
            Some(NOT_OPENABLE)
        );
        let five: Vec<String> = (0..5).map(|i| format!("/x/set/{i}.fits")).collect();
        assert!(open(&five).err().unwrap_or_default().contains("at most 4"));
    }
}
