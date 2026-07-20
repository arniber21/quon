//! Emit context: load → move → store stage emission, reclaim/eviction (B8/B13).
//!
//! # Invariants
//!
//! - Each movement stage is load (SLM→AOD) → move (AOD) → store (AOD→SLM).
//! - Destinations must be empty (v0: no swaps).
//! - AOD capacity (rows/cols) is checked before emit.
//! - `verify_aod_legality` runs on every group before occupancy mutation.
//! - Reclaim/evict (B8/B13): between layers, vacate non-participant or
//!   partial-overlap pairs; evict while need > free.

use std::collections::{BTreeMap, BTreeSet};

use crate::geometry::{euclidean_um, movement_duration_us, ordered_pair};
use crate::layout::{AodTrapRef, AtomId, Position, SiteId};
use crate::movement::bank::{pair_occupants, site_position_map};
use crate::movement::duals::{AodGrid, DualCandidate, pack_legs_greedy};
use crate::movement::geometry::{POS_EPS_UM, check_entangling_geometry, verify_aod_legality};
use crate::movement::types::{
    CandidateLeg, InteractionPair, MoveSpec, MovementParams, MovementPlanError,
};
use crate::schedule::{
    AtomMove, MovementGroup, NeutralAtomAction, ScheduleLayer, TransferDirection, TrapTransfer,
};

pub(crate) struct EmitCtx<'a> {
    pub params: &'a MovementParams,
    pub site_pos: &'a BTreeMap<SiteId, Position>,
    pub aod_grid: &'a AodGrid,
    pub home: &'a BTreeMap<AtomId, SiteId>,
    pub pair_sites: &'a BTreeSet<SiteId>,
    pub pairs: &'a [InteractionPair],
    pub occ_site: &'a mut BTreeMap<SiteId, AtomId>,
    pub occ_atom: &'a mut BTreeMap<AtomId, SiteId>,
    pub out_layers: &'a mut Vec<ScheduleLayer>,
    pub next_cycle: &'a mut u32,
    pub rearrangement_steps: &'a mut u64,
    pub rearrangement_time_us: &'a mut u64,
    pub trap_transfers: &'a mut u64,
    pub transfer_time_us: &'a mut u64,
}

