//! Main flat AOD movement planner entrypoint (issue #106).
//!
//! # Invariants
//!
//! - Public entrypoint [`plan_aod_movement`] is the sole planner surface; it
//!   orchestrates bank creation, dual generation, IS packing, emit, and
//!   geometry-gated skip.
//! - Between-layer reclaim / eviction (B8 + B13) runs before dual generation.
//! - R1–R3 runs on all occupied atoms before entangle (B11).
//! - Non-entangling input layers are passed through unchanged.

use std::collections::{BTreeMap, BTreeSet};

use crate::geometry::ordered_pair;
use crate::layout::{AtomBinding, AtomId, NeutralAtomLayout, Position, SiteId, TrapBinding};
use crate::movement::bank::{ensure_interaction_pairs, site_position_map};
use crate::movement::duals::{AodGrid, generate_duals, select_duals_sortis};
use crate::movement::emit::{
    EmitCtx, can_skip_geometry_gated, collect_entangle2, gate_atom_set, layer_has_entangle,
    layer_has_entangle_n, layers_have_entangle, max_entangle2_width,
};
use crate::movement::geometry::check_entangling_geometry;
use crate::movement::types::{
    InteractionPair, MovementParams, MovementPlanError, MovementPlanResult,
};
use crate::schedule::{NeutralAtomAction, ScheduleLayer};
use crate::schedule_entry::GraphScheduleRequest;

