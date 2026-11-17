//! Minor Planet Center observation reports in the 80-column optical format.
//!
//! Column layout (1-based, as in the MPC documentation):
//!
//! | Columns | Field                                          |
//! |---------|------------------------------------------------|
//! | 1–5     | packed minor planet number (blank if none)     |
//! | 6–12    | provisional or temporary designation           |
//! | 13      | `*` on the discovery observation               |
//! | 14      | note 1                                         |
//! | 15      | note 2 / observation type (`C` = CCD)           |
//! | 16–32   | UTC date, `YYYY MM DD.dddddd`                  |
//! | 33–44   | RA (J2000), `HH MM SS.sss`                     |
//! | 45–56   | Dec (J2000), `sDD MM SS.ss`                    |
//! | 57–65   | blank                                          |
//! | 66–70   | magnitude                                      |
//! | 71      | band                                           |
//! | 72–77   | blank                                          |
//! | 78–80   | observatory code                               |

use crate::time::CalendarDate;

#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// Packed number of a numbered minor planet, e.g. `"00433"` or `"A0345"`.
    pub number: Option<String>,
    /// Provisional or temporary designation, up to 7 characters.
    pub designation: String,
    pub discovery: bool,
    pub note1: char,
    /// Observation type; `'C'` for CCD images.
    pub note2: char,
    pub date: CalendarDate,
    /// J2000 right ascension and declination in degrees.
    pub ra: f64,
    pub dec: f64,
    pub magnitude: Option<f64>,
    pub band: char,
    /// Three-character MPC observatory code, e.g. `"F51"` for Pan-STARRS 1.
    pub observatory: String,
}

/// Formats one observation as an exactly 80-character line.
pub fn format_80(obs: &Observation) -> String {
    let mut line = String::with_capacity(80);
    line += &format!("{:>5.5}", obs.number.as_deref().unwrap_or(""));
    line += &format!("{:<7.7}", obs.designation);
    line.push(if obs.discovery { '*' } else { ' ' });
    line.push(obs.note1);
    line.push(obs.note2);
    line += &format_date(obs.date);
    line += &format_ra(obs.ra);
    line += &format_dec(obs.dec);
    line += &" ".repeat(9);
    match obs.magnitude {
        // F5.2 with one decimal: "21.3 ", " 9.8 ".
        Some(m) => line += &format!("{m:>4.1} "),
        None => line += "     ",
    }
    line.push(if obs.magnitude.is_some() {
        obs.band
    } else {
        ' '
    });
    line += &" ".repeat(6);
    line += &format!("{:<3.3}", obs.observatory);
    debug_assert_eq!(line.len(), 80, "{line:?}");
    line
}

/// `YYYY MM DD.dddddd` (17 characters, ~0.09 s resolution).
fn format_date(date: CalendarDate) -> String {
    let micro_days = (date.day * 1e6).round() as u64;
    format!(
        "{:04} {:02} {:02}.{:06}",
        date.year,
        date.month,
        micro_days / 1_000_000,
        micro_days % 1_000_000
    )
}

