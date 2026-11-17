//! The MPC observation report of a search: the team's observer details
//! (kept in `report.txt` in the data folder, never with the images or in
//! git), temporary designations, and turning measured marks into the
//! 80-column report IASC's submission box takes and an ADES PSV file.
//!
//! Measuring is done by the core ([`openceres_core::reduce`]) on a
//! background thread (see [`crate::measure`]); this module only decides
//! what goes into the report and why something can't.

use std::path::PathBuf;

use openceres_core::ades::{self, Telescope};
use openceres_core::mpc::{self, Details};
use openceres_core::reduce::{MarkError, Measurement, Reduction};
use openceres_core::time;

use crate::history::Tagged;
use crate::prefs;

/// Copying a report whose positions come from the header WCS (no Gaia
/// plate solution) is not allowed: those positions are several arcseconds
/// off and would be rejected or, worse, mislead orbit computers.
pub const ALLOW_PREVIEW_COPY: bool = false;
/// Fewest measured frames for an object to be reported. Three positions
/// show that the motion is consistent (two always fit a straight line).
pub const MIN_FRAMES: usize = 3;
/// A track whose positions stray further than this (arcseconds RMS) from
/// uniform motion probably has a wrong mark.
pub const MAX_TRACK_RMS: f64 = 0.5;

/// The team's details for report headers, shared by everyone who measures
/// on this computer. Who measured is per search (`searched_by`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    /// Observatory code; empty = from the images (e.g. `F51` for Pan-STARRS 1).
    pub code: String,
    /// Comma-separated, initials + surname.
    pub observers: String,
    pub contact_name: String,
    pub contact_address: String,
    pub contact_email: String,
    /// Telescope: aperture (m), focal ratio, design, detector.
    pub aperture: String,
    pub f_ratio: String,
    pub design: String,
    pub detector: String,
    /// Start of temporary designations, e.g. `OC` → `OC0001`.
    pub prefix: String,
    pub ack: String,
    pub ack_emails: String,
}

const KEYS: [&str; 12] = [
    "code",
    "observers",
    "contact_name",
    "contact_address",
    "contact_email",
    "aperture",
    "f_ratio",
    "design",
    "detector",
    "prefix",
    "ack",
    "ack_emails",
];

impl Config {
    fn field(&mut self, key: &str) -> Option<&mut String> {
        Some(match key {
            "code" => &mut self.code,
            "observers" => &mut self.observers,
            "contact_name" => &mut self.contact_name,
            "contact_address" => &mut self.contact_address,
            "contact_email" => &mut self.contact_email,
            "aperture" => &mut self.aperture,
            "f_ratio" => &mut self.f_ratio,
            "design" => &mut self.design,
            "detector" => &mut self.detector,
            "prefix" => &mut self.prefix,
            "ack" => &mut self.ack,
            "ack_emails" => &mut self.ack_emails,
            _ => return None,
        })
    }

    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=')
                && let Some(f) = c.field(k.trim())
            {
                *f = v.trim().to_owned();
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        let mut c = self.clone();
        KEYS.iter()
            .map(|k| {
                let v = c
                    .field(k)
                    .map(|v| v.replace(['\n', '\r'], " "))
                    .unwrap_or_default();
                format!("{k}={v}\n")
            })
            .collect()
    }

