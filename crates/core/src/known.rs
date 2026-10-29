//! Known solar-system objects in a field: online from IMCCE's SkyBoT service
//! (<https://ssp.imcce.fr/webservices/skybot/>), or offline from the MPC's
//! MPCORB orbit catalogue.
//!
//! For SkyBoT the core only builds the request and parses the reply; fetching
//! is up to the caller, so this module stays dependency-free. For MPCORB the
//! caller supplies the (decompressed) file as a reader.

use std::io::BufRead;

use crate::ephem;
use crate::fits::Header;
use crate::orbit::{self, Elements, State};
use crate::time::{self, CalendarDate};

const SKYBOT: &str = "https://ssp.imcce.fr/webservices/skybot/api/conesearch.php";

#[derive(Debug, Clone, PartialEq)]
pub struct KnownObject {
    /// Minor planet number, if numbered (e.g. `"133105"`).
    pub number: Option<String>,
    /// Name or provisional designation (e.g. `"2003 OG10"`).
    pub name: String,
    /// Astrometric J2000 position at the query epoch, degrees.
    pub ra: f64,
    pub dec: f64,
    /// SkyBoT class, e.g. `MB>Inner`, `NEA>Apollo`, `Comet`.
    pub class: String,
    /// Predicted visual magnitude.
    pub magnitude: Option<f64>,
    /// Ephemeris uncertainty, arcseconds.
    pub uncertainty: f64,
    /// Apparent motion in arcseconds per hour: RA (already × cos Dec) and Dec.
    pub motion_ra: f64,
    pub motion_dec: f64,
}

impl KnownObject {
    /// Short label: the number if numbered, else the designation.
    pub fn label(&self) -> &str {
        self.number.as_deref().unwrap_or(&self.name)
    }

    /// Position at `jd`, moving linearly from the query epoch `epoch_jd`.
    /// Accurate to well under an arcsecond over an hour for main-belt
    /// asteroids, which is the span of an IASC image set.
    pub fn position_at(&self, epoch_jd: f64, jd: f64) -> (f64, f64) {
        let hours = (jd - epoch_jd) * 24.0;
        let dec = self.dec + self.motion_dec * hours / 3600.0;
        let cos_dec = self.dec.to_radians().cos().max(1e-6);
        let ra = self.ra + self.motion_ra * hours / 3600.0 / cos_dec;
        (ra.rem_euclid(360.0), dec)
    }
}

/// MPC observatory code for a frame, from its TELESCOP keyword.
pub fn observatory_code(header: &Header) -> Option<&'static str> {
    match header
        .text("TELESCOP")?
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "PS1" | "PAN-STARRS1" => Some("F51"),
        "PS2" | "PAN-STARRS2" => Some("F52"),
        _ => None,
    }
}

/// SkyBoT cone-search URL for a field centred on (`ra`, `dec`) degrees with
/// `radius` degrees at Julian date `jd`, seen from `observatory` (MPC code,
/// geocentre `500` if unknown).
pub fn skybot_url(ra: f64, dec: f64, radius: f64, jd: f64, observatory: Option<&str>) -> String {
    format!(
        "{SKYBOT}?-ep={jd:.6}&-ra={ra:.6}&-dec={dec:.6}&-rd={radius:.4}&-mime=text&-output=basic\
         &-observer={}&-filter=0&-objFilter=111&-refsys=EQJ2000&-from=OpenCeres",
        observatory.unwrap_or("500")
    )
}