/// `HH MM SS.sss` (12 characters). Rounds in integer milliseconds of time so
/// that values never print as 60 seconds.
fn format_ra(ra_deg: f64) -> String {
    let ms = ((ra_deg.rem_euclid(360.0) / 15.0 * 3_600_000.0).round() as u64) % 86_400_000;
    format!(
        "{:02} {:02} {:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// `sDD MM SS.ss` (12 characters), rounded in hundredths of an arcsecond.
fn format_dec(dec_deg: f64) -> String {
    let cs = (dec_deg.abs() * 360_000.0).round() as u64;
    // Sign after rounding, so tiny negative values print as +00 00 00.00.
    let sign = if dec_deg < 0.0 && cs > 0 { '-' } else { '+' };
    format!(
        "{sign}{:02} {:02} {:02}.{:02}",
        cs / 360_000,
        cs / 6000 % 60,
        cs / 100 % 60,
        cs % 100
    )
}

/// The complete 80-column report: header lines, then one line per
/// observation, each line ending in a newline.
pub fn report(details: &Details, observations: &[Observation]) -> String {
    let mut out = String::new();
    for line in header_lines(details) {
        out += &line;
        out.push('\n');
    }
    for o in observations {
        out += &format_80(o);
        out.push('\n');
    }
    out
}

/// Observational details for the header lines that must precede the
/// observations (MPC, "Specifying Observational Details"): `COD` first,
/// then `CON` (if given), then the others.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    /// Observatory code, e.g. `F51`.
    pub code: String,
    /// Contact: name, then address (one line of text), then e-mail.
    pub contact_name: String,
    pub contact_address: String,
    pub contact_email: String,
    /// Names as initials + surname, e.g. `J. R. Smith`.
    pub observers: Vec<String>,
    pub measurers: Vec<String>,
    /// e.g. `1.8-m f/4.4 Ritchey-Chretien + CCD`.
    pub telescope: String,
    /// Reference catalogue, e.g. `Gaia-DR3`.
    pub catalogue: String,
    /// Text the MPC e-mails back on receipt (`ACK`), and where to (`AC2`).
    pub ack: String,
    pub ack_emails: String,
    pub comments: Vec<String>,
}

/// The catalogue name for Gaia DR3 as the MPC's catalogue list spells it.
pub const GAIA_DR3: &str = "Gaia-DR3";

/// Header lines, each at most 80 characters. Lists of names that do not
/// fit on one line continue on another line with the same keyword, never
/// breaking a name.
pub fn header_lines(d: &Details) -> Vec<String> {
    let mut out = vec![format!("COD {}", d.code.trim())];
    let clean = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = clean(&d.contact_name);
    if !name.is_empty() {
        let address = clean(&d.contact_address);
        let mut parts = vec![name];
        // The address may be split at its commas over several CON lines.
        parts.extend(
            address
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from),
        );
        out.extend(wrap("CON", &parts));
        let email = d.contact_email.trim();
        if !email.is_empty() {
            out.push(format!("CON [{email}]"));
        }
    }
    let names = |v: &[String]| -> Vec<String> {
        v.iter()
            .map(|n| clean(n))
            .filter(|n| !n.is_empty())
            .collect()
    };
    let observers = names(&d.observers);
    if !observers.is_empty() {
        out.extend(wrap("OBS", &observers));
    }
    let measurers = names(&d.measurers);
    if !measurers.is_empty() {
        out.extend(wrap("MEA", &measurers));
    }
    for (key, value) in [("TEL", &d.telescope), ("NET", &d.catalogue)] {
        let v = clean(value);
        if !v.is_empty() {
            out.push(truncate(format!("{key} {v}")));
        }
    }
    for c in &d.comments {
        let c = clean(c);
        if !c.is_empty() {
            out.push(truncate(format!("COM {c}")));
        }
    }
    let ack = clean(&d.ack);
    if !ack.is_empty() {
        out.push(truncate(format!("ACK {ack}")));
    }
    let emails: Vec<String> = d
        .ack_emails
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(String::from)
        .collect();
    if !emails.is_empty() {
        out.push(truncate(format!("AC2 {}", emails.join(","))));
    }
    out
}

fn truncate(mut line: String) -> String {
    if line.chars().count() > 80 {
        line = line.chars().take(80).collect();
    }
    line
}

