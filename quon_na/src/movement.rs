//! Flat (non-zoned) AOD-constrained movement planner (issue #106).
//!
//! # Attribution (B9)
//!
//! This module implements a **Quon** pipeline:
//! - interaction-pair bank destinations,
//! - both-atom dual orientations onto a free pair,
//! - B7 second packing pass that serializes dual legs,
//! - B8/B13 multi-layer reuse / eviction / partial-overlap reclaim.
//!
//! [Enola] Sec. 5 is cited **only** for: (1) three per-axis move conflict types
//! and (2) the greedy longest-first maximal independent-set *idea* (sortIS
//! spirit — not KaMIS). Enola Sec. 5 duals are one-atom “move either endpoint”
//! candidates; **this planner does not implement those duals**.
//!
//! # Not in scope
//!
//! - RAP zoned joint placement-routing ([RAP] / issue #107) — use
//!   [`crate::zoned::schedule_zoned`] instead.
//! - Atomique’s flat 300 µs stage cost — duration uses shared √-law helpers
//!   [`crate::zoned::movement_duration_us`] / [`crate::zoned::euclidean_um`].
//!
//! # Transfer policy (B6)
//!
//! | Mode | Params | Transfers / moved atom |
//! | --- | --- | --- |
//! | **Quon reuse (default)** | `return_home=false`, `transfers_per_moved_atom=2` | SLM→AOD load + AOD→SLM store onto pair; atoms stay; B8/B13 between layers |
//! | **Enola-comparable** | `return_home=true`, `transfers_per_moved_atom=4` | load/store to pair, then load/store home each layer |
//!
//! Do **not** document the default as Enola Sec. 2/6.1.
//!
//! # Geometry / legality
//!
//! - Bank origin (B14): `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`.
//! - Geometry-gated skip (B10): skip moves only when partners ≤ `rydberg_range_um`
//!   **and** full R1–R3 over all occupied atoms would pass.
//! - R1–R3 (B11): dialect-identical `≤` reject predicates; scope = all occupied atoms.
//!   Idle spectators on dense `#104` grids are moved to isolated parking sites
//!   (below the placement bbox) before each entangle so R2/R3 can pass.
//! - Conflict oracle (B12): Enola three types + M3 dest separation
//!   (`|dst_i − dst_j| < min_row_col_separation_um`).
//! - AOD indices (B4): dense row/col overlay from unique site coordinates;
//!   MoveSpec row/col follow the **source** site for the move duration.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::layout::{
    AodTrapRef, AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
use crate::schedule::{
    AtomMove, MovementGroup, NeutralAtomAction, ScheduleLayer, TransferDirection, TrapTransfer,
};
use crate::schedule_entry::GraphScheduleRequest;
use crate::zoned::{euclidean_um, movement_duration_us};

/// Extra µm beyond `pair_pitch_um` so placement↔bank edge is dialect-strict `>` min (B14).
pub const BANK_ISOLATION_EPS_UM: f64 = 0.01;

const POS_EPS_UM: f64 = 1e-6;

/// Parameters for flat AOD movement (#106).
///
/// Sourced from NeutralAtomTarget JSON by callers; kept crate-local to avoid a
/// backend dependency (same pattern as #105).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementParams {
    pub acceleration_m_s2: f64,
    pub trap_transfer_us: u64,
    pub rydberg_range_um: f64,
    /// R3 isolation: non-partners must be farther than this (generic_rna: 18.75).
    pub min_rydberg_spacing_um: f64,
    pub min_row_col_separation_um: f64,
    pub aod_rows: u32,
    pub aod_cols: u32,
    pub num_aods: u32,
    /// Intra-pair gap (µm); must satisfy `0 < pair_gap_um ≤ rydberg_range_um`.
    pub pair_gap_um: f64,
    /// Center-to-center pitch between distinct interaction pairs.
    /// May equal `min_rydberg_spacing_um`; bank origin adds [`BANK_ISOLATION_EPS_UM`] (B14).
    pub pair_pitch_um: f64,
    /// Quon reuse default: 2 transfers per moved atom. Not Enola Sec. 2/6.1.
    pub transfers_per_moved_atom: u32,
    /// When true, return bank occupants home after each entangle (Enola-comparable 4-xfer).
    pub return_home: bool,
}

