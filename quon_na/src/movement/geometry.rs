//! R1–R3 entangling-geometry predicates and AOD legality checks (B11/B12).
//!
//! # Invariants
//!
//! - R1: partner distance must be `≤ rydberg_range_um` (reject `>`).
//! - R2: non-partner distance must be `> rydberg_range_um` (reject `≤`).
//! - R3: non-partner distance must be `> min_rydberg_spacing_um` (reject `≤`).
//! - Scope = **all occupied atoms** (including dense #104 idles). No spectator
//!   parking workaround — dense multi-atom grids with idle–idle R2/R3
//!   violations fail [`MovementPlanError::EntanglingGeometry`].
//! - Predicates are dialect-identical `≤` reject rules.
//! - AOD legality: coupling (same row/col ⇒ same Δ), order preservation, M3
//!   destination separation (strict `<` on dest delta).

use std::collections::{BTreeMap, BTreeSet};

use crate::geometry::{euclidean_um, ordered_pair};
use crate::layout::{AtomId, Position, SiteId};
use crate::movement::types::{CandidateLeg, MoveSpec, MovementPlanError};

/// R1–R3 check with dialect-identical `≤` predicates over **all occupied** atoms (B11).
pub fn check_entangling_geometry(
    cycle: u32,
    partner_gates: &[(AtomId, AtomId)],
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
    params: &crate::movement::types::MovementParams,
) -> Result<(), MovementPlanError> {
    let mut atoms: Vec<(AtomId, Position)> = Vec::new();
    for (&atom, &site) in occ_atom {
        let pos = site_pos
            .get(&site)
            .copied()
            .ok_or(MovementPlanError::MissingSite(site))?;
        atoms.push((atom, pos));
    }
    atoms.sort_by_key(|(a, _)| *a);
    verify_entangling_geometry_predicates(
        cycle,
        partner_gates,
        &atoms,
        params.rydberg_range_um,
        params.min_rydberg_spacing_um,
    )
}

