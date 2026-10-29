//! Minor-planet motion from osculating heliocentric elements: two-body
//! (Keplerian) positions, and a numerical integration that adds the
//! planets' attraction for accurate positions far from the elements' epoch.

use std::f64::consts::{PI, TAU};

/// Gaussian gravitational constant k: GM☉ = k² au³/day² (IAU 1976). The MPC's
/// mean motions satisfy n = k·a^(−3/2) with it.
pub const GAUSS_K: f64 = 0.017_202_098_95;
const GM_SUN: f64 = GAUSS_K * GAUSS_K;

/// Osculating elliptic elements, angles in degrees referred to the ecliptic
/// and equinox J2000, as in the MPC orbit files.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elements {
    /// Epoch of osculation, JD (TT).
    pub epoch: f64,
    /// Mean anomaly at the epoch.
    pub mean_anomaly: f64,
    pub perihelion: f64,
    pub node: f64,
    pub inclination: f64,
    pub eccentricity: f64,
    /// Mean daily motion, degrees per day.
    pub mean_motion: f64,
    /// Semimajor axis, au.
    pub semimajor_axis: f64,
}

/// Solves Kepler's equation E − e·sin E = M for elliptic orbits (0 ≤ e < 1),
/// angles in radians. Newton's method from Danby's starting value
/// E₀ = M + 0.85·e·sign(sin M), with M reduced to (−π, π]; this converges for
/// every M and e < 1 and typically needs 3–6 steps even at e = 0.99.
pub fn solve_kepler(mean_anomaly: f64, e: f64) -> f64 {
    let m = (mean_anomaly + PI).rem_euclid(TAU) - PI;
    let mut big_e = m + 0.85 * e * if m >= 0.0 { 1.0 } else { -1.0 };
    for _ in 0..50 {
        let (s, c) = big_e.sin_cos();
        let step = (big_e - e * s - m) / (1.0 - e * c);
        big_e -= step;
        if step.abs() < 1e-14 {
            break;
        }
    }
    big_e
}

impl Elements {
    /// Heliocentric position, ecliptic J2000, au, at `jd_tt` (two-body).
    pub fn position(&self, jd_tt: f64) -> [f64; 3] {
        self.state(jd_tt).0
    }

    /// Heliocentric position (au) and velocity (au/day), ecliptic J2000, at
    /// `jd_tt`, from two-body motion about the Sun.
    pub fn state(&self, jd_tt: f64) -> ([f64; 3], [f64; 3]) {
        let e = self.eccentricity;
        let a = self.semimajor_axis;
        let m = (self.mean_anomaly + self.mean_motion * (jd_tt - self.epoch)).to_radians();
        let (se, ce) = solve_kepler(m, e).sin_cos();
        let b = (1.0 - e * e).sqrt();
        // In the orbital plane, x towards perihelion.
        let position = [a * (ce - e), a * b * se, 0.0];
        let speed = (GM_SUN * a).sqrt() / (a * (1.0 - e * ce));
        let velocity = [-speed * se, speed * b * ce, 0.0];
        let rotate = |v| to_ecliptic(v, self.perihelion, self.node, self.inclination);
        (rotate(position), rotate(velocity))
    }

    /// Perihelion distance, au.
    pub fn perihelion_distance(&self) -> f64 {
        self.semimajor_axis * (1.0 - self.eccentricity)
    }
}

/// Orbital-plane vector (x towards perihelion) to ecliptic: Rz(Ω)·Rx(i)·Rz(ω),
/// angles in degrees.
fn to_ecliptic([px, py, _]: [f64; 3], perihelion: f64, node: f64, inclination: f64) -> [f64; 3] {
    let (sw, cw) = perihelion.to_radians().sin_cos();
    let (sn, cn) = node.to_radians().sin_cos();
    let (si, ci) = inclination.to_radians().sin_cos();
    let x1 = cw * px - sw * py;
    let y1 = sw * px + cw * py;
    let y2 = ci * y1;
    let z2 = si * y1;
    [cn * x1 - sn * y2, sn * x1 + cn * y2, z2]
}

/// A planet for the perturbation model: mass in solar masses and
/// Keplerian elements a (au), e, I, L, ϖ, Ω (degrees) with their rates per
/// Julian century.
struct Planet {
    mass: f64,
    elements: [f64; 6],
    rates: [f64; 6],
}

const fn planet(mass: f64, elements: [f64; 6], rates: [f64; 6]) -> Planet {
    Planet {
        mass,
        elements,
        rates,
    }
}