impl MovementParams {
    /// Defaults matching `targets/neutral_atom/generic_rna_v0.json` / architecture §8.6.
    pub fn generic_rna_v0() -> Self {
        Self {
            acceleration_m_s2: 2750.0,
            trap_transfer_us: 15,
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            min_row_col_separation_um: 2.0,
            aod_rows: 100,
            aod_cols: 100,
            num_aods: 1,
            pair_gap_um: 2.0,
            pair_pitch_um: 18.75,
            transfers_per_moved_atom: 2,
            return_home: false,
        }
    }

    fn validate(&self) -> Result<(), MovementPlanError> {
        if self.acceleration_m_s2 <= 0.0 {
            return Err(MovementPlanError::InvalidAcceleration(
                self.acceleration_m_s2,
            ));
        }
        if self.rydberg_range_um <= 0.0 {
            return Err(MovementPlanError::InvalidRydbergRange(
                self.rydberg_range_um,
            ));
        }
        if self.min_rydberg_spacing_um <= 0.0 {
            return Err(MovementPlanError::InvalidMinRydbergSpacing(
                self.min_rydberg_spacing_um,
            ));
        }
        if self.min_row_col_separation_um <= 0.0 {
            return Err(MovementPlanError::InvalidMinRowColSeparation(
                self.min_row_col_separation_um,
            ));
        }
        if self.pair_gap_um <= 0.0 || self.pair_gap_um > self.rydberg_range_um {
            return Err(MovementPlanError::InvalidPairGap {
                gap: self.pair_gap_um,
                rb: self.rydberg_range_um,
            });
        }
        if self.pair_pitch_um < self.min_rydberg_spacing_um {
            return Err(MovementPlanError::InvalidPairPitch {
                pitch: self.pair_pitch_um,
                min: self.min_rydberg_spacing_um,
            });
        }
        if self.pair_pitch_um <= self.pair_gap_um {
            return Err(MovementPlanError::InvalidPairPitch {
                pitch: self.pair_pitch_um,
                min: self.pair_gap_um,
            });
        }
        if self.num_aods == 0 {
            return Err(MovementPlanError::AodCapacity {
                axis: "aods",
                needed: 1,
                limit: 0,
            });
        }
        Ok(())
    }
}

/// Result of [`plan_aod_movement`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementPlanResult {
    pub request: GraphScheduleRequest,
    pub rearrangement_steps: u64,
    pub rearrangement_time_us: u64,
    pub trap_transfers: u64,
    pub transfer_time_us: u64,
    pub skipped_already_adjacent: u64,
}

/// Errors from flat AOD movement planning.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum MovementPlanError {
    #[error("layout is required for AOD movement planning")]
    MissingLayout,
    #[error("no entangling layers to plan movement for")]
    EmptySchedule,
    #[error("acceleration_m_s2 must be positive, got {0}")]
    InvalidAcceleration(f64),
    #[error("rydberg_range_um must be positive, got {0}")]
    InvalidRydbergRange(f64),
    #[error("min_rydberg_spacing_um must be positive, got {0}")]
    InvalidMinRydbergSpacing(f64),
    #[error("min_row_col_separation_um must be positive, got {0}")]
    InvalidMinRowColSeparation(f64),
    #[error("pair_gap_um ({gap}) must be in (0, rydberg_range_um={rb}]")]
    InvalidPairGap { gap: f64, rb: f64 },
    #[error("pair_pitch_um ({pitch}) must be >= min_rydberg_spacing_um ({min})")]
    InvalidPairPitch { pitch: f64, min: f64 },
    #[error("missing site or binding for atom {0:?}")]
    MissingAtom(AtomId),
    #[error("collision: site {site:?} claimed by multiple atoms in cycle {cycle}")]
    Collision { cycle: u32, site: SiteId },
    #[error("transfer into occupied site {site:?} at cycle {cycle}")]
    TransferIntoOccupied { cycle: u32, site: SiteId },
    #[error("AOD capacity exceeded: need {needed} {axis} but target allows {limit}")]
    AodCapacity {
        axis: &'static str,
        needed: u32,
        limit: u32,
    },
    #[error("unsatisfiable move set under AOD conflicts for layer cycle {0}")]
    Unsatisfiable(u32),
    #[error("no free interaction pair for gate ({lhs:?}, {rhs:?}) at cycle {cycle}")]
    NoInteractionPair {
        cycle: u32,
        lhs: AtomId,
        rhs: AtomId,
    },
    #[error("entangling geometry violation (R1–R3) at cycle {cycle}: {detail}")]
    EntanglingGeometry { cycle: u32, detail: String },
    #[error("multi-qubit entangle (k={k}) unsupported by flat movement planner at cycle {cycle}")]
    UnsupportedEntangleArity { cycle: u32, k: usize },
    #[error("schedule layer conflict: {0}")]
    Conflict(String),
    #[error("missing site {0:?} in layout")]
    MissingSite(SiteId),
}

