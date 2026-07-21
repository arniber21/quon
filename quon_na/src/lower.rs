//! Lowering from the planners' [`ScheduleLayer`] representation to the
//! canonical `quantum.na` [`ScheduleSpec`] (ADR-0011, issue #167).
//!
//! Per ADR-0011 this module is the **only** place allowed to translate between
//! the planner schedule types ([`crate::schedule`]) and the dialect spec types
//! ([`crate::dialect`]) — including the two `TransferDirection` enums.
//!
//! The planners rewrite `layout.initial_bindings` to the **final** occupancy
//! (both [`crate::zoned::schedule_zoned`] and
//! [`crate::movement::plan_aod_movement`] do this), so per-layer atom
//! positions are reconstructed by replaying `Move` actions backwards from the
//! final bindings. Trap transfers are in-place (the atom stays at its site)
//! and do not affect occupancy; a forward pass over transfers recovers the
//! AOD trap metadata each grouped move needs, and transfers in turn take the
//! associated move's final (possibly overlaid) trap ref so the emitted IR is
//! self-consistent.
//!
//! The lowering is total on planner-produced schedules: every failure mode is
//! a typed [`ScheduleLowerError`], never a panic.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::dialect::{
    ActionSpec, EntanglePairSpec, LayerSpec, MoveSpec, PositionedAtom, ScheduleSpec,
    TransferDirection as SpecTransferDirection, TransferSpec,
};
use crate::layout::{AodTrapRef, AtomId, NeutralAtomLayout, Position, SiteId, TrapBinding};
use crate::schedule::{
    LocalGateKind, MeasurementBasis, NeutralAtomAction, ScheduleLayer, TransferDirection,
    TrapTransfer,
};
use crate::schedule_entry::GraphScheduleRequest;

/// Target-derived header attributes for the `quantum.na.schedule` op.
///
/// These mirror the legality limits the dialect verifier enforces
/// (`rydberg_range_um`, `min_rydberg_spacing_um`, `aod_min_separation_um`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleLowerParams {
    pub target_id: String,
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub aod_min_separation_um: f64,
}

impl ScheduleLowerParams {
    /// Derive the schedule header from a loaded neutral-atom target.
    ///
    /// `target_id` is passed separately because [`backend::NeutralAtomTarget`]
    /// does not carry the descriptor id (`BackendTarget.id` does).
    pub fn from_target(target_id: impl Into<String>, na: &backend::NeutralAtomTarget) -> Self {
        Self {
            target_id: target_id.into(),
            rydberg_range_um: na.interaction.rydberg_range_um,
            min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
            aod_min_separation_um: na.movement.min_row_col_separation_um,
        }
    }
}

/// A schedule that cannot be lowered to `quantum.na`.
///
/// Every variant indicates either a planner bug (inconsistent occupancy) or a
/// schedule shape the dialect cannot represent (`EntangleN` with arity > 2).
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ScheduleLowerError {
    #[error("schedule has no layout; run placement / zoned scheduling first")]
    MissingLayout,
    #[error("cycle {cycle}: site {site} is not in the layout")]
    MissingSite { cycle: u32, site: u32 },
    #[error("layout defines site {site} more than once")]
    DuplicateSite { site: u32 },
    #[error("cycle {cycle}: atom {atom} has no occupancy (not in final bindings or any move)")]
    UnboundAtom { cycle: u32, atom: u32 },
    #[error(
        "cycle {cycle}: move of atom {atom} to site {to} is inconsistent with downstream occupancy at site {found}"
    )]
    InconsistentMove {
        cycle: u32,
        atom: u32,
        to: u32,
        found: u32,
    },
    #[error("cycle {cycle}: atom {atom} moves without a preceding slm_to_aod transfer")]
    MissingAodBinding { cycle: u32, atom: u32 },
    #[error("cycle {cycle}: EntangleN with {arity} atoms is not representable as quantum.na pairs")]
    UnsupportedEntangleN { cycle: u32, arity: usize },
}

/// Lower a planned schedule request (layers + layout) to the canonical
/// `quantum.na` [`ScheduleSpec`].
///
/// This is the production glue between the planner pipeline
/// ([`crate::pipeline::run_from_module`] / [`crate::pipeline::run_from_graph`])
/// and [`crate::dialect::dump_schedule_text`].
pub fn lower_schedule(
    request: &GraphScheduleRequest,
    params: &ScheduleLowerParams,
) -> Result<ScheduleSpec, ScheduleLowerError> {
    let layout = request
        .layout
        .as_ref()
        .ok_or(ScheduleLowerError::MissingLayout)?;
    lower_layers(&request.layers, layout, params)
}

