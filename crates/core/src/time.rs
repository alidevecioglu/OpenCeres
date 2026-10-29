//! Observation times: Julian dates and the calendar dates MPC reports use.

/// Modified Julian Date offset: MJD = JD − 2400000.5.
pub const MJD_OFFSET: f64 = 2_400_000.5;

/// A UTC calendar date with the time of day as a fraction of the day,
/// e.g. 2026-10-29.512345.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalendarDate {
    pub year: i32,
    pub month: u32,
    pub day: f64,
}

/// Julian date to Gregorian calendar date (Meeus, Astronomical Algorithms, ch. 7).
pub fn jd_to_calendar(jd: f64) -> CalendarDate {
    let jd = jd + 0.5;
    let z = jd.floor();
    let f = jd - z;
    let a = if z < 2_299_161.0 {
        z
    } else {
        let alpha = ((z - 1_867_216.25) / 36_524.25).floor();
        z + 1.0 + alpha - (alpha / 4.0).floor()
    };
    let b = a + 1524.0;
    let c = ((b - 122.1) / 365.25).floor();
    let d = (365.25 * c).floor();
    let e = ((b - d) / 30.6001).floor();

    let day = b - d - (30.6001 * e).floor() + f;
    let month = if e < 14.0 { e - 1.0 } else { e - 13.0 };
    let year = if month > 2.0 { c - 4716.0 } else { c - 4715.0 };
    CalendarDate {
        year: year as i32,
        month: month as u32,
        day,
    }
}

/// Gregorian calendar date to Julian date (Meeus, ch. 7).
pub fn calendar_to_jd(date: CalendarDate) -> f64 {
    let (mut y, mut m) = (date.year as f64, date.month as f64);
    if m <= 2.0 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + date.day + b - 1524.5
}

/// Parses a FITS `DATE-OBS` value such as `2026-10-29T12:17:46.5` (or a plain date) to JD.
pub fn parse_iso(value: &str) -> Option<f64> {
    let (date, time) = value.split_once('T').unwrap_or((value, "00:00:00"));
    let mut d = date.split('-');
    let year: i32 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let mut t = time.trim_end_matches('Z').split(':');
    let hour: f64 = t.next()?.parse().ok()?;
    let minute: f64 = t.next().unwrap_or("0").parse().ok()?;
    let second: f64 = t.next().unwrap_or("0").parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let fraction = (hour * 3600.0 + minute * 60.0 + second) / 86_400.0;
    Some(calendar_to_jd(CalendarDate {
        year,
        month,
        day: day as f64 + fraction,
    }))
}

/// Julian date (UTC) of the middle of the exposure, which is what MPC
/// reports need. Uses `MJD-OBS` or `DATE-OBS` (both the exposure *start*,
/// per the FITS standard) plus half of `EXPTIME`.
pub fn mid_exposure_jd(header: &crate::fits::Header) -> Option<f64> {
    let start = header
        .float("MJD-OBS")
        .map(|mjd| mjd + MJD_OFFSET)
        .or_else(|| parse_iso(header.text("DATE-OBS")?))?;
    let exposure = header.float("EXPTIME").unwrap_or(0.0);
    Some(start + exposure / 2.0 / 86_400.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j2000_epoch() {
        let d = jd_to_calendar(2_451_545.0);
        assert_eq!((d.year, d.month), (2000, 1));
        assert!((d.day - 1.5).abs() < 1e-9);
        assert_eq!(calendar_to_jd(d), 2_451_545.0);
    }

    #[test]
    fn meeus_examples() {
        // Meeus example 7.a: 1957 October 4.81 = JD 2436116.31
        let jd = calendar_to_jd(CalendarDate {
            year: 1957,
            month: 10,
            day: 4.81,
        });
        assert!((jd - 2_436_116.31).abs() < 1e-6);
        // Leap day survives a round trip.
        let d = jd_to_calendar(calendar_to_jd(CalendarDate {
            year: 2024,
            month: 2,
            day: 29.25,
        }));
        assert_eq!((d.year, d.month), (2024, 2));
        assert!((d.day - 29.25).abs() < 1e-8);
    }

    #[test]
    fn parses_date_obs() {
        let jd = parse_iso("2026-10-29T12:00:00.0").unwrap();
        let d = jd_to_calendar(jd);
        assert_eq!((d.year, d.month), (2026, 10));
        assert!((d.day - 29.5).abs() < 1e-8);
        assert_eq!(parse_iso("2026-10-29"), Some(jd - 0.5));
        assert_eq!(parse_iso("2026-13-01T00:00:00"), None);
    }

    #[test]
    fn mid_exposure_adds_half_the_exposure() {
        let bytes = crate::fits::tests::make_fits(
            1,
            1,
            &[
                "DATE-OBS= '2026-10-04T10:15:39.832315'",
                "EXPTIME =                  45.",
            ],
            &[0.0],
        );
        let header = crate::fits::parse(&bytes).unwrap().header;
        let mid = jd_to_calendar(mid_exposure_jd(&header).unwrap());
        // 10:15:39.832315 + 22.5 s = 10:16:02.332315
        let expected = 4.0 + (10.0 * 3600.0 + 16.0 * 60.0 + 2.332315) / 86_400.0;
        assert_eq!((mid.year, mid.month), (2026, 10));
        assert!((mid.day - expected).abs() * 86_400.0 < 1e-3, "{}", mid.day);
    }
}