/// The eight planets (Earth as the Earth–Moon barycentre). Elements: E. M.
/// Standish, "Keplerian Elements for Approximate Positions of the Major
/// Planets", JPL, Table 1 (mean ecliptic and equinox J2000, 1800–2050 AD;
/// <https://ssd.jpl.nasa.gov/planets/approx_pos.html>), good to arcminutes or
/// better — ample for perturbations. Masses: GM(planet system) / GM☉ from
/// JPL's Astrodynamic Parameters (<https://ssd.jpl.nasa.gov/astro_par.html>).
#[rustfmt::skip]
const PLANETS: [Planet; 8] = [
    // Mercury
    planet(1.6601208254589484e-07, [0.38709927, 0.20563593, 7.00497902, 252.25032350, 77.45779628, 48.33076593], [0.00000037, 0.00001906, -0.00594749, 149472.67411175, 0.16047689, -0.12534081]),
    // Venus
    planet(2.447838287796944e-06, [0.72333566, 0.00677672, 3.39467605, 181.97909950, 131.60246718, 76.67984255], [0.00000390, -0.00004107, -0.00078890, 58517.81538729, 0.00268329, -0.27769418]),
    // Earth–Moon barycentre
    planet(3.04043264896262e-06, [1.00000261, 0.01671123, -0.00001531, 100.46457166, 102.93768193, 0.0], [0.00000562, -0.00004392, -0.01294668, 35999.37244981, 0.32327364, 0.0]),
    // Mars
    planet(3.2271560829322774e-07, [1.52371034, 0.09339410, 1.84969142, -4.55343205, -23.94362959, 49.55953891], [0.00001847, 0.00007882, -0.00813131, 19140.30268499, 0.44441088, -0.29257343]),
    // Jupiter
    planet(0.0009547919099414248, [5.20288700, 0.04838624, 1.30439695, 34.39644051, 14.72847983, 100.47390909], [-0.00011607, -0.00013253, -0.00183714, 3034.74612775, 0.21252668, 0.20469106]),
    // Saturn
    planet(0.00028588567002459455, [9.53667594, 0.05386179, 2.48599187, 49.95424423, 92.59887831, 113.66242448], [-0.00125060, -0.00050991, 0.00193609, 1222.49362201, -0.41897216, -0.28867794]),
    // Uranus
    planet(4.36624961322212e-05, [19.18916464, 0.04725744, 0.77263783, 313.23810451, 170.95427630, 74.01692503], [-0.00196176, -0.00004397, -0.00242939, 428.48202785, 0.40805281, 0.04240589]),
    // Neptune
    planet(5.151383772654274e-05, [30.06992276, 0.00859048, 1.77004347, -55.12002969, 44.96476227, 131.78422574], [0.00026291, 0.00005105, 0.00035372, 218.45945325, -0.32241464, -0.00508664]),
];

impl Planet {
    /// Heliocentric position, ecliptic J2000, au (Standish's recipe).
    fn position(&self, jd_tt: f64) -> [f64; 3] {
        let t = (jd_tt - 2_451_545.0) / 36_525.0;
        let [a, e, i, l, varpi, node] =
            std::array::from_fn(|k| self.elements[k] + self.rates[k] * t);
        let m = (l - varpi).to_radians();
        let (se, ce) = solve_kepler(m, e).sin_cos();
        let plane = [a * (ce - e), a * (1.0 - e * e).sqrt() * se, 0.0];
        to_ecliptic(plane, varpi - node, node, i)
    }
}

/// Heliocentric acceleration of a massless body at `r` (ecliptic, au/day²):
/// the Sun plus each planet's direct pull minus its pull on the Sun.
fn acceleration(r: [f64; 3], jd_tt: f64) -> [f64; 3] {
    let inv3 = |v: [f64; 3]| {
        let d2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        1.0 / (d2 * d2.sqrt())
    };
    let s = -GM_SUN * inv3(r);
    let mut acc = [s * r[0], s * r[1], s * r[2]];
    for p in &PLANETS {
        let rp = p.position(jd_tt);
        let d = [rp[0] - r[0], rp[1] - r[1], rp[2] - r[2]];
        let (direct, indirect) = (inv3(d), inv3(rp));
        let gm = GM_SUN * p.mass;
        for k in 0..3 {
            acc[k] += gm * (d[k] * direct - rp[k] * indirect);
        }
    }
    acc
}

/// A heliocentric state (ecliptic J2000, au, au/day) at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub jd_tt: f64,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub acceleration: [f64; 3],
}

impl State {
    /// Position at `jd_tt` near the state's epoch, by the second-order
    /// Taylor series. Intended for the light-time and motion offsets
    /// (≤ ~1 h): the neglected jerk term is < 1e-9 au there even for NEAs.
    pub fn position_near(&self, jd_tt: f64) -> [f64; 3] {
        let dt = jd_tt - self.jd_tt;
        std::array::from_fn(|k| {
            self.position[k] + self.velocity[k] * dt + 0.5 * self.acceleration[k] * dt * dt
        })
    }
}

