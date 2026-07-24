//! Shared geometry / legality predicates for NA planners.
//!
//! # Invariants
//!
//! - R1: partner distance must be `≤ rydberg_range_um` (reject `>`).
//! - R2: non-partner distance must be `> rydberg_range_um` (reject `≤`).
//! - R3: non-partner distance must be `> min_rydberg_spacing_um` (reject `≤`).
//! - Scope is **all occupied atoms** — no spectator parking workaround.
//! - Predicates are dialect-identical `≤` reject rules.
//!
//! Both [`crate::movement`] and [`crate::compaction`] call these predicates
//! so R1–R3 never diverge between the flat and compaction planners.

use backend::AodSpeedModel as BackendSpeedModel;
use backend::AodSpeedModelKind as BackendSpeedModelKind;
use serde::{Deserialize, Serialize};

use crate::layout::Position;

/// Euclidean distance between two positions (µm).
pub fn euclidean_um(a: Position, b: Position) -> f64 {
    let dx = a.x_um - b.x_um;
    let dy = a.y_um - b.y_um;
    (dx * dx + dy * dy).sqrt()
}

/// Move duration in µs: `t = √(d / a)` with `d` in metres, `a` in m/s², result ×1e6.
pub fn movement_duration_us(d_max_um: f64, acceleration_m_s2: f64) -> u64 {
    if d_max_um <= 0.0 || acceleration_m_s2 <= 0.0 {
        return 0;
    }
    let d_m = d_max_um * 1e-6;
    let t_s = (d_m / acceleration_m_s2).sqrt();
    (t_s * 1e6).ceil() as u64
}

/// Speed-model kind for movement timing (mirrors `backend::AodSpeedModelKind`;
/// kept crate-local + serde so it can live on the serde-derived
/// [`crate::movement::MovementParams`] / [`crate::zoned::ZonedArchitecture`]
/// without embedding the backend's non-serde domain type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeedModelKind {
    /// √-law `t = √(d/a)` (default; the [RAP] Table I timing model).
    #[default]
    Sqrt,
    /// Jerk-limited symmetric S-curve (issue #308); see [`movement_duration_us_jerk`].
    JerkLimited,
}

/// Movement timing model (mirrors `backend::AodSpeedModel`). The jerk-limited
/// parameters default to `0.0` (unused under [`SpeedModelKind::Sqrt`]); see the
/// kind variants for the per-kind contract. Provenance per architecture_model.md
/// §8.6: jerk/cruise values are target-specific, not universal constants.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedModel {
    pub kind: SpeedModelKind,
    pub acceleration_m_s2: f64,
    #[serde(default)]
    pub jerk_m_s3: f64,
    #[serde(default)]
    pub max_velocity_m_s: f64,
}

impl Default for SpeedModel {
    fn default() -> Self {
        Self {
            kind: SpeedModelKind::Sqrt,
            acceleration_m_s2: 0.0,
            jerk_m_s3: 0.0,
            max_velocity_m_s: 0.0,
        }
    }
}

impl From<&BackendSpeedModel> for SpeedModel {
    fn from(b: &BackendSpeedModel) -> Self {
        Self {
            kind: match b.kind {
                BackendSpeedModelKind::Sqrt => SpeedModelKind::Sqrt,
                BackendSpeedModelKind::JerkLimited => SpeedModelKind::JerkLimited,
            },
            acceleration_m_s2: b.acceleration_m_s2,
            jerk_m_s3: b.jerk_m_s3,
            max_velocity_m_s: b.max_velocity_m_s,
        }
    }
}

