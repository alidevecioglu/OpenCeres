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
        Some(m) => line += &format!("{:<5}", format!("{m:.1}")),
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
    }
}