type Phase = ([f64; 3], [f64; 3]);

fn rk4_step((r, v): Phase, jd: f64, h: f64) -> Phase {
    let axpy = |x: [f64; 3], s: f64, y: [f64; 3]| std::array::from_fn(|k| x[k] + s * y[k]);
    let a1 = acceleration(r, jd);
    let (r2, v2) = (axpy(r, h / 2.0, v), axpy(v, h / 2.0, a1));
    let a2 = acceleration(r2, jd + h / 2.0);
    let (r3, v3) = (axpy(r, h / 2.0, v2), axpy(v, h / 2.0, a2));
    let a3 = acceleration(r3, jd + h / 2.0);
    let (r4, v4) = (axpy(r, h, v3), axpy(v, h, a3));
    let a4 = acceleration(r4, jd + h);
    let r_new =
        std::array::from_fn(|k| r[k] + h / 6.0 * (v[k] + 2.0 * v2[k] + 2.0 * v3[k] + v4[k]));
    let v_new =
        std::array::from_fn(|k| v[k] + h / 6.0 * (a1[k] + 2.0 * a2[k] + 2.0 * a3[k] + a4[k]));
    (r_new, v_new)
}

/// Position tolerance per integration step, au (≈ 0.3 m).
const STEP_TOLERANCE: f64 = 2e-12;

