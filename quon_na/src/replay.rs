//! Replay an emitted `quantum.na` schedule against the declared architecture.
//!
//! The planners rewrite [`crate::layout::NeutralAtomLayout::initial_bindings`]
//! to final occupancy. This module does not read that vector, `ScheduleLayer`,
//! or any other planner-internal map. It starts from
//! [`NeutralAtomLayout::declared_initial_bindings`] and the target's sites,
//! zones, and interaction limits, then walks the post-compaction
//! [`ScheduleSpec`] that was actually emitted.

use std::collections::{BTreeMap, BTreeSet};

use crate::dialect::{
    ActionSpec, PositionedAtom, ScheduleSpec, TransferDirection, VerifyError, verify_schedule_spec,
};
use crate::layout::{AtomBinding, AtomSite, NeutralAtomLayout, TrapBinding};
use crate::pipeline::zoned_architecture;
use crate::zoned::{ZoneKind, ZoneSpec};
use backend::NeutralAtomTarget;

/// Architecture the emitted schedule is checked against.
///
/// Built from the serialized target and the placement-time bindings. It is
/// not the planner's working occupancy.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredArchitecture {
    pub sites: Vec<AtomSite>,
    pub initial_bindings: Vec<AtomBinding>,
    pub zones: Vec<ZoneSpec>,
    /// When set, entangling atoms must sit in an entanglement zone.
    pub check_zones: bool,
    /// When set, measure, reset, and reuse must sit in a readout zone.
    pub require_readout_zone: bool,
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub aod_min_separation_um: f64,
}

impl DeclaredArchitecture {
    /// Declared start state for one compile.
    ///
    /// `zoned` is the backend that emitted the schedule. Flat AOD does not
    /// apply zone residency even when the target JSON lists zones.
    pub fn from_compiled(
        na: &NeutralAtomTarget,
        layout: &NeutralAtomLayout,
        zoned: bool,
    ) -> Result<Self, VerifyError> {
        if layout.declared_initial_bindings.is_empty() {
            return Err(VerifyError::MissingDeclaredBindings);
        }
        let arch = zoned_architecture(na);
        Ok(Self {
            sites: layout.sites.clone(),
            initial_bindings: layout.declared_initial_bindings.clone(),
            zones: arch.zones,
            check_zones: zoned,
            require_readout_zone: zoned && arch.require_readout_zone,
            rydberg_range_um: na.interaction.rydberg_range_um,
            min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
            aod_min_separation_um: na.movement.min_row_col_separation_um,
        })
    }
}

/// Structural `quantum.na` checks, then a forward replay from the declared
/// initial bindings.
pub fn verify_emitted_schedule(
    spec: &ScheduleSpec,
    declared: &DeclaredArchitecture,
) -> Result<(), VerifyError> {
    verify_schedule_spec(spec)?;
    replay_emitted_schedule(spec, declared)
}

