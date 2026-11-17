//! Measuring the open set for the MPC report, in the background: each
//! frame is read again (the app keeps no pixels), plate-solved against the
//! Gaia stars if there are any, and the marks on it measured
//! ([`openceres_core::reduce`]). About a tenth of a second per Pan-STARRS
//! frame plus reading it, so it simply runs again whenever the marks or the
//! catalogue change.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use eframe::egui;
use openceres_core::gaia::GaiaStar;
use openceres_core::reduce::{self, Reduction};

use crate::history::Tagged;
use crate::load;

/// What a measurement was made from, to tell when it is out of date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key(u64);

/// Per frame: mark positions, and the object each belongs to.
pub type FrameMarks = (Vec<Vec<(f64, f64)>>, Vec<Vec<usize>>);

/// The marks of `objects` frame by frame: for each of `frames` frames, the
/// positions and the object each belongs to.
pub fn marks_by_frame(objects: &[Tagged], frames: usize) -> FrameMarks {
    let mut marks = vec![Vec::new(); frames];
    let mut owners = vec![Vec::new(); frames];
    for (oi, o) in objects.iter().enumerate() {
        for (fi, p) in o.at.iter().enumerate().take(frames) {
            if let Some(p) = p {
                marks[fi].push(*p);
                owners[fi].push(oi);
            }
        }
    }
    (marks, owners)
}

/// The key of measuring `marks` with or without a catalogue.
pub fn key(marks: &[Vec<(f64, f64)>], catalogue: bool) -> Key {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    catalogue.hash(&mut h);
    for frame in marks {
        frame.len().hash(&mut h);
        for (x, y) in frame {
            x.to_bits().hash(&mut h);
            y.to_bits().hash(&mut h);
        }
    }
    Key(h.finish())
}

pub struct Measured {
    pub key: Key,
    /// One per frame of the set, in its order.
    pub frames: Vec<Reduction>,
    pub owners: Vec<Vec<usize>>,
    pub solved: bool,
}

pub struct Job {
    pub rx: Receiver<Result<Measured, String>>,
}

/// Starts measuring: `names` are the set's frames in order, `paths` the
/// files it was opened from.
pub fn start(
    ctx: &egui::Context,
    paths: Vec<String>,
    names: Vec<String>,
    objects: &[Tagged],
    catalogue: Option<Arc<Vec<GaiaStar>>>,
) -> Job {
    let (marks, owners) = marks_by_frame(objects, names.len());
    let key = key(&marks, catalogue.is_some());
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut frames: Vec<Option<Reduction>> = vec![None; names.len()];
        let result = load::for_each_image(&paths, &names, |name, image| {
            for (i, n) in names.iter().enumerate() {
                if n == name && frames[i].is_none() {
                    frames[i] = Some(reduce::reduce(
                        &image,
                        catalogue.as_deref().map(Vec::as_slice),
                        &marks[i],
                    ));
                }
            }
        })
        .and_then(|()| {
            frames
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| "Some of the images could not be read again.".to_owned())
        })
        .map(|frames| Measured {
            key,
            frames,
            owners,
            solved: catalogue.is_some(),
        });
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    Job { rx }
}
