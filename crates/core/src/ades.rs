//! Observation reports in ADES PSV (pipe-separated values), the MPC's
//! newer format (IAU Astrometry Data Exchange Standard, version 2022: the
//! field definitions in `adesmaster.xml` and the MPC's "ADES field values"
//! page for the `mode`, `astCat` and `band` codes).
//!
//! A PSV file is a header of `# group` / `! keyword value` lines, then one
//! line naming the columns and one line per observation, fields separated
//! by `|` and padded with spaces for reading.

use crate::mpc::Details;
use crate::time;

/// The MPC's code for Gaia DR3 in `astCat` and `photCat`.
pub const GAIA_DR3: &str = "Gaia3";

/// The telescope, as the ADES `telescope` group describes it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Telescope {
    /// Aperture in metres.
    pub aperture: Option<f64>,
    pub f_ratio: Option<f64>,
    /// e.g. `Reflector`, `Ritchey-Chretien`, `Schmidt-Cassegrain`.
    pub design: String,
    /// e.g. `CCD`, `CMOS`.
    pub detector: String,
}

impl Telescope {
    /// The 80-column `TEL` text: aperture, focal ratio, design and detector,
    /// e.g. `1.80-m f/4.4 Ritchey-Chretien + CCD`; empty if no aperture.
    pub fn tel_line(&self) -> String {
        let Some(a) = self.aperture.filter(|a| *a > 0.0) else {
            return String::new();
        };
        let mut s = format!("{a:.2}-m");
        if let Some(f) = self.f_ratio.filter(|f| *f > 0.0) {
            s += &format!(" f/{}", trim_number(f, 1));
        }
        if !self.design.trim().is_empty() {
            s += &format!(" {}", self.design.trim());
        }
        let detector = self.detector.trim();
        s += &format!(" + {}", if detector.is_empty() { "CCD" } else { detector });
        s
    }
}

fn trim_number(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// One optical observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// Observer-assigned tracklet name (the temporary designation).
    pub trk_sub: String,
    /// `CCD`, `CMO`, …
    pub mode: String,
    pub station: String,
    /// UTC Julian date of mid-exposure.
    pub jd: f64,
    pub ra: f64,
    pub dec: f64,
    /// 1σ in RA ×cos Dec and Dec, arcseconds.
    pub rms_ra: f64,
    pub rms_dec: f64,
    pub mag: Option<(f64, f64)>,
    pub band: String,
    /// Photometric aperture radius, arcseconds.
    pub phot_ap: Option<f64>,
    pub snr: Option<f64>,
    /// Seeing (FWHM), arcseconds.
    pub seeing: Option<f64>,
    /// Exposure time, seconds.
    pub exposure: Option<f64>,
    /// RMS of the astrometric fit (arcseconds) and stars in it.
    pub rms_fit: Option<f64>,
    pub n_stars: Option<usize>,
    pub discovery: bool,
}

/// `2026-10-04T10:16:02.83Z`: rounded in centiseconds, so a time just
/// before midnight never prints as second 60.
pub fn iso_time(jd: f64) -> String {
    let cs = ((jd - time::MJD_OFFSET) * 8_640_000.0).round() as i64;
    let (day, rem) = (cs.div_euclid(8_640_000), cs.rem_euclid(8_640_000));
    let date = time::jd_to_calendar(day as f64 + time::MJD_OFFSET);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:02}Z",
        date.year,
        date.month,
        date.day.floor() as u32,
        rem / 360_000,
        rem / 6000 % 60,
        rem / 100 % 60,
        rem % 100
    )
}