/// Dialect-identical R1–R3 predicates (`dialect.rs` `verify_entangling_geometry`).
///
/// Rejects partner distance `>` `rydberg_range_um` (R1) and non-partner distance
/// `≤` `rydberg_range_um` (R2) / `≤` `min_rydberg_spacing_um` (R3). The MLIR dialect
/// path only feeds entangle-layer atoms; the planner passes **all occupied** atoms
/// via [`check_entangling_geometry`].
pub fn verify_entangling_geometry_predicates(
    cycle: u32,
    partner_gates: &[(AtomId, AtomId)],
    atoms: &[(AtomId, Position)],
    rydberg_range_um: f64,
    min_rydberg_spacing_um: f64,
) -> Result<(), MovementPlanError> {
    let partner_pairs: BTreeSet<(AtomId, AtomId)> = partner_gates
        .iter()
        .map(|&(a, b)| ordered_pair(a, b))
        .collect();

    let pos_of_atom = |id: AtomId| -> Result<Position, MovementPlanError> {
        atoms
            .iter()
            .find(|(a, _)| *a == id)
            .map(|(_, p)| *p)
            .ok_or(MovementPlanError::MissingAtom(id))
    };

    for &(a, b) in partner_gates {
        let pa = pos_of_atom(a)?;
        let pb = pos_of_atom(b)?;
        let d = euclidean_um(pa, pb);
        if d > rydberg_range_um {
            return Err(MovementPlanError::EntanglingGeometry {
                cycle,
                detail: format!(
                    "R1: partners {:?}–{:?} distance {d} > rydberg_range_um {rydberg_range_um}",
                    a, b
                ),
            });
        }
    }

    for i in 0..atoms.len() {
        for j in (i + 1)..atoms.len() {
            let (a, pa) = atoms[i];
            let (b, pb) = atoms[j];
            if partner_pairs.contains(&ordered_pair(a, b)) {
                continue;
            }
            let d = euclidean_um(pa, pb);
            if d <= rydberg_range_um {
                return Err(MovementPlanError::EntanglingGeometry {
                    cycle,
                    detail: format!(
                        "R2: non-partners {:?}–{:?} distance {d} ≤ rydberg_range_um {rydberg_range_um}",
                        a, b
                    ),
                });
            }
            if d <= min_rydberg_spacing_um {
                return Err(MovementPlanError::EntanglingGeometry {
                    cycle,
                    detail: format!(
                        "R3: non-partners {:?}–{:?} distance {d} ≤ min_rydberg_spacing_um {min_rydberg_spacing_um}",
                        a, b
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Pure AOD legality check matching dialect `verify_aod_legality` (coupling +
/// order + M3 separation with strict `<` on dest delta).
pub fn verify_aod_legality(
    cycle: u32,
    moves: &[MoveSpec],
    min_separation_um: f64,
) -> Result<(), MovementPlanError> {
    verify_coupled_motion(cycle, moves)?;
    verify_axis_order_and_separation(cycle, moves, Axis::Row, min_separation_um)?;
    verify_axis_order_and_separation(cycle, moves, Axis::Column, min_separation_um)?;
    Ok(())
}

/// Whether two candidate legs conflict under Enola three types + M3 separation (B12).
pub fn legs_conflict(
    a: &crate::movement::types::CandidateLeg,
    b: &crate::movement::types::CandidateLeg,
    min_sep_um: f64,
) -> bool {
    if a.atom == b.atom {
        return true;
    }
    if a.to == b.to {
        return true;
    }
    if a.aod_id != b.aod_id {
        return false;
    }
    // Capacity is checked at pack time; per-pair conflicts:
    axis_conflict(a, b, Axis::Row, min_sep_um) || axis_conflict(a, b, Axis::Column, min_sep_um)
}

// ---------------------------------------------------------------------------
// AOD axis helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) enum Axis {
    Row,
    Column,
}

pub(crate) const POS_EPS_UM: f64 = 1e-6;

fn axis_conflict(
    a: &crate::movement::types::CandidateLeg,
    b: &crate::movement::types::CandidateLeg,
    axis: Axis,
    min_sep_um: f64,
) -> bool {
    let (a_from, a_to, a_idx) = match axis {
        Axis::Row => (a.from_pos.y_um, a.to_pos.y_um, a.row),
        Axis::Column => (a.from_pos.x_um, a.to_pos.x_um, a.col),
    };
    let (b_from, b_to, b_idx) = match axis {
        Axis::Row => (b.from_pos.y_um, b.to_pos.y_um, b.row),
        Axis::Column => (b.from_pos.x_um, b.to_pos.x_um, b.col),
    };

    // Same source row ⇒ shared destination row (coupling / no split)
    if a_idx == b_idx {
        // Same AOD index on this axis: destinations must match for coupling via Δ.
        // Enola: same source row ⇒ same dest row. Using index equality as shared row.
        if a_to.total_cmp(&b_to) != std::cmp::Ordering::Equal && (a_to - b_to).abs() > POS_EPS_UM {
            // Also check coordinate-based same source row
        }
    }

    let same_src_row = a_from.total_cmp(&b_from) == std::cmp::Ordering::Equal
        || (a_from - b_from).abs() <= POS_EPS_UM;
    let same_dst_row =
        a_to.total_cmp(&b_to) == std::cmp::Ordering::Equal || (a_to - b_to).abs() <= POS_EPS_UM;

    // Type 1: same source row ⇒ must share destination row
    if same_src_row && !same_dst_row {
        return true;
    }
    // Type 2: same destination row ⇒ must share source row
    if same_dst_row && !same_src_row {
        return true;
    }
    // Type 3: order preservation (match verifier total_cmp)
    let source_order = a_from.total_cmp(&b_from);
    let dest_order = a_to.total_cmp(&b_to);
    if source_order != dest_order && source_order != std::cmp::Ordering::Equal {
        // Verifier: if source_order != dest_order → conflict (including Equal dest with unequal src)
        return true;
    }
    if source_order != dest_order {
        return true;
    }

    // M3 dest separation: |dst_i - dst_j| < min (strict)
    // Only when distinct dest coordinates (same dest already site-conflicted).
    if !same_dst_row && (a_to - b_to).abs() < min_sep_um {
        return true;
    }

    // Coupling via shared AOD row/col index with different Δ
    if a_idx == b_idx {
        let a_delta = a_to - a_from;
        let b_delta = b_to - b_from;
        if (a_delta - b_delta).abs() > f64::EPSILON {
            return true;
        }
    }

    false
}

fn verify_coupled_motion(cycle: u32, moves: &[MoveSpec]) -> Result<(), MovementPlanError> {
    let mut row_deltas: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    let mut col_deltas: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    for m in moves {
        let row_key = (m.aod_id, m.row);
        let row_delta = m.to_y_um - m.from_y_um;
        if let Some(existing) = row_deltas.insert(row_key, row_delta)
            && (existing - row_delta).abs() > f64::EPSILON
        {
            return Err(MovementPlanError::Conflict(format!(
                "AOD row coupling at cycle {cycle} aod {} row {}",
                m.aod_id, m.row
            )));
        }
        let col_key = (m.aod_id, m.col);
        let col_delta = m.to_x_um - m.from_x_um;
        if let Some(existing) = col_deltas.insert(col_key, col_delta)
            && (existing - col_delta).abs() > f64::EPSILON
        {
            return Err(MovementPlanError::Conflict(format!(
                "AOD column coupling at cycle {cycle} aod {} col {}",
                m.aod_id, m.col
            )));
        }
    }
    Ok(())
}

fn verify_axis_order_and_separation(
    cycle: u32,
    moves: &[MoveSpec],
    axis: Axis,
    min_separation_um: f64,
) -> Result<(), MovementPlanError> {
    let mut representatives: BTreeMap<(u32, u32), (f64, f64)> = BTreeMap::new();
    for m in moves {
        let key = match axis {
            Axis::Row => (m.aod_id, m.row),
            Axis::Column => (m.aod_id, m.col),
        };
        let coordinates = match axis {
            Axis::Row => (m.from_y_um, m.to_y_um),
            Axis::Column => (m.from_x_um, m.to_x_um),
        };
        representatives.entry(key).or_insert(coordinates);
    }
    let items: Vec<_> = representatives.into_iter().collect();
    for i in 0..items.len() {
        for ((rhs_aod, _), (rhs_from, rhs_to)) in items.iter().skip(i + 1) {
            let ((lhs_aod, _), (lhs_from, lhs_to)) = items[i];
            if lhs_aod != *rhs_aod {
                continue;
            }
            let source_order = lhs_from.total_cmp(rhs_from);
            let dest_order = lhs_to.total_cmp(rhs_to);
            if source_order != dest_order {
                return Err(MovementPlanError::Conflict(format!(
                    "AOD order preservation at cycle {cycle}"
                )));
            }
            if (lhs_to - rhs_to).abs() < min_separation_um {
                return Err(MovementPlanError::Conflict(format!(
                    "AOD M3 separation at cycle {cycle}: dest delta {} < {min_separation_um}",
                    (lhs_to - rhs_to).abs()
                )));
            }
        }
    }
    Ok(())
}