/// Parses SkyBoT's plain-text reply. An empty field is `Ok(vec![])`; a
/// service error (flag -1) or unreadable reply is `Err` with the message.
pub fn parse_skybot(text: &str) -> Result<Vec<KnownObject>, String> {
    let flag = text
        .lines()
        .find_map(|l| l.strip_prefix("# Flag:"))
        .and_then(|f| f.trim().parse::<i32>().ok());
    match flag {
        Some(0) => return Ok(Vec::new()),
        Some(1) => {}
        Some(_) => {
            let message = text
                .lines()
                .find(|l| !l.starts_with('#') && !l.trim().is_empty())
                .unwrap_or("unknown error");
            return Err(format!("SkyBoT: {}", message.trim()));
        }
        None => return Err("SkyBoT returned an unexpected reply".into()),
    }

    let mut objects = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let f: Vec<&str> = line.split('|').map(str::trim).collect();
        if f.len() < 10 {
            return Err(format!("SkyBoT: unreadable line {line:?}"));
        }
        let num = |s: &str| s.parse::<f64>().ok();
        let bad = || format!("SkyBoT: unreadable line {line:?}");
        objects.push(KnownObject {
            number: (f[0] != "-" && !f[0].is_empty()).then(|| f[0].to_string()),
            name: f[1].to_string(),
            ra: sexagesimal(f[2]).ok_or_else(bad)? * 15.0,
            dec: sexagesimal(f[3]).ok_or_else(bad)?,
            class: f[4].to_string(),
            magnitude: num(f[5]),
            uncertainty: num(f[6]).unwrap_or(0.0),
            motion_ra: num(f[8]).ok_or_else(bad)?,
            motion_dec: num(f[9]).ok_or_else(bad)?,
        });
    }
    Ok(objects)
}

/// `"01 24 57.8767"` or `"+04 08 8.727"` → decimal units (hours or degrees).
fn sexagesimal(s: &str) -> Option<f64> {
    let negative = s.starts_with('-');
    let mut parts = s
        .trim_start_matches(['+', '-'])
        .split_whitespace()
        .map(str::parse::<f64>);
    let (a, b, c) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    let value = a + b / 60.0 + c / 3600.0;
    Some(if negative { -value } else { value })
}

/// A field to search for known objects.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldQuery {
    /// Field centre, degrees (astrometric J2000).
    pub ra: f64,
    pub dec: f64,
    /// Search radius, degrees.
    pub radius: f64,
    /// UTC Julian date of the observation (mid-exposure).
    pub jd: f64,
    /// MPC observatory code; `None` means the geocentre.
    pub observatory: Option<String>,
}

/// Margin added to the radius in the cheap two-body first pass, degrees:
/// 0.2° plus 1° per year² from the epoch, capped at 10°. Light time
/// shifts a position by at most v·(Δ/c)/Δ ≈ 30″ (v ≤ 40 km/s); planetary
/// perturbations move main-belt objects by arcseconds over a few months
/// (3″ for (133105) over 117 days) and grow roughly with the square of the
/// time from the epoch.
fn prefilter_margin(years_from_epoch: f64) -> f64 {
    (0.2 + years_from_epoch * years_from_epoch).min(10.0)
}

/// Half-width of the finite difference used for the motion, days (30 min).
const MOTION_STEP: f64 = 30.0 / 1440.0;