/// Integrates the orbit from the elements' epoch to `jd_tt` under the
/// gravity of the Sun and the eight planets: classical Runge–Kutta (4th
/// order) with step-size control by step doubling (the difference between
/// one step and two half steps estimates the error; Richardson
/// extrapolation then adds one order). `None` if the integration fails,
/// e.g. on a collision-like close approach.
pub fn propagate(elements: &Elements, jd_tt: f64) -> Option<State> {
    let mut jd = elements.epoch;
    let mut y = elements.state(jd);
    let span = jd_tt - jd;
    let direction = if span >= 0.0 { 1.0 } else { -1.0 };
    let mut h = direction * span.abs().min(1.0);
    let mut steps = 0;
    while (jd_tt - jd) * direction > 1e-9 {
        steps += 1;
        if steps > 100_000 {
            return None;
        }
        if (jd + h - jd_tt) * direction > 0.0 {
            h = jd_tt - jd;
        }
        let full = rk4_step(y, jd, h);
        let half = rk4_step(rk4_step(y, jd, h / 2.0), jd + h / 2.0, h / 2.0);
        let err = (0..3)
            .map(|k| (half.0[k] - full.0[k]).abs())
            .fold(0.0, f64::max)
            / 15.0;
        if !err.is_finite() {
            return None;
        }
        let factor = (0.9 * (STEP_TOLERANCE / err.max(1e-30)).powf(0.2)).clamp(0.2, 4.0);
        if err <= STEP_TOLERANCE || h.abs() < 1e-6 {
            jd += h;
            y = (
                std::array::from_fn(|k| half.0[k] + (half.0[k] - full.0[k]) / 15.0),
                std::array::from_fn(|k| half.1[k] + (half.1[k] - full.1[k]) / 15.0),
            );
            h = direction * (h.abs() * factor).min(20.0);
        } else {
            h *= factor;
        }
    }
    Some(State {
        jd_tt,
        position: y.0,
        velocity: y.1,
        acceleration: acceleration(y.0, jd_tt),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kepler_solves_the_equation_everywhere() {
        for &e in &[0.0, 0.1, 0.5, 0.9, 0.97, 0.99, 0.999] {
            for k in -40..=40 {
                let m = k as f64 * 0.157 + 0.001;
                let big_e = solve_kepler(m, e);
                let residual = big_e - e * big_e.sin() - ((m + PI).rem_euclid(TAU) - PI);
                assert!(residual.abs() < 1e-12, "e={e} M={m}: {residual:e}");
            }
        }
    }

    #[test]
    fn kepler_meeus_example() {
        // Meeus example 30.a: e = 0.100, M = 5° → E = 5.554589°.
        let big_e = solve_kepler(5f64.to_radians(), 0.1).to_degrees();
        assert!((big_e - 5.554_589).abs() < 1e-6, "{big_e}");
    }

    fn circular() -> Elements {
        Elements {
            epoch: 2_451_545.0,
            mean_anomaly: 0.0,
            perihelion: 0.0,
            node: 0.0,
            inclination: 0.0,
            eccentricity: 0.0,
            mean_motion: 0.985_607_7,
            semimajor_axis: 1.0,
        }
    }

    #[test]
    fn circular_orbit_moves_at_the_mean_motion() {
        let o = circular();
        let p = o.position(2_451_545.0 + 91.31);
        // ~90° along the orbit after a quarter period.
        assert!(p[0].abs() < 1e-3 && (p[1] - 1.0).abs() < 1e-6 && p[2] == 0.0);
    }

    #[test]
    fn orientation_angles() {
        // At perihelion, with ω = 90°, Ω = 0, i = 90°: the body is straight
        // above the ecliptic at distance q.
        let o = Elements {
            perihelion: 90.0,
            inclination: 90.0,
            eccentricity: 0.5,
            semimajor_axis: 2.0,
            ..circular()
        };
        let p = o.position(o.epoch);
        assert!(p[0].abs() < 1e-12 && p[1].abs() < 1e-12 && (p[2] - 1.0).abs() < 1e-12);
        // Ω rotates the node line: with i = 0, ω = 0, Ω = 90° perihelion lies on +y.
        let o = Elements {
            node: 90.0,
            eccentricity: 0.5,
            semimajor_axis: 2.0,
            ..circular()
        };
        let p = o.position(o.epoch);
        assert!(p[0].abs() < 1e-12 && (p[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn velocity_is_the_derivative_of_position() {
        let o = Elements {
            eccentricity: 0.6,
            semimajor_axis: 1.8,
            mean_motion: (GAUSS_K / 1.8f64.powf(1.5)).to_degrees(),
            inclination: 20.0,
            node: 40.0,
            perihelion: 75.0,
            mean_anomaly: 30.0,
            // A small epoch keeps the time resolution far below the step h.
            epoch: 0.0,
        };
        let (_, v) = o.state(o.epoch + 10.0);
        let h = 1e-3;
        let (p1, p0) = (
            o.position(o.epoch + 10.0 + h),
            o.position(o.epoch + 10.0 - h),
        );
        for k in 0..3 {
            let fd = (p1[k] - p0[k]) / (2.0 * h);
            assert!((fd - v[k]).abs() < 1e-9, "{k}: {fd} vs {}", v[k]);
        }
    }

    #[test]
    fn earth_moon_barycentre_follows_vsop87() {
        // Standish's approximate EMB stays within its quoted accuracy
        // (tens of arcseconds) of VSOP87 Earth; the EMB–Earth offset is 3e-5 au.
        for jd in [2_451_545.0, 2_461_317.5, 2_469_000.0] {
            let emb = PLANETS[2].position(jd);
            let earth = crate::ephem::earth_ecliptic(jd);
            let d = (0..3)
                .map(|k| (emb[k] - earth[k]).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(d < 2e-4, "{d}");
        }
    }

    #[test]
    fn perturbed_orbit_matches_jpl() {
        // (133105) 2003 OG10: MPC osculating elements at 2026 June 9.0 TT,
        // integrated to 2026 Oct 4.0, against the osculating elements JPL
        // Horizons gives for Oct 4.0 TDB (heliocentric, ecliptic J2000).
        let june = Elements {
            epoch: 2_461_200.5,
            mean_anomaly: 14.32920,
            perihelion: 208.44664,
            node: 104.03668,
            inclination: 2.39951,
            eccentricity: 0.1984535,
            mean_motion: 0.25984287,
            semimajor_axis: 2.4321789,
        };
        let october = Elements {
            epoch: 2_461_317.5,
            mean_anomaly: 44.743_037_317_190_19,
            perihelion: 208.434_892_086_501_1,
            node: 104.036_075_979_627_1,
            inclination: 2.399_504_920_027_304,
            eccentricity: 0.198_421_967_831_130_4,
            mean_motion: 0.259_864_664_592_841_7,
            semimajor_axis: 2.432_042_888_750_884,
        };
        let jpl = october.position(october.epoch);
        let ours = propagate(&june, october.epoch).unwrap();
        let err = |p: [f64; 3]| (0..3).map(|k| (p[k] - jpl[k]).powi(2)).sum::<f64>().sqrt();
        // The MPC elements are rounded to 1e-5° (≈ 2e-7 au at 2.4 au).
        assert!(err(ours.position) < 1e-6, "{:e}", err(ours.position));
        // Two-body motion is off by over ten times as much.
        assert!(err(june.position(october.epoch)) > 1e-5);
        // Integrating back recovers the start.
        let back = propagate(
            &Elements {
                epoch: october.epoch,
                ..october
            },
            june.epoch,
        )
        .unwrap();
        let start = june.position(june.epoch);
        let d = (0..3)
            .map(|k| (back.position[k] - start[k]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(d < 1e-6, "{d:e}");
    }
}