    pub fn load() -> Config {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Config::default, |t| Config::parse(&t))
    }

    /// Best effort, like the preferences.
    pub fn save(&self) {
        if let Some(path) = path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, self.to_text());
        }
    }

    pub fn telescope(&self) -> Telescope {
        let num = |s: &str| {
            s.trim()
                .trim_start_matches("f/")
                .replace(',', ".")
                .parse::<f64>()
                .ok()
        };
        Telescope {
            aperture: num(&self.aperture),
            f_ratio: num(&self.f_ratio),
            design: self.design.trim().to_owned(),
            detector: self.detector.trim().to_owned(),
        }
    }

    /// Header details for a report by `measured_by` of images from
    /// observatory `image_code`.
    pub fn details(&self, measured_by: &str, image_code: Option<&str>) -> Details {
        let list = |s: &str| -> Vec<String> {
            s.split(',')
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
                .collect()
        };
        Details {
            code: self.code(image_code),
            contact_name: self.contact_name.trim().to_owned(),
            contact_address: self.contact_address.trim().to_owned(),
            contact_email: self.contact_email.trim().to_owned(),
            observers: list(&self.observers),
            measurers: list(measured_by),
            telescope: self.telescope().tel_line(),
            catalogue: mpc::GAIA_DR3.to_owned(),
            ack: self.ack.trim().to_owned(),
            ack_emails: self.ack_emails.trim().to_owned(),
            comments: Vec::new(),
        }
    }

    /// The observatory code: the one set here, else the images'.
    pub fn code(&self, image_code: Option<&str>) -> String {
        let own = self.code.trim().to_uppercase();
        if own.is_empty() {
            image_code.unwrap_or_default().to_owned()
        } else {
            own
        }
    }

    /// Missing header details that stop the report from being copied.
    pub fn missing(&self, measured_by: &str, image_code: Option<&str>) -> Vec<&'static str> {
        let mut out = Vec::new();
        let code = self.code(image_code);
        if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push("observatory code");
        }
        if self.observers.trim().is_empty() {
            out.push("observers");
        }
        if measured_by.trim().is_empty() {
            out.push("who measured");
        }
        out
    }

    /// Advice that does not block copying: names not in the MPC's style,
    /// no contact or telescope, a bad designation prefix.
    pub fn advice(&self, measured_by: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .observers
            .split(',')
            .chain(measured_by.split(','))
            .chain(std::iter::once(self.contact_name.as_str()))
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .filter_map(mpc::name_problem)
            .collect();
        if self.contact_name.trim().is_empty() || self.contact_email.trim().is_empty() {
            out.push("No contact name and e-mail: the MPC uses the observatory's defaults.".into());
        }
        if self.telescope().tel_line().is_empty() {
            out.push("No telescope: the MPC uses the observatory's defaults.".into());
        }
        if let Some(p) = mpc::designation_problem(&designation(self.prefix(), 1)) {
            out.push(format!("Designation prefix: {p}."));
        }
        out
    }

    pub fn prefix(&self) -> &str {
        let p = self.prefix.trim();
        if p.is_empty() { DEFAULT_PREFIX } else { p }
    }
}

pub const DEFAULT_PREFIX: &str = "OC";

fn path() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("report.txt"))
}

/// Temporary designation number `n` with `prefix`: six characters (the
/// MPC's preferred length) when the number fits, e.g. `OC0001`, else
/// as long as needed up to the maximum of seven.
pub fn designation(prefix: &str, n: u32) -> String {
    let width = 6usize.saturating_sub(prefix.len()).max(1);
    let d = format!("{prefix}{n:0width$}");
    d.chars().take(7).collect()
}

/// What stops an object (or a frame of it) from going into the report.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    Ready,
    /// Positions from the header WCS: no Gaia plate solution yet.
    Preview,
    /// Fewer than [`MIN_FRAMES`] frames measured.
    TooFew(usize),
    /// Measured on enough frames, but a mark was placed without snapping
    /// (the student should check it).
    Unsnapped(Vec<usize>),
    /// The positions don't line up on a uniform motion.
    Track(f64),
    /// Not measured yet.
    Waiting,
}

impl Check {
    pub fn text(&self) -> String {
        match self {
            Check::Ready => "Ready".into(),
            Check::Preview => "Preview only".into(),
            Check::TooFew(n) => format!("Measured on {n} frames, {MIN_FRAMES} needed"),
            Check::Unsnapped(f) => format!(
                "Clicked, not snapped on frame {}",
                f.iter()
                    .map(|i| (i + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Check::Track(rms) => format!("Positions stray {rms:.2}″ from a straight track"),
            Check::Waiting => "Measuring…".into(),
        }
    }

    /// Whether the object can be in a submittable report.
    pub fn reportable(&self) -> bool {
        matches!(self, Check::Ready | Check::Unsnapped(_))
    }
}

/// One object's row in the review.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub index: usize,
    pub label: String,
    pub designation: String,
    /// Frames measured and frames marked.
    pub measured: usize,
    pub marked: usize,
    /// RMS (arcseconds) of the positions about uniform motion.
    pub track_rms: Option<f64>,
    pub mag: Option<f64>,
    pub known: Option<String>,
    pub check: Check,
    pub include: bool,
    pub discovery: bool,
    /// Frames left out and why, e.g. `frame 1: it touches a gap…`.
    pub notes: Vec<String>,
}

/// Measurements of the open set: one reduction per frame (index as the
/// set's frames) and, per frame, which object each of its marks belongs to.
pub struct Measured<'a> {
    pub frames: &'a [Reduction],
    /// For frame `f`, `owners[f][k]` is the object of mark `k`.
    pub owners: &'a [Vec<usize>],
    /// Whether the positions come from Gaia plate solutions.
    pub solved: bool,
}

impl Measured<'_> {
    fn mark(&self, object: usize, frame: usize) -> Option<&Result<Measurement, MarkError>> {
        let k = self.owners.get(frame)?.iter().position(|&o| o == object)?;
        self.frames.get(frame)?.marks.get(k)
    }
}

