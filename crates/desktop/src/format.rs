//! Text shown to the user: sky coordinates and observation times.

/// RA in degrees as `HH MM SS.ss`, rounded to the last printed digit.
pub fn ra(deg: f64) -> String {
    let cs = (deg.rem_euclid(360.0) / 15.0 * 360_000.0).round() as i64 % (24 * 360_000);
    let (h, m, s) = (cs / 360_000, cs % 360_000 / 6000, cs % 6000);
    format!("{h:02} {m:02} {:02}.{:02}", s / 100, s % 100)
}

/// Dec in degrees as `±DD MM SS.s`, rounded to the last printed digit.
pub fn dec(deg: f64) -> String {
    let sign = if deg < 0.0 { '-' } else { '+' };
    let ds = (deg.abs() * 36_000.0).round() as i64;
    let (d, m, s) = (ds / 36_000, ds % 36_000 / 600, ds % 600);
    format!("{sign}{d:02} {m:02} {:02}.{}", s / 10, s % 10)
}

/// `2026-10-04 08:12:33.1 UTC` from DATE-OBS (fractional seconds cut to one
/// digit), else the MJD.
pub fn obs_time(date_obs: Option<&str>, mjd: Option<f64>) -> String {
    if let Some(date) = date_obs {
        return format!("{} UTC", one_decimal(&date.replacen('T', " ", 1)));
    }
    match mjd {
        Some(m) => format!("MJD {m:.5}"),
        None => "No observation time".into(),
    }
}

/// `08:12:33 UTC` from DATE-OBS (seconds truncated), for the compact frame
/// badge; falls back to [`obs_time`] when there is no clock time.
pub fn clock(date_obs: Option<&str>, mjd: Option<f64>) -> String {
    if let Some(t) = date_obs
        .and_then(|d| d.split_once('T').or_else(|| d.split_once(' ')))
        .map(|(_, t)| t)
        .filter(|t| t.len() >= 8 && t.as_bytes()[2] == b':')
    {
        return format!("{} UTC", &t[..8]);
    }
    obs_time(date_obs, mjd)
}

/// Cuts the first `:SS.ddd…` down to `:SS.d`.
fn one_decimal(s: &str) -> String {
    let b = s.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    for i in 0..b.len() {
        if b[i] == b':'
            && digit(i + 1)
            && digit(i + 2)
            && b.get(i + 3) == Some(&b'.')
            && digit(i + 4)
        {
            let keep = i + 5;
            let end = (keep..b.len()).find(|&j| !digit(j)).unwrap_or(b.len());
            if end > keep {
                return format!("{}{}", &s[..keep], &s[end..]);
            }
            return s.to_owned();
        }
    }
    s.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_coordinates() {
        assert_eq!(ra(0.0), "00 00 00.00");
        assert_eq!(ra(359.999_999_9), "00 00 00.00"); // rounds up to 24h, wraps
        assert_eq!(ra(-15.0), "23 00 00.00");
        assert_eq!(ra(83.633_08), "05 34 31.94");
        assert_eq!(dec(22.014_47), "+22 00 52.1");
        assert_eq!(dec(-5.391_11), "-05 23 28.0");
        assert_eq!(dec(-0.000_001), "-00 00 00.0");
    }

    #[test]
    fn observation_times() {
        assert_eq!(
            obs_time(Some("2026-10-04T08:12:33.123456"), Some(1.0)),
            "2026-10-04 08:12:33.1 UTC"
        );
        assert_eq!(
            obs_time(Some("2026-10-04T08:12:33"), None),
            "2026-10-04 08:12:33 UTC"
        );
        assert_eq!(obs_time(None, Some(61317.5)), "MJD 61317.50000");
        assert_eq!(obs_time(None, None), "No observation time");
        assert_eq!(clock(Some("2026-10-04T08:12:33.987"), None), "08:12:33 UTC");
        assert_eq!(clock(Some("2026-10-04"), None), "2026-10-04 UTC");
        assert_eq!(clock(None, Some(61317.5)), "MJD 61317.50000");
    }
}