impl EmitCtx<'_> {
    pub(crate) fn reclaim_and_evict(
        &mut self,
        next_atoms: &BTreeSet<AtomId>,
        next_gates: &BTreeSet<(AtomId, AtomId)>,
        cycle: u32,
    ) -> Result<(), MovementPlanError> {
        // B13: vacate whole pair on partial overlap.
        let mut to_vacate: Vec<InteractionPair> = Vec::new();
        for &pair in self.pairs {
            let occ = pair_occupants(pair, self.occ_site);
            let Some((u, v)) = occ else {
                continue;
            };
            let gate_uv = ordered_pair(u, v);
            if next_gates.contains(&gate_uv) {
                // Full reuse candidate — keep if B10 would pass (checked later as skip).
                continue;
            }
            let u_in = next_atoms.contains(&u);
            let v_in = next_atoms.contains(&v);
            if !u_in && !v_in {
                // Full non-participant — B8 eviction candidate.
                to_vacate.push(pair);
                continue;
            }
            // Partial overlap: exactly one in A, or next gate shares one atom.
            let partial = (u_in ^ v_in)
                || next_gates.iter().any(|&(a, b)| {
                    let share_u = a == u || b == u;
                    let share_v = a == v || b == v;
                    (share_u || share_v) && !(share_u && share_v)
                });
            if partial {
                to_vacate.push(pair);
            }
        }

        for pair in to_vacate {
            self.vacate_pair_home(pair, cycle)?;
        }

        // B8: while need > free, evict non-participant occupied pairs.
        loop {
            let need = count_need_free_pairs(
                next_gates,
                next_atoms,
                self.pairs,
                self.occ_site,
                self.occ_atom,
                self.site_pos,
                self.params,
            )?;
            let free =
                crate::movement::bank::free_interaction_pairs(self.pairs, self.occ_site).len();
            if need <= free {
                break;
            }
            let mut evicted = false;
            for &pair in self.pairs {
                let Some((u, v)) = pair_occupants(pair, self.occ_site) else {
                    continue;
                };
                if !next_atoms.contains(&u) && !next_atoms.contains(&v) {
                    self.vacate_pair_home(pair, cycle)?;
                    evicted = true;
                    break;
                }
            }
            if !evicted {
                break;
            }
        }
        Ok(())
    }

    fn vacate_pair_home(
        &mut self,
        pair: InteractionPair,
        cycle: u32,
    ) -> Result<(), MovementPlanError> {
        let mut legs = Vec::new();
        for site in [pair.left, pair.right] {
            if let Some(&atom) = self.occ_site.get(&site) {
                let home = *self
                    .home
                    .get(&atom)
                    .ok_or(MovementPlanError::MissingAtom(atom))?;
                if home == site {
                    continue;
                }
                if self.occ_site.contains_key(&home) {
                    // Home occupied — should not happen for placement homes of bank atoms.
                    // Try anyway only if same atom (no-op).
                    if self.occ_site.get(&home) != Some(&atom) {
                        return Err(MovementPlanError::TransferIntoOccupied { cycle, site: home });
                    }
                }
                let from_pos = *self
                    .site_pos
                    .get(&site)
                    .ok_or(MovementPlanError::MissingSite(site))?;
                let to_pos = *self
                    .site_pos
                    .get(&home)
                    .ok_or(MovementPlanError::MissingSite(home))?;
                let (row, col) = self.aod_grid.indices(site)?;
                legs.push(CandidateLeg {
                    atom,
                    from: site,
                    to: home,
                    from_pos,
                    to_pos,
                    aod_id: 0,
                    row,
                    col,
                    dual_id: u64::MAX,
                    distance_um: euclidean_um(from_pos, to_pos),
                });
            }
        }
        if !legs.is_empty() {
            self.emit_legs_packed(legs, cycle)?;
        }
        Ok(())
    }

    pub(crate) fn return_all_bank_home(&mut self, cycle: u32) -> Result<(), MovementPlanError> {
        for &pair in self.pairs {
            if pair_occupants(pair, self.occ_site).is_some() {
                self.vacate_pair_home(pair, cycle)?;
            }
        }
        Ok(())
    }

    pub(crate) fn emit_duals_b7(
        &mut self,
        selected: &[DualCandidate],
        cycle: u32,
    ) -> Result<BTreeSet<(AtomId, AtomId)>, MovementPlanError> {
        let mut covered = BTreeSet::new();
        let mut pending: Vec<CandidateLeg> = Vec::new();
        // Force-split: legs from same dual never enter same pack round.
        // Emit as: expand all legs, then pack with dual_id exclusion in same group.
        for dual in selected {
            covered.insert(ordered_pair(dual.gate.0, dual.gate.1));
            for leg in &dual.legs {
                if leg.distance_um > POS_EPS_UM {
                    pending.push(leg.clone());
                } else {
                    // Already at dest — claim occupancy if needed.
                    self.occ_atom.insert(leg.atom, leg.to);
                    self.occ_site.insert(leg.to, leg.atom);
                }
            }
        }
        // Sort pending: longer first, then lower AtomId (B7 order hint).
        pending.sort_by(|a, b| {
            b.distance_um
                .total_cmp(&a.distance_um)
                .then_with(|| a.atom.cmp(&b.atom))
        });

        while !pending.is_empty() {
            let group = pack_legs_greedy(&pending, self.params)?;
            if group.is_empty() {
                return Err(MovementPlanError::Unsatisfiable(cycle));
            }
            let group_atoms: BTreeSet<AtomId> = group.iter().map(|l| l.atom).collect();
            self.emit_load_move_store(&group, cycle)?;
            pending.retain(|l| !group_atoms.contains(&l.atom));
        }
        Ok(covered)
    }

    fn emit_legs_packed(
        &mut self,
        mut pending: Vec<CandidateLeg>,
        cycle: u32,
    ) -> Result<(), MovementPlanError> {
        pending.sort_by(|a, b| {
            b.distance_um
                .total_cmp(&a.distance_um)
                .then_with(|| a.atom.cmp(&b.atom))
        });
        while !pending.is_empty() {
            let group = pack_legs_greedy(&pending, self.params)?;
            if group.is_empty() {
                return Err(MovementPlanError::Unsatisfiable(cycle));
            }
            let group_atoms: BTreeSet<AtomId> = group.iter().map(|l| l.atom).collect();
            self.emit_load_move_store(&group, cycle)?;
            pending.retain(|l| !group_atoms.contains(&l.atom));
        }
        Ok(())
    }

    pub(crate) fn emit_load_move_store(
        &mut self,
        group: &[CandidateLeg],
        cycle: u32,
    ) -> Result<(), MovementPlanError> {
        if group.is_empty() {
            return Ok(());
        }

        // Dest must be empty (v0 no swaps); source occupied by mover.
        let mut dests = BTreeSet::new();
        for leg in group {
            if !dests.insert(leg.to) {
                return Err(MovementPlanError::Collision {
                    cycle,
                    site: leg.to,
                });
            }
            if let Some(&owner) = self.occ_site.get(&leg.to)
                && owner != leg.atom
            {
                return Err(MovementPlanError::TransferIntoOccupied {
                    cycle: *self.next_cycle,
                    site: leg.to,
                });
            }
        }

        // Capacity
        let rows: BTreeSet<u32> = group.iter().map(|l| l.row).collect();
        let cols: BTreeSet<u32> = group.iter().map(|l| l.col).collect();
        if rows.len() as u32 > self.params.aod_rows {
            return Err(MovementPlanError::AodCapacity {
                axis: "rows",
                needed: rows.len() as u32,
                limit: self.params.aod_rows,
            });
        }
        if cols.len() as u32 > self.params.aod_cols {
            return Err(MovementPlanError::AodCapacity {
                axis: "cols",
                needed: cols.len() as u32,
                limit: self.params.aod_cols,
            });
        }

        let mut load = Vec::new();
        let mut moves = Vec::new();
        let mut store = Vec::new();
        let mut d_max = 0.0_f64;
        let mut specs = Vec::with_capacity(group.len());

        for leg in group {
            d_max = d_max.max(leg.distance_um);
            let aod = AodTrapRef {
                aod_id: leg.aod_id,
                row: leg.row,
                col: leg.col,
            };
            load.push(NeutralAtomAction::Transfer(TrapTransfer {
                atom: leg.atom,
                direction: TransferDirection::SlmToAod,
                site: leg.from,
                aod: aod.clone(),
                duration_us: self.params.trap_transfer_us,
            }));
            moves.push(AtomMove {
                atom: leg.atom,
                from: leg.from,
                to: leg.to,
            });
            store.push(NeutralAtomAction::Transfer(TrapTransfer {
                atom: leg.atom,
                direction: TransferDirection::AodToSlm,
                site: leg.to,
                aod,
                duration_us: self.params.trap_transfer_us,
            }));
            specs.push(MoveSpec {
                atom: leg.atom.0,
                from_site: leg.from.0,
                to_site: leg.to.0,
                aod_id: leg.aod_id,
                row: leg.row,
                col: leg.col,
                from_x_um: leg.from_pos.x_um,
                from_y_um: leg.from_pos.y_um,
                to_x_um: leg.to_pos.x_um,
                to_y_um: leg.to_pos.y_um,
            });
        }
        verify_aod_legality(
            *self.next_cycle,
            &specs,
            self.params.min_row_col_separation_um,
        )?;

        // Load cycle: vacate source SLM.
        for leg in group {
            if self.occ_site.get(&leg.from) == Some(&leg.atom) {
                self.occ_site.remove(&leg.from);
            }
        }
        let load_layer = ScheduleLayer {
            cycle: *self.next_cycle,
            actions: load,
        };
        load_layer
            .validate_occupancy()
            .map_err(|e| MovementPlanError::Conflict(e.to_string()))?;
        *self.trap_transfers += load_layer.actions.len() as u64;
        *self.transfer_time_us += load_layer.actions.len() as u64 * self.params.trap_transfer_us;
        self.out_layers.push(load_layer);
        *self.next_cycle = self.next_cycle.saturating_add(1);

        let duration_us = movement_duration_us(d_max, self.params.acceleration_m_s2);
        let move_layer = ScheduleLayer {
            cycle: *self.next_cycle,
            actions: vec![NeutralAtomAction::Move(MovementGroup {
                moves,
                duration_us,
            })],
        };
        move_layer
            .validate_occupancy()
            .map_err(|e| MovementPlanError::Conflict(e.to_string()))?;
        *self.rearrangement_steps += 1;
        *self.rearrangement_time_us += duration_us;
        self.out_layers.push(move_layer);
        *self.next_cycle = self.next_cycle.saturating_add(1);

        // Store: dest must be empty.
        for leg in group {
            if let Some(&owner) = self.occ_site.get(&leg.to)
                && owner != leg.atom
            {
                return Err(MovementPlanError::TransferIntoOccupied {
                    cycle: *self.next_cycle,
                    site: leg.to,
                });
            }
            self.occ_site.insert(leg.to, leg.atom);
            self.occ_atom.insert(leg.atom, leg.to);
        }
        let store_layer = ScheduleLayer {
            cycle: *self.next_cycle,
            actions: store,
        };
        store_layer
            .validate_occupancy()
            .map_err(|e| MovementPlanError::Conflict(e.to_string()))?;
        *self.trap_transfers += store_layer.actions.len() as u64;
        *self.transfer_time_us += store_layer.actions.len() as u64 * self.params.trap_transfer_us;
        self.out_layers.push(store_layer);
        *self.next_cycle = self.next_cycle.saturating_add(1);

        let _ = cycle;
        let _ = self.pair_sites;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Geometry-gated skip helpers
// ---------------------------------------------------------------------------

pub(crate) fn can_skip_geometry_gated(
    a: AtomId,
    b: AtomId,
    layer_gates: &[(AtomId, AtomId)],
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
    params: &MovementParams,
) -> Result<bool, MovementPlanError> {
    let pa = crate::movement::bank::pos_of(a, occ_atom, site_pos)?;
    let pb = crate::movement::bank::pos_of(b, occ_atom, site_pos)?;
    if euclidean_um(pa, pb) > params.rydberg_range_um {
        return Ok(false);
    }
    match check_entangling_geometry(0, &[(a, b)], occ_atom, site_pos, params) {
        Ok(()) => {
            let mut in_place = Vec::new();
            for &(x, y) in layer_gates {
                let px = crate::movement::bank::pos_of(x, occ_atom, site_pos)?;
                let py = crate::movement::bank::pos_of(y, occ_atom, site_pos)?;
                if euclidean_um(px, py) <= params.rydberg_range_um {
                    in_place.push((x, y));
                }
            }
            Ok(check_entangling_geometry(0, &in_place, occ_atom, site_pos, params).is_ok())
        }
        Err(_) => Ok(false),
    }
}

fn count_need_free_pairs(
    next_gates: &BTreeSet<(AtomId, AtomId)>,
    _next_atoms: &BTreeSet<AtomId>,
    pairs: &[InteractionPair],
    occ_site: &BTreeMap<SiteId, AtomId>,
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
    params: &MovementParams,
) -> Result<usize, MovementPlanError> {
    let mut need = 0usize;
    for &(a, b) in next_gates {
        if gate_on_reusable_pair(a, b, pairs, occ_site, occ_atom, site_pos, params)? {
            continue;
        }
        need += 1;
    }
    Ok(need)
}

fn gate_on_reusable_pair(
    a: AtomId,
    b: AtomId,
    pairs: &[InteractionPair],
    occ_site: &BTreeMap<SiteId, AtomId>,
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
    params: &MovementParams,
) -> Result<bool, MovementPlanError> {
    for &pair in pairs {
        let Some((u, v)) = pair_occupants(pair, occ_site) else {
            continue;
        };
        if ordered_pair(u, v) != ordered_pair(a, b) {
            continue;
        }
        return can_skip_geometry_gated(a, b, &[(a, b)], occ_atom, site_pos, params);
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Layer query helpers (shared with plan.rs)
// ---------------------------------------------------------------------------

pub(crate) fn layers_have_entangle(layers: &[ScheduleLayer]) -> bool {
    layers.iter().any(layer_has_entangle)
}

pub(crate) fn layer_has_entangle(layer: &ScheduleLayer) -> bool {
    layer.actions.iter().any(|a| {
        matches!(
            a,
            NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
        )
    })
}

pub(crate) fn layer_has_entangle_n(layer: &ScheduleLayer) -> bool {
    layer
        .actions
        .iter()
        .any(|a| matches!(a, NeutralAtomAction::EntangleN { .. }))
}

pub(crate) fn max_entangle2_width(layers: &[ScheduleLayer]) -> usize {
    layers
        .iter()
        .map(|l| {
            l.actions
                .iter()
                .filter(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
                .count()
        })
        .max()
        .unwrap_or(0)
}

pub(crate) fn collect_entangle2(
    layer: &ScheduleLayer,
) -> Result<Vec<(AtomId, AtomId)>, MovementPlanError> {
    let mut gates = Vec::new();
    for action in &layer.actions {
        match action {
            NeutralAtomAction::Entangle2 { atoms, .. } => {
                gates.push((atoms[0], atoms[1]));
            }
            NeutralAtomAction::EntangleN { atoms, .. } => {
                return Err(MovementPlanError::UnsupportedEntangleArity {
                    cycle: layer.cycle,
                    k: atoms.len(),
                });
            }
            _ => {}
        }
    }
    Ok(gates)
}

pub(crate) fn gate_atom_set(gates: &[(AtomId, AtomId)]) -> BTreeSet<AtomId> {
    let mut s = BTreeSet::new();
    for &(a, b) in gates {
        s.insert(a);
        s.insert(b);
    }
    s
}

/// Exercise the B5 cross-cycle store-into-occupied reject on the real emit path.
pub fn try_transfer_into_occupied(
    layout: &crate::layout::NeutralAtomLayout,
    params: &MovementParams,
    mover: AtomId,
    from: SiteId,
    to: SiteId,
    occ_site: &mut BTreeMap<SiteId, AtomId>,
    occ_atom: &mut BTreeMap<AtomId, SiteId>,
) -> Result<(), MovementPlanError> {
    params.validate()?;
    let site_pos = site_position_map(layout)?;
    let aod_grid = AodGrid::from_layout(layout);
    let home = BTreeMap::new();
    let pairs: Vec<InteractionPair> = Vec::new();
    let pair_sites = BTreeSet::new();
    let mut out_layers = Vec::new();
    let mut next_cycle = 0u32;
    let mut rearrangement_steps = 0u64;
    let mut rearrangement_time_us = 0u64;
    let mut trap_transfers = 0u64;
    let mut transfer_time_us = 0u64;
    let from_pos = *site_pos
        .get(&from)
        .ok_or(MovementPlanError::MissingSite(from))?;
    let to_pos = *site_pos
        .get(&to)
        .ok_or(MovementPlanError::MissingSite(to))?;
    let (row, col) = aod_grid.indices(from)?;
    let leg = CandidateLeg {
        atom: mover,
        from,
        to,
        from_pos,
        to_pos,
        aod_id: 0,
        row,
        col,
        dual_id: u64::MAX,
        distance_um: euclidean_um(from_pos, to_pos),
    };
    let mut ctx = EmitCtx {
        params,
        site_pos: &site_pos,
        aod_grid: &aod_grid,
        home: &home,
        pair_sites: &pair_sites,
        pairs: &pairs,
        occ_site,
        occ_atom,
        out_layers: &mut out_layers,
        next_cycle: &mut next_cycle,
        rearrangement_steps: &mut rearrangement_steps,
        rearrangement_time_us: &mut rearrangement_time_us,
        trap_transfers: &mut trap_transfers,
        transfer_time_us: &mut transfer_time_us,
    };
    ctx.emit_load_move_store(&[leg], 0)
}

/// Lower schedule atom moves to dialect-shaped [`MoveSpec`]s (B4).
pub fn atom_moves_to_move_specs(
    moves: &[AtomMove],
    layout: &crate::layout::NeutralAtomLayout,
    aod_meta: &BTreeMap<AtomId, AodTrapRef>,
) -> Result<Vec<MoveSpec>, MovementPlanError> {
    let site_pos = site_position_map(layout)?;
    let mut specs = Vec::with_capacity(moves.len());
    for m in moves {
        let from = site_pos
            .get(&m.from)
            .copied()
            .ok_or(MovementPlanError::MissingSite(m.from))?;
        let to = site_pos
            .get(&m.to)
            .copied()
            .ok_or(MovementPlanError::MissingSite(m.to))?;
        let aod = aod_meta
            .get(&m.atom)
            .cloned()
            .ok_or(MovementPlanError::MissingAtom(m.atom))?;
        specs.push(MoveSpec {
            atom: m.atom.0,
            from_site: m.from.0,
            to_site: m.to.0,
            aod_id: aod.aod_id,
            row: aod.row,
            col: aod.col,
            from_x_um: from.x_um,
            from_y_um: from.y_um,
            to_x_um: to.x_um,
            to_y_um: to.y_um,
        });
    }
    Ok(specs)
}

// Silence unused import in some configurations.
