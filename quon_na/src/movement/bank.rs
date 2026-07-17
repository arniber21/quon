//! Interaction-pair bank creation and detection (B1 / B14).
//!
//! # Invariants
//!
//! - Bank origin: `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`.
//! - Bank pairs have gap `pair_gap_um` and pitch `pair_pitch_um`.
//! - [`ensure_interaction_pairs`] is idempotent: if a bank already exists with
//!   enough pairs, it returns them without appending.

use std::collections::{BTreeMap, BTreeSet};

use crate::layout::{AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding};
use crate::movement::geometry::POS_EPS_UM;
use crate::movement::types::{
    BANK_ISOLATION_EPS_UM, InteractionPair, MovementParams, MovementPlanError,
};

/// Ensure layout has a bank of interaction pairs (B1 / B14).
///
/// Idempotent: if a pair bank already exists with enough pairs, returns them
/// without appending. Bank origin:
/// `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`.
pub fn ensure_interaction_pairs(
    layout: &mut NeutralAtomLayout,
    params: &MovementParams,
    min_pairs: usize,
) -> Result<Vec<InteractionPair>, MovementPlanError> {
    params.validate()?;
    let min_pairs = min_pairs.max(1);

    if let Some(existing) = detect_bank_pairs(layout, params) {
        if existing.len() >= min_pairs {
            return Ok(existing);
        }
        // Need more pairs — append from existing.len() onward.
        let start = existing.len();
        let (x0, y0) = bank_origin(layout, params);
        append_bank_pairs(layout, params, x0, y0, start, min_pairs)?;
        return detect_bank_pairs(layout, params)
            .filter(|p| p.len() >= min_pairs)
            .ok_or_else(|| MovementPlanError::Conflict("failed to extend pair bank".into()));
    }

    let (x0, y0) = bank_origin(layout, params);
    append_bank_pairs(layout, params, x0, y0, 0, min_pairs)?;
    detect_bank_pairs(layout, params)
        .filter(|p| p.len() >= min_pairs)
        .ok_or_else(|| MovementPlanError::Conflict("failed to create pair bank".into()))
}

pub(crate) fn bank_origin(layout: &NeutralAtomLayout, params: &MovementParams) -> (f64, f64) {
    bank_origin_for_detect(layout, params)
}

fn append_bank_pairs(
    layout: &mut NeutralAtomLayout,
    params: &MovementParams,
    x0: f64,
    y0: f64,
    start_i: usize,
    min_pairs: usize,
) -> Result<(), MovementPlanError> {
    let start_id = match layout.sites.iter().map(|s| s.id.0).max() {
        Some(max_id) => max_id.saturating_add(1),
        None => 0,
    };
    for (offset, i) in (start_i..min_pairs).enumerate() {
        let left = SiteId(start_id + (offset as u32) * 2);
        let right = SiteId(start_id + (offset as u32) * 2 + 1);
        let lx = x0 + (i as f64) * params.pair_pitch_um;
        let ly = y0 + ((i % 2) as f64) * params.pair_pitch_um;
        layout.sites.push(AtomSite {
            id: left,
            position: Position { x_um: lx, y_um: ly },
        });
        layout.sites.push(AtomSite {
            id: right,
            position: Position {
                x_um: lx + params.pair_gap_um,
                y_um: ly,
            },
        });
    }
    Ok(())
}

fn detect_bank_pairs(
    layout: &NeutralAtomLayout,
    params: &MovementParams,
) -> Option<Vec<InteractionPair>> {
    let (x0, _) = bank_origin_for_detect(layout, params);
    let mut by_pos: BTreeMap<(OrderedF64, OrderedF64), SiteId> = BTreeMap::new();
    for s in &layout.sites {
        by_pos.insert(
            (OrderedF64(s.position.x_um), OrderedF64(s.position.y_um)),
            s.id,
        );
    }
    let mut pairs = Vec::new();
    let mut i = 0usize;
    loop {
        let lx = x0 + (i as f64) * params.pair_pitch_um;
        let ly = {
            // y0 from placement mid — recompute
            let (_, y0) = bank_origin_for_detect(layout, params);
            y0 + ((i % 2) as f64) * params.pair_pitch_um
        };
        let rx = lx + params.pair_gap_um;
        let left = by_pos.get(&(OrderedF64(lx), OrderedF64(ly))).copied();
        let right = by_pos.get(&(OrderedF64(rx), OrderedF64(ly))).copied();
        match (left, right) {
            (Some(l), Some(r)) => {
                pairs.push(InteractionPair { left: l, right: r });
                i += 1;
            }
            _ => break,
        }
        if i > 10_000 {
            break;
        }
    }
    if pairs.is_empty() { None } else { Some(pairs) }
}