/// Move duration (µs) under the jerk-limited symmetric S-curve model (#308).
///
/// Rest-to-rest point-to-point over distance `d_max_um` (µm) with jerk limit
/// `J` (`jerk_m_s3`, m/s³), acceleration cap `a` (`acceleration_m_s2`, m/s²),
/// and optional cruise-velocity cap `v` (`max_velocity_m_s`, m/s; `0.0`
/// disables cruising). The profile is a symmetric double-S (jerk-shaped) curve;
/// the binding constraint picks one of three continuous regimes:
///
/// - **Triangular S-curve** (`d < d_AB = 2a³/J²`): acceleration never reaches
///   `a` (peak accel `a_p = (d·J²/2)^(1/3)`); 4 jerk phases;
///   `t = 4·(d/(2J))^(1/3)`.
/// - **Trapezoidal-acceleration S-curve** (`d_AB ≤ d < d_v`, or no cruise cap):
///   acceleration reaches `a` then a constant-acceleration phase of duration
///   `t_a` runs until peak velocity `a²/J + a·t_a` (still under `v`);
///   `t = 4a/J + 2·t_a`, where `t_a` solves
///   `d/2 = a³/J² + (3a²/(2J))·t_a + ½·a·t_a²`.
/// - **Trapezoidal (cruise)** (`d ≥ d_v`, requires `v > a²/J`): the profile
///   reaches cruise velocity `v`, cruises for `(d − d_v)/v`, then decelerates
///   symmetrically; `t = 2·(2a/J + t₁) + (d − d_v)/v` with
///   `t₁ = (v − a²/J)/a` and `d_v = 2·(a³/J² + (3a²/(2J))·t₁ + ½·a·t₁²)`.
///
/// The regimes are continuous at `d_AB` and `d_v` (validated against numerical
/// integration of the constructed acceleration profile in the unit tests).
/// Returns µs (`ceil`), and `0` for non-positive distance or non-positive
/// `a`/`J`.
///
/// **Provenance:** standard symmetric S-curve derivation. The jerk and
/// cruise-velocity values are target-specific placeholders — calibrating to
/// the QMAP eval scripts' jerk-limited model (which coincides with the √-law
/// only at d = 110 µm; see `docs/neutral_atom/literature_notes.md`'s [RAP]
/// caveats) is out of scope for #308; the default model stays [`SpeedModelKind::Sqrt`].
pub fn movement_duration_us_jerk(
    d_max_um: f64,
    jerk_m_s3: f64,
    max_velocity_m_s: f64,
    acceleration_m_s2: f64,
) -> u64 {
    if d_max_um <= 0.0 || acceleration_m_s2 <= 0.0 || jerk_m_s3 <= 0.0 {
        return 0;
    }
    let t_s = jerk_limited_time_s(
        d_max_um * 1e-6,
        jerk_m_s3,
        acceleration_m_s2,
        max_velocity_m_s,
    );
    (t_s * 1e6).ceil() as u64
}

/// Symmetric S-curve rest-to-rest time (seconds) for distance `d` (m) with
/// jerk `j` (m/s³), accel cap `a` (m/s²), cruise cap `v` (m/s; `≤0` disables).
fn jerk_limited_time_s(d: f64, j: f64, a: f64, v: f64) -> f64 {
    // Boundary where peak accel just reaches a (triangular → trapezoidal-accel).
    let d_ab = 2.0 * a.powi(3) / (j * j);
    if d < d_ab {
        // Triangular S-curve: peak accel a_p < a, 4 jerk phases.
        let ap = (d * j * j / 2.0).cbrt();
        return 4.0 * ap / j;
    }
    // Trapezoidal-acceleration: solve the half-distance quadratic for t_a.
    //   0.5·a·t_a^2 + (3a²/(2J))·t_a + (a³/J² − d/2) = 0
    let coef_a = 0.5 * a;
    let coef_b = 1.5 * a * a / j;
    let coef_c = a.powi(3) / (j * j) - d / 2.0;
    let disc = (coef_b * coef_b - 4.0 * coef_a * coef_c).max(0.0);
    let ta_unbounded = (-coef_b + disc.sqrt()) / (2.0 * coef_a);
    let vpnc = a * a / j; // peak velocity when accel just reaches a, no cruise
    let cruise = v > 0.0 && v > vpnc;
    if !cruise {
        return 4.0 * a / j + 2.0 * ta_unbounded;
    }
    let vp_unbounded = a * a / j + a * ta_unbounded;
    if vp_unbounded <= v {
        // Cruise cap not reached: trapezoidal-acceleration.
        return 4.0 * a / j + 2.0 * ta_unbounded;
    }
    // Trapezoidal (cruise): reach v, cruise, decelerate symmetrically.
    let t1 = (v - a * a / j) / a;
    let halfv = a.powi(3) / (j * j) + (1.5 * a * a / j) * t1 + 0.5 * a * t1 * t1;
    let d_v = 2.0 * halfv;
    let t_ramp = 2.0 * a / j + t1;
    2.0 * t_ramp + (d - d_v) / v
}

