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