fn bank_origin_for_detect(layout: &NeutralAtomLayout, params: &MovementParams) -> (f64, f64) {
    // Placement bbox from **initial bindings only**. Never scan all `layout.sites`:
    // after `append_bank_pairs`, that would inflate `max_x` and make detect miss
    // the bank just appended. Also do not treat pair_gap-spaced placement sites
    // as "already bank" — skip fixtures use gap == `pair_gap_um` (B14).
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for b in &layout.initial_bindings {
        let site = match b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        if let Some(s) = layout.sites.iter().find(|x| x.id == site) {
            any = true;
            max_x = max_x.max(s.position.x_um);
            min_y = min_y.min(s.position.y_um);
            max_y = max_y.max(s.position.y_um);
        }
    }
    if any {
        (
            max_x + params.pair_pitch_um + BANK_ISOLATION_EPS_UM,
            (min_y + max_y) * 0.5,
        )
    } else {
        (params.pair_pitch_um + BANK_ISOLATION_EPS_UM, 0.0)
    }
}

// ---------------------------------------------------------------------------
// Ordered F64 (for BTreeMap keying by coordinates)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OrderedF64(pub f64);

impl Eq for OrderedF64 {}

impl PartialOrd for OrderedF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

// ---------------------------------------------------------------------------
// Layout / occupancy helpers
// ---------------------------------------------------------------------------

pub(crate) fn site_position_map(
    layout: &NeutralAtomLayout,
) -> Result<BTreeMap<SiteId, Position>, MovementPlanError> {
    let mut m = BTreeMap::new();
    for s in &layout.sites {
        m.insert(s.id, s.position);
    }
    Ok(m)
}

pub(crate) fn pos_of(
    atom: crate::layout::AtomId,
    occ_atom: &BTreeMap<crate::layout::AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
) -> Result<Position, MovementPlanError> {
    let site = *occ_atom
        .get(&atom)
        .ok_or(MovementPlanError::MissingAtom(atom))?;
    site_pos
        .get(&site)
        .copied()
        .ok_or(MovementPlanError::MissingSite(site))
}

pub(crate) fn layout_from_sites(site_pos: &BTreeMap<SiteId, Position>) -> NeutralAtomLayout {
    NeutralAtomLayout {
        sites: site_pos
            .iter()
            .map(|(&id, &position)| AtomSite { id, position })
            .collect(),
        initial_bindings: Vec::new(),
    }
}

pub(crate) fn pair_occupants(
    pair: InteractionPair,
    occ_site: &BTreeMap<SiteId, crate::layout::AtomId>,
) -> Option<(crate::layout::AtomId, crate::layout::AtomId)> {
    let a = occ_site.get(&pair.left).copied()?;
    let b = occ_site.get(&pair.right).copied()?;
    Some((a, b))
}

pub(crate) fn free_interaction_pairs(
    pairs: &[InteractionPair],
    occ_site: &BTreeMap<SiteId, crate::layout::AtomId>,
) -> Vec<InteractionPair> {
    pairs
        .iter()
        .copied()
        .filter(|p| !occ_site.contains_key(&p.left) && !occ_site.contains_key(&p.right))
        .collect()
}

// Unused but kept for parity with original code's internal helpers.
#[allow(dead_code)]
pub(crate) fn _pos_eps() -> f64 {
    POS_EPS_UM
}

// Re-export BTreeSet for callers that need the OrderedF64 type.
#[allow(dead_code)]
pub(crate) fn _ordered_f64_set() -> BTreeSet<OrderedF64> {
    BTreeSet::new()
}