/// Dispatch: move duration (µs) under the given [`SpeedModel`]. The √-law path
/// is [`movement_duration_us`]; the jerk-limited path is
/// [`movement_duration_us_jerk`]. Placement argmin still uses [`sqrt_d_max`]
/// (model-independent), so switching the model changes only durations/aggregates
/// — not placement or schedule structure (issue #308 acceptance).
pub fn movement_duration_for_model(d_max_um: f64, model: &SpeedModel) -> u64 {
    match model.kind {
        SpeedModelKind::Sqrt => movement_duration_us(d_max_um, model.acceleration_m_s2),
        SpeedModelKind::JerkLimited => movement_duration_us_jerk(
            d_max_um,
            model.jerk_m_s3,
            model.max_velocity_m_s,
            model.acceleration_m_s2,
        ),
    }
}

/// `√(d_max / a)` duration contribution for one movement group ([RAP] Eq. (1)).
///
/// `d_max_um` is micrometres; `acceleration_m_s2` is m/s². Returns seconds×1e6
/// scaled consistently as a dimensionless `√(µm)` proxy when `a` is fixed — we
/// return `sqrt(d_max_um)` when comparing placements at fixed `a` (Eq. (1)
/// factor `1/√a` cancels in argmin). For absolute time use
/// [`movement_duration_us`].
pub fn sqrt_d_max(d_max_um: f64) -> f64 {
    if d_max_um <= 0.0 {
        0.0
    } else {
        d_max_um.sqrt()
    }
}

/// Sum of `√(d_max)` over groups — [RAP] Eq. (1) (up to `1/√a`).
pub fn routing_cost_eq1(group_d_max_um: &[f64]) -> f64 {
    group_d_max_um.iter().map(|&d| sqrt_d_max(d)).sum()
}

