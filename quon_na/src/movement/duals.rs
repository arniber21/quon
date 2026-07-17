//! Dual generation, sortIS selection, and greedy leg packing (B7/B9).
//!
//! # Invariants
//!
//! - Duals are both-atom orientations onto a free interaction pair (Quon,
//!   not Enola one-atom duals).
//! - B7 packing serializes legs from the same dual across different pack
//!   rounds (force-split via `dual_id` exclusion).
//! - Greedy longest-first IS selection (sortIS spirit — not KaMIS).

use std::collections::BTreeSet;

use crate::geometry::euclidean_um;
use crate::layout::{AtomId, Position, SiteId};
use crate::movement::bank::{OrderedF64, layout_from_sites, site_position_map};
use crate::movement::geometry::legs_conflict;
use crate::movement::types::{CandidateLeg, InteractionPair, MovementParams, MovementPlanError};

#[derive(Clone, Debug)]
pub(crate) struct DualCandidate {
    pub gate: (AtomId, AtomId),
    pub pair: InteractionPair,
    pub legs: Vec<CandidateLeg>,
    pub distance_um: f64,
}

pub(crate) struct AodGrid {
    site_row_col: std::collections::BTreeMap<SiteId, (u32, u32)>,
}

impl AodGrid {
    pub(crate) fn from_layout(layout: &crate::layout::NeutralAtomLayout) -> Self {
        let mut ys: BTreeSet<OrderedF64> = BTreeSet::new();
        let mut xs: BTreeSet<OrderedF64> = BTreeSet::new();
        for s in &layout.sites {
            ys.insert(OrderedF64(s.position.y_um));
            xs.insert(OrderedF64(s.position.x_um));
        }
        let row_of_y: std::collections::BTreeMap<OrderedF64, u32> =
            ys.iter().enumerate().map(|(i, y)| (*y, i as u32)).collect();
        let col_of_x: std::collections::BTreeMap<OrderedF64, u32> =
            xs.iter().enumerate().map(|(i, x)| (*x, i as u32)).collect();
        let mut site_row_col = std::collections::BTreeMap::new();
        for s in &layout.sites {
            let row = match row_of_y.get(&OrderedF64(s.position.y_um)) {
                Some(&r) => r,
                None => 0,
            };
            let col = match col_of_x.get(&OrderedF64(s.position.x_um)) {
                Some(&c) => c,
                None => 0,
            };
            site_row_col.insert(s.id, (row, col));
        }
        Self { site_row_col }
    }

    pub(crate) fn indices(&self, site: SiteId) -> Result<(u32, u32), MovementPlanError> {
        self.site_row_col
            .get(&site)
            .copied()
            .ok_or(MovementPlanError::MissingSite(site))
    }
}

/// Generate dual candidates for all gate×pair×orientation combinations.
pub(crate) fn generate_duals(
    gates: &[(AtomId, AtomId)],
    free_pairs: &[InteractionPair],
    occ_atom: &std::collections::BTreeMap<AtomId, SiteId>,
    site_pos: &std::collections::BTreeMap<SiteId, Position>,
    params: &MovementParams,
) -> Result<Vec<DualCandidate>, MovementPlanError> {
    let mut duals = Vec::new();
    let mut next_id = 0u64;
    // AOD indices from site_pos keys as a mini layout.
    let layout = layout_from_sites(site_pos);
    let grid = AodGrid::from_layout(&layout);

    for &(a, b) in gates {
        let site_a = *occ_atom.get(&a).ok_or(MovementPlanError::MissingAtom(a))?;
        let site_b = *occ_atom.get(&b).ok_or(MovementPlanError::MissingAtom(b))?;
        let pos_a = *site_pos
            .get(&site_a)
            .ok_or(MovementPlanError::MissingSite(site_a))?;
        let pos_b = *site_pos
            .get(&site_b)
            .ok_or(MovementPlanError::MissingSite(site_b))?;

        for &pair in free_pairs {
            for (dest_a, dest_b) in [(pair.left, pair.right), (pair.right, pair.left)] {
                let pos_da = *site_pos
                    .get(&dest_a)
                    .ok_or(MovementPlanError::MissingSite(dest_a))?;
                let pos_db = *site_pos
                    .get(&dest_b)
                    .ok_or(MovementPlanError::MissingSite(dest_b))?;
                let mut legs = Vec::new();
                for (atom, from, to, from_pos, to_pos) in [
                    (a, site_a, dest_a, pos_a, pos_da),
                    (b, site_b, dest_b, pos_b, pos_db),
                ] {
                    let dist = euclidean_um(from_pos, to_pos);
                    let (row, col) = grid.indices(from)?;
                    legs.push(CandidateLeg {
                        atom,
                        from,
                        to,
                        from_pos,
                        to_pos,
                        aod_id: 0,
                        row,
                        col,
                        dual_id: next_id,
                        distance_um: dist,
                    });
                }
                let distance_um = legs.iter().map(|l| l.distance_um).fold(0.0_f64, f64::max);
                duals.push(DualCandidate {
                    gate: (a, b),
                    pair,
                    legs,
                    distance_um,
                });
                next_id += 1;
            }
        }
    }
    let _ = params;
    Ok(duals)
}