/// Expand entangling layers with AOD-legal movement (Quon pair-bank duals +
/// Enola-inspired conflict types / greedy longest-first IS; see B9).
pub fn plan_aod_movement(
    mut req: GraphScheduleRequest,
    params: &MovementParams,
) -> Result<MovementPlanResult, MovementPlanError> {
    params.validate()?;
    let layout = req
        .layout
        .as_mut()
        .ok_or(MovementPlanError::MissingLayout)?;
    if req.layers.is_empty() || !layers_have_entangle(&req.layers) {
        return Err(MovementPlanError::EmptySchedule);
    }

    let w_max = max_entangle2_width(&req.layers);
    let pairs = ensure_interaction_pairs(layout, params, w_max.max(1))?;
    let pair_sites: BTreeSet<SiteId> = pairs.iter().flat_map(|p| [p.left, p.right]).collect();

    let home: BTreeMap<AtomId, SiteId> = layout
        .initial_bindings
        .iter()
        .map(|b| {
            let site = match b.trap {
                TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
            };
            (b.atom, site)
        })
        .collect();

    // Rebuild maps after bank append.
    let site_pos = site_position_map(layout)?;
    let aod_grid = AodGrid::from_layout(layout);

    let mut occ_site: BTreeMap<SiteId, AtomId> = BTreeMap::new();
    let mut occ_atom: BTreeMap<AtomId, SiteId> = BTreeMap::new();
    for b in &layout.initial_bindings {
        let site = match b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        if occ_site.insert(site, b.atom).is_some() {
            return Err(MovementPlanError::Collision { cycle: 0, site });
        }
        occ_atom.insert(b.atom, site);
    }

    let input_layers = std::mem::take(&mut req.layers);
    let mut out_layers = Vec::new();
    let mut next_cycle: u32 = 0;
    let mut rearrangement_steps = 0u64;
    let mut rearrangement_time_us = 0u64;
    let mut trap_transfers = 0u64;
    let mut transfer_time_us = 0u64;
    let mut skipped_already_adjacent = 0u64;

    let entangle_layer_indices: Vec<usize> = input_layers
        .iter()
        .enumerate()
        .filter(|(_, l)| layer_has_entangle(l))
        .map(|(i, _)| i)
        .collect();

    for (ei, &layer_idx) in entangle_layer_indices.iter().enumerate() {
        let layer = &input_layers[layer_idx];
        let gates = collect_entangle2(layer)?;
        if gates.is_empty() && layer_has_entangle_n(layer) {
            // collect_entangle2 already errors on EntangleN; unreachable
        }

        // Between-layer reclaim / eviction (B8 + B13) before dual generation.
        if ei > 0 && !params.return_home {
            let next_atoms = gate_atom_set(&gates);
            let next_gate_set: BTreeSet<(AtomId, AtomId)> =
                gates.iter().map(|&(a, b)| ordered_pair(a, b)).collect();
            let mut planner = EmitCtx {
                params,
                site_pos: &site_pos,
                aod_grid: &aod_grid,
                home: &home,
                pair_sites: &pair_sites,
                pairs: &pairs,
                occ_site: &mut occ_site,
                occ_atom: &mut occ_atom,
                out_layers: &mut out_layers,
                next_cycle: &mut next_cycle,
                rearrangement_steps: &mut rearrangement_steps,
                rearrangement_time_us: &mut rearrangement_time_us,
                trap_transfers: &mut trap_transfers,
                transfer_time_us: &mut transfer_time_us,
            };
            planner.reclaim_and_evict(&next_atoms, &next_gate_set, layer.cycle)?;
        }

        let mut unsatisfied = Vec::new();
        for &(a, b) in &gates {
            if can_skip_geometry_gated(a, b, &gates, &occ_atom, &site_pos, params)? {
                skipped_already_adjacent += 1;
            } else {
                unsatisfied.push((a, b));
            }
        }

        // Relocate unsatisfied gates onto free pairs.
        let mut remaining = unsatisfied;
        let mut rounds_without_progress = 0u32;
        while !remaining.is_empty() {
            let free_pairs = crate::movement::bank::free_interaction_pairs(&pairs, &occ_site);
            if free_pairs.is_empty() {
                let (lhs, rhs) = remaining[0];
                return Err(MovementPlanError::NoInteractionPair {
                    cycle: layer.cycle,
                    lhs,
                    rhs,
                });
            }

            let duals = generate_duals(&remaining, &free_pairs, &occ_atom, &site_pos, params)?;
            if duals.is_empty() {
                let (lhs, rhs) = remaining[0];
                return Err(MovementPlanError::NoInteractionPair {
                    cycle: layer.cycle,
                    lhs,
                    rhs,
                });
            }

            let selected = select_duals_sortis(&duals);
            if selected.is_empty() {
                return Err(MovementPlanError::Unsatisfiable(layer.cycle));
            }

            let mut planner = EmitCtx {
                params,
                site_pos: &site_pos,
                aod_grid: &aod_grid,
                home: &home,
                pair_sites: &pair_sites,
                pairs: &pairs,
                occ_site: &mut occ_site,
                occ_atom: &mut occ_atom,
                out_layers: &mut out_layers,
                next_cycle: &mut next_cycle,
                rearrangement_steps: &mut rearrangement_steps,
                rearrangement_time_us: &mut rearrangement_time_us,
                trap_transfers: &mut trap_transfers,
                transfer_time_us: &mut transfer_time_us,
            };
            let covered = planner.emit_duals_b7(&selected, layer.cycle)?;
            let before = remaining.len();
            remaining.retain(|&(a, b)| !covered.contains(&ordered_pair(a, b)));
            if remaining.len() == before {
                rounds_without_progress += 1;
                if rounds_without_progress > 8 {
                    return Err(MovementPlanError::Unsatisfiable(layer.cycle));
                }
            } else {
                rounds_without_progress = 0;
            }
        }

        // R1–R3 before entangle (B11): all occupied atoms, no spectator parking.
        check_entangling_geometry(layer.cycle, &gates, &occ_atom, &site_pos, params)?;

        let mut entangle_actions = Vec::new();
        for action in &layer.actions {
            match action {
                NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. } => {
                    entangle_actions.push(action.clone());
                }
                other => entangle_actions.push(other.clone()),
            }
        }
        // Prefer only entangles + non-move/transfer from original; drop any moves.
        entangle_actions.retain(|a| {
            !matches!(
                a,
                NeutralAtomAction::Move(_) | NeutralAtomAction::Transfer(_)
            )
        });

        let entangle_layer = ScheduleLayer {
            cycle: next_cycle,
            actions: entangle_actions,
        };
        entangle_layer
            .validate_conflicts()
            .map_err(|e| MovementPlanError::Conflict(e.to_string()))?;
        out_layers.push(entangle_layer);
        next_cycle = next_cycle.saturating_add(1);

        if params.return_home {
            let mut planner = EmitCtx {
                params,
                site_pos: &site_pos,
                aod_grid: &aod_grid,
                home: &home,
                pair_sites: &pair_sites,
                pairs: &pairs,
                occ_site: &mut occ_site,
                occ_atom: &mut occ_atom,
                out_layers: &mut out_layers,
                next_cycle: &mut next_cycle,
                rearrangement_steps: &mut rearrangement_steps,
                rearrangement_time_us: &mut rearrangement_time_us,
                trap_transfers: &mut trap_transfers,
                transfer_time_us: &mut transfer_time_us,
            };
            planner.return_all_bank_home(layer.cycle)?;
        }
    }

    // Pass through non-entangling input layers (none expected from #105).
    for (i, layer) in input_layers.iter().enumerate() {
        if !entangle_layer_indices.contains(&i) && !layer.actions.is_empty() {
            let mut actions = layer.actions.clone();
            actions.retain(|a| {
                !matches!(
                    a,
                    NeutralAtomAction::Move(_) | NeutralAtomAction::Transfer(_)
                )
            });
            if !actions.is_empty() {
                out_layers.push(ScheduleLayer {
                    cycle: next_cycle,
                    actions,
                });
                next_cycle = next_cycle.saturating_add(1);
            }
        }
    }

    // Update layout bindings to final occupancy.
    let layout = req
        .layout
        .as_mut()
        .ok_or(MovementPlanError::MissingLayout)?;
    layout.initial_bindings = occ_atom
        .iter()
        .map(|(&atom, &site)| AtomBinding {
            atom,
            trap: TrapBinding::Slm { site },
        })
        .collect();
    layout.initial_bindings.sort_by_key(|b| b.atom);

    req.layers = out_layers;
    Ok(MovementPlanResult {
        request: req,
        rearrangement_steps,
        rearrangement_time_us,
        trap_transfers,
        transfer_time_us,
        skipped_already_adjacent,
    })
}
