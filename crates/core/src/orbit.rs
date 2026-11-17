//! Minor-planet and comet motion from osculating heliocentric elements:
//! two-body (Keplerian) positions for elliptic, parabolic and hyperbolic
//! orbits, and a numerical integration that adds the planets' attraction for
//! accurate positions far from the elements' epoch.

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

/// Osculating elements in perihelion form, as the MPC gives them for comets:
/// any eccentricity (elliptic, parabolic or hyperbolic). Angles in degrees,
/// ecliptic and equinox J2000.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conic {
    /// Time of perihelion passage, JD (TT).
    pub perihelion_time: f64,
    /// Perihelion distance q, au.
    pub perihelion_distance: f64,
    pub eccentricity: f64,
    pub perihelion: f64,
    pub node: f64,
    pub inclination: f64,
}

impl Conic {
    /// Heliocentric position (au) and velocity (au/day), ecliptic J2000, at
    /// `jd_tt`, from two-body motion about the Sun.
    ///
    /// Uses the universal variable χ (Danby, *Fundamentals of Celestial
    /// Mechanics*, 2nd ed., §6.9; Vallado, *Fundamentals of Astrodynamics
    /// and Applications*, Algorithm 8), started at perihelion where r = q and
    /// r·v = 0. With α = 1/a = (1 − e)/q and z = αχ², Kepler's equation is
    /// √μ·Δt = e·χ³·c₃(z) + q·χ: one formula for every conic, so orbits with
    /// e near 1 need no special case. Its derivative in χ is r > 0, and it is
    /// convex for χ ≥ 0 up to aphelion, so Newton's method started above the
    /// root converges monotonically.
    pub fn state(&self, jd_tt: f64) -> ([f64; 3], [f64; 3]) {
        let q = self.perihelion_distance;
        let e = self.eccentricity;
        let alpha = (1.0 - e) / q;
        let mut dt = jd_tt - self.perihelion_time;
        // Bound for χ: q·χ ≤ √μ·|Δt| always; for ellipses also half an
        // orbit (eccentric anomaly π), after reducing Δt to [−P/2, P/2].
        let mut hi = f64::INFINITY;
        if alpha > 0.0 {
            let period = TAU / (GAUSS_K * alpha.powf(1.5));
            dt -= period * (dt / period).round();
            hi = PI / alpha.sqrt();
        }
        let target = GAUSS_K * dt.abs();
        // Newton inside a shrinking bracket [lo, hi]; bisection whenever a
        // step would leave it or sinh overflows far above the root.
        let (mut lo, mut hi) = (0.0, hi.min(target / q));
        let mut chi = hi;
        for _ in 0..500 {
            let (c2, c3) = stumpff(alpha * chi * chi);
            let f = e * chi * chi * chi * c3 + q * chi - target;
            let r = q + e * chi * chi * c2;
            if f.is_finite() && f <= 0.0 {
                lo = chi;
            } else {
                hi = chi;
            }
            let newton = chi - f / r;
            let next = if f.is_finite() && r.is_finite() && newton >= lo && newton <= hi {
                newton
            } else {
                0.5 * (lo + hi)
            };
            let done = (next - chi).abs() <= 1e-15 * next || hi - lo <= 1e-15 * hi;
            chi = next;
            if done {
                break;
            }
        }
        let chi = chi.copysign(dt);
        let z = alpha * chi * chi;
        let (c2, c3) = stumpff(z);
        let r = q + e * chi * chi * c2;
        // Lagrange coefficients from the perihelion state r₀ = (q, 0),
        // v₀ = (0, v_p): x = f·q and y = g·v_p, with g = Δt − χ³c₃/√μ
        // rewritten through Kepler's equation as q·χ·(1 − z·c₃)/√μ, which
        // avoids a cancellation.
        let vp = (GM_SUN * (1.0 + e) / q).sqrt();
        let position = [
            q - chi * chi * c2,
            vp * q * chi * (1.0 - z * c3) / GAUSS_K,
            0.0,
        ];
        let fdot = GAUSS_K / (r * q) * chi * (z * c3 - 1.0);
        let gdot = 1.0 - chi * chi * c2 / r;
        let velocity = [q * fdot, vp * gdot, 0.0];
        let rotate = |v| to_ecliptic(v, self.perihelion, self.node, self.inclination);
        (rotate(position), rotate(velocity))
    }
}