/// Greedy longest-first maximal IS selection (sortIS spirit).
pub(crate) fn select_duals_sortis(duals: &[DualCandidate]) -> Vec<DualCandidate> {
    let mut ordered = duals.to_vec();
    ordered.sort_by(|a, b| {
        b.distance_um
            .total_cmp(&a.distance_um)
            .then_with(|| a.gate.0.cmp(&b.gate.0))
            .then_with(|| a.gate.1.cmp(&b.gate.1))
            .then_with(|| a.pair.left.cmp(&b.pair.left))
    });
    let mut selected = Vec::new();
    let mut used_gates: BTreeSet<(AtomId, AtomId)> = BTreeSet::new();
    let mut used_sites: BTreeSet<SiteId> = BTreeSet::new();
    for d in ordered {
        let g = crate::geometry::ordered_pair(d.gate.0, d.gate.1);
        if used_gates.contains(&g) {
            continue;
        }
        let sites = [d.pair.left, d.pair.right];
        if sites.iter().any(|s| used_sites.contains(s)) {
            continue;
        }
        // Also conflict if destination sites overlap any selected dual's dests.
        let dests: BTreeSet<SiteId> = d.legs.iter().map(|l| l.to).collect();
        if dests.iter().any(|s| used_sites.contains(s)) {
            continue;
        }
        used_gates.insert(g);
        used_sites.insert(d.pair.left);
        used_sites.insert(d.pair.right);
        selected.push(d);
    }
    selected
}

/// Greedy longest-first leg packing with dual-exclusion and capacity preview (B7).
pub(crate) fn pack_legs_greedy(
    pending: &[CandidateLeg],
    params: &MovementParams,
) -> Result<Vec<CandidateLeg>, MovementPlanError> {
    let mut chosen: Vec<CandidateLeg> = Vec::new();
    let mut used_duals: BTreeSet<u64> = BTreeSet::new();
    for leg in pending {
        // B7: never co-schedule two legs from the same dual.
        if leg.dual_id != u64::MAX && used_duals.contains(&leg.dual_id) {
            continue;
        }
        let conflicts = chosen
            .iter()
            .any(|c| legs_conflict(c, leg, params.min_row_col_separation_um));
        if conflicts {
            continue;
        }
        // Capacity preview
        let mut rows: BTreeSet<u32> = chosen.iter().map(|l| l.row).collect();
        let mut cols: BTreeSet<u32> = chosen.iter().map(|l| l.col).collect();
        rows.insert(leg.row);
        cols.insert(leg.col);
        if rows.len() as u32 > params.aod_rows || cols.len() as u32 > params.aod_cols {
            continue;
        }
        if leg.dual_id != u64::MAX {
            used_duals.insert(leg.dual_id);
        }
        chosen.push(leg.clone());
    }
    Ok(chosen)
}

// Re-export for site_position_map callers in plan.rs.
pub(crate) use std::collections::BTreeMap;