/// Searches an MPCORB.DAT-format catalogue (the uncompressed text; the
/// caller decompresses `MPCORB.DAT.gz`) for objects within `query.radius` of
/// the field centre at `query.jd`.
///
/// Positions are astrometric J2000 (light-time corrected, topocentric for
/// the observatory, no aberration or light deflection), the same convention
/// as SkyBoT's astrometric RA/Dec and as an image WCS fitted to a star
/// catalogue. A cheap two-body pass picks the candidates; their orbits are
/// then integrated from the osculating epoch with the planets' perturbations
/// ([`orbit::propagate`]), since two-body motion alone drifts by
/// arcseconds within months.
///
/// The file is streamed line by line; memory use is independent of its
/// size. Unreadable lines (the header, blank separators, hyperbolic orbits)
/// are skipped.
pub fn mpcorb_search(
    mut reader: impl BufRead,
    query: &FieldQuery,
) -> Result<Vec<KnownObject>, String> {
    let site = match query.observatory.as_deref() {
        Some(code) => Some(
            ephem::observatory(code).ok_or_else(|| format!("unknown observatory code {code:?}"))?,
        ),
        None => None,
    };
    let jds = [query.jd - MOTION_STEP, query.jd, query.jd + MOTION_STEP];
    let observers = jds.map(|jd| ephem::observer(site.as_ref(), jd));
    let centre = ephem::unit(query.ra, query.dec);
    let cos_radius = query.radius.min(180.0).to_radians().cos();
    let t = ephem::tt_from_utc(query.jd);

    let mut found = Vec::new();
    let mut line = Vec::with_capacity(256);
    loop {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("reading MPCORB: {e}"))?
            == 0
        {
            break;
        }
        let Some(elements) = parse_elements(&line) else {
            continue;
        };
        // Cheap pass: two-body geometric position, no light time.
        let rough = ephem::sub(heliocentric(&elements, t), observers[1]);
        let margin = prefilter_margin((t - elements.epoch).abs() / 365.25);
        let cos_rough = (query.radius + margin).min(180.0).to_radians().cos();
        if dot(rough, centre) < cos_rough * ephem::norm(rough) {
            continue;
        }
        let Some(state) = orbit::propagate(&elements, t) else {
            continue;
        };
        let here = astrometric(&state, t, observers[1]);
        if dot(here.topocentric, centre) < cos_radius * here.distance {
            continue;
        }
        let Some(record) = parse_record(&line) else {
            continue;
        };
        let before = astrometric(&state, ephem::tt_from_utc(jds[0]), observers[0]);
        let after = astrometric(&state, ephem::tt_from_utc(jds[2]), observers[2]);
        let (ra, dec) = ephem::ra_dec(here.topocentric);
        let (ra0, dec0) = ephem::ra_dec(before.topocentric);
        let (ra1, dec1) = ephem::ra_dec(after.topocentric);
        let hours = 2.0 * MOTION_STEP * 24.0;
        let dra = ((ra1 - ra0 + 540.0).rem_euclid(360.0) - 180.0) * dec.to_radians().cos();

        let r = ephem::norm(here.heliocentric);
        let sun_observer = ephem::norm(observers[1]);
        found.push(KnownObject {
            number: record.number,
            name: record.name,
            ra,
            dec,
            class: classify(&elements).to_string(),
            magnitude: record
                .h
                .map(|h| magnitude(h, record.g, r, here.distance, sun_observer)),
            uncertainty: record.u.map_or(0.0, uncertainty_from_u),
            motion_ra: dra * 3600.0 / hours,
            motion_dec: (dec1 - dec0) * 3600.0 / hours,
        });
    }
    Ok(found)
}

/// Date of an MPCORB file for display, e.g. `"2026-10-28"`.
///
/// The header carries no date, so this is the latest "date of last
/// observation" among the first data lines: the brightest numbered objects
/// are observed every night, so that is the date of the last orbit update
/// the file includes. `None` if the file has no readable orbit lines.
pub fn mpcorb_date(mut reader: impl BufRead) -> Option<String> {
    const SAMPLE: usize = 2000;
    let mut latest: Option<u32> = None;
    let mut seen = 0;
    let mut line = Vec::with_capacity(256);
    while seen < SAMPLE {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            break;
        }
        if parse_elements(&line).is_none() {
            continue;
        }
        seen += 1;
        let date = text(&line, 194, 202).and_then(|d| d.parse::<u32>().ok());
        if let Some(d) = date.filter(|d| *d > 10_000_000) {
            latest = latest.max(Some(d));
        }
    }
    latest.map(|d| format!("{:04}-{:02}-{:02}", d / 10_000, d / 100 % 100, d % 100))
}

struct Astrometric {
    /// Observer → object, equatorial J2000, au.
    topocentric: [f64; 3],
    /// Sun → object at the emission time.
    heliocentric: [f64; 3],
    distance: f64,
}

fn heliocentric(elements: &Elements, jd_tt: f64) -> [f64; 3] {
    ephem::ecliptic_to_equatorial(elements.position(jd_tt))
}