/// The whole PSV document.
pub fn psv(details: &Details, telescope: &Telescope, observations: &[Observation]) -> String {
    let mut out = String::from("# version=2022\n");
    out += "# observatory\n";
    out += &format!("! mpcCode {}\n", details.code.trim());
    let name = details.contact_name.trim();
    if !name.is_empty() {
        out += "# submitter\n";
        out += &format!("! name {name}\n");
    }
    for (group, names) in [
        ("observers", &details.observers),
        ("measurers", &details.measurers),
    ] {
        let names: Vec<&str> = names
            .iter()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
            .collect();
        if !names.is_empty() {
            out += &format!("# {group}\n");
            for n in names {
                out += &format!("! name {n}\n");
            }
        }
    }
    if let Some(a) = telescope.aperture.filter(|a| *a > 0.0) {
        out += "# telescope\n";
        out += &format!("! aperture {}\n", trim_number(a, 2));
        if !telescope.design.trim().is_empty() {
            out += &format!("! design {}\n", telescope.design.trim());
        }
        let detector = telescope.detector.trim();
        out += &format!(
            "! detector {}\n",
            if detector.is_empty() { "CCD" } else { detector }
        );
        if let Some(f) = telescope.f_ratio.filter(|f| *f > 0.0) {
            out += &format!("! fRatio {}\n", trim_number(f, 1));
        }
    }
    let comments: Vec<&str> = details
        .comments
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if !comments.is_empty() {
        out += "# comment\n";
        for c in comments {
            out += &format!("! line {c}\n");
        }
    }

    let columns = [
        "trkSub", "mode", "stn", "obsTime", "ra", "dec", "rmsRA", "rmsDec", "astCat", "mag",
        "rmsMag", "band", "photCat", "photAp", "logSNR", "seeing", "exp", "rmsFit", "nStars",
        "disc",
    ];
    let opt = |v: Option<f64>, d: usize| v.map_or_else(String::new, |v| format!("{v:.d$}"));
    let rows: Vec<Vec<String>> = observations
        .iter()
        .map(|o| {
            vec![
                o.trk_sub.clone(),
                o.mode.clone(),
                o.station.clone(),
                iso_time(o.jd),
                format!("{:.6}", o.ra.rem_euclid(360.0)),
                format!("{:+.6}", o.dec),
                format!("{:.3}", o.rms_ra),
                format!("{:.3}", o.rms_dec),
                GAIA_DR3.into(),
                opt(o.mag.map(|m| m.0), 2),
                opt(o.mag.map(|m| m.1), 2),
                if o.mag.is_some() {
                    o.band.clone()
                } else {
                    String::new()
                },
                if o.mag.is_some() {
                    GAIA_DR3.into()
                } else {
                    String::new()
                },
                opt(o.phot_ap.filter(|_| o.mag.is_some()), 2),
                opt(o.snr.filter(|s| *s > 0.0).map(f64::log10), 2),
                opt(o.seeing, 2),
                opt(o.exposure, 0),
                opt(o.rms_fit, 3),
                o.n_stars.map_or_else(String::new, |n| n.to_string()),
                if o.discovery {
                    "*".into()
                } else {
                    String::new()
                },
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..columns.len())
        .map(|c| {
            rows.iter()
                .map(|r| r[c].len())
                .chain([columns[c].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String]| -> String {
        cells
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("|")
            .trim_end()
            .to_owned()
    };
    out += &line(&columns.map(String::from));
    out.push('\n');
    for r in &rows {
        out += &line(r);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> Observation {
        Observation {
            trk_sub: "OC0001".into(),
            mode: "CCD".into(),
            station: "F51".into(),
            jd: time::parse_iso("2026-10-04T10:16:02.83232").unwrap(),
            ra: 21.2345678,
            dec: -4.1234567,
            rms_ra: 0.0712,
            rms_dec: 0.0654,
            mag: Some((20.123, 0.084)),
            band: "G".into(),
            phot_ap: Some(1.234),
            snr: Some(25.0),
            seeing: Some(1.31),
            exposure: Some(45.0),
            rms_fit: Some(0.0912),
            n_stars: Some(87),
            discovery: true,
        }
    }

    #[test]
    fn writes_header_and_columns() {
        let details = Details {
            code: "F51".into(),
            contact_name: "A. Teacher".into(),
            observers: vec!["K. Chambers".into()],
            measurers: vec!["A. Student".into(), "B. Student".into()],
            ..Details::default()
        };
        let tel = Telescope {
            aperture: Some(1.8),
            f_ratio: Some(4.4),
            design: "Ritchey-Chretien".into(),
            detector: "CCD".into(),
        };
        assert_eq!(tel.tel_line(), "1.80-m f/4.4 Ritchey-Chretien + CCD");
        let mut second = observation();
        second.discovery = false;
        second.mag = None;
        let text = psv(&details, &tel, &[observation(), second]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            &lines[..4],
            [
                "# version=2022",
                "# observatory",
                "! mpcCode F51",
                "# submitter"
            ]
        );
        assert!(text.contains("# measurers\n! name A. Student\n! name B. Student\n"));
        assert!(text.contains(
            "# telescope\n! aperture 1.8\n! design Ritchey-Chretien\n! detector CCD\n! fRatio 4.4\n"
        ));
        let header: Vec<&str> = lines[lines.len() - 3].split('|').map(str::trim).collect();
        let row: Vec<&str> = lines[lines.len() - 2].split('|').map(str::trim).collect();
        let field = |name: &str| row[header.iter().position(|h| *h == name).unwrap()];
        assert_eq!(field("trkSub"), "OC0001");
        assert_eq!(field("mode"), "CCD");
        assert_eq!(field("stn"), "F51");
        assert_eq!(field("obsTime"), "2026-10-04T10:16:02.83Z");
        assert_eq!(field("ra"), "21.234568");
        assert_eq!(field("dec"), "-4.123457");
        assert_eq!(field("rmsRA"), "0.071");
        assert_eq!(field("rmsDec"), "0.065");
        assert_eq!(field("astCat"), "Gaia3");
        assert_eq!(field("mag"), "20.12");
        assert_eq!(field("rmsMag"), "0.08");
        assert_eq!(field("band"), "G");
        assert_eq!(field("photCat"), "Gaia3");
        assert_eq!(field("logSNR"), "1.40");
        assert_eq!(field("exp"), "45");
        assert_eq!(field("nStars"), "87");
        assert_eq!(field("disc"), "*");
        // Every row has the header's number of fields (trailing blanks trimmed
        // only at the end of the line).
        let last: Vec<&str> = lines[lines.len() - 1].split('|').collect();
        assert!(last.len() <= header.len());
        assert_eq!(
            last[header.iter().position(|h| *h == "mag").unwrap()].trim(),
            ""
        );
    }

    #[test]
    fn time_rounds_with_carry() {
        let jd = time::parse_iso("2026-12-31T23:59:59.996").unwrap();
        assert_eq!(iso_time(jd), "2027-01-01T00:00:00.00Z");
        let jd = time::parse_iso("2026-03-01T00:00:00.004").unwrap();
        assert_eq!(iso_time(jd), "2026-03-01T00:00:00.00Z");
    }
}
