//! Earth and observer positions for computing minor-planet ephemerides.
//!
//! All vectors are heliocentric (or geocentric for the observer offset),
//! equatorial J2000, in au. Times are Julian dates; [`tt_from_utc`] converts
//! the UTC dates used elsewhere in the core.

#[path = "vsop87_earth.rs"]
mod vsop87_earth;

/// TT − UTC in days. TAI − UTC has been 37 s since 2017 January 1 (IERS
/// Bulletin C; no leap second announced since), and TT − TAI = 32.184 s.
/// Update this when the IERS announces a new leap second.
pub const TT_MINUS_UTC: f64 = 69.184 / 86_400.0;

/// Julian date of J2000.0 (TT).
pub const J2000: f64 = 2_451_545.0;

/// Obliquity of the ecliptic at J2000.0, IAU 2006: 84381.406″.
pub const OBLIQUITY_J2000: f64 = 84_381.406 / 3600.0;

/// Speed of light in au per day (c = 299792.458 km/s, au = 149597870.7 km).
pub const C_AU_PER_DAY: f64 = 299_792.458 * 86_400.0 / 149_597_870.7;

/// Earth equatorial radius (IERS / WGS84) in au; MPC parallax constants are in these units.
const EARTH_RADIUS_AU: f64 = 6378.137 / 149_597_870.7;

pub fn tt_from_utc(jd_utc: f64) -> f64 {
    jd_utc + TT_MINUS_UTC
}

/// Rotates an ecliptic J2000 vector to equatorial J2000.
pub fn ecliptic_to_equatorial([x, y, z]: [f64; 3]) -> [f64; 3] {
    let (s, c) = OBLIQUITY_J2000.to_radians().sin_cos();
    [x, c * y - s * z, s * y + c * z]
}

/// Heliocentric position of the Earth's centre, ecliptic J2000, au, from
/// the truncated VSOP87A series (Bretagnon & Francou 1988). The argument is
/// TT, which equals TDB to ~2 ms. Error < 2e-7 au (0.04″) over 2000–2050.
pub fn earth_ecliptic(jd_tt: f64) -> [f64; 3] {
    let t = (jd_tt - J2000) / 365_250.0;
    vsop87_earth::EARTH.map(|series| {
        let mut total = 0.0;
        let mut power = 1.0;
        for terms in series {
            let sum: f64 = terms.iter().map(|[a, b, c]| a * (b + c * t).cos()).sum();
            total += sum * power;
            power *= t;
        }
        total
    })
}

/// Heliocentric position of the Earth's centre, equatorial J2000, au.
pub fn earth(jd_tt: f64) -> [f64; 3] {
    ecliptic_to_equatorial(earth_ecliptic(jd_tt))
}

/// An observatory: east longitude and parallax constants ρcosφ′, ρsinφ′
/// (Earth radii), as published in the MPC's list of observatory codes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observatory {
    pub longitude: f64,
    pub rho_cos_phi: f64,
    pub rho_sin_phi: f64,
}

/// A few observatories, copied from the MPC's ObsCodes list
/// (<https://www.minorplanetcenter.net/iau/lists/ObsCodes.html>, 2026-10-29).
const OBSERVATORIES: &[(&str, Observatory)] = &[
    ("500", obs(0.0, 0.0, 0.0)),                 // Geocentric
    ("568", obs(204.5278, 0.94171, 0.33725)),    // Maunakea
    ("F51", obs(203.74409, 0.936241, 0.351543)), // Pan-STARRS 1, Haleakala
    ("F52", obs(203.74409, 0.936239, 0.351545)), // Pan-STARRS 2, Haleakala
];

const fn obs(longitude: f64, rho_cos_phi: f64, rho_sin_phi: f64) -> Observatory {
    Observatory {
        longitude,
        rho_cos_phi,
        rho_sin_phi,
    }
}

/// Looks up an MPC observatory code in the built-in table.
pub fn observatory(code: &str) -> Option<Observatory> {
    OBSERVATORIES
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code.trim()))
        .map(|(_, o)| *o)
}

/// Greenwich mean sidereal time in degrees (Meeus, Astronomical Algorithms,
/// eq. 12.4). Takes UT1; UTC is within 0.9 s, which moves the observer by
/// < 0.4 km — irrelevant here.
pub fn gmst(jd_ut: f64) -> f64 {
    let d = jd_ut - J2000;
    let t = d / 36_525.0;
    (280.460_618_37 + 360.985_647_366_29 * d + 0.000_387_933 * t * t - t * t * t / 38_710_000.0)
        .rem_euclid(360.0)
}

/// Rotation from the mean equator and equinox of date to J2000, using the
/// IAU 1976 precession angles ζ, z, θ (Lieske et al. 1977; Meeus eq. 21.3).
fn precess_to_j2000(v: [f64; 3], jd_tt: f64) -> [f64; 3] {
    let t = (jd_tt - J2000) / 36_525.0;
    let arcsec = |a: f64, b: f64, c: f64| ((a + (b + c * t) * t) * t / 3600.0).to_radians();
    let zeta = arcsec(2306.2181, 0.30188, 0.017998);
    let z = arcsec(2306.2181, 1.09468, 0.018203);
    let theta = arcsec(2004.3109, -0.42665, -0.041833);
    // J2000 → date is Rz(−z)·Ry(θ)·Rz(−ζ); apply its transpose.
    let v = rot_z(v, z);
    let v = rot_y(v, -theta);
    rot_z(v, zeta)
}