/// Everything the review shows.
pub struct Built {
    pub mpc: String,
    pub ades: String,
    pub rows: Vec<Row>,
    /// Number of 80-column observation lines.
    pub lines: usize,
}

pub struct Inputs<'a> {
    pub config: &'a Config,
    pub measured_by: &'a str,
    pub image_code: Option<&'a str>,
    pub objects: &'a [Tagged],
    pub measured: Option<Measured<'a>>,
    /// Designation prefix and first number for this search.
    pub prefix: &'a str,
    pub start: u32,
}

/// Builds the report from the objects' measurements.
pub fn build(inp: &Inputs) -> Built {
    let details = inp.config.details(inp.measured_by, inp.image_code);
    let telescope = inp.config.telescope();
    let mut rows = Vec::new();
    let mut lines80 = Vec::new();
    let mut ades_rows = Vec::new();
    for (next, (oi, o)) in (inp.start..).zip(inp.objects.iter().enumerate()) {
        let designation = designation(inp.prefix, next);
        let marked = o.marked();
        let mut row = Row {
            index: oi,
            label: o.label.clone(),
            designation: designation.clone(),
            measured: 0,
            marked,
            track_rms: None,
            mag: None,
            known: o.known.clone(),
            check: Check::Waiting,
            include: !o.exclude,
            discovery: o.discovery,
            notes: Vec::new(),
        };
        let Some(m) = &inp.measured else {
            rows.push(row);
            continue;
        };
        // (frame, measurement) of the frames that measured well.
        let mut good: Vec<(usize, &Measurement)> = Vec::new();
        for (fi, p) in o.at.iter().enumerate() {
            if p.is_none() {
                continue;
            }
            match m.mark(oi, fi) {
                Some(Ok(meas)) => match &meas.problem {
                    None => good.push((fi, meas)),
                    Some(problem) => row.notes.push(format!("Frame {}: {problem}.", fi + 1)),
                },
                Some(Err(e)) => row.notes.push(format!("Frame {}: {e}.", fi + 1)),
                None => {}
            }
        }
        row.measured = good.len();
        let times: Vec<Option<f64>> = good.iter().map(|(f, _)| m.frames[*f].mid_jd).collect();
        row.track_rms = track_rms(
            &good
                .iter()
                .zip(&times)
                .filter_map(|((_, g), t)| Some((t.as_ref().copied()?, g.ra, g.dec)))
                .collect::<Vec<_>>(),
        );
        let mags: Vec<f64> = good
            .iter()
            .filter(|(_, g)| g.mag_problem.is_none())
            .filter_map(|(_, g)| g.mag.map(|v| v.0))
            .collect();
        row.mag = (!mags.is_empty()).then(|| mags.iter().sum::<f64>() / mags.len() as f64);
        let unsnapped: Vec<usize> = good
            .iter()
            .map(|(f, _)| *f)
            .filter(|f| o.unsnapped.get(*f).copied().unwrap_or(false))
            .collect();
        row.check = if !m.solved {
            Check::Preview
        } else if good.len() < MIN_FRAMES {
            Check::TooFew(good.len())
        } else if let Some(rms) = row.track_rms.filter(|r| *r > MAX_TRACK_RMS) {
            Check::Track(rms)
        } else if !unsnapped.is_empty() {
            Check::Unsnapped(unsnapped)
        } else {
            Check::Ready
        };
        let goes_in =
            row.include && (row.check.reportable() || (!m.solved && good.len() >= MIN_FRAMES));
        if goes_in {
            for (k, ((fi, g), t)) in good.iter().zip(&times).enumerate() {
                let Some(jd) = *t else { continue };
                let frame = &m.frames[*fi];
                let mag = g.mag.filter(|_| g.mag_problem.is_none());
                lines80.push(mpc::Observation {
                    number: None,
                    designation: designation.clone(),
                    discovery: row.discovery && k == 0,
                    note1: ' ',
                    note2: 'C',
                    date: time::jd_to_calendar(jd),
                    ra: g.ra,
                    dec: g.dec,
                    magnitude: mag.map(|m| m.0),
                    band: 'G',
                    observatory: details.code.clone(),
                });
                let plate = frame.plate();
                let (sr, sd) = g.sigma.unwrap_or((1.0, 1.0));
                ades_rows.push(ades::Observation {
                    trk_sub: designation.clone(),
                    mode: "CCD".into(),
                    station: details.code.clone(),
                    jd,
                    ra: g.ra,
                    dec: g.dec,
                    rms_ra: sr,
                    rms_dec: sd,
                    mag,
                    band: "G".into(),
                    phot_ap: frame.scale.map(|s| s * frame.aperture),
                    snr: Some(g.snr()),
                    seeing: frame.scale.map(|s| s * frame.fwhm),
                    exposure: frame.exposure,
                    rms_fit: plate.map(|p| {
                        (p.rms.0.powi(2) + p.rms.1.powi(2)).sqrt() / std::f64::consts::SQRT_2
                    }),
                    n_stars: plate.map(|p| p.stars),
                    discovery: row.discovery && k == 0,
                });
            }
        }
        rows.push(row);
    }
    Built {
        mpc: mpc::report(&details, &lines80),
        ades: ades::psv(&details, &telescope, &ades_rows),
        lines: lines80.len(),
        rows,
    }
}