/// Stumpff functions c₂(z) = (1 − cos √z)/z and c₃(z) = (√z − sin √z)/z^(3/2),
/// continued to z < 0 with cosh and sinh. Near 0, where the closed forms
/// lose digits, their power series c₂ = Σ (−z)^k/(2k+2)!, c₃ = Σ (−z)^k/(2k+3)!.
fn stumpff(z: f64) -> (f64, f64) {
    if z.abs() < 1.0 {
        // 12 terms: the next ones are below 1e-20.
        let (mut c2, mut c3) = (0.0, 0.0);
        let (mut term2, mut term3) = (0.5, 1.0 / 6.0);
        for k in 0..12 {
            c2 += term2;
            c3 += term3;
            let k = k as f64;
            term2 *= -z / ((2.0 * k + 3.0) * (2.0 * k + 4.0));
            term3 *= -z / ((2.0 * k + 4.0) * (2.0 * k + 5.0));
        }
        (c2, c3)
    } else if z > 0.0 {
        let s = z.sqrt();
        ((1.0 - s.cos()) / z, (s - s.sin()) / (z * s))
    } else {
        let s = (-z).sqrt();
        ((s.cosh() - 1.0) / -z, (s.sinh() - s) / (-z * s))
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
    /// A two-body state: the acceleration is the Sun's alone.
    pub fn two_body((position, velocity): ([f64; 3], [f64; 3]), jd_tt: f64) -> State {
        let d2: f64 = position.iter().map(|x| x * x).sum();
        let s = -GM_SUN / (d2 * d2.sqrt());
        State {
            jd_tt,
            position,
            velocity,
            acceleration: position.map(|x| s * x),
        }
    }

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
    integrate(elements.state(elements.epoch), elements.epoch, jd_tt)
}

/// Integrates a heliocentric state (ecliptic J2000 position, au, and
/// velocity, au/day, at `from`, TT) to `jd_tt` as [`propagate`] does.
pub fn integrate(start: ([f64; 3], [f64; 3]), from: f64, jd_tt: f64) -> Option<State> {
    let mut jd = from;
    let mut y = start;
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

    /// A test orbit with perihelion at JD 0, so that times keep a
    /// resolution far below a microsecond (near JD 2.46 million a double
    /// resolves only 40 µs, which is 1e-11 au at comet speeds).
    fn conic(e: f64, q: f64) -> Conic {
        Conic {
            perihelion_time: 0.0,
            perihelion_distance: q,
            eccentricity: e,
            perihelion: 75.0,
            node: 40.0,
            inclination: 20.0,
        }
    }

    fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
        (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
    }

    #[test]
    fn conic_matches_the_elliptic_elements() {
        // The same ellipse from perihelion form and from mean-anomaly form
        // (the asteroid path, checked against Meeus and JPL above), over
        // several revolutions in both directions.
        for &(e, q) in &[(0.0, 1.0), (0.1, 2.2), (0.5, 1.0), (0.9, 0.5), (0.99, 0.3)] {
            let c = conic(e, q);
            let a: f64 = q / (1.0 - e);
            let n = (GAUSS_K / a.powf(1.5)).to_degrees();
            let epoch = c.perihelion_time + 123.4;
            let el = Elements {
                epoch,
                mean_anomaly: n * (epoch - c.perihelion_time),
                perihelion: c.perihelion,
                node: c.node,
                inclination: c.inclination,
                eccentricity: e,
                mean_motion: n,
                semimajor_axis: a,
            };
            let period = 360.0 / n;
            for k in -12..=12 {
                let t = c.perihelion_time + k as f64 * 0.37 * period + 0.3;
                let (p, v) = c.state(t);
                let (p0, v0) = el.state(t);
                let scale = a.max(1.0);
                assert!(
                    distance(p, p0) < 1e-11 * scale,
                    "e={e} k={k}: {:e}",
                    distance(p, p0)
                );
                assert!(
                    distance(v, v0) < 1e-11 * scale,
                    "e={e} k={k}: {:e}",
                    distance(v, v0)
                );
            }
        }
    }

    #[test]
    fn parabola_matches_barker() {
        // Barker's equation (Meeus, Astronomical Algorithms, ch. 34):
        // W = 3k/√2 · (t − T)/q^1.5, s³ + 3s − W = 0 solved in closed form,
        // v = 2·atan s, r = q(1 + s²).
        let q = 0.7;
        let c = conic(1.0, q);
        for dt in [
            -3000.0, -400.0, -30.0, -1.0, -1e-3, 0.0, 0.5, 10.0, 90.0, 1000.0, 8000.0,
        ] {
            let w = 3.0 * GAUSS_K / 2f64.sqrt() * dt / q.powf(1.5);
            let y = (w / 2.0 + (w * w / 4.0 + 1.0).sqrt()).cbrt();
            let s = y - 1.0 / y;
            let nu = 2.0 * s.atan();
            let r = q * (1.0 + s * s);
            let plane = [r * nu.cos(), r * nu.sin(), 0.0];
            let expected = to_ecliptic(plane, c.perihelion, c.node, c.inclination);
            let (p, _) = c.state(c.perihelion_time + dt);
            // The closed form loses a few digits for large W (y − 1/y).
            assert!(
                distance(p, expected) < 1e-12 * r.max(1.0),
                "dt={dt}: {:e}",
                distance(p, expected)
            );
        }
    }

    #[test]
    fn hyperbola_matches_hyperbolic_kepler() {
        // e·sinh H − H = N with N = k·(t − T)/(−a)^1.5, solved by bisection
        // here (independent of the solver under test); x = |a|(e − cosh H),
        // y = |a|·√(e² − 1)·sinh H.
        for &(e, q) in &[(1.0005, 1.3), (1.05, 2.0), (1.5, 0.8), (3.0, 1.0)] {
            let c = conic(e, q);
            let big_a = q / (e - 1.0);
            for dt in [-2000.0, -100.0, -2.0, 0.0, 3.0, 50.0, 700.0, 5000.0] {
                let n = GAUSS_K * dt / big_a.powf(1.5);
                let (mut lo, mut hi) = (-60.0f64, 60.0f64);
                for _ in 0..200 {
                    let mid = 0.5 * (lo + hi);
                    if e * mid.sinh() - mid < n {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let h = 0.5 * (lo + hi);
                let plane = [
                    big_a * (e - h.cosh()),
                    big_a * (e * e - 1.0).sqrt() * h.sinh(),
                    0.0,
                ];
                let expected = to_ecliptic(plane, c.perihelion, c.node, c.inclination);
                let (p, _) = c.state(c.perihelion_time + dt);
                let r = distance(expected, [0.0; 3]);
                assert!(
                    distance(p, expected) < 1e-11 * r.max(1.0),
                    "e={e} dt={dt}: {:e}",
                    distance(p, expected)
                );
            }
        }
    }

    #[test]
    fn conic_is_continuous_and_consistent_through_e_1() {
        // Energy v²/2 − μ/r = −μ(1 − e)/(2q) and angular momentum
        // |r × v| = √(μq(1 + e)) hold everywhere; velocity is the
        // derivative of position; and e just below, at and just above 1
        // give nearly the same path.
        let times = [-900.0, -60.0, -0.5, 0.0, 0.25, 7.0, 365.0, 2500.0];
        for &e in &[0.999_999, 0.9999999999, 1.0, 1.0000000001, 1.000_001] {
            let c = conic(e, 0.4);
            for dt in times {
                let t = c.perihelion_time + dt;
                let (p, v) = c.state(t);
                let r = distance(p, [0.0; 3]);
                let v2: f64 = v.iter().map(|x| x * x).sum();
                let energy = v2 / 2.0 - GM_SUN / r;
                let expected = -GM_SUN * (1.0 - e) / (2.0 * 0.4);
                assert!(
                    (energy - expected).abs() < 1e-14,
                    "e={e} dt={dt}: {energy:e} vs {expected:e}"
                );
                let h = [
                    p[1] * v[2] - p[2] * v[1],
                    p[2] * v[0] - p[0] * v[2],
                    p[0] * v[1] - p[1] * v[0],
                ];
                let h_expected = (GM_SUN * 0.4 * (1.0 + e)).sqrt();
                assert!((distance(h, [0.0; 3]) - h_expected).abs() < 1e-13);
                let step = 1e-3;
                let (p1, p0) = (c.state(t + step).0, c.state(t - step).0);
                for k in 0..3 {
                    let fd = (p1[k] - p0[k]) / (2.0 * step);
                    assert!((fd - v[k]).abs() < 1e-9, "e={e} dt={dt}: {fd} vs {}", v[k]);
                }
                let parabola = conic(1.0, 0.4).state(t).0;
                assert!(distance(p, parabola) < 1e-4 * r.max(1.0), "e={e} dt={dt}");
            }
        }
    }

    #[test]
    fn integrate_starts_from_any_state() {
        // A comet-like ellipse integrated from its perihelion-form state is
        // the asteroid path's integration of the same orbit.
        let c = Conic {
            perihelion_time: 2_461_000.5,
            ..conic(0.6, 1.1)
        };
        let a: f64 = 1.1 / 0.4;
        let n = (GAUSS_K / a.powf(1.5)).to_degrees();
        let epoch = c.perihelion_time + 40.0;
        let el = Elements {
            epoch,
            mean_anomaly: n * 40.0,
            perihelion: c.perihelion,
            node: c.node,
            inclination: c.inclination,
            eccentricity: 0.6,
            mean_motion: n,
            semimajor_axis: a,
        };
        let t = epoch + 200.0;
        let ours = integrate(c.state(epoch), epoch, t).unwrap();
        let theirs = propagate(&el, t).unwrap();
        assert!(
            distance(ours.position, theirs.position) < 1e-10,
            "{:e}",
            distance(ours.position, theirs.position)
        );
        // Perturbations are included: the planets move it off the conic.
        assert!(distance(ours.position, c.state(t).0) > 1e-7);
        // The two-body state's acceleration is the Sun's pull alone.
        let s = State::two_body(c.state(t), t);
        let r = distance(s.position, [0.0; 3]);
        let g = distance(s.acceleration, [0.0; 3]);
        assert!((g - GM_SUN / (r * r)).abs() < 1e-18);
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