/// Lower schedule layers against a layout whose `initial_bindings` reflect
/// the **final** occupancy (the state planners leave the layout in).
pub fn lower_layers(
    layers: &[ScheduleLayer],
    layout: &NeutralAtomLayout,
    params: &ScheduleLowerParams,
) -> Result<ScheduleSpec, ScheduleLowerError> {
    let site_pos = site_positions(layout)?;
    let occupancy_after = replay_occupancy(layers, layout)?;
    let move_specs = lower_move_groups(layers, &site_pos)?;
    let transfer_refs = transfer_refs_from_moves(layers, &move_specs);

    let mut layer_specs = Vec::with_capacity(layers.len());
    for (layer_index, layer) in layers.iter().enumerate() {
        let mut actions = Vec::with_capacity(layer.actions.len());
        for (action_index, action) in layer.actions.iter().enumerate() {
            actions.push(lower_action(
                action,
                layer.cycle,
                &site_pos,
                &occupancy_after[layer_index],
                move_specs.get(&(layer_index, action_index)),
                transfer_refs.get(&(layer_index, action_index)),
            )?);
        }
        layer_specs.push(LayerSpec {
            cycle: layer.cycle,
            actions,
        });
    }

    Ok(ScheduleSpec {
        target_id: params.target_id.clone(),
        rydberg_range_um: params.rydberg_range_um,
        min_rydberg_spacing_um: params.min_rydberg_spacing_um,
        aod_min_separation_um: params.aod_min_separation_um,
        layers: layer_specs,
    })
}

/// Build the final (overlaid) [`MoveSpec`]s for every `Move` action, keyed by
/// `(layer index, action index)`.
///
/// A forward pass over trap transfers recovers each moved atom's AOD trap
/// (transfers take effect for later layers: an atom cannot both transfer and
/// move in the same layer, per occupancy validation), then degenerate
/// placeholder refs are replaced per group by
/// [`overlay_degenerate_aod_indices`].
fn lower_move_groups(
    layers: &[ScheduleLayer],
    site_pos: &BTreeMap<SiteId, Position>,
) -> Result<BTreeMap<(usize, usize), Vec<MoveSpec>>, ScheduleLowerError> {
    let mut aod_bindings: BTreeMap<AtomId, AodTrapRef> = BTreeMap::new();
    let mut move_specs = BTreeMap::new();

    for (layer_index, layer) in layers.iter().enumerate() {
        for (action_index, action) in layer.actions.iter().enumerate() {
            let NeutralAtomAction::Move(group) = action else {
                continue;
            };
            let mut moves = Vec::with_capacity(group.moves.len());
            for atom_move in &group.moves {
                let from = position(site_pos, layer.cycle, atom_move.from)?;
                let to = position(site_pos, layer.cycle, atom_move.to)?;
                let aod = aod_bindings.get(&atom_move.atom).ok_or(
                    ScheduleLowerError::MissingAodBinding {
                        cycle: layer.cycle,
                        atom: atom_move.atom.0,
                    },
                )?;
                moves.push(MoveSpec {
                    atom: atom_move.atom.0,
                    from_site: atom_move.from.0,
                    to_site: atom_move.to.0,
                    aod_id: aod.aod_id,
                    row: aod.row,
                    col: aod.col,
                    from_x_um: from.x_um,
                    from_y_um: from.y_um,
                    to_x_um: to.x_um,
                    to_y_um: to.y_um,
                });
            }
            overlay_degenerate_aod_indices(&mut moves);
            move_specs.insert((layer_index, action_index), moves);
        }
        apply_transfers(layer, &mut aod_bindings);
    }
    Ok(move_specs)
}

/// Compute the AOD trap ref each `Transfer` action must carry so the emitted
/// IR is self-consistent with its move ops, keyed by
/// `(layer index, action index)`.
///
/// [`overlay_degenerate_aod_indices`] may rewrite a grouped move's placeholder
/// refs, so the surrounding transfers must follow: a `slm_to_aod` load takes
/// the ref of the atom's **next** move, an `aod_to_slm` store the ref of its
/// **previous** move (that is the trap the atom actually occupied). Transfers
/// with no associated move keep the planner's ref.
fn transfer_refs_from_moves(
    layers: &[ScheduleLayer],
    move_specs: &BTreeMap<(usize, usize), Vec<MoveSpec>>,
) -> BTreeMap<(usize, usize), AodTrapRef> {
    let mut transfer_refs = BTreeMap::new();

    // Backward pass: loads take the ref of the atom's next move.
    let mut next_move_ref: BTreeMap<u32, AodTrapRef> = BTreeMap::new();
    for (layer_index, layer) in layers.iter().enumerate().rev() {
        for (action_index, action) in layer.actions.iter().enumerate().rev() {
            match action {
                NeutralAtomAction::Move(_) => {
                    if let Some(moves) = move_specs.get(&(layer_index, action_index)) {
                        for spec in moves {
                            next_move_ref.insert(spec.atom, move_spec_ref(spec));
                        }
                    }
                }
                NeutralAtomAction::Transfer(transfer)
                    if transfer.direction == TransferDirection::SlmToAod =>
                {
                    if let Some(aod) = next_move_ref.get(&transfer.atom.0) {
                        transfer_refs.insert((layer_index, action_index), aod.clone());
                    }
                }
                _ => {}
            }
        }
    }

    // Forward pass: stores take the ref of the atom's previous move.
    let mut prev_move_ref: BTreeMap<u32, AodTrapRef> = BTreeMap::new();
    for (layer_index, layer) in layers.iter().enumerate() {
        for (action_index, action) in layer.actions.iter().enumerate() {
            match action {
                NeutralAtomAction::Move(_) => {
                    if let Some(moves) = move_specs.get(&(layer_index, action_index)) {
                        for spec in moves {
                            prev_move_ref.insert(spec.atom, move_spec_ref(spec));
                        }
                    }
                }
                NeutralAtomAction::Transfer(transfer)
                    if transfer.direction == TransferDirection::AodToSlm =>
                {
                    if let Some(aod) = prev_move_ref.get(&transfer.atom.0) {
                        transfer_refs.insert((layer_index, action_index), aod.clone());
                    }
                }
                _ => {}
            }
        }
    }

    transfer_refs
}