/// Position seen by an observer at `observer` (heliocentric) at `jd_tt`,
/// corrected for light time: the object is where it was Δ/c earlier. Three
/// passes converge to microseconds.
fn astrometric(state: &State, jd_tt: f64, observer: [f64; 3]) -> Astrometric {
    let mut tau = 0.0;
    let mut result = None;
    for _ in 0..3 {
        let heliocentric = ephem::ecliptic_to_equatorial(state.position_near(jd_tt - tau));
        let topocentric = ephem::sub(heliocentric, observer);
        let distance = ephem::norm(topocentric);
        tau = distance / ephem::C_AU_PER_DAY;
        result = Some(Astrometric {
            topocentric,
            heliocentric,
            distance,
        });
    }
    result.expect("at least one iteration")
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Predicted V magnitude in the H, G system (Bowell et al. 1989, in
/// Asteroids II): r, Δ and the Sun–observer distance in au.
fn magnitude(h: f64, g: f64, r: f64, delta: f64, sun_observer: f64) -> f64 {
    let cos_phase = ((r * r + delta * delta - sun_observer * sun_observer) / (2.0 * r * delta))
        .clamp(-1.0, 1.0);
    let tan_half = (cos_phase.acos() / 2.0).tan();
    let phi1 = (-3.33 * tan_half.powf(0.63)).exp();
    let phi2 = (-1.87 * tan_half.powf(1.22)).exp();
    let phase = ((1.0 - g) * phi1 + g * phi2).max(1e-12);
    h + 5.0 * (r * delta).log10() - 2.5 * phase.log10()
}

/// The MPC's U parameter as an angle: the upper end of its in-orbit
/// longitude runoff per decade, e^(U·ln(648000)/9) arcseconds (MPC, "Uncertainty
/// Parameter U"). A pessimistic quality indicator rather than a 1σ position
/// error for tonight, but it orders orbits the same way.
fn uncertainty_from_u(u: u8) -> f64 {
    (u as f64 * 648_000f64.ln() / 9.0).exp()
}

/// Dynamical class from the osculating elements, using the usual cuts on
/// a, q and aphelion Q (au) and SkyBoT-style names.
fn classify(o: &Elements) -> &'static str {
    let a = o.semimajor_axis;
    let q = o.perihelion_distance();
    let big_q = a * (1.0 + o.eccentricity);
    match () {
        _ if q < 1.3 && a < 1.0 && big_q < 0.983 => "NEA>Atira",
        _ if q < 1.3 && a < 1.0 => "NEA>Aten",
        _ if q < 1.017 => "NEA>Apollo",
        _ if q < 1.3 => "NEA>Amor",
        _ if q < 1.666 && a < 3.2 => "Mars-Crosser",
        _ if (1.78..2.0).contains(&a) => "Hungaria",
        _ if (2.0..2.5).contains(&a) => "MB>Inner",
        _ if (2.5..2.82).contains(&a) => "MB>Middle",
        _ if (2.82..3.27).contains(&a) => "MB>Outer",
        _ if (3.27..3.7).contains(&a) => "MB>Cybele",
        _ if (3.7..4.2).contains(&a) => "MB>Hilda",
        _ if (5.05..5.35).contains(&a) => "Trojan",
        _ if (5.5..30.1).contains(&a) => "Centaur",
        _ if a >= 30.1 => "KBO",
        _ => "",
    }
}

/// Text of 0-based byte columns `a..b`, trimmed; `None` if the line is too short.
fn text(line: &[u8], a: usize, b: usize) -> Option<&str> {
    std::str::from_utf8(line.get(a..b)?).ok().map(str::trim)
}

fn float(line: &[u8], a: usize, b: usize) -> Option<f64> {
    text(line, a, b)?.parse().ok()
}

/// The orbital elements of an MPCORB line (MPC, "Export Format for
/// Minor-Planet Orbits"; columns 21–103), or `None` for anything else,
/// including parabolic/hyperbolic orbits.
fn parse_elements(line: &[u8]) -> Option<Elements> {
    if line.len() < 103 {
        return None;
    }
    let elements = Elements {
        epoch: unpack_epoch(text(line, 20, 25)?)?,
        mean_anomaly: float(line, 26, 35)?,
        perihelion: float(line, 37, 46)?,
        node: float(line, 48, 57)?,
        inclination: float(line, 59, 68)?,
        eccentricity: float(line, 70, 79)?,
        mean_motion: float(line, 80, 91)?,
        semimajor_axis: float(line, 92, 103)?,
    };
    let ok = (0.0..1.0).contains(&elements.eccentricity)
        && elements.semimajor_axis > 0.0
        && elements.mean_motion > 0.0;
    ok.then_some(elements)
}

/// The descriptive fields of an MPCORB line, read only for objects in the field.
struct Record {
    number: Option<String>,
    name: String,
    h: Option<f64>,
    g: f64,
    u: Option<u8>,
}

fn parse_record(line: &[u8]) -> Option<Record> {
    let packed = text(line, 0, 7)?;
    let readable = text(line, 166, 194).unwrap_or("");
    let (number, name) = match readable.strip_prefix('(').and_then(|r| r.split_once(')')) {
        Some((n, rest)) => (Some(n.trim().to_string()), rest.trim().to_string()),
        None if !readable.is_empty() => (None, readable.to_string()),
        None => match unpack_number(packed) {
            Some(n) => (Some(n.to_string()), n.to_string()),
            None => (None, unpack_provisional(packed)?),
        },
    };
    let u = line
        .get(105)
        .filter(|c| c.is_ascii_digit())
        .map(|c| c - b'0');
    Some(Record {
        number,
        name,
        h: float(line, 8, 13),
        g: float(line, 14, 19).unwrap_or(0.15),
        u,
    })
}

/// MPC packed digit: 0–9, A–Z = 10–35, a–z = 36–61.
fn packed_digit(c: u8) -> Option<u32> {
    match c {
        b'0'..=b'9' => Some((c - b'0') as u32),
        b'A'..=b'Z' => Some((c - b'A') as u32 + 10),
        b'a'..=b'z' => Some((c - b'a') as u32 + 36),
        _ => None,
    }
}

/// Unpacks an MPC packed epoch such as `K2669` (2026 June 9.0) to a Julian
/// date (TT, 0h). Century letter I/J/K = 18/19/20, then the year, then month
/// and day as packed digits (MPC, "Packed Dates").
pub fn unpack_epoch(packed: &str) -> Option<f64> {
    let b = packed.as_bytes();
    if b.len() != 5 {
        return None;
    }
    let century = match b[0] {
        c @ b'I'..=b'L' => (c - b'I') as i32 + 18,
        _ => return None,
    };
    let year: i32 = packed[1..3].parse().ok()?;
    let month = packed_digit(b[3])?;
    let day = packed_digit(b[4])?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(time::calendar_to_jd(CalendarDate {
        year: century * 100 + year,
        month,
        day: day as f64,
    }))
}

/// Unpacks a packed permanent minor-planet number: `03202` → 3202,
/// `A0345` → 100345, `~0000` → 620000 (MPC, "Packed Provisional and
/// Permanent Designations").
pub fn unpack_number(packed: &str) -> Option<u32> {
    let b = packed.trim().as_bytes();
    if b.len() != 5 {
        return None;
    }
    if b[0] == b'~' {
        let mut n = 0;
        for &c in &b[1..] {
            n = n * 62 + packed_digit(c)?;
        }
        return Some(620_000 + n);
    }
    if !b[1..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let rest: u32 = std::str::from_utf8(&b[1..]).ok()?.parse().ok()?;
    Some(packed_digit(b[0])? * 10_000 + rest)
}

/// Unpacks a packed provisional minor-planet designation: `J95X00A` →
/// `1995 XA`, `K07Tf8A` → `2007 TA418`, `PLS2040` → `2040 P-L`.
pub fn unpack_provisional(packed: &str) -> Option<String> {
    let b = packed.trim().as_bytes();
    if b.len() != 7 {
        return None;
    }
    for (prefix, survey) in [
        ("PLS", "P-L"),
        ("T1S", "T-1"),
        ("T2S", "T-2"),
        ("T3S", "T-3"),
    ] {
        if let Some(n) = packed.strip_prefix(prefix) {
            return n
                .bytes()
                .all(|c| c.is_ascii_digit())
                .then(|| format!("{n} {survey}"));
        }
    }
    let century = match b[0] {
        c @ b'I'..=b'K' => (c - b'I') as u32 + 18,
        _ => return None,
    };
    if !b[1].is_ascii_digit()
        || !b[2].is_ascii_digit()
        || !b[3].is_ascii_uppercase()
        || !b[4].is_ascii_alphanumeric()
        || !b[5].is_ascii_digit()
        || !b[6].is_ascii_uppercase()
    {
        return None;
    }
    let cycle = packed_digit(b[4])? * 10 + (b[5] - b'0') as u32;
    let year = format!("{century}{}", &packed[1..3]);
    let (half_month, letter) = (b[3] as char, b[6] as char);
    Some(if cycle == 0 {
        format!("{year} {half_month}{letter}")
    } else {
        format!("{year} {half_month}{letter}{cycle}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape of a real reply (field XY57, 2026-10-04, F51), shortened.
    const REPLY: &str = "# Flag: 1
# Ticket: 188040994265769708
# Num | Name | RA(h) | DE(deg) | Class | Mv | Err(arcsec) | d(arcsec) | dRA(arcsec/h) | dDEC(arcsec/h) | Dg(ua) | Dh(ua)
 133105 | 2003 OG10 | 01 24 57.8767 | +04 08 8.727 | MB>Inner | 19.0 | 0.002 | 159.329 | -35.7196 | -13.919 | 1.15870680786 | 2.14878289216
 - | 2026 RA34 | 01 24 53.4496 | -04 13 41.746 | MB>Inner | 21.5 | 0.000 | 383.942 | -31.9941 | -8.465 | 0.83229847030 | 1.82399461000
";

    #[test]
    fn parses_reply() {
        let objects = parse_skybot(REPLY).unwrap();
        assert_eq!(objects.len(), 2);
        let a = &objects[0];
        assert_eq!(
            (a.number.as_deref(), a.name.as_str(), a.label()),
            (Some("133105"), "2003 OG10", "133105")
        );
        assert!((a.ra - 15.0 * (1.0 + 24.0 / 60.0 + 57.8767 / 3600.0)).abs() < 1e-9);
        assert!((a.dec - (4.0 + 8.0 / 60.0 + 8.727 / 3600.0)).abs() < 1e-9);
        assert_eq!((a.class.as_str(), a.magnitude), ("MB>Inner", Some(19.0)));
        assert_eq!((a.motion_ra, a.motion_dec), (-35.7196, -13.919));
        let b = &objects[1];
        assert_eq!((b.number.as_deref(), b.label()), (None, "2026 RA34"));
        assert!(b.dec < -4.2 && b.dec > -4.3);
    }

    #[test]
    fn empty_field_and_errors() {
        assert_eq!(parse_skybot("# Flag: 0\n# Ticket: 1\n"), Ok(vec![]));
        assert!(
            parse_skybot("# Flag: -1\nEpoch out of range\n")
                .unwrap_err()
                .contains("Epoch out of range")
        );
        assert!(parse_skybot("<html>busy</html>").is_err());
    }

    #[test]
    fn linear_motion() {
        let o = &parse_skybot(REPLY).unwrap()[0];
        let (ra, dec) = o.position_at(2_461_317.5, 2_461_317.5 + 1.0 / 24.0);
        // One hour: Dec moves by the Dec rate; RA by the rate divided by cos Dec.
        assert!(((dec - o.dec) * 3600.0 - o.motion_dec).abs() < 1e-6);
        let cos_dec = o.dec.to_radians().cos();
        assert!(((ra - o.ra) * 3600.0 * cos_dec - o.motion_ra).abs() < 1e-6);
    }

    #[test]
    fn url_and_observatory() {
        let url = skybot_url(21.285129, 4.141669, 0.13, 2_461_317.928239, Some("F51"));
        assert!(url.starts_with(SKYBOT));
        for part in [
            "-ep=2461317.928239",
            "-ra=21.285129",
            "-dec=4.141669",
            "-rd=0.1300",
            "-observer=F51",
            "-mime=text",
        ] {
            assert!(url.contains(part), "{part} missing in {url}");
        }
        let header = crate::fits::parse(&crate::fits::tests::make_fits(
            1,
            1,
            &["TELESCOP= 'PS1     '"],
            &[0.0],
        ))
        .unwrap()
        .header;
        assert_eq!(observatory_code(&header), Some("F51"));
    }

    /// An MPCORB line in the documented fixed columns.
    fn mpcorb_line(packed: &str, readable: &str, o: &Elements, epoch: &str, u: char) -> String {
        let line = format!(
            "{packed:<7} {:>5.2} {:>5.2} {epoch} {:>9.5}  {:>9.5}  {:>9.5}  {:>9.5}  {:>9.7} {:>11.8} {:>11.7}  {u} E2026-U87  1118  19 2002-2026 0.80 M-v 3Ek MPCORBFIT  0000 {readable:<28}20261026",
            16.42,
            0.15,
            o.mean_anomaly,
            o.perihelion,
            o.node,
            o.inclination,
            o.eccentricity,
            o.mean_motion,
            o.semimajor_axis
        );
        assert_eq!(line.len(), 202, "{line}");
        line
    }

    fn sample_orbit() -> Elements {
        Elements {
            epoch: unpack_epoch("K2669").unwrap(),
            mean_anomaly: 14.3292,
            perihelion: 208.44664,
            node: 104.03668,
            inclination: 2.39951,
            eccentricity: 0.1984535,
            mean_motion: 0.25984287,
            semimajor_axis: 2.4321789,
        }
    }

    #[test]
    fn unpacks_epochs() {
        // K2669 = 2026 June 9.0; J9611 = 1996 Jan 1.0; K26AV = 2026 Oct 31.0.
        let jd = |y, m, d| {
            time::calendar_to_jd(CalendarDate {
                year: y,
                month: m,
                day: d,
            })
        };
        assert_eq!(unpack_epoch("K2669"), Some(jd(2026, 6, 9.0)));
        assert_eq!(unpack_epoch("J9611"), Some(jd(1996, 1, 1.0)));
        assert_eq!(unpack_epoch("K26AV"), Some(jd(2026, 10, 31.0)));
        assert_eq!(unpack_epoch("K26D1"), None);
        assert_eq!(unpack_epoch("2026"), None);
    }

    #[test]
    fn unpacks_designations() {
        assert_eq!(unpack_number("03202"), Some(3202));
        assert_eq!(unpack_number("A0345"), Some(100_345));
        assert_eq!(unpack_number("a0017"), Some(360_017));
        assert_eq!(unpack_number("K3289"), Some(203_289));
        assert_eq!(unpack_number("~0000"), Some(620_000));
        assert_eq!(unpack_number("~AZaz"), Some(3_140_113));
        assert_eq!(unpack_number("~zzzz"), Some(15_396_335));
        assert_eq!(unpack_number("K26R34A"), None);
        for (packed, readable) in [
            ("J95X00A", "1995 XA"),
            ("J95X01L", "1995 XL1"),
            ("J98SA8Q", "1998 SQ108"),
            ("K07Tf8A", "2007 TA418"),
            ("K08Aa0A", "2008 AA360"),
            ("PLS2040", "2040 P-L"),
            ("T3S4101", "4101 T-3"),
        ] {
            assert_eq!(unpack_provisional(packed).as_deref(), Some(readable));
        }
        assert_eq!(unpack_provisional("D3105  "), None);
    }

    #[test]
    fn parses_mpcorb_lines() {
        let o = sample_orbit();
        let numbered = mpcorb_line("D3105", "(133105) 2003 OG10", &o, "K2669", '0');
        let parsed = parse_elements(numbered.as_bytes()).unwrap();
        assert_eq!(parsed, o);
        let r = parse_record(numbered.as_bytes()).unwrap();
        assert_eq!(
            (r.number.as_deref(), r.name.as_str()),
            (Some("133105"), "2003 OG10")
        );
        assert_eq!((r.h, r.g, r.u), (Some(16.42), 0.15, Some(0)));

        // Without the readable column, the packed designation is unpacked.
        let provisional = mpcorb_line("K26R34A", "", &o, "K2669", 'E');
        let r = parse_record(provisional.trim_end().as_bytes()).unwrap();
        assert_eq!((r.number, r.name.as_str(), r.u), (None, "2026 RA34", None));

        // Hyperbolic orbits, short lines and header text are rejected.
        let hyperbolic = Elements {
            eccentricity: 1.2,
            ..o
        };
        assert!(
            parse_elements(mpcorb_line("K26R34A", "", &hyperbolic, "K2669", '0').as_bytes())
                .is_none()
        );
        assert!(parse_elements(b"Des'n     H     G   Epoch     M        Peri.").is_none());
        assert!(parse_elements(&[b'-'; 160]).is_none());
        assert_eq!(classify(&o), "MB>Inner");
        assert!((uncertainty_from_u(1) - 4.4).abs() < 0.05);
    }

    #[test]
    fn mpcorb_search_finds_objects_in_the_field() {
        let o = sample_orbit();
        let far = Elements {
            node: o.node + 90.0,
            ..o
        };
        let text = format!(
            "MINOR PLANET CENTER ORBIT DATABASE (MPCORB)\n\nDes'n     H\n{}\n{}\n\n{}\r\n",
            "-".repeat(160),
            mpcorb_line("D3105", "(133105) 2003 OG10", &o, "K2669", '0'),
            mpcorb_line("K26R34A", "2026 RA34", &far, "K2669", '2'),
        );
        let jd = 2_461_317.928_239;
        // Centre the field on the object as seen from F51, from first principles.
        let site = ephem::observatory("F51").unwrap();
        let obs = ephem::observer(Some(&site), jd);
        let state = orbit::propagate(&o, ephem::tt_from_utc(jd)).unwrap();
        let first = astrometric(&state, ephem::tt_from_utc(jd), obs);
        let (ra, dec) = ephem::ra_dec(first.topocentric);
        let query = FieldQuery {
            ra: ra + 0.05,
            dec,
            radius: 0.1,
            jd,
            observatory: Some("F51".into()),
        };
        let found = mpcorb_search(text.as_bytes(), &query).unwrap();
        assert_eq!(found.len(), 1);
        let k = &found[0];
        assert_eq!(
            (k.label(), k.name.as_str(), k.class.as_str()),
            ("133105", "2003 OG10", "MB>Inner")
        );
        assert!((k.ra - ra).abs() < 1e-9 && (k.dec - dec).abs() < 1e-9);
        // Motion: the finite difference agrees with positions an hour apart.
        let (ra2, dec2) = k.position_at(jd, jd + 1.0 / 24.0);
        let t1 = ephem::tt_from_utc(jd + 1.0 / 24.0);
        let later = astrometric(
            &orbit::propagate(&o, t1).unwrap(),
            t1,
            ephem::observer(Some(&site), jd + 1.0 / 24.0),
        );
        let (ra_true, dec_true) = ephem::ra_dec(later.topocentric);
        assert!((ra2 - ra_true).abs() * 3600.0 < 0.05 && (dec2 - dec_true).abs() * 3600.0 < 0.05);
        // A main-belt asteroid near opposition: retrograde, tens of ″/h.
        assert!(
            k.motion_ra < -10.0 && k.motion_ra > -60.0,
            "{}",
            k.motion_ra
        );
        let m = k.magnitude.unwrap();
        assert!(m > 17.0 && m < 21.0, "{m}");

        // Topocentric parallax matters: from the geocentre the object shifts by arcseconds.
        let geo = mpcorb_search(
            text.as_bytes(),
            &FieldQuery {
                observatory: None,
                ..query.clone()
            },
        )
        .unwrap();
        let shift = (geo[0].dec - k.dec).abs().max((geo[0].ra - k.ra).abs()) * 3600.0;
        assert!(shift > 1.0 && shift < 15.0, "{shift}");

        assert!(
            mpcorb_search(
                text.as_bytes(),
                &FieldQuery {
                    observatory: Some("ZZZ".into()),
                    ..query
                }
            )
            .is_err()
        );
        assert_eq!(mpcorb_date(text.as_bytes()).as_deref(), Some("2026-10-26"));
        assert_eq!(mpcorb_date(&b"no orbits here\n"[..]), None);
    }
}