/// `KEY a, b, c` over as many lines as needed (each ≤ 80 characters).
fn wrap(key: &str, items: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for item in items {
        let item: String = item.chars().take(80 - key.len() - 1).collect();
        if line.is_empty() {
            line = format!("{key} {item}");
        } else if line.chars().count() + 2 + item.chars().count() <= 80 {
            line += ", ";
            line += &item;
        } else {
            lines.push(std::mem::take(&mut line));
            line = format!("{key} {item}");
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Problems with a name for `OBS`/`MEA`/`CON` under the MPC's rules
/// (initials + surname, initials separated by spaces, mixed case), or
/// `None` if it looks right. Advisory: unusual real names exist.
pub fn name_problem(name: &str) -> Option<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() < 2 {
        return Some(format!(
            "“{name}”: give initials and surname, e.g. “J. Smith”"
        ));
    }
    let (surname, initials) = words.split_last()?;
    if !initials.iter().all(|w| is_initial(w)) {
        return Some(format!(
            "“{name}”: first names as initials with a full stop, e.g. “A. B. Smith”"
        ));
    }
    let letters: Vec<char> = surname.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() > 1 && letters.iter().all(|c| c.is_uppercase()) {
        return Some(format!("“{name}”: the surname must not be all capitals"));
    }
    if letters.first().is_none_or(|c| !c.is_uppercase()) {
        return Some(format!("“{name}”: the surname starts with a capital"));
    }
    None
}

/// `J.`, `J.-F.` (hyphenated first name), or a lower-case particle such as
/// `van`/`de` before the surname.
fn is_initial(w: &str) -> bool {
    let initial = |p: &str| {
        let mut c = p.chars();
        matches!((c.next(), c.next(), c.next()), (Some(a), Some('.'), None) if a.is_uppercase())
    };
    w.split('-').all(initial) || w.chars().all(|c| c.is_lowercase())
}

/// Problems with a temporary designation (columns 6–12): alphanumeric,
/// starting with a letter, at most 7 characters (MPC: preferably 6), and
/// not shaped like a packed provisional designation, which the MPC would
/// read as one. `None` if it is fine.
pub fn designation_problem(d: &str) -> Option<String> {
    if d.is_empty() {
        return Some("empty".into());
    }
    if d.len() > 7 {
        return Some("longer than 7 characters".into());
    }
    if !d.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some("letters and digits only".into());
    }
    if !d.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Some("must start with a letter".into());
    }
    let b = d.as_bytes();
    // Packed provisional designation: century letter, 2 digits, half-month
    // letter, 2 order characters, capital letter (e.g. K26T03A).
    if b.len() == 7
        && matches!(b[0], b'I'..=b'K')
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_uppercase()
        && b[6].is_ascii_uppercase()
    {
        return Some("looks like a packed provisional designation".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs() -> Observation {
        Observation {
            number: None,
            designation: "OC0001".into(),
            discovery: true,
            note1: ' ',
            note2: 'C',
            date: CalendarDate {
                year: 2026,
                month: 10,
                day: 29.512345,
            },
            ra: 15.0 * (3.0 + 4.0 / 60.0 + 5.678 / 3600.0),
            dec: -(7.0 + 8.0 / 60.0 + 9.01 / 3600.0),
            magnitude: Some(21.34),
            band: 'w',
            observatory: "F51".into(),
        }
    }

    #[test]
    fn formats_columns_exactly() {
        let line = format_80(&obs());
        assert_eq!(line.len(), 80);
        assert_eq!(&line[0..5], "     ");
        assert_eq!(&line[5..12], "OC0001 ");
        assert_eq!(&line[12..15], "* C");
        assert_eq!(&line[15..32], "2026 10 29.512345");
        assert_eq!(&line[32..44], "03 04 05.678");
        assert_eq!(&line[44..56], "-07 08 09.01");
        assert_eq!(&line[56..65], "         ");
        assert_eq!(&line[65..71], "21.3 w");
        assert_eq!(&line[77..80], "F51");
    }

    #[test]
    fn rounding_carries_instead_of_printing_sixty() {
        assert_eq!(
            format_ra(15.0 * (23.0 + 59.0 / 60.0 + 59.9999 / 3600.0)),
            "00 00 00.000"
        );
        assert_eq!(format_ra(15.0 * (1.0 + 59.9999 / 3600.0)), "01 01 00.000");
        assert_eq!(
            format_dec(10.0 + 59.0 / 60.0 + 59.999 / 3600.0),
            "+11 00 00.00"
        );
        assert_eq!(format_dec(-0.0001 / 3600.0), "+00 00 00.00");
    }

    #[test]
    fn numbered_object_without_magnitude() {
        let mut o = obs();
        o.number = Some("00433".into());
        o.designation.clear();
        o.discovery = false;
        o.magnitude = None;
        let line = format_80(&o);
        assert_eq!(line.len(), 80);
        assert_eq!(&line[0..15], "00433         C");
        assert_eq!(&line[65..71], "      ");
        o.magnitude = Some(9.84);
        assert_eq!(&format_80(&o)[65..71], " 9.8 w");
    }

    fn details() -> Details {
        Details {
            code: "F51".into(),
            contact_name: "A. Teacher".into(),
            contact_address: "Example School, 1 Main Street, Springfield".into(),
            contact_email: "teacher@example.org".into(),
            observers: vec!["K. Chambers".into(), "R. J. Wainscoat".into()],
            measurers: (0..12).map(|i| format!("S. Student{i}")).collect(),
            telescope: "1.8-m f/4.4 Ritchey-Chretien + CCD".into(),
            catalogue: GAIA_DR3.into(),
            ack: "IASC batch 1".into(),
            ack_emails: "teacher@example.org, team@example.org".into(),
            comments: vec![],
        }
    }

    #[test]
    fn header_follows_the_mpc_layout() {
        let lines = header_lines(&details());
        assert_eq!(lines[0], "COD F51");
        assert_eq!(
            lines[1],
            "CON A. Teacher, Example School, 1 Main Street, Springfield"
        );
        assert_eq!(lines[2], "CON [teacher@example.org]");
        assert_eq!(lines[3], "OBS K. Chambers, R. J. Wainscoat");
        // Twelve measurers don't fit one line: MEA repeats, names unbroken.
        let mea: Vec<&String> = lines.iter().filter(|l| l.starts_with("MEA ")).collect();
        assert!(mea.len() >= 2, "{mea:?}");
        let names: Vec<&str> = mea.iter().flat_map(|l| l[4..].split(", ")).collect();
        assert_eq!(names.len(), 12);
        assert!(names.iter().all(|n| n.starts_with("S. Student")));
        assert!(lines.contains(&"TEL 1.8-m f/4.4 Ritchey-Chretien + CCD".to_string()));
        assert!(lines.contains(&"NET Gaia-DR3".to_string()));
        assert!(lines.contains(&"ACK IASC batch 1".to_string()));
        assert_eq!(
            lines.last().unwrap(),
            "AC2 teacher@example.org,team@example.org"
        );
        assert!(
            lines
                .iter()
                .all(|l| l.chars().count() <= 80 && l.as_bytes()[3] == b' ')
        );
    }

    #[test]
    fn minimal_header_and_long_contact() {
        let d = Details {
            code: "F51".into(),
            observers: vec!["A. Student".into()],
            ..Details::default()
        };
        assert_eq!(header_lines(&d), ["COD F51", "OBS A. Student"]);
        let d = Details {
            contact_name: "A. Teacher".into(),
            contact_address:
                "Department of Physics and Astronomy, University of Somewhere Far Away, \
                              1234 Very Long Avenue Name, Springfield, Some Country"
                    .into(),
            ..details()
        };
        let con: Vec<String> = header_lines(&d)
            .into_iter()
            .filter(|l| l.starts_with("CON "))
            .collect();
        assert!(
            con.len() >= 3 && con.iter().all(|l| l.len() <= 80),
            "{con:?}"
        );
        assert!(con[0].starts_with("CON A. Teacher, "));
    }

    #[test]
    fn names_follow_the_mpc_rules() {
        for good in [
            "J. Smith",
            "J. R. R. Tolkien",
            "J.-F. Hulot",
            "A. van Dijk",
            "Ş. Yılmaz",
        ] {
            assert_eq!(name_problem(good), None, "{good}");
        }
        for bad in [
            "J.M. Jarre",
            "Vangelis Papathanassiou",
            "M. OLDFIELD",
            "Smith",
            "j. smith",
        ] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn temporary_designations() {
        for good in ["OC0001", "TRK12A", "A", "ABC1234"] {
            assert_eq!(designation_problem(good), None, "{good}");
        }
        for bad in ["", "1ABC", "OC 001", "OC-001", "ABCD12345", "K26T03A"] {
            assert!(designation_problem(bad).is_some(), "{bad}");
        }
    }
}