fn move_spec_ref(spec: &MoveSpec) -> AodTrapRef {
    AodTrapRef {
        aod_id: spec.aod_id,
        row: spec.row,
        col: spec.col,
    }
}

fn lower_action(
    action: &NeutralAtomAction,
    cycle: u32,
    site_pos: &BTreeMap<SiteId, Position>,
    occupancy: &BTreeMap<AtomId, SiteId>,
    move_specs: Option<&Vec<MoveSpec>>,
    transfer_ref: Option<&AodTrapRef>,
) -> Result<ActionSpec, ScheduleLowerError> {
    match action {
        NeutralAtomAction::Move(group) => {
            // Precomputed by `lower_move_groups`; a Move action always has an
            // entry, but fall back to an empty group rather than panic.
            let moves = move_specs.cloned().unwrap_or_default();
            Ok(ActionSpec::Move {
                moves,
                duration_us: group.duration_us,
            })
        }
        NeutralAtomAction::Transfer(transfer) => {
            Ok(ActionSpec::Transfer(lower_transfer(transfer, transfer_ref)))
        }
        NeutralAtomAction::Entangle2 { atoms, duration_us } => {
            let lhs = positioned_atom(site_pos, occupancy, cycle, atoms[0])?;
            let rhs = positioned_atom(site_pos, occupancy, cycle, atoms[1])?;
            Ok(ActionSpec::Entangle {
                pairs: vec![EntanglePairSpec { lhs, rhs }],
                duration_us: *duration_us,
            })
        }
        NeutralAtomAction::EntangleN { atoms, duration_us } => {
            // The dialect entangle op is pairwise; planner backends either
            // rewrite EntangleN (zoned) or reject it (flat AOD), so reaching
            // one here means the schedule bypassed movement planning.
            if let [a, b] = atoms.as_slice() {
                let lhs = positioned_atom(site_pos, occupancy, cycle, *a)?;
                let rhs = positioned_atom(site_pos, occupancy, cycle, *b)?;
                return Ok(ActionSpec::Entangle {
                    pairs: vec![EntanglePairSpec { lhs, rhs }],
                    duration_us: *duration_us,
                });
            }
            Err(ScheduleLowerError::UnsupportedEntangleN {
                cycle,
                arity: atoms.len(),
            })
        }
        NeutralAtomAction::LocalGate {
            atom,
            gate,
            duration_us,
        } => {
            let (theta, phi, lambda) = match *gate {
                LocalGateKind::H => (None, None, None),
                LocalGateKind::Rz(theta) => (Some(theta), None, None),
                LocalGateKind::U3 { theta, phi, lambda } => (Some(theta), Some(phi), Some(lambda)),
            };
            Ok(ActionSpec::LocalGate {
                atom: atom.0,
                gate: gate.as_str().to_string(),
                theta,
                phi,
                lambda,
                duration_us: *duration_us,
            })
        }
        NeutralAtomAction::GlobalRy {
            theta_rad,
            duration_us,
        } => Ok(ActionSpec::GlobalRy {
            theta: *theta_rad,
            duration_us: *duration_us,
        }),
        NeutralAtomAction::Measure {
            atom,
            basis,
            duration_us,
        } => Ok(ActionSpec::Measure {
            atom: atom.0,
            basis: basis_name(*basis).to_string(),
            duration_us: *duration_us,
        }),
        NeutralAtomAction::Reset { atom, duration_us } => Ok(ActionSpec::Reset {
            atom: atom.0,
            duration_us: *duration_us,
        }),
        NeutralAtomAction::Reuse {
            atom,
            region,
            duration_us,
        } => Ok(ActionSpec::Reuse {
            atom: atom.0,
            region: region.map(|r| r.0),
            duration_us: *duration_us,
        }),
        NeutralAtomAction::Wait { duration_us } => Ok(ActionSpec::Wait {
            duration_us: *duration_us,
        }),
    }
}