/// RMS (arcseconds, per axis) of positions (jd, ra, dec) about a uniform
/// motion fitted in time; `None` with fewer than three.
pub fn track_rms(points: &[(f64, f64, f64)]) -> Option<f64> {
    if points.len() < 3 {
        return None;
    }
    let (t0, ra0, dec0) = points[0];
    let cos = dec0.to_radians().cos();
    let p: Vec<(f64, f64, f64)> = points
        .iter()
        .map(|&(t, ra, dec)| {
            let dra = ((ra - ra0 + 180.0).rem_euclid(360.0) - 180.0) * cos * 3600.0;
            ((t - t0) * 1440.0, dra, (dec - dec0) * 3600.0)
        })
        .collect();
    let n = p.len() as f64;
    let mt = p.iter().map(|q| q.0).sum::<f64>() / n;
    let var: f64 = p.iter().map(|q| (q.0 - mt).powi(2)).sum();
    if var <= 0.0 {
        return None;
    }
    let mut sum = 0.0;
    for k in [1, 2] {
        let get = |q: &(f64, f64, f64)| if k == 1 { q.1 } else { q.2 };
        let mean = p.iter().map(get).sum::<f64>() / n;
        let slope = p.iter().map(|q| (q.0 - mt) * (get(q) - mean)).sum::<f64>() / var;
        sum += p
            .iter()
            .map(|q| (get(q) - mean - slope * (q.0 - mt)).powi(2))
            .sum::<f64>();
    }
    // Two constants per axis are fitted.
    Some((sum / (2.0 * (n - 2.0))).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openceres_core::photometry::Flux;

    #[test]
    fn config_round_trips() {
        let c = Config {
            code: "F51".into(),
            observers: "K. Chambers, R. J. Wainscoat".into(),
            contact_name: "A. Teacher".into(),
            contact_address: "School, Street 1, City".into(),
            contact_email: "a@example.org".into(),
            aperture: "1.8".into(),
            f_ratio: "4.4".into(),
            design: "Ritchey-Chretien".into(),
            detector: "CCD".into(),
            prefix: "TRK".into(),
            ack: "batch\n1".into(),
            ack_emails: "a@example.org".into(),
        };
        let back = Config::parse(&c.to_text());
        assert_eq!(back.ack, "batch 1");
        assert_eq!(
            Config {
                ack: "batch 1".into(),
                ..c
            },
            back
        );
    }

    #[test]
    fn designations_are_six_characters_when_possible() {
        assert_eq!(designation("OC", 1), "OC0001");
        assert_eq!(designation("TRK", 42), "TRK042");
        assert_eq!(designation("ABCDEF", 3), "ABCDEF3");
        assert_eq!(designation("OC", 123456), "OC12345");
        assert!(mpc::designation_problem(&designation("OC", 1)).is_none());
    }

    #[test]
    fn missing_details_block_and_code_comes_from_images() {
        let c = Config::default();
        assert_eq!(c.missing("", Some("F51")), ["observers", "who measured"]);
        assert_eq!(
            c.missing("A. Student", None),
            ["observatory code", "observers"]
        );
        let c = Config {
            observers: "K. Chambers".into(),
            ..Config::default()
        };
        assert!(c.missing("A. Student", Some("F51")).is_empty());
        assert_eq!(c.details("A. Student", Some("F51")).code, "F51");
        assert_eq!(
            Config {
                code: "g96".into(),
                ..c
            }
            .code(Some("F51")),
            "G96"
        );
    }

    #[test]
    fn track_rms_measures_scatter_about_uniform_motion() {
        let pts: Vec<(f64, f64, f64)> = (0..4)
            .map(|i| {
                let t = 2_461_000.0 + i as f64 * 15.0 / 1440.0;
                (t, 21.0 + i as f64 * 1e-4, 4.0 - i as f64 * 5e-5)
            })
            .collect();
        assert!(track_rms(&pts).unwrap() < 1e-6);
        let mut bent = pts.clone();
        bent[2].2 += 1.0 / 3600.0;
        assert!(track_rms(&bent).unwrap() > 0.3);
        assert_eq!(track_rms(&pts[..2]), None);
    }

    fn measurement(ra: f64, dec: f64) -> Measurement {
        Measurement {
            x: 0.0,
            y: 0.0,
            sigma_px: (0.05, 0.05),
            ra,
            dec,
            sigma: Some((0.05, 0.06)),
            solved: true,
            moved: 0.1,
            problem: None,
            flux: Flux {
                flux: 1000.0,
                error: 50.0,
                clean: true,
            },
            mag: Some((20.04, 0.05)),
            mag_problem: None,
        }
    }

    fn reduction(jd: f64, marks: Vec<Result<Measurement, MarkError>>) -> Reduction {
        Reduction {
            mid_jd: Some(jd),
            exposure: Some(45.0),
            detected: 300,
            fwhm: 5.0,
            aperture: 5.0,
            solution: None,
            zero_point: None,
            scale: Some(0.256),
            marks,
        }
    }

    #[test]
    fn builds_exact_80_column_lines_and_ades() {
        let config = Config {
            observers: "K. Chambers".into(),
            ..Config::default()
        };
        let jd0 = time::parse_iso("2026-10-04T10:16:02.83").unwrap();
        let frames: Vec<Reduction> = (0..4)
            .map(|i| {
                let t = jd0 + i as f64 * 15.0 / 1440.0;
                let m = if i == 1 {
                    // Object B was not found on frame 2.
                    vec![
                        Ok(measurement(21.0 + i as f64 * 1e-4, 4.1)),
                        Err(MarkError::NoSource),
                    ]
                } else {
                    vec![
                        Ok(measurement(21.0 + i as f64 * 1e-4, 4.1)),
                        Ok(measurement(22.0, 4.2 + i as f64 * 1e-4)),
                    ]
                };
                reduction(t, m)
            })
            .collect();
        let owners = vec![vec![0, 1]; 4];
        let objects = vec![
            Tagged {
                label: "A".into(),
                at: vec![Some((1.0, 1.0)); 4],
                discovery: true,
                unsnapped: vec![false; 4],
                ..Tagged::default()
            },
            Tagged {
                label: "B".into(),
                at: vec![Some((2.0, 2.0)); 4],
                unsnapped: vec![false, false, true, false],
                ..Tagged::default()
            },
        ];
        let inp = Inputs {
            config: &config,
            measured_by: "A. Student",
            image_code: Some("F51"),
            objects: &objects,
            measured: Some(Measured {
                frames: &frames,
                owners: &owners,
                solved: true,
            }),
            prefix: "OC",
            start: 7,
        };
        let b = build(&inp);
        let lines: Vec<&str> = b.mpc.lines().collect();
        assert_eq!(
            &lines[..3],
            ["COD F51", "OBS K. Chambers", "MEA A. Student"]
        );
        assert_eq!(b.rows[0].check, Check::Ready);
        assert_eq!(b.rows[1].check, Check::Unsnapped(vec![2]));
        assert!(b.rows[1].notes[0].starts_with("Frame 2: no source"));
        assert_eq!(b.lines, 7);
        let obs: Vec<&str> = lines.iter().copied().filter(|l| l.len() == 80).collect();
        assert_eq!(obs.len(), 7);
        assert!(lines.iter().all(|l| l.len() <= 80));
        // Columns 6–12 the designation, 13 the asterisk (A's first line only), 15 C.
        assert_eq!(&obs[0][..15], "     OC0007 * C");
        assert_eq!(&obs[1][..15], "     OC0007   C");
        assert_eq!(&obs[4][..15], "     OC0008   C");
        assert_eq!(&obs[0][15..32], "2026 10 04.427811");
        assert_eq!(&obs[0][65..71], "20.0 G");
        assert_eq!(&obs[0][77..80], "F51");
        assert!(
            b.ades
                .lines()
                .any(|l| l.split('|').any(|f| f.trim() == "Gaia3"))
        );
        assert_eq!(b.ades.lines().filter(|l| l.starts_with("OC000")).count(), 7);

        // Excluded objects and preview reports.
        let mut excluded = objects.clone();
        excluded[1].exclude = true;
        let b = build(&Inputs {
            objects: &excluded,
            ..inp
        });
        assert_eq!(b.lines, 4);
    }
}