/// Rotates the coordinate frame by `angle` about z (vector components change by −angle).
fn rot_z([x, y, z]: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    [c * x + s * y, -s * x + c * y, z]
}

fn rot_y([x, y, z]: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    [c * x - s * z, y, s * x + c * z]
}

/// Geocentric position of an observatory, equatorial J2000, au, at UTC `jd_utc`.
/// Nutation (< 20″ in the direction of a 6400 km vector) is ignored.
pub fn observer_offset(site: &Observatory, jd_utc: f64) -> [f64; 3] {
    let lst = (gmst(jd_utc) + site.longitude).to_radians();
    let of_date = [
        site.rho_cos_phi * lst.cos() * EARTH_RADIUS_AU,
        site.rho_cos_phi * lst.sin() * EARTH_RADIUS_AU,
        site.rho_sin_phi * EARTH_RADIUS_AU,
    ];
    precess_to_j2000(of_date, tt_from_utc(jd_utc))
}

/// Heliocentric position of the observer, equatorial J2000, au, at UTC
/// `jd_utc`; the geocentre if `site` is `None`.
pub fn observer(site: Option<&Observatory>, jd_utc: f64) -> [f64; 3] {
    let e = earth(tt_from_utc(jd_utc));
    match site {
        Some(s) => add(e, observer_offset(s, jd_utc)),
        None => e,
    }
}

pub fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// RA, Dec in degrees (RA in [0, 360)) of an equatorial vector.
pub fn ra_dec(v: [f64; 3]) -> (f64, f64) {
    let ra = v[1].atan2(v[0]).to_degrees().rem_euclid(360.0);
    let dec = (v[2] / norm(v)).clamp(-1.0, 1.0).asin().to_degrees();
    (ra, dec)
}

/// Unit vector towards RA, Dec in degrees.
pub fn unit(ra: f64, dec: f64) -> [f64; 3] {
    let (sr, cr) = ra.to_radians().sin_cos();
    let (sd, cd) = dec.to_radians().sin_cos();
    [cd * cr, cd * sr, sd]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earth_matches_published_check_value() {
        // vsop87.chk gives the full series at J2000.0; the truncated table must
        // stay within the truncation error the generator measured.
        let e = earth_ecliptic(J2000);
        let err = norm(sub(e, vsop87_earth::CHECK_J2000));
        assert!(err < vsop87_earth::MAX_ERROR + 1e-9, "{err:e} au");
    }

    #[test]
    fn earth_matches_full_series_in_2026() {
        let e = earth_ecliptic(2_461_317.5);
        let err = norm(sub(e, vsop87_earth::FULL_2461317));
        // 1e-6 au is 0.2″ seen from the Sun; the bound is far tighter.
        assert!(err < vsop87_earth::MAX_ERROR, "{err:e} au");
        // Sanity: about 1 au from the Sun, near the ecliptic.
        assert!((norm(e) - 1.0).abs() < 0.02 && e[2].abs() < 1e-4);
    }

    #[test]
    fn observatory_table() {
        let f51 = observatory("F51").unwrap();
        assert_eq!(
            (f51.longitude, f51.rho_cos_phi, f51.rho_sin_phi),
            (203.74409, 0.936241, 0.351543)
        );
        assert_eq!(observatory("f52").unwrap().rho_sin_phi, 0.351545);
        assert_eq!(observatory("500").unwrap().rho_cos_phi, 0.0);
        assert_eq!(observatory("XXX"), None);
        // Haleakala: geocentric latitude ≈ 20.6°, distance ≈ 1 Earth radius.
        let lat = f51.rho_sin_phi.atan2(f51.rho_cos_phi).to_degrees();
        assert!((lat - 20.58).abs() < 0.05, "{lat}");
        // The geocentre has no offset; F51 is one Earth radius away.
        assert_eq!(
            observer_offset(&observatory("500").unwrap(), J2000),
            [0.0; 3]
        );
        let r = norm(observer_offset(&f51, 2_461_317.9)) / EARTH_RADIUS_AU;
        assert!((r - f51.rho_cos_phi.hypot(f51.rho_sin_phi)).abs() < 1e-12);
    }

    #[test]
    fn sidereal_time_meeus_example() {
        // Meeus example 12.a: 1987 April 10, 0h UT → 13h10m46.3668s.
        let expected = (13.0 + 10.0 / 60.0 + 46.3668 / 3600.0) * 15.0;
        assert!((gmst(2_446_895.5) - expected).abs() * 3600.0 < 0.01);
    }

    #[test]
    fn precession_round_trip_and_size() {
        // A point at RA 0, Dec 0 moves by m = 46.12″/yr in RA and n = 20.04″/yr
        // in Dec (the annual precession constants), √(m² + n²) = 50.29″/yr.
        let jd = 2_461_317.5;
        let v = precess_to_j2000([1.0, 0.0, 0.0], jd);
        let shift = (2.0 * (1.0 - v[0])).sqrt();
        let years = (jd - J2000) / 365.25;
        let expected = (50.29 * years / 3600.0).to_radians();
        assert!(
            (shift - expected).abs() / expected < 0.01,
            "{shift} vs {expected}"
        );
        // Date → J2000 moves the point to negative RA and Dec.
        assert!(v[1] < 0.0 && v[2] < 0.0);
        assert!((norm(v) - 1.0).abs() < 1e-15);
    }

    #[test]
    fn utc_to_tt() {
        assert!(((tt_from_utc(J2000) - J2000) * 86_400.0 - 69.184).abs() < 1e-4);
        assert!((C_AU_PER_DAY - 173.144_632_674).abs() < 1e-6);
    }
}