/// The single allowed mapping between the planner and dialect
/// `TransferDirection` enums (ADR-0011).
///
/// `aod_override` carries the associated move's final (possibly overlaid)
/// trap ref from [`transfer_refs_from_moves`]; without one the planner's ref
/// is kept.
fn lower_transfer(transfer: &TrapTransfer, aod_override: Option<&AodTrapRef>) -> TransferSpec {
    let aod = aod_override.unwrap_or(&transfer.aod);
    TransferSpec {
        atom: transfer.atom.0,
        site: transfer.site.0,
        aod_id: aod.aod_id,
        row: aod.row,
        col: aod.col,
        direction: match transfer.direction {
            TransferDirection::SlmToAod => SpecTransferDirection::SlmToAod,
            TransferDirection::AodToSlm => SpecTransferDirection::AodToSlm,
        },
        duration_us: transfer.duration_us,
    }
}

fn basis_name(basis: MeasurementBasis) -> &'static str {
    match basis {
        MeasurementBasis::X => "x",
        MeasurementBasis::Y => "y",
        MeasurementBasis::Z => "z",
    }
}

/// Replace degenerate planner AOD row/col indices with a dense overlay
/// derived from the moves' source coordinates.
///
/// The zoned planner (RAP, #107) stamps every trap transfer with the
/// placeholder `AodTrapRef { aod_id: 0, row: 0, col: 0 }`, so a grouped move
/// would put all atoms in one AOD row/column and trivially fail the dialect's
/// coupled-motion verifier. When two moves on the same AOD share a (row, col)
/// ref, all of that AOD's indices are recomputed as the rank of the source
/// y/x coordinate — the same "dense row/col overlay from unique site
/// coordinates" convention the flat planner uses (B4 in [`crate::movement`]).
/// Flat-planner refs are unique per atom and are kept as-is.
fn overlay_degenerate_aod_indices(moves: &mut [MoveSpec]) {
    let mut refs_by_aod: BTreeMap<u32, BTreeSet<(u32, u32)>> = BTreeMap::new();
    let mut degenerate: BTreeSet<u32> = BTreeSet::new();
    for spec in moves.iter() {
        if !refs_by_aod
            .entry(spec.aod_id)
            .or_default()
            .insert((spec.row, spec.col))
        {
            degenerate.insert(spec.aod_id);
        }
    }

    for aod_id in degenerate {
        let mut rows: Vec<f64> = Vec::new();
        let mut cols: Vec<f64> = Vec::new();
        for spec in moves.iter().filter(|m| m.aod_id == aod_id) {
            rows.push(spec.from_y_um);
            cols.push(spec.from_x_um);
        }
        rows.sort_by(f64::total_cmp);
        rows.dedup_by(|a, b| a.total_cmp(b).is_eq());
        cols.sort_by(f64::total_cmp);
        cols.dedup_by(|a, b| a.total_cmp(b).is_eq());
        for spec in moves.iter_mut().filter(|m| m.aod_id == aod_id) {
            spec.row = coordinate_rank(&rows, spec.from_y_um);
            spec.col = coordinate_rank(&cols, spec.from_x_um);
        }
    }
}

/// Index of `value` in the sorted, deduplicated coordinate list.
fn coordinate_rank(sorted: &[f64], value: f64) -> u32 {
    sorted
        .iter()
        .position(|c| c.total_cmp(&value).is_eq())
        .unwrap_or(0) as u32
}

fn site_positions(
    layout: &NeutralAtomLayout,
) -> Result<BTreeMap<SiteId, Position>, ScheduleLowerError> {
    let mut site_pos = BTreeMap::new();
    for site in &layout.sites {
        if site_pos.insert(site.id, site.position).is_some() {
            return Err(ScheduleLowerError::DuplicateSite { site: site.id.0 });
        }
    }
    Ok(site_pos)
}

/// Reconstruct atom occupancy at the **end** of each layer by replaying
/// `Move` actions backwards from the final bindings.
fn replay_occupancy(
    layers: &[ScheduleLayer],
    layout: &NeutralAtomLayout,
) -> Result<Vec<BTreeMap<AtomId, SiteId>>, ScheduleLowerError> {
    let mut occupancy: BTreeMap<AtomId, SiteId> = layout
        .initial_bindings
        .iter()
        .map(|binding| {
            let site = match binding.trap {
                TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
            };
            (binding.atom, site)
        })
        .collect();

    let mut occupancy_after = vec![BTreeMap::new(); layers.len()];
    for (index, layer) in layers.iter().enumerate().rev() {
        occupancy_after[index] = occupancy.clone();
        for action in &layer.actions {
            let NeutralAtomAction::Move(group) = action else {
                continue;
            };
            for atom_move in &group.moves {
                match occupancy.insert(atom_move.atom, atom_move.from) {
                    Some(site) if site == atom_move.to => {}
                    Some(site) => {
                        return Err(ScheduleLowerError::InconsistentMove {
                            cycle: layer.cycle,
                            atom: atom_move.atom.0,
                            to: atom_move.to.0,
                            found: site.0,
                        });
                    }
                    None => {
                        return Err(ScheduleLowerError::UnboundAtom {
                            cycle: layer.cycle,
                            atom: atom_move.atom.0,
                        });
                    }
                }
            }
        }
    }
    Ok(occupancy_after)
}