/// One reserved interaction pair: two empty sites with gap ≤ r_b.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionPair {
    pub left: SiteId,
    pub right: SiteId,
}

/// Dialect-aligned move payload for verifier tests (MLIR-free).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveSpec {
    pub atom: u32,
    pub from_site: u32,
    pub to_site: u32,
    pub aod_id: u32,
    pub row: u32,
    pub col: u32,
    pub from_x_um: f64,
    pub from_y_um: f64,
    pub to_x_um: f64,
    pub to_y_um: f64,
}

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

/// Isolated parking sites for spectators (idle atoms) so dense `#104` placement
/// does not leave R2/R3 violations among non-partners at entangle time (B11).
///
/// Sites are placed below the placement bbox at pitch `pair_pitch_um + BANK_ISOLATION_EPS_UM`
/// so every parking↔parking and parking↔bank/placement distance is dialect-strict `>` min.
fn ensure_parking_sites(
    layout: &mut NeutralAtomLayout,
    params: &MovementParams,
    count: usize,
) -> Result<Vec<SiteId>, MovementPlanError> {
    params.validate()?;
    let count = count.max(1);
    let pitch = params.pair_pitch_um + BANK_ISOLATION_EPS_UM;
    // Detect existing parking: sites tagged by y below placement min_y - pitch/2
    // with x spaced by pitch. Simpler: append fresh each plan if not enough.
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut any = false;
    for b in &layout.initial_bindings {
        let site = match b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        if let Some(s) = layout.sites.iter().find(|x| x.id == site) {
            any = true;
            min_y = min_y.min(s.position.y_um);
            max_x = max_x.max(s.position.x_um);
        }
    }
    if !any {
        min_y = 0.0;
        max_x = 0.0;
    }
    let y0 = min_y - pitch;
    let x0 = 0.0;

    // Reuse existing parking sites if present (same y0 row).
    let mut existing: Vec<SiteId> = layout
        .sites
        .iter()
        .filter(|s| (s.position.y_um - y0).abs() < POS_EPS_UM)
        .map(|s| s.id)
        .collect();
    existing.sort();
    if existing.len() >= count {
        return Ok(existing.into_iter().take(count).collect());
    }

    let start_id = match layout.sites.iter().map(|s| s.id.0).max() {
        Some(max_id) => max_id.saturating_add(1),
        None => 0,
    };
    let start = existing.len();
    for (offset, i) in (start..count).enumerate() {
        let id = SiteId(start_id + offset as u32);
        layout.sites.push(AtomSite {
            id,
            position: Position {
                x_um: x0 + (i as f64) * pitch,
                y_um: y0,
            },
        });
        existing.push(id);
    }
    let _ = max_x;
    Ok(existing)
}

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
    let n_atoms = layout.initial_bindings.len();
    let pairs = ensure_interaction_pairs(layout, params, w_max.max(1))?;
    let parking = ensure_parking_sites(layout, params, n_atoms.max(1))?;
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

    // Rebuild maps after bank + parking append.
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
            let free_pairs = free_interaction_pairs(&pairs, &occ_site);
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

        // Park spectators so dense #104 idle atoms do not violate R2/R3 (B11).
        {
            let gate_atoms = gate_atom_set(&gates);
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
            planner.park_spectators(&gate_atoms, &parking, layer.cycle)?;
        }

        // R1–R3 before entangle (B11).
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