/// Walk `spec` from `declared.initial_bindings`, updating trap ownership and
/// site occupancy after every transfer and move.
pub fn replay_emitted_schedule(
    spec: &ScheduleSpec,
    declared: &DeclaredArchitecture,
) -> Result<(), VerifyError> {
    check_limit(
        "rydberg_range_um",
        declared.rydberg_range_um,
        spec.rydberg_range_um,
    )?;
    check_limit(
        "min_rydberg_spacing_um",
        declared.min_rydberg_spacing_um,
        spec.min_rydberg_spacing_um,
    )?;
    check_limit(
        "aod_min_separation_um",
        declared.aod_min_separation_um,
        spec.aod_min_separation_um,
    )?;

    let sites = site_table(&declared.sites)?;
    let mut atoms: BTreeMap<u32, AtomState> = BTreeMap::new();
    let mut holders: BTreeMap<u32, u32> = BTreeMap::new();
    for binding in &declared.initial_bindings {
        let site = binding_site(binding);
        let _ = sites.get(&site).ok_or(VerifyError::UnboundBindingSite {
            atom: binding.atom.0,
            site,
        })?;
        if let Some(first) = holders.insert(site, binding.atom.0) {
            return Err(VerifyError::DuplicateInitialSite {
                site,
                first,
                second: binding.atom.0,
            });
        }
        atoms.insert(
            binding.atom.0,
            AtomState {
                site,
                trap: trap_from_binding(binding),
            },
        );
    }

    for layer in &spec.layers {
        for action in &layer.actions {
            match action {
                ActionSpec::Move { moves, .. } => {
                    apply_move_group(layer.cycle, moves, &sites, &mut atoms, &mut holders)?;
                }
                ActionSpec::Transfer(transfer) => {
                    apply_transfer(layer.cycle, transfer, &sites, &mut atoms)?;
                }
                ActionSpec::Entangle { pairs, .. } => {
                    let mut seen = Vec::new();
                    for pair in pairs {
                        for end in [pair.lhs, pair.rhs] {
                            let state = require_atom(&atoms, layer.cycle, end.atom)?;
                            check_stamped_position(layer.cycle, &sites, state, end)?;
                            if declared.check_zones {
                                require_zone(
                                    layer.cycle,
                                    end.atom,
                                    &declared.zones,
                                    &sites,
                                    state,
                                    ZoneKind::Entanglement,
                                    "entangle",
                                )?;
                            }
                            seen.push(end);
                        }
                    }
                    check_entangle_geometry(
                        layer.cycle,
                        &seen,
                        declared.rydberg_range_um,
                        declared.min_rydberg_spacing_um,
                    )?;
                }
                ActionSpec::LocalGate { atom, .. } => {
                    let _ = require_atom(&atoms, layer.cycle, *atom)?;
                }
                ActionSpec::Measure { atom, .. }
                | ActionSpec::Reset { atom, .. }
                | ActionSpec::Reuse { atom, .. } => {
                    let state = require_atom(&atoms, layer.cycle, *atom)?;
                    if declared.require_readout_zone {
                        require_zone(
                            layer.cycle,
                            *atom,
                            &declared.zones,
                            &sites,
                            state,
                            ZoneKind::Readout,
                            "readout",
                        )?;
                    }
                }
                ActionSpec::GlobalRy { .. } | ActionSpec::Wait { .. } => {}
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum TrapOwn {
    Slm,
    Aod { aod_id: u32, row: u32, col: u32 },
}

struct AtomState {
    site: u32,
    trap: TrapOwn,
}

fn binding_site(binding: &AtomBinding) -> u32 {
    match binding.trap {
        TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site.0,
    }
}

fn trap_from_binding(binding: &AtomBinding) -> TrapOwn {
    match binding.trap {
        TrapBinding::Slm { .. } => TrapOwn::Slm,
        TrapBinding::Aod { ref aod, .. } => TrapOwn::Aod {
            aod_id: aod.aod_id,
            row: aod.row,
            col: aod.col,
        },
    }
}

fn site_table(sites: &[AtomSite]) -> Result<BTreeMap<u32, (f64, f64)>, VerifyError> {
    let mut table = BTreeMap::new();
    for site in sites {
        if table
            .insert(site.id.0, (site.position.x_um, site.position.y_um))
            .is_some()
        {
            return Err(VerifyError::DuplicateArchitectureSite { site: site.id.0 });
        }
    }
    Ok(table)
}

fn check_limit(field: &'static str, declared: f64, emitted: f64) -> Result<(), VerifyError> {
    if declared.total_cmp(&emitted).is_eq() {
        Ok(())
    } else {
        Err(VerifyError::ArchitectureLimitMismatch {
            field,
            declared,
            emitted,
        })
    }
}

fn require_atom(
    atoms: &BTreeMap<u32, AtomState>,
    cycle: u32,
    atom: u32,
) -> Result<&AtomState, VerifyError> {
    atoms
        .get(&atom)
        .ok_or(VerifyError::ReplayUnboundAtom { cycle, atom })
}

fn site_xy(
    sites: &BTreeMap<u32, (f64, f64)>,
    cycle: u32,
    site: u32,
) -> Result<(f64, f64), VerifyError> {
    sites
        .get(&site)
        .copied()
        .ok_or(VerifyError::UnknownArchitectureSite { cycle, site })
}

fn coords_match(lhs: (f64, f64), rhs_x: f64, rhs_y: f64) -> bool {
    (lhs.0 - rhs_x).abs() <= 1e-6 && (lhs.1 - rhs_y).abs() <= 1e-6
}

fn apply_move_group(
    cycle: u32,
    moves: &[crate::dialect::MoveSpec],
    sites: &BTreeMap<u32, (f64, f64)>,
    atoms: &mut BTreeMap<u32, AtomState>,
    holders: &mut BTreeMap<u32, u32>,
) -> Result<(), VerifyError> {
    let mut claimed = BTreeSet::new();
    let mut departures = Vec::with_capacity(moves.len());
    for atom_move in moves {
        if !claimed.insert(atom_move.atom) {
            return Err(VerifyError::DuplicateOccupancyAtom {
                cycle,
                atom: atom_move.atom,
            });
        }
        let state = require_atom(atoms, cycle, atom_move.atom)?;
        if state.site != atom_move.from_site {
            return Err(VerifyError::MoveOriginMismatch {
                cycle,
                atom: atom_move.atom,
                held_site: state.site,
                found_site: atom_move.from_site,
            });
        }
        let from_xy = site_xy(sites, cycle, atom_move.from_site)?;
        if !coords_match(from_xy, atom_move.from_x_um, atom_move.from_y_um) {
            return Err(VerifyError::SiteCoordinateMismatch {
                cycle,
                atom: atom_move.atom,
                site: atom_move.from_site,
                end: "from",
                x_um: atom_move.from_x_um,
                y_um: atom_move.from_y_um,
                site_x_um: from_xy.0,
                site_y_um: from_xy.1,
            });
        }
        let to_xy = site_xy(sites, cycle, atom_move.to_site)?;
        if !coords_match(to_xy, atom_move.to_x_um, atom_move.to_y_um) {
            return Err(VerifyError::SiteCoordinateMismatch {
                cycle,
                atom: atom_move.atom,
                site: atom_move.to_site,
                end: "to",
                x_um: atom_move.to_x_um,
                y_um: atom_move.to_y_um,
                site_x_um: to_xy.0,
                site_y_um: to_xy.1,
            });
        }
        match state.trap {
            TrapOwn::Aod { aod_id, row, col }
                if aod_id == atom_move.aod_id && row == atom_move.row && col == atom_move.col => {}
            TrapOwn::Aod { aod_id, row, col } => {
                return Err(VerifyError::MoveOwnership {
                    cycle,
                    atom: atom_move.atom,
                    held: "aod",
                    aod_id: atom_move.aod_id,
                    row: atom_move.row,
                    col: atom_move.col,
                    bound_aod_id: aod_id,
                    bound_row: row,
                    bound_col: col,
                });
            }
            TrapOwn::Slm => {
                return Err(VerifyError::MoveOwnership {
                    cycle,
                    atom: atom_move.atom,
                    held: "slm",
                    aod_id: atom_move.aod_id,
                    row: atom_move.row,
                    col: atom_move.col,
                    bound_aod_id: 0,
                    bound_row: 0,
                    bound_col: 0,
                });
            }
        }
        departures.push((atom_move.atom, atom_move.from_site, atom_move.to_site));
    }

    for &(_, from_site, _) in &departures {
        holders.remove(&from_site);
    }
    for &(atom, _, to_site) in &departures {
        if let Some(occupant) = holders.insert(to_site, atom) {
            return Err(VerifyError::DestinationOccupied {
                cycle,
                atom,
                site: to_site,
                occupant,
            });
        }
        if let Some(state) = atoms.get_mut(&atom) {
            state.site = to_site;
        }
    }
    Ok(())
}

fn apply_transfer(
    cycle: u32,
    transfer: &crate::dialect::TransferSpec,
    sites: &BTreeMap<u32, (f64, f64)>,
    atoms: &mut BTreeMap<u32, AtomState>,
) -> Result<(), VerifyError> {
    let _ = site_xy(sites, cycle, transfer.site)?;
    let state = require_atom(atoms, cycle, transfer.atom)?;
    let direction = match transfer.direction {
        TransferDirection::SlmToAod => "slm_to_aod",
        TransferDirection::AodToSlm => "aod_to_slm",
    };
    let legal = match (transfer.direction, state.trap) {
        (TransferDirection::SlmToAod, TrapOwn::Slm) => state.site == transfer.site,
        (TransferDirection::AodToSlm, TrapOwn::Aod { aod_id, row, col }) => {
            state.site == transfer.site
                && aod_id == transfer.aod_id
                && row == transfer.row
                && col == transfer.col
        }
        _ => false,
    };
    if !legal {
        let held = match state.trap {
            TrapOwn::Slm => "slm",
            TrapOwn::Aod { .. } => "aod",
        };
        return Err(VerifyError::TransferOwnership {
            cycle,
            atom: transfer.atom,
            direction,
            held,
            held_site: state.site,
            site: transfer.site,
        });
    }
    if let Some(state) = atoms.get_mut(&transfer.atom) {
        state.trap = match transfer.direction {
            TransferDirection::SlmToAod => TrapOwn::Aod {
                aod_id: transfer.aod_id,
                row: transfer.row,
                col: transfer.col,
            },
            TransferDirection::AodToSlm => TrapOwn::Slm,
        };
    }
    Ok(())
}

fn check_stamped_position(
    cycle: u32,
    sites: &BTreeMap<u32, (f64, f64)>,
    state: &AtomState,
    end: PositionedAtom,
) -> Result<(), VerifyError> {
    let (site_x, site_y) = site_xy(sites, cycle, state.site)?;
    if coords_match((site_x, site_y), end.x_um, end.y_um) {
        Ok(())
    } else {
        Err(VerifyError::EntanglePositionMismatch {
            cycle,
            atom: end.atom,
            site: state.site,
            x_um: end.x_um,
            y_um: end.y_um,
            site_x_um: site_x,
            site_y_um: site_y,
        })
    }
}

fn check_entangle_geometry(
    cycle: u32,
    atoms: &[PositionedAtom],
    rydberg_range_um: f64,
    min_spacing_um: f64,
) -> Result<(), VerifyError> {
    let mut partners = BTreeSet::new();
    // Pairs are adjacent in `atoms` (lhs, rhs, lhs, rhs, ...).
    let mut index = 0;
    while index + 1 < atoms.len() {
        let lhs = atoms[index];
        let rhs = atoms[index + 1];
        let distance = distance(lhs, rhs);
        if distance > rydberg_range_um {
            return Err(VerifyError::EntanglingPairOutOfRange {
                cycle,
                lhs: lhs.atom,
                rhs: rhs.atom,
                distance_um: distance,
                rydberg_range_um,
            });
        }
        partners.insert(pair_key(lhs.atom, rhs.atom));
        index += 2;
    }
    for i in 0..atoms.len() {
        for rhs in atoms.iter().skip(i + 1) {
            let lhs = atoms[i];
            if partners.contains(&pair_key(lhs.atom, rhs.atom)) {
                continue;
            }
            let distance = distance(lhs, *rhs);
            if distance <= rydberg_range_um {
                return Err(VerifyError::CompulsoryEntanglement {
                    cycle,
                    lhs: lhs.atom,
                    rhs: rhs.atom,
                    distance_um: distance,
                    rydberg_range_um,
                });
            }
            if distance <= min_spacing_um {
                return Err(VerifyError::RydbergSpacing {
                    cycle,
                    lhs: lhs.atom,
                    rhs: rhs.atom,
                    distance_um: distance,
                    min_spacing_um,
                });
            }
        }
    }
    Ok(())
}

fn require_zone(
    cycle: u32,
    atom: u32,
    zones: &[ZoneSpec],
    sites: &BTreeMap<u32, (f64, f64)>,
    state: &AtomState,
    expected: ZoneKind,
    operation: &'static str,
) -> Result<(), VerifyError> {
    let (x, y) = site_xy(sites, cycle, state.site)?;
    let found = zone_at(zones, x, y);
    if found == Some(expected) {
        Ok(())
    } else {
        Err(VerifyError::ZoneCapability {
            cycle,
            atom,
            site: state.site,
            found: zone_name(found),
            operation,
        })
    }
}

fn zone_at(zones: &[ZoneSpec], x: f64, y: f64) -> Option<ZoneKind> {
    for zone in zones {
        let x1 = zone.origin_um.0 + f64::from(zone.cols) * zone.site_pitch_um.0;
        let y1 = zone.origin_um.1 + f64::from(zone.rows) * zone.site_pitch_um.1;
        if x >= zone.origin_um.0 && x <= x1 && y >= zone.origin_um.1 && y <= y1 {
            return Some(zone.kind);
        }
    }
    None
}

fn zone_name(kind: Option<ZoneKind>) -> &'static str {
    match kind {
        Some(ZoneKind::Storage) => "storage",
        Some(ZoneKind::Entanglement) => "entanglement",
        Some(ZoneKind::Readout) => "readout",
        None => "no zone",
    }
}

fn distance(lhs: PositionedAtom, rhs: PositionedAtom) -> f64 {
    let dx = lhs.x_um - rhs.x_um;
    let dy = lhs.y_um - rhs.y_um;
    (dx * dx + dy * dy).sqrt()
}

fn pair_key(lhs: u32, rhs: u32) -> (u32, u32) {
    if lhs <= rhs { (lhs, rhs) } else { (rhs, lhs) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::{
        ActionSpec, EntanglePairSpec, LayerSpec, MoveSpec, PositionedAtom, ScheduleSpec,
        TransferDirection, TransferSpec,
    };
    use crate::layout::{AtomBinding, AtomId, AtomSite, Position, SiteId, TrapBinding};

    fn site(id: u32, x_um: f64, y_um: f64) -> AtomSite {
        AtomSite {
            id: SiteId(id),
            position: Position { x_um, y_um },
        }
    }

    fn slm(atom: u32, site: u32) -> AtomBinding {
        AtomBinding {
            atom: AtomId(atom),
            trap: TrapBinding::Slm { site: SiteId(site) },
        }
    }

    fn limits() -> (f64, f64, f64) {
        (7.5, 18.75, 2.0)
    }

    fn declared(bindings: Vec<AtomBinding>, check_zones: bool) -> DeclaredArchitecture {
        let (rydberg_range_um, min_rydberg_spacing_um, aod_min_separation_um) = limits();
        DeclaredArchitecture {
            sites: vec![
                site(0, 0.0, 0.0),
                site(1, 10.0, 10.0),
                site(2, 0.0, 50.0),
                site(3, 6.0, 52.0),
            ],
            initial_bindings: bindings,
            zones: vec![
                ZoneSpec {
                    zone_id: 0,
                    kind: ZoneKind::Storage,
                    rows: 1,
                    cols: 1,
                    origin_um: (0.0, 0.0),
                    site_pitch_um: (20.0, 20.0),
                    pair_gap_um: None,
                },
                ZoneSpec {
                    zone_id: 1,
                    kind: ZoneKind::Entanglement,
                    rows: 1,
                    cols: 1,
                    origin_um: (0.0, 40.0),
                    site_pitch_um: (20.0, 20.0),
                    pair_gap_um: Some(2.0),
                },
            ],
            check_zones,
            require_readout_zone: false,
            rydberg_range_um,
            min_rydberg_spacing_um,
            aod_min_separation_um,
        }
    }

    fn spec(layers: Vec<LayerSpec>) -> ScheduleSpec {
        let (rydberg_range_um, min_rydberg_spacing_um, aod_min_separation_um) = limits();
        ScheduleSpec {
            target_id: "replay-test".into(),
            rydberg_range_um,
            min_rydberg_spacing_um,
            aod_min_separation_um,
            layers,
        }
    }

    fn load(atom: u32, site: u32, row: u32, col: u32) -> ActionSpec {
        ActionSpec::Transfer(TransferSpec {
            atom,
            site,
            aod_id: 0,
            row,
            col,
            direction: TransferDirection::SlmToAod,
            duration_us: 15,
        })
    }

    fn store(atom: u32, site: u32, row: u32, col: u32) -> ActionSpec {
        ActionSpec::Transfer(TransferSpec {
            atom,
            site,
            aod_id: 0,
            row,
            col,
            direction: TransferDirection::AodToSlm,
            duration_us: 15,
        })
    }

    fn move_pair() -> ActionSpec {
        ActionSpec::Move {
            moves: vec![
                MoveSpec {
                    atom: 0,
                    from_site: 0,
                    to_site: 2,
                    aod_id: 0,
                    row: 0,
                    col: 0,
                    from_x_um: 0.0,
                    from_y_um: 0.0,
                    to_x_um: 0.0,
                    to_y_um: 50.0,
                },
                MoveSpec {
                    atom: 1,
                    from_site: 1,
                    to_site: 3,
                    aod_id: 0,
                    row: 1,
                    col: 1,
                    from_x_um: 10.0,
                    from_y_um: 10.0,
                    to_x_um: 6.0,
                    to_y_um: 52.0,
                },
            ],
            duration_us: 20,
        }
    }

    fn entangle_at_dest() -> ActionSpec {
        ActionSpec::Entangle {
            pairs: vec![EntanglePairSpec {
                lhs: PositionedAtom {
                    atom: 0,
                    x_um: 0.0,
                    y_um: 50.0,
                },
                rhs: PositionedAtom {
                    atom: 1,
                    x_um: 6.0,
                    y_um: 52.0,
                },
            }],
            duration_us: 1,
        }
    }

    fn legal_layers() -> Vec<LayerSpec> {
        vec![
            LayerSpec {
                cycle: 0,
                actions: vec![load(0, 0, 0, 0), load(1, 1, 1, 1)],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![move_pair()],
            },
            LayerSpec {
                cycle: 2,
                actions: vec![entangle_at_dest()],
            },
            LayerSpec {
                cycle: 3,
                actions: vec![store(0, 2, 0, 0), store(1, 3, 1, 1)],
            },
        ]
    }

    #[test]
    fn legal_schedule_replays_and_round_trips() {
        let declared = declared(vec![slm(0, 0), slm(1, 1)], true);
        let spec = spec(legal_layers());
        verify_emitted_schedule(&spec, &declared).expect("legal replay");
        let text = serde_json::to_string(&spec).expect("serialize");
        let parsed: ScheduleSpec = serde_json::from_str(&text).expect("deserialize");
        replay_emitted_schedule(&parsed, &declared).expect("round trip");
    }

    #[test]
    fn move_without_load_is_rejected() {
        let declared = declared(vec![slm(0, 0)], false);
        let spec = spec(vec![LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Move {
                moves: vec![MoveSpec {
                    atom: 0,
                    from_site: 0,
                    to_site: 2,
                    aod_id: 0,
                    row: 0,
                    col: 0,
                    from_x_um: 0.0,
                    from_y_um: 0.0,
                    to_x_um: 0.0,
                    to_y_um: 50.0,
                }],
                duration_us: 20,
            }],
        }]);
        // Per-op structural checks allow a move with no prior load.
        verify_schedule_spec(&spec).expect("structural");
        assert!(matches!(
            replay_emitted_schedule(&spec, &declared),
            Err(VerifyError::MoveOwnership { held: "slm", .. })
        ));
    }

    #[test]
    fn move_onto_stationary_atom_is_rejected() {
        let declared = declared(vec![slm(0, 0), slm(1, 1)], false);
        let spec = spec(vec![
            LayerSpec {
                cycle: 0,
                actions: vec![load(0, 0, 0, 0)],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Move {
                    moves: vec![MoveSpec {
                        atom: 0,
                        from_site: 0,
                        to_site: 1,
                        aod_id: 0,
                        row: 0,
                        col: 0,
                        from_x_um: 0.0,
                        from_y_um: 0.0,
                        to_x_um: 10.0,
                        to_y_um: 10.0,
                    }],
                    duration_us: 20,
                }],
            },
        ]);
        assert!(matches!(
            replay_emitted_schedule(&spec, &declared),
            Err(VerifyError::DestinationOccupied {
                atom: 0,
                site: 1,
                occupant: 1,
                ..
            })
        ));
    }

    #[test]
    fn move_from_the_wrong_site_is_rejected() {
        let declared = declared(vec![slm(0, 0)], false);
        let spec = spec(vec![
            LayerSpec {
                cycle: 0,
                actions: vec![load(0, 0, 0, 0)],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Move {
                    moves: vec![MoveSpec {
                        atom: 0,
                        from_site: 1,
                        to_site: 2,
                        aod_id: 0,
                        row: 0,
                        col: 0,
                        from_x_um: 10.0,
                        from_y_um: 10.0,
                        to_x_um: 0.0,
                        to_y_um: 50.0,
                    }],
                    duration_us: 20,
                }],
            },
        ]);
        assert!(matches!(
            replay_emitted_schedule(&spec, &declared),
            Err(VerifyError::MoveOriginMismatch {
                held_site: 0,
                found_site: 1,
                ..
            })
        ));
    }

    #[test]
    fn entangle_outside_zone_is_rejected() {
        let mut close = declared(vec![slm(0, 0), slm(1, 1)], true);
        close.sites = vec![site(0, 0.0, 0.0), site(1, 2.0, 0.0)];
        close.initial_bindings = vec![slm(0, 0), slm(1, 1)];
        let spec = spec(vec![LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Entangle {
                pairs: vec![EntanglePairSpec {
                    lhs: PositionedAtom {
                        atom: 0,
                        x_um: 0.0,
                        y_um: 0.0,
                    },
                    rhs: PositionedAtom {
                        atom: 1,
                        x_um: 2.0,
                        y_um: 0.0,
                    },
                }],
                duration_us: 1,
            }],
        }]);
        assert!(matches!(
            replay_emitted_schedule(&spec, &close),
            Err(VerifyError::ZoneCapability {
                operation: "entangle",
                found: "storage",
                ..
            })
        ));
    }

    #[test]
    fn same_cycle_after_wait_is_rejected() {
        let declared = declared(vec![slm(0, 0)], false);
        let spec = spec(vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Wait { duration_us: 1 }],
            },
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Wait { duration_us: 1 }],
            },
        ]);
        assert!(matches!(
            verify_emitted_schedule(&spec, &declared),
            Err(VerifyError::RoundBarrierCycleOrder { .. })
        ));
    }

    #[test]
    fn zoned_schedule_replays_from_declared_bindings_not_final_occupancy() {
        use crate::entangling_schedule::schedule_entangling_layers;
        use crate::graph::{
            DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
            LogicalQubitId, SegmentKind,
        };
        use crate::lower::{ScheduleLowerParams, lower_schedule};
        use crate::schedule_entry::schedule_from_graph;
        use crate::zoned::{PlacerMode, schedule_zoned, toy_zoned_architecture};

        let id = InteractionId(0);
        let graph = InteractionGraph::from_interactions(
            vec![LogicalQubitId(0), LogicalQubitId(1)],
            vec![Interaction {
                id,
                qubits: vec![LogicalQubitId(0), LogicalQubitId(1)],
                gate_name: "CZ".into(),
                dag_layer: 0,
                on_critical_path: false,
            }],
            vec![InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![id],
            }],
            DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("request");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let arch = toy_zoned_architecture();
        let result =
            schedule_zoned(scheduled.request, &arch, PlacerMode::RoutingAgnostic).expect("zoned");
        let layout = result.request.layout.as_ref().expect("layout");
        assert!(
            !layout.declared_initial_bindings.is_empty(),
            "placement must record a declared start"
        );
        assert_ne!(
            layout.declared_initial_bindings, layout.initial_bindings,
            "planner rewrites initial_bindings to final occupancy"
        );
        let params = ScheduleLowerParams {
            target_id: "toy".into(),
            rydberg_range_um: arch.rydberg_range_um,
            min_rydberg_spacing_um: arch.min_rydberg_spacing_um,
            aod_min_separation_um: arch.aod_min_separation_um,
        };
        let spec = lower_schedule(&result.request, &params).expect("lower");
        let declared = DeclaredArchitecture {
            sites: layout.sites.clone(),
            initial_bindings: layout.declared_initial_bindings.clone(),
            zones: arch.zones.clone(),
            check_zones: true,
            require_readout_zone: arch.require_readout_zone,
            rydberg_range_um: arch.rydberg_range_um,
            min_rydberg_spacing_um: arch.min_rydberg_spacing_um,
            aod_min_separation_um: arch.aod_min_separation_um,
        };
        verify_emitted_schedule(&spec, &declared).expect("replay from declared start");

        let mut from_final = declared;
        from_final.initial_bindings = layout.initial_bindings.clone();
        assert!(
            replay_emitted_schedule(&spec, &from_final).is_err(),
            "starting from planner-final occupancy must not verify"
        );
    }
}