fn apply_transfers(layer: &ScheduleLayer, aod_bindings: &mut BTreeMap<AtomId, AodTrapRef>) {
    for action in &layer.actions {
        let NeutralAtomAction::Transfer(transfer) = action else {
            continue;
        };
        match transfer.direction {
            TransferDirection::SlmToAod => {
                aod_bindings.insert(transfer.atom, transfer.aod.clone());
            }
            TransferDirection::AodToSlm => {
                aod_bindings.remove(&transfer.atom);
            }
        }
    }
}

fn position(
    site_pos: &BTreeMap<SiteId, Position>,
    cycle: u32,
    site: SiteId,
) -> Result<Position, ScheduleLowerError> {
    site_pos
        .get(&site)
        .copied()
        .ok_or(ScheduleLowerError::MissingSite {
            cycle,
            site: site.0,
        })
}

fn positioned_atom(
    site_pos: &BTreeMap<SiteId, Position>,
    occupancy: &BTreeMap<AtomId, SiteId>,
    cycle: u32,
    atom: AtomId,
) -> Result<PositionedAtom, ScheduleLowerError> {
    let site = occupancy
        .get(&atom)
        .copied()
        .ok_or(ScheduleLowerError::UnboundAtom {
            cycle,
            atom: atom.0,
        })?;
    let pos = position(site_pos, cycle, site)?;
    Ok(PositionedAtom {
        atom: atom.0,
        x_um: pos.x_um,
        y_um: pos.y_um,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{AtomBinding, AtomSite};
    use crate::schedule::{AtomMove, MovementGroup};

    fn params() -> ScheduleLowerParams {
        ScheduleLowerParams {
            target_id: "test_target".to_string(),
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            aod_min_separation_um: 2.0,
        }
    }

    fn site(id: u32, x_um: f64, y_um: f64) -> AtomSite {
        AtomSite {
            id: SiteId(id),
            position: Position { x_um, y_um },
        }
    }

    fn slm_binding(atom: u32, site: u32) -> AtomBinding {
        AtomBinding {
            atom: AtomId(atom),
            trap: TrapBinding::Slm { site: SiteId(site) },
        }
    }

    fn load(atom: u32, site: u32) -> NeutralAtomAction {
        NeutralAtomAction::Transfer(TrapTransfer {
            atom: AtomId(atom),
            direction: TransferDirection::SlmToAod,
            site: SiteId(site),
            aod: AodTrapRef {
                aod_id: 0,
                row: 0,
                col: 0,
            },
            duration_us: 15,
        })
    }

    fn store(atom: u32, site: u32) -> NeutralAtomAction {
        NeutralAtomAction::Transfer(TrapTransfer {
            atom: AtomId(atom),
            direction: TransferDirection::AodToSlm,
            site: SiteId(site),
            aod: AodTrapRef {
                aod_id: 0,
                row: 0,
                col: 0,
            },
            duration_us: 15,
        })
    }

    /// Two-atom layout mirroring the zoned Bell shape: storage sites 0/1,
    /// entangle-pair sites 2/3, planner-final bindings at the pair.
    fn bell_layout() -> NeutralAtomLayout {
        NeutralAtomLayout {
            sites: vec![
                site(0, 0.0, 0.0),
                site(1, 4.0, 0.0),
                site(2, 0.0, 310.0),
                site(3, 2.0, 310.0),
            ],
            initial_bindings: vec![slm_binding(0, 2), slm_binding(1, 3)],
        }
    }

    fn bell_layers() -> Vec<ScheduleLayer> {
        vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![load(0, 0), load(1, 1)],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![
                        AtomMove {
                            atom: AtomId(0),
                            from: SiteId(0),
                            to: SiteId(2),
                        },
                        AtomMove {
                            atom: AtomId(1),
                            from: SiteId(1),
                            to: SiteId(3),
                        },
                    ],
                    duration_us: 336,
                })],
            },
            ScheduleLayer {
                cycle: 2,
                actions: vec![store(0, 2), store(1, 3)],
            },
            ScheduleLayer {
                cycle: 3,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [AtomId(0), AtomId(1)],
                    duration_us: 1,
                }],
            },
        ]
    }

    #[test]
    fn lowers_bell_shape_with_reconstructed_positions() {
        let spec = match lower_layers(&bell_layers(), &bell_layout(), &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("bell lowering failed: {error}"),
        };

        assert_eq!(spec.target_id, "test_target");
        assert_eq!(spec.rydberg_range_um, 7.5);
        assert_eq!(spec.layers.len(), 4);
        let cycles: Vec<u32> = spec.layers.iter().map(|l| l.cycle).collect();
        assert_eq!(cycles, vec![0, 1, 2, 3]);

        // Load transfers keep the planner direction and site.
        let ActionSpec::Transfer(load_spec) = &spec.layers[0].actions[0] else {
            panic!("expected transfer, got {:?}", spec.layers[0].actions[0]);
        };
        assert_eq!(load_spec.direction, SpecTransferDirection::SlmToAod);
        assert_eq!(load_spec.site, 0);
        // Placeholder transfer refs follow the move's dense overlay so the
        // emitted IR never claims two atoms in one AOD trap.
        let ActionSpec::Transfer(load_spec_1) = &spec.layers[0].actions[1] else {
            panic!("expected transfer, got {:?}", spec.layers[0].actions[1]);
        };
        assert_eq!((load_spec.row, load_spec.col), (0, 0));
        assert_eq!((load_spec_1.row, load_spec_1.col), (0, 1));

        // The grouped move carries source/destination stage coordinates and a
        // dense AOD overlay (degenerate zoned refs recomputed: same row, two
        // columns ranked by source x).
        let ActionSpec::Move { moves, duration_us } = &spec.layers[1].actions[0] else {
            panic!("expected move, got {:?}", spec.layers[1].actions[0]);
        };
        assert_eq!(*duration_us, 336);
        assert_eq!(moves.len(), 2);
        assert_eq!(
            (moves[0].from_x_um, moves[0].from_y_um),
            (0.0, 0.0),
            "atom 0 source position"
        );
        assert_eq!((moves[0].to_x_um, moves[0].to_y_um), (0.0, 310.0));
        assert_eq!((moves[1].from_x_um, moves[1].from_y_um), (4.0, 0.0));
        assert_eq!((moves[1].to_x_um, moves[1].to_y_um), (2.0, 310.0));
        assert_eq!((moves[0].row, moves[0].col), (0, 0));
        assert_eq!((moves[1].row, moves[1].col), (0, 1));

        // Store transfers map to the dialect direction enum and carry the
        // trap ref the atom occupied during the preceding move.
        let ActionSpec::Transfer(store_spec) = &spec.layers[2].actions[1] else {
            panic!("expected transfer, got {:?}", spec.layers[2].actions[1]);
        };
        assert_eq!(store_spec.direction, SpecTransferDirection::AodToSlm);
        assert_eq!((store_spec.row, store_spec.col), (0, 1));

        // Entangle positions are the post-move pair sites.
        let ActionSpec::Entangle { pairs, duration_us } = &spec.layers[3].actions[0] else {
            panic!("expected entangle, got {:?}", spec.layers[3].actions[0]);
        };
        assert_eq!(*duration_us, 1);
        assert_eq!(
            pairs,
            &vec![EntanglePairSpec {
                lhs: PositionedAtom {
                    atom: 0,
                    x_um: 0.0,
                    y_um: 310.0
                },
                rhs: PositionedAtom {
                    atom: 1,
                    x_um: 2.0,
                    y_um: 310.0
                },
            }]
        );
    }

    #[test]
    fn entangle_before_move_uses_pre_move_positions() {
        // Entangle at cycle 0 while atoms still sit at storage sites; the
        // final bindings reflect the post-move state, so positions must be
        // reconstructed by reverse replay.
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [AtomId(0), AtomId(1)],
                    duration_us: 1,
                }],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![load(0, 0), load(1, 1)],
            },
            ScheduleLayer {
                cycle: 2,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![
                        AtomMove {
                            atom: AtomId(0),
                            from: SiteId(0),
                            to: SiteId(2),
                        },
                        AtomMove {
                            atom: AtomId(1),
                            from: SiteId(1),
                            to: SiteId(3),
                        },
                    ],
                    duration_us: 100,
                })],
            },
        ];

        let spec = match lower_layers(&layers, &bell_layout(), &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("lowering failed: {error}"),
        };
        let ActionSpec::Entangle { pairs, .. } = &spec.layers[0].actions[0] else {
            panic!("expected entangle, got {:?}", spec.layers[0].actions[0]);
        };
        assert_eq!((pairs[0].lhs.x_um, pairs[0].lhs.y_um), (0.0, 0.0));
        assert_eq!((pairs[0].rhs.x_um, pairs[0].rhs.y_um), (4.0, 0.0));
    }

    #[test]
    fn preserves_action_order_within_layer() {
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![
                NeutralAtomAction::Wait { duration_us: 5 },
                NeutralAtomAction::Measure {
                    atom: AtomId(0),
                    basis: MeasurementBasis::Z,
                    duration_us: 1500,
                },
                NeutralAtomAction::Reset {
                    atom: AtomId(1),
                    duration_us: 1500,
                },
            ],
        }];

        let spec = match lower_layers(&layers, &bell_layout(), &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("lowering failed: {error}"),
        };
        assert_eq!(
            spec.layers[0].actions,
            vec![
                ActionSpec::Wait { duration_us: 5 },
                ActionSpec::Measure {
                    atom: 0,
                    basis: "z".to_string(),
                    duration_us: 1500,
                },
                ActionSpec::Reset {
                    atom: 1,
                    duration_us: 1500,
                },
            ]
        );
    }

    #[test]
    fn lowers_entangle_n_of_two_as_pair() {
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::EntangleN {
                atoms: vec![AtomId(0), AtomId(1)],
                duration_us: 2,
            }],
        }];
        let layout = NeutralAtomLayout {
            sites: vec![site(0, 0.0, 0.0), site(1, 2.0, 0.0)],
            initial_bindings: vec![slm_binding(0, 0), slm_binding(1, 1)],
        };

        let spec = match lower_layers(&layers, &layout, &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("lowering failed: {error}"),
        };
        let ActionSpec::Entangle { pairs, .. } = &spec.layers[0].actions[0] else {
            panic!("expected entangle, got {:?}", spec.layers[0].actions[0]);
        };
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].lhs.atom, 0);
        assert_eq!(pairs[0].rhs.atom, 1);
    }

    #[test]
    fn rejects_missing_layout() {
        let request = GraphScheduleRequest {
            graph: crate::graph::InteractionGraph {
                vertices: Vec::new(),
                interactions: Vec::new(),
                edges: Vec::new(),
                segments: Vec::new(),
                gamma: crate::graph::DEFAULT_GAMMA,
            },
            layers: Vec::new(),
            layout: None,
        };
        assert_eq!(
            lower_schedule(&request, &params()),
            Err(ScheduleLowerError::MissingLayout)
        );
    }

    #[test]
    fn rejects_entangle_n_above_pair_arity() {
        let layers = vec![ScheduleLayer {
            cycle: 4,
            actions: vec![NeutralAtomAction::EntangleN {
                atoms: vec![AtomId(0), AtomId(1), AtomId(2)],
                duration_us: 2,
            }],
        }];
        assert_eq!(
            lower_layers(&layers, &bell_layout(), &params()),
            Err(ScheduleLowerError::UnsupportedEntangleN { cycle: 4, arity: 3 })
        );
    }

    #[test]
    fn rejects_entangle_atom_without_occupancy() {
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [AtomId(0), AtomId(9)],
                duration_us: 1,
            }],
        }];
        assert_eq!(
            lower_layers(&layers, &bell_layout(), &params()),
            Err(ScheduleLowerError::UnboundAtom { cycle: 0, atom: 9 })
        );
    }

    #[test]
    fn rejects_move_without_prior_load_transfer() {
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Move(MovementGroup {
                moves: vec![AtomMove {
                    atom: AtomId(0),
                    from: SiteId(0),
                    to: SiteId(2),
                }],
                duration_us: 100,
            })],
        }];
        assert_eq!(
            lower_layers(&layers, &bell_layout(), &params()),
            Err(ScheduleLowerError::MissingAodBinding { cycle: 0, atom: 0 })
        );
    }

    #[test]
    fn rejects_move_inconsistent_with_final_bindings() {
        // Final bindings put atom 0 at site 2, but the last move claims it
        // ended at site 3.
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![load(0, 0)],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![AtomMove {
                        atom: AtomId(0),
                        from: SiteId(0),
                        to: SiteId(3),
                    }],
                    duration_us: 100,
                })],
            },
        ];
        let layout = NeutralAtomLayout {
            sites: bell_layout().sites,
            initial_bindings: vec![slm_binding(0, 2)],
        };
        assert_eq!(
            lower_layers(&layers, &layout, &params()),
            Err(ScheduleLowerError::InconsistentMove {
                cycle: 1,
                atom: 0,
                to: 3,
                found: 2,
            })
        );
    }

    #[test]
    fn rejects_moved_atom_missing_from_final_bindings() {
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![load(7, 0)],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![AtomMove {
                        atom: AtomId(7),
                        from: SiteId(0),
                        to: SiteId(2),
                    }],
                    duration_us: 100,
                })],
            },
        ];
        assert_eq!(
            lower_layers(&layers, &bell_layout(), &params()),
            Err(ScheduleLowerError::UnboundAtom { cycle: 1, atom: 7 })
        );
    }

    #[test]
    fn rejects_move_to_site_outside_layout() {
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![load(0, 0)],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![AtomMove {
                        atom: AtomId(0),
                        from: SiteId(0),
                        to: SiteId(99),
                    }],
                    duration_us: 100,
                })],
            },
        ];
        let layout = NeutralAtomLayout {
            sites: bell_layout().sites,
            initial_bindings: vec![slm_binding(0, 99)],
        };
        assert_eq!(
            lower_layers(&layers, &layout, &params()),
            Err(ScheduleLowerError::MissingSite { cycle: 1, site: 99 })
        );
    }

    #[test]
    fn rejects_duplicate_layout_site() {
        let layout = NeutralAtomLayout {
            sites: vec![site(0, 0.0, 0.0), site(0, 1.0, 1.0)],
            initial_bindings: Vec::new(),
        };
        assert_eq!(
            lower_layers(&[], &layout, &params()),
            Err(ScheduleLowerError::DuplicateSite { site: 0 })
        );
    }

    #[test]
    fn keeps_unique_flat_planner_aod_refs() {
        // Distinct planner refs (flat AOD path) must survive unchanged.
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: AtomId(0),
                        direction: TransferDirection::SlmToAod,
                        site: SiteId(0),
                        aod: AodTrapRef {
                            aod_id: 0,
                            row: 3,
                            col: 4,
                        },
                        duration_us: 15,
                    }),
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: AtomId(1),
                        direction: TransferDirection::SlmToAod,
                        site: SiteId(1),
                        aod: AodTrapRef {
                            aod_id: 0,
                            row: 3,
                            col: 5,
                        },
                        duration_us: 15,
                    }),
                ],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![
                        AtomMove {
                            atom: AtomId(0),
                            from: SiteId(0),
                            to: SiteId(2),
                        },
                        AtomMove {
                            atom: AtomId(1),
                            from: SiteId(1),
                            to: SiteId(3),
                        },
                    ],
                    duration_us: 100,
                })],
            },
        ];

        let spec = match lower_layers(&layers, &bell_layout(), &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("lowering failed: {error}"),
        };
        let ActionSpec::Move { moves, .. } = &spec.layers[1].actions[0] else {
            panic!("expected move, got {:?}", spec.layers[1].actions[0]);
        };
        assert_eq!((moves[0].row, moves[0].col), (3, 4));
        assert_eq!((moves[1].row, moves[1].col), (3, 5));
        // Unique refs are already consistent, so transfers keep them too.
        for (index, expected) in [(0usize, (3u32, 4u32)), (1, (3, 5))] {
            let ActionSpec::Transfer(load_spec) = &spec.layers[0].actions[index] else {
                panic!("expected transfer, got {:?}", spec.layers[0].actions[index]);
            };
            assert_eq!((load_spec.row, load_spec.col), expected);
        }
    }

    #[test]
    fn transfer_without_associated_move_keeps_planner_ref() {
        // A lone load with no subsequent move: nothing to reconcile against,
        // so the planner ref survives verbatim.
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![load(0, 0)],
        }];
        let layout = NeutralAtomLayout {
            sites: bell_layout().sites,
            initial_bindings: vec![slm_binding(0, 0)],
        };
        let spec = match lower_layers(&layers, &layout, &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("lowering failed: {error}"),
        };
        let ActionSpec::Transfer(load_spec) = &spec.layers[0].actions[0] else {
            panic!("expected transfer, got {:?}", spec.layers[0].actions[0]);
        };
        assert_eq!((load_spec.aod_id, load_spec.row, load_spec.col), (0, 0, 0));
    }

    #[test]
    fn repeated_episodes_get_their_own_transfer_refs() {
        // Two load→move→store episodes for the same atoms; each episode's
        // transfers must take that episode's overlay refs, not the other's.
        let mut layers = bell_layers();
        layers.truncate(3); // load, move (storage → pair), store
        // Second episode: move back from the pair to storage. Source x order
        // is reversed (pair sites are at x = 0.0 / 2.0 like storage 0.0 / 4.0,
        // so ranks stay 0 / 1 — assert per-episode lookup, not recomputation).
        layers.push(ScheduleLayer {
            cycle: 3,
            actions: vec![load(0, 2), load(1, 3)],
        });
        layers.push(ScheduleLayer {
            cycle: 4,
            actions: vec![NeutralAtomAction::Move(MovementGroup {
                moves: vec![
                    AtomMove {
                        atom: AtomId(0),
                        from: SiteId(2),
                        to: SiteId(0),
                    },
                    AtomMove {
                        atom: AtomId(1),
                        from: SiteId(3),
                        to: SiteId(1),
                    },
                ],
                duration_us: 336,
            })],
        });
        layers.push(ScheduleLayer {
            cycle: 5,
            actions: vec![store(0, 0), store(1, 1)],
        });
        let layout = NeutralAtomLayout {
            sites: bell_layout().sites,
            initial_bindings: vec![slm_binding(0, 0), slm_binding(1, 1)],
        };

        let spec = match lower_layers(&layers, &layout, &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("lowering failed: {error}"),
        };
        for layer_index in [3, 5] {
            for (action_index, expected_col) in [(0usize, 0u32), (1, 1)] {
                let ActionSpec::Transfer(transfer_spec) =
                    &spec.layers[layer_index].actions[action_index]
                else {
                    panic!(
                        "expected transfer, got {:?}",
                        spec.layers[layer_index].actions[action_index]
                    );
                };
                assert_eq!(
                    (transfer_spec.row, transfer_spec.col),
                    (0, expected_col),
                    "layer {layer_index} action {action_index}"
                );
            }
        }
    }

    #[test]
    fn lowered_bell_spec_dumps_as_quantum_na() {
        let spec = match lower_layers(&bell_layers(), &bell_layout(), &params()) {
            Ok(spec) => spec,
            Err(error) => panic!("bell lowering failed: {error}"),
        };
        let text = match crate::dialect::dump_schedule_text(&spec) {
            Ok(text) => text,
            Err(error) => panic!("dump failed: {error}"),
        };
        for op in [
            "quantum.na.schedule",
            "quantum.na.layer",
            "quantum.na.transfer",
            "quantum.na.move",
            "quantum.na.entangle",
        ] {
            assert!(text.contains(op), "missing {op} in:\n{text}");
        }
    }
}