/// Canonical ordering for an unordered pair of atom ids (used by both movement
/// and compaction to key partner-pair sets).
pub fn ordered_pair(
    a: crate::layout::AtomId,
    b: crate::layout::AtomId,
) -> (crate::layout::AtomId, crate::layout::AtomId) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Numerically integrate the constructed symmetric S-curve acceleration
    /// profile and assert it covers exactly `d` metres in the returned time —
    /// the analytic [`jerk_limited_time_s`] must match this independent check.
    fn simulate(d: f64, j: f64, a: f64, v: f64, n: usize) -> (f64, f64) {
        let tj = a / j;
        let d_ab = 2.0 * a.powi(3) / (j * j);
        let (t, accel): (f64, Box<dyn Fn(f64) -> f64>) = if d < d_ab {
            let ap = (d * j * j / 2.0).cbrt();
            let tp = ap / j;
            let t = 4.0 * tp;
            let f = move |t_: f64| {
                if t_ < tp {
                    j * t_
                } else if t_ < 2.0 * tp {
                    ap - j * (t_ - tp)
                } else if t_ < 3.0 * tp {
                    -j * (t_ - 2.0 * tp)
                } else {
                    -ap + j * (t_ - 3.0 * tp)
                }
            };
            (t, Box::new(f))
        } else {
            let coef_a = 0.5 * a;
            let coef_b = 1.5 * a * a / j;
            let coef_c = a.powi(3) / (j * j) - d / 2.0;
            let disc = (coef_b * coef_b - 4.0 * coef_a * coef_c).max(0.0);
            let ta_unb = (-coef_b + disc.sqrt()) / (2.0 * coef_a);
            let vpnc = a * a / j;
            let cruise = v > 0.0 && v > vpnc;
            let vp_unb = a * a / j + a * ta_unb;
            if !cruise || vp_unb <= v {
                let ta = ta_unb;
                let t = 4.0 * a / j + 2.0 * ta;
                let t_ramp = 2.0 * tj + ta;
                let f = move |t_: f64| {
                    if t_ < tj {
                        j * t_
                    } else if t_ < tj + ta {
                        a
                    } else if t_ < 2.0 * tj + ta {
                        a - j * (t_ - tj - ta)
                    } else if t_ < 3.0 * tj + ta {
                        -j * (t_ - 2.0 * tj - ta)
                    } else if t_ < 3.0 * tj + 2.0 * ta {
                        -a
                    } else {
                        -a + j * (t_ - 3.0 * tj - 2.0 * ta)
                    }
                };
                let _ = t_ramp;
                (t, Box::new(f))
            } else {
                let t1 = (v - a * a / j) / a;
                let halfv = a.powi(3) / (j * j) + (1.5 * a * a / j) * t1 + 0.5 * a * t1 * t1;
                let d_v = 2.0 * halfv;
                let tc = (d - d_v) / v;
                let t_ramp = 2.0 * tj + t1;
                let t = 2.0 * t_ramp + tc;
                let f = move |t_: f64| {
                    if t_ < tj {
                        j * t_
                    } else if t_ < tj + t1 {
                        a
                    } else if t_ < 2.0 * tj + t1 {
                        a - j * (t_ - tj - t1)
                    } else if t_ < 2.0 * tj + t1 + tc {
                        0.0
                    } else if t_ < 3.0 * tj + t1 + tc {
                        -j * (t_ - 2.0 * tj - t1 - tc)
                    } else if t_ < 3.0 * tj + 2.0 * t1 + tc {
                        -a
                    } else {
                        -a + j * (t_ - 3.0 * tj - 2.0 * t1 - tc)
                    }
                };
                (t, Box::new(f))
            }
        };
        let h = t / n as f64;
        let mut vel = 0.0;
        let mut pos = 0.0;
        for i in 0..n {
            vel += accel(i as f64 * h) * h;
            pos += vel * h;
        }
        (t, pos)
    }

    #[test]
    fn jerk_duration_matches_numerical_integration_across_regimes() {
        // (d_um, J, v, label) — a=2750 throughout, exercising triangular,
        // trapezoidal-accel, and cruise regimes plus both A/B and B/C boundaries.
        let a = 2750.0;
        let cases: &[(f64, f64, f64, &str)] = &[
            (0.5e-6, 1e8, 0.5, "triangular"),
            (4.0e-6, 1e8, 0.5, "near A/B low"),
            (4.159375e-6, 1e8, 0.5, "A/B boundary"),
            (4.2e-6, 1e8, 0.5, "just past A/B"),
            (10e-6, 1e8, 0.5, "trapezoidal-accel"),
            (110e-6, 1e8, 0.5, "RAP anchor distance"),
            (6.38636e-6, 1e8, 0.1, "B/C boundary"),
            (6.4e-6, 1e8, 0.1, "just past B/C"),
            (50e-6, 1e8, 0.1, "cruise"),
            (420e-6, 1e8, 0.1, "cruise long"),
            (110e-6, 1e8, 0.0, "no cruise cap"),
        ];
        for &(d_um, j, v, label) in cases {
            let d = d_um * 1e-6;
            let t_an = jerk_limited_time_s(d, j, a, v);
            let (t_sim, pos_sim) = simulate(d, j, a, v, 400_000);
            assert!(
                (t_an - t_sim).abs() < 1e-9,
                "{label}: analytic time {t_an} != simulated {t_sim}"
            );
            assert!(
                (pos_sim - d).abs() < 1e-7,
                "{label}: simulated distance {pos_sim} != target {d}"
            );
        }
    }

    #[test]
    fn jerk_duration_is_continuous_at_regime_boundaries() {
        let a: f64 = 2750.0;
        let j: f64 = 1e8;
        // A/B boundary (peak accel reaches a): the triangular-S formula evaluated
        // at d_ab (4·a_p/J with a_p = a) must equal the trapezoidal-accel value the
        // dispatcher returns at d_ab (4a/J + 2·t_a with t_a = 0).
        let d_ab = 2.0 * a.powi(3) / (j * j);
        let t_regime_a_at_boundary = 4.0 * (d_ab * j * j / 2.0).cbrt() / j;
        let t_at_boundary = jerk_limited_time_s(d_ab, j, a, 0.5);
        assert!(
            (t_at_boundary - t_regime_a_at_boundary).abs() < 1e-12,
            "A/B boundary: regime A {t_regime_a_at_boundary} != dispatcher {t_at_boundary}"
        );
        // Monotonic across the join (small neighborhood, loose tolerance).
        let below = jerk_limited_time_s(d_ab * 0.999, j, a, 0.5);
        let above = jerk_limited_time_s(d_ab * 1.001, j, a, 0.5);
        assert!(
            below < t_at_boundary && t_at_boundary < above,
            "A/B not monotonic"
        );
        // B/C boundary (cruise velocity just reached), v=0.1.
        let v = 0.1;
        let t1 = (v - a * a / j) / a;
        let halfv = a.powi(3) / (j * j) + (1.5 * a * a / j) * t1 + 0.5 * a * t1 * t1;
        let d_v = 2.0 * halfv;
        // Trapezoidal-accel formula at d_v (t_a = t1) == cruise formula at d_v
        // (2·(2a/J + t1) + 0 cruise).
        let t_regime_b_at_boundary = 4.0 * a / j + 2.0 * t1;
        let t_at_v = jerk_limited_time_s(d_v, j, a, v);
        assert!(
            (t_at_v - t_regime_b_at_boundary).abs() < 1e-12,
            "B/C boundary: regime B {t_regime_b_at_boundary} != dispatcher {t_at_v}"
        );
        let below = jerk_limited_time_s(d_v * 0.999, j, a, v);
        let above = jerk_limited_time_s(d_v * 1.001, j, a, v);
        assert!(below < t_at_v && t_at_v < above, "B/C not monotonic");
    }

    #[test]
    fn jerk_duration_is_monotonic_and_positive() {
        let a = 2750.0;
        let j = 1e8;
        let mut prev = 0u64;
        for d_um in [1.0, 2.0, 4.0, 8.0, 16.0, 50.0, 110.0, 200.0, 420.0] {
            let t = movement_duration_us_jerk(d_um, j, 0.5, a);
            assert!(t > 0, "d={d_um}um: duration must be positive, got {t}");
            assert!(
                t >= prev,
                "d={d_um}um: duration {t} < prev {prev} (not monotonic)"
            );
            prev = t;
        }
    }

    #[test]
    fn jerk_duration_differs_from_sqrt_and_dispatch_matches() {
        let a = 2750.0;
        let sqrt_model = SpeedModel {
            kind: SpeedModelKind::Sqrt,
            acceleration_m_s2: a,
            jerk_m_s3: 0.0,
            max_velocity_m_s: 0.0,
        };
        let jerk_model = SpeedModel {
            kind: SpeedModelKind::JerkLimited,
            acceleration_m_s2: a,
            jerk_m_s3: 1e8,
            max_velocity_m_s: 0.5,
        };
        // Dispatch reproduces the direct sqrt/jerk helpers.
        for d_um in [10.0, 110.0, 420.0] {
            assert_eq!(
                movement_duration_for_model(d_um, &sqrt_model),
                movement_duration_us(d_um, a),
            );
            assert_eq!(
                movement_duration_for_model(d_um, &jerk_model),
                movement_duration_us_jerk(d_um, 1e8, 0.5, a),
            );
        }
        // The two models give strictly different (both finite, positive) durations
        // for every representative distance — the acceptance invariant.
        for d_um in [10.0, 50.0, 110.0, 200.0, 420.0] {
            let s = movement_duration_for_model(d_um, &sqrt_model);
            let k = movement_duration_for_model(d_um, &jerk_model);
            assert!(
                s > 0 && k > 0 && s != k,
                "d={d_um}um sqrt={s} jerk={k} (must differ)"
            );
        }
    }

    #[test]
    fn jerk_duration_zero_for_nonpositive_inputs() {
        assert_eq!(movement_duration_us_jerk(0.0, 1e8, 0.5, 2750.0), 0);
        assert_eq!(movement_duration_us_jerk(110.0, 0.0, 0.5, 2750.0), 0);
        assert_eq!(movement_duration_us_jerk(110.0, 1e8, 0.5, 0.0), 0);
        assert_eq!(movement_duration_us_jerk(-1.0, 1e8, 0.5, 2750.0), 0);
    }

    #[test]
    fn speed_model_round_trips_from_backend() {
        let b = BackendSpeedModel {
            kind: BackendSpeedModelKind::JerkLimited,
            acceleration_m_s2: 2750.0,
            jerk_m_s3: 1e8,
            max_velocity_m_s: 0.5,
        };
        let m = SpeedModel::from(&b);
        assert_eq!(m.kind, SpeedModelKind::JerkLimited);
        assert_eq!(m.acceleration_m_s2, 2750.0);
        assert_eq!(m.jerk_m_s3, 1e8);
        assert_eq!(m.max_velocity_m_s, 0.5);
        // sqrt backend model -> sqrt speed model.
        let b2 = BackendSpeedModel {
            kind: BackendSpeedModelKind::Sqrt,
            acceleration_m_s2: 2750.0,
            jerk_m_s3: 0.0,
            max_velocity_m_s: 0.0,
        };
        assert_eq!(SpeedModel::from(&b2).kind, SpeedModelKind::Sqrt);
    }
}