/// Lower schedule atom moves to dialect-shaped [`MoveSpec`]s (B4).
pub fn atom_moves_to_move_specs(
    moves: &[AtomMove],
    layout: &NeutralAtomLayout,
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

/// R1–R3 check with dialect-identical `≤` predicates over **all occupied** atoms (B11).
pub fn check_entangling_geometry(
    cycle: u32,
    partner_gates: &[(AtomId, AtomId)],
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
    params: &MovementParams,
) -> Result<(), MovementPlanError> {
    let partner_pairs: BTreeSet<(AtomId, AtomId)> = partner_gates
        .iter()
        .map(|&(a, b)| ordered_pair(a, b))
        .collect();

    let mut atoms: Vec<(AtomId, Position)> = Vec::new();
    for (&atom, &site) in occ_atom {
        let pos = site_pos
            .get(&site)
            .copied()
            .ok_or(MovementPlanError::MissingSite(site))?;
        atoms.push((atom, pos));
    }
    atoms.sort_by_key(|(a, _)| *a);

    for &(a, b) in partner_gates {
        let pa = pos_of(a, occ_atom, site_pos)?;
        let pb = pos_of(b, occ_atom, site_pos)?;
        let d = euclidean_um(pa, pb);
        if d > params.rydberg_range_um {
            return Err(MovementPlanError::EntanglingGeometry {
                cycle,
                detail: format!(
                    "R1: partners {:?}–{:?} distance {d} > rydberg_range_um {}",
                    a, b, params.rydberg_range_um
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
            if d <= params.rydberg_range_um {
                return Err(MovementPlanError::EntanglingGeometry {
                    cycle,
                    detail: format!(
                        "R2: non-partners {:?}–{:?} distance {d} ≤ rydberg_range_um {}",
                        a, b, params.rydberg_range_um
                    ),
                });
            }
            if d <= params.min_rydberg_spacing_um {
                return Err(MovementPlanError::EntanglingGeometry {
                    cycle,
                    detail: format!(
                        "R3: non-partners {:?}–{:?} distance {d} ≤ min_rydberg_spacing_um {}",
                        a, b, params.min_rydberg_spacing_um
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Whether two candidate legs conflict under Enola three types + M3 separation (B12).
pub fn legs_conflict(a: &CandidateLeg, b: &CandidateLeg, min_sep_um: f64) -> bool {
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

/// One planned leg with coordinates and AOD indices (planner-internal).
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateLeg {
    pub atom: AtomId,
    pub from: SiteId,
    pub to: SiteId,
    pub from_pos: Position,
    pub to_pos: Position,
    pub aod_id: u32,
    pub row: u32,
    pub col: u32,
    pub dual_id: u64,
    pub distance_um: f64,
}

// --- internals ---------------------------------------------------------------

#[derive(Clone, Copy)]
enum Axis {
    Row,
    Column,
}

#[derive(Clone, Debug)]
struct DualCandidate {
    gate: (AtomId, AtomId),
    pair: InteractionPair,
    legs: Vec<CandidateLeg>,
    distance_um: f64,
}

struct AodGrid {
    site_row_col: BTreeMap<SiteId, (u32, u32)>,
}

impl AodGrid {
    fn from_layout(layout: &NeutralAtomLayout) -> Self {
        let mut ys: BTreeSet<OrderedF64> = BTreeSet::new();
        let mut xs: BTreeSet<OrderedF64> = BTreeSet::new();
        for s in &layout.sites {
            ys.insert(OrderedF64(s.position.y_um));
            xs.insert(OrderedF64(s.position.x_um));
        }
        let row_of_y: BTreeMap<OrderedF64, u32> =
            ys.iter().enumerate().map(|(i, y)| (*y, i as u32)).collect();
        let col_of_x: BTreeMap<OrderedF64, u32> =
            xs.iter().enumerate().map(|(i, x)| (*x, i as u32)).collect();
        let mut site_row_col = BTreeMap::new();
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

    fn indices(&self, site: SiteId) -> Result<(u32, u32), MovementPlanError> {
        self.site_row_col
            .get(&site)
            .copied()
            .ok_or(MovementPlanError::MissingSite(site))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OrderedF64(f64);

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

struct EmitCtx<'a> {
    params: &'a MovementParams,
    site_pos: &'a BTreeMap<SiteId, Position>,
    aod_grid: &'a AodGrid,
    home: &'a BTreeMap<AtomId, SiteId>,
    pair_sites: &'a BTreeSet<SiteId>,
    pairs: &'a [InteractionPair],
    occ_site: &'a mut BTreeMap<SiteId, AtomId>,
    occ_atom: &'a mut BTreeMap<AtomId, SiteId>,
    out_layers: &'a mut Vec<ScheduleLayer>,
    next_cycle: &'a mut u32,
    rearrangement_steps: &'a mut u64,
    rearrangement_time_us: &'a mut u64,
    trap_transfers: &'a mut u64,
    transfer_time_us: &'a mut u64,
}

impl EmitCtx<'_> {
    fn reclaim_and_evict(
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
            let free = free_interaction_pairs(self.pairs, self.occ_site).len();
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

    fn return_all_bank_home(&mut self, cycle: u32) -> Result<(), MovementPlanError> {
        for &pair in self.pairs {
            if pair_occupants(pair, self.occ_site).is_some() {
                self.vacate_pair_home(pair, cycle)?;
            }
        }
        Ok(())
    }

    /// Move non-gate atoms onto isolated parking sites when their current
    /// positions would fail R2/R3 against partners or each other (B11).
    fn park_spectators(
        &mut self,
        gate_atoms: &BTreeSet<AtomId>,
        parking: &[SiteId],
        cycle: u32,
    ) -> Result<(), MovementPlanError> {
        let spectators: Vec<AtomId> = self
            .occ_atom
            .keys()
            .copied()
            .filter(|a| !gate_atoms.contains(a))
            .collect();
        if spectators.is_empty() {
            return Ok(());
        }

        // Check whether current geometry already passes for gate partners alone
        // with spectators in place — if yes, nothing to do.
        let gates: Vec<(AtomId, AtomId)> = {
            // Reconstruct partner pairs from gate_atoms is incomplete; caller
            // already will check. Park if any spectator is within min spacing
            // of any occupied atom that is a gate atom, or of another spectator.
            let mut need = false;
            for &s in &spectators {
                let ps = pos_of(s, self.occ_atom, self.site_pos)?;
                for (&other, _) in self.occ_atom.iter() {
                    if other == s {
                        continue;
                    }
                    let po = pos_of(other, self.occ_atom, self.site_pos)?;
                    let d = euclidean_um(ps, po);
                    if d <= self.params.min_rydberg_spacing_um {
                        need = true;
                        break;
                    }
                }
                if need {
                    break;
                }
            }
            if !need {
                return Ok(());
            }
            Vec::new()
        };
        let _ = gates;

        let mut free_parking: Vec<SiteId> = parking
            .iter()
            .copied()
            .filter(|s| !self.occ_site.contains_key(s))
            .collect();
        // Also free parking currently held by spectators we will move (reuse).
        for &atom in &spectators {
            if let Some(&site) = self.occ_atom.get(&atom)
                && parking.contains(&site)
            {
                // Already parked — keep if isolated enough.
                let ps = pos_of(atom, self.occ_atom, self.site_pos)?;
                let mut ok = true;
                for (&other, _) in self.occ_atom.iter() {
                    if other == atom {
                        continue;
                    }
                    let po = pos_of(other, self.occ_atom, self.site_pos)?;
                    if euclidean_um(ps, po) <= self.params.min_rydberg_spacing_um {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    continue;
                }
            }
        }

        let mut legs = Vec::new();
        for &atom in &spectators {
            let from = *self
                .occ_atom
                .get(&atom)
                .ok_or(MovementPlanError::MissingAtom(atom))?;
            // Skip if already sufficiently isolated.
            {
                let ps = pos_of(atom, self.occ_atom, self.site_pos)?;
                let mut ok = true;
                for (&other, _) in self.occ_atom.iter() {
                    if other == atom {
                        continue;
                    }
                    let po = pos_of(other, self.occ_atom, self.site_pos)?;
                    if euclidean_um(ps, po) <= self.params.min_rydberg_spacing_um {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    continue;
                }
            }
            let Some(to) = free_parking.pop() else {
                return Err(MovementPlanError::EntanglingGeometry {
                    cycle,
                    detail: "insufficient parking sites for spectators (R2/R3)".into(),
                });
            };
            if to == from {
                continue;
            }
            let from_pos = *self
                .site_pos
                .get(&from)
                .ok_or(MovementPlanError::MissingSite(from))?;
            let to_pos = *self
                .site_pos
                .get(&to)
                .ok_or(MovementPlanError::MissingSite(to))?;
            let (row, col) = self.aod_grid.indices(from)?;
            legs.push(CandidateLeg {
                atom,
                from,
                to,
                from_pos,
                to_pos,
                aod_id: 0,
                row,
                col,
                dual_id: u64::MAX,
                distance_um: euclidean_um(from_pos, to_pos),
            });
        }
        if !legs.is_empty() {
            self.emit_legs_packed(legs, cycle)?;
        }
        Ok(())
    }

    fn emit_duals_b7(
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

    fn emit_load_move_store(
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

fn layout_from_sites(site_pos: &BTreeMap<SiteId, Position>) -> NeutralAtomLayout {
    NeutralAtomLayout {
        sites: site_pos
            .iter()
            .map(|(&id, &position)| AtomSite { id, position })
            .collect(),
        initial_bindings: Vec::new(),
    }
}

fn pack_legs_greedy(
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

fn select_duals_sortis(duals: &[DualCandidate]) -> Vec<DualCandidate> {
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
        let g = ordered_pair(d.gate.0, d.gate.1);
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

fn generate_duals(
    gates: &[(AtomId, AtomId)],
    free_pairs: &[InteractionPair],
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
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

fn can_skip_geometry_gated(
    a: AtomId,
    b: AtomId,
    layer_gates: &[(AtomId, AtomId)],
    occ_atom: &BTreeMap<AtomId, SiteId>,
    site_pos: &BTreeMap<SiteId, Position>,
    params: &MovementParams,
) -> Result<bool, MovementPlanError> {
    let pa = pos_of(a, occ_atom, site_pos)?;
    let pb = pos_of(b, occ_atom, site_pos)?;
    if euclidean_um(pa, pb) > params.rydberg_range_um {
        return Ok(false);
    }
    // Provisional: only this gate as partner among layer gates that are also skippable?
    // B10/B11: check geometry for the prospective entangle layer with zero moves.
    // Use all layer gates that are currently within r_b as partners; if any fail, no skip.
    // Locked: check with partner set = all gates in the layer that are distance-adjacent
    // OR just the gates being considered for in-place entangle.
    // Plan: "with this gate's partners marked as the only allowed ≤ r_b pair among the
    // layer's entangle set" for the skip check of this gate — but also "full R1–R3 over
    // all occupied atoms would pass without any moves".
    // For skip of gate (a,b): partner_pairs = {(a,b)} only for the provisional check
    // of whether THIS gate can entangle in place. Idle atoms still in R2/R3 scope.
    // Other layer gates that also want in-place must be checked separately.
    match check_entangling_geometry(0, &[(a, b)], occ_atom, site_pos, params) {
        Ok(()) => {
            // Also: if other layer gates are distance-adjacent and would entangle in same
            // layer, partners must include them for a joint check. For skip decision of
            // one gate, plan says check would pass without moves for the prospective
            // entangle layer. Collect all gates that are currently distance-adjacent.
            let mut in_place = Vec::new();
            for &(x, y) in layer_gates {
                let px = pos_of(x, occ_atom, site_pos)?;
                let py = pos_of(y, occ_atom, site_pos)?;
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
        // Same-atom reuse on a pair with B10 skip.
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

fn pair_occupants(
    pair: InteractionPair,
    occ_site: &BTreeMap<SiteId, AtomId>,
) -> Option<(AtomId, AtomId)> {
    let a = occ_site.get(&pair.left).copied()?;
    let b = occ_site.get(&pair.right).copied()?;
    Some((a, b))
}

fn free_interaction_pairs(
    pairs: &[InteractionPair],
    occ_site: &BTreeMap<SiteId, AtomId>,
) -> Vec<InteractionPair> {
    pairs
        .iter()
        .copied()
        .filter(|p| !occ_site.contains_key(&p.left) && !occ_site.contains_key(&p.right))
        .collect()
}

fn bank_origin(layout: &NeutralAtomLayout, params: &MovementParams) -> (f64, f64) {
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

fn site_position_map(
    layout: &NeutralAtomLayout,
) -> Result<BTreeMap<SiteId, Position>, MovementPlanError> {
    let mut m = BTreeMap::new();
    for s in &layout.sites {
        m.insert(s.id, s.position);
    }
    Ok(m)
}

fn pos_of(
    atom: AtomId,
    occ_atom: &BTreeMap<AtomId, SiteId>,
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

fn ordered_pair(a: AtomId, b: AtomId) -> (AtomId, AtomId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn layers_have_entangle(layers: &[ScheduleLayer]) -> bool {
    layers.iter().any(layer_has_entangle)
}

fn layer_has_entangle(layer: &ScheduleLayer) -> bool {
    layer.actions.iter().any(|a| {
        matches!(
            a,
            NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
        )
    })
}

fn layer_has_entangle_n(layer: &ScheduleLayer) -> bool {
    layer
        .actions
        .iter()
        .any(|a| matches!(a, NeutralAtomAction::EntangleN { .. }))
}

fn max_entangle2_width(layers: &[ScheduleLayer]) -> usize {
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

fn collect_entangle2(layer: &ScheduleLayer) -> Result<Vec<(AtomId, AtomId)>, MovementPlanError> {
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

fn gate_atom_set(gates: &[(AtomId, AtomId)]) -> BTreeSet<AtomId> {
    let mut s = BTreeSet::new();
    for &(a, b) in gates {
        s.insert(a);
        s.insert(b);
    }
    s
}

fn axis_conflict(a: &CandidateLeg, b: &CandidateLeg, axis: Axis, min_sep_um: f64) -> bool {
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
