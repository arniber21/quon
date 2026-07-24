//! MQT NAViz interop: serialize a [`ScheduleLayer`] sequence to `.naviz` and a
//! [`backend::NeutralAtomTarget`] + [`NeutralAtomLayout`] to `.namachine`
//! (Quon issue #303).
//!
//! [MQT NAViz](https://github.com/munich-quantum-toolkit/naviz) is a
//! visualizer for neutral-atom quantum computers. Its input format describes a
//! computation as a sequence of timed instructions (`load` / `store` / `move` /
//! `rz` / `ry` / `cz`) over atoms placed on a machine with rectangular zones
//! and static traps. See `docs/file_format.md` in the NAViz repo for the
//! grammar.
//!
//! ## Timing model
//!
//! NAViz recomputes each instruction's *duration* from the machine's `time` /
//! `movement` / `distance` blocks, not from per-instruction durations. The
//! emitter therefore uses **relative** timing (`@+`, "immediately after the
//! preceding instruction ends") — mirroring NAViz's own `mqt-na` converter —
//! and emits one instruction group (`@+ [ … ]`) per schedule layer so a
//! layer's parallel actions start together. This yields a faithful back-to-back
//! playback in schedule order; the schedule's own `duration_us` fields are
//! reflected in the `.namachine` `time`/`movement` blocks so NAViz's recomputed
//! durations match the compiled schedule.
//!
//! ## Lifecycle ops
//!
//! `Measure`, `Reset`, `Reuse`, and `Wait` have no NAViz opcode (the grammar
//! only defines load/store/move/rz/ry/cz). They are emitted as comments — NAViz
//! ignores comments — so the schedule's qubit lifecycle stays visible in the
//! file without breaking the parser. `H` / `U3` single-qubit gates likewise
//! have no native NAViz opcode; they are approximated as single-axis rotations
//! (`ry(π/2)` for `H`, `ry(θ)` for `U3`) purely for visualization — this only
//! affects rendering, never the compiled `quantum.na` schedule.

use std::collections::HashMap;
use std::fmt::Write;

use backend::{AodSpeedModelKind, NeutralAtomTarget, ZoneKind};

use crate::layout::{AtomId, NeutralAtomLayout, Position, SiteId, TrapBinding};
use crate::schedule::{
    LocalGateKind, MeasurementBasis, NeutralAtomAction, ScheduleLayer, TransferDirection,
};

/// Nominal shuttle `max_speed` (in distance/time units, i.e. µm/µs) emitted for
/// the √-law speed model, which defines no cruise velocity. NAViz interpolates
/// shuttles with constant jerk respecting `max_speed`; 1 µm/µs (= 1 m/s) is a
/// safe upper bound that does not clip typical µm-scale moves. Jerk-limited
/// models with a configured cruise velocity emit that value instead.
const SQRT_NOMINAL_MAX_SPEED: f64 = 1.0;

/// Serialize a schedule (`layers`) + `layout` to MQT NAViz `.naviz` text.
///
/// `machine_id` is the NAViz machine identifier written in the `#target`
/// directive; it must match the stem of the sibling `.namachine` file name.
pub fn schedule_to_naviz(
    layers: &[ScheduleLayer],
    layout: &NeutralAtomLayout,
    machine_id: &str,
) -> String {
    let site_pos: HashMap<SiteId, Position> =
        layout.sites.iter().map(|s| (s.id, s.position)).collect();

    // Track each atom's current position as it shuttles, seeded from the
    // initial trap bindings.
    let mut atom_pos: HashMap<AtomId, Position> = HashMap::new();
    for binding in &layout.initial_bindings {
        let site = match binding.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        if let Some(p) = site_pos.get(&site) {
            atom_pos.insert(binding.atom, *p);
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "#target {machine_id}");
    let _ = writeln!(out);

    // Atom setup declarations, in stable AtomId order.
    let mut atoms: Vec<AtomId> = atom_pos.keys().copied().collect();
    atoms.sort_unstable();
    for atom in &atoms {
        let p = atom_pos[atom];
        let _ = writeln!(
            out,
            "atom ({}, {}) atom{}",
            num(p.x_um),
            num(p.y_um),
            atom.0
        );
    }
    if !atoms.is_empty() {
        let _ = writeln!(out);
    }

    for layer in layers {
        let mut ops: Vec<String> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for action in &layer.actions {
            emit_action(
                action,
                &site_pos,
                &mut atom_pos,
                &mut ops,
                &mut notes,
                layer.cycle,
            );
        }
        for note in &notes {
            let _ = writeln!(out, "{note}");
        }
        if ops.is_empty() {
            // A layer with only lifecycle ops (measure/reset/wait) produces no
            // NAViz group; the comments above already document it. NAViz does
            // not advance time for comments, which is correct — these ops are
            // not visualized.
            continue;
        }
        let _ = writeln!(out, "@+ [");
        for op in &ops {
            let _ = writeln!(out, "\t{op}");
        }
        let _ = writeln!(out, "]");
    }

    out
}

/// Serialize a target + layout to MQT NAViz `.namachine` text.
///
/// `machine_name` is the human-readable display name (typically the backend
/// target `id`); the NAViz *machine id* is the file-name stem, not this name.
pub fn target_to_namachine(
    target: &NeutralAtomTarget,
    layout: &NeutralAtomLayout,
    machine_name: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "name: \"{machine_name}\"");
    let _ = writeln!(out);

    let max_speed = movement_max_speed(target);
    let _ = writeln!(out, "movement {{");
    let _ = writeln!(out, "\tmax_speed: {}", num(max_speed));
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    let t = &target.timing;
    let _ = writeln!(out, "time {{");
    let _ = writeln!(out, "\tload: {}", num(target.movement.trap_transfer_us));
    let _ = writeln!(out, "\tstore: {}", num(target.movement.trap_transfer_us));
    let _ = writeln!(out, "\try: {}", num(t.single_qubit_us));
    let _ = writeln!(out, "\trz: {}", num(t.single_qubit_us));
    let _ = writeln!(out, "\tcz: {}", num(t.cz_us));
    let _ = writeln!(out, "\tunit: \"us\"");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    let _ = writeln!(out, "distance {{");
    let _ = writeln!(
        out,
        "\tinteraction: {}",
        num(target.interaction.rydberg_range_um)
    );
    let _ = writeln!(out, "\tunit: \"um\"");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    for zone in &target.zones {
        let (x_min, y_min, x_max, y_max) = zone.bounds_um();
        let _ = writeln!(out, "zone {} {{", zone_id(&zone.kind, zone.zone_id));
        let _ = writeln!(out, "\tfrom: ({}, {})", num(x_min), num(y_min));
        let _ = writeln!(out, "\tto: ({}, {})", num(x_max), num(y_max));
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    for site in &layout.sites {
        let _ = writeln!(out, "trap trap{} {{", site.id.0);
        let _ = writeln!(
            out,
            "\tposition: ({}, {})",
            num(site.position.x_um),
            num(site.position.y_um)
        );
        let _ = writeln!(out, "}}");
    }

    out
}

/// Resolve the NAViz `movement.max_speed` from the target's speed model.
fn movement_max_speed(target: &NeutralAtomTarget) -> f64 {
    match target.movement.speed_model.kind {
        AodSpeedModelKind::JerkLimited if target.movement.speed_model.max_velocity_m_s > 0.0 => {
            // 1 m/s == 1 µm/µs, so the SI cruise velocity is numerically equal
            // in the (µm, µs) unit system NAViz uses for this machine.
            target.movement.speed_model.max_velocity_m_s
        }
        _ => SQRT_NOMINAL_MAX_SPEED,
    }
}

/// Build a NAViz zone id of the form `zone_<kind>_<zone_id>`.
fn zone_id(kind: &ZoneKind, zone_id: u32) -> String {
    let kind_str = match kind {
        ZoneKind::Storage => "storage",
        ZoneKind::Entanglement => "entanglement",
        ZoneKind::Readout => "readout",
    };
    format!("zone_{kind_str}_{zone_id}")
}

/// Append NAViz instructions (`ops`) and/or comment lines (`notes`) for one
/// schedule action, updating `atom_pos` as atoms shuttle.
#[allow(clippy::too_many_arguments)]
fn emit_action(
    action: &NeutralAtomAction,
    site_pos: &HashMap<SiteId, Position>,
    atom_pos: &mut HashMap<AtomId, Position>,
    ops: &mut Vec<String>,
    notes: &mut Vec<String>,
    cycle: u32,
) {
    match action {
        NeutralAtomAction::Move(group) => {
            for m in &group.moves {
                if let Some(p) = site_pos.get(&m.to) {
                    atom_pos.insert(m.atom, *p);
                    ops.push(format!(
                        "move ({}, {}) atom{}",
                        num(p.x_um),
                        num(p.y_um),
                        m.atom.0
                    ));
                }
            }
        }
        NeutralAtomAction::Transfer(transfer) => {
            if let Some(p) = site_pos.get(&transfer.site) {
                atom_pos.insert(transfer.atom, *p);
                let op = match transfer.direction {
                    TransferDirection::SlmToAod => "load",
                    TransferDirection::AodToSlm => "store",
                };
                ops.push(format!(
                    "{op} ({}, {}) atom{}",
                    num(p.x_um),
                    num(p.y_um),
                    transfer.atom.0
                ));
            }
        }
        NeutralAtomAction::Entangle2 { atoms, .. } => {
            ops.push(format!("cz {{ atom{}, atom{} }}", atoms[0].0, atoms[1].0));
        }
        NeutralAtomAction::EntangleN { atoms, .. } => {
            let set = atoms
                .iter()
                .map(|a| format!("atom{}", a.0))
                .collect::<Vec<_>>()
                .join(", ");
            ops.push(format!("cz {{ {set} }}"));
        }
        NeutralAtomAction::LocalGate { atom, gate, .. } => {
            let id = format!("atom{}", atom.0);
            match gate {
                // H / U3 have no native NAViz opcode; approximate as a
                // single-axis rotation purely for visualization.
                LocalGateKind::H => {
                    ops.push(format!("ry {} {id}", num(std::f64::consts::FRAC_PI_2)));
                }
                LocalGateKind::Rz(theta) => {
                    ops.push(format!("rz {} {id}", num(*theta)));
                }
                LocalGateKind::U3 { theta, .. } => {
                    ops.push(format!("ry {} {id}", num(*theta)));
                }
            }
        }
        NeutralAtomAction::GlobalRy { theta_rad, .. } => {
            let mut atoms: Vec<AtomId> = atom_pos.keys().copied().collect();
            atoms.sort_unstable();
            let set = atoms
                .iter()
                .map(|a| format!("atom{}", a.0))
                .collect::<Vec<_>>()
                .join(", ");
            ops.push(format!("ry {} {{ {set} }}", num(*theta_rad)));
        }
        NeutralAtomAction::Measure { atom, basis, .. } => {
            notes.push(format!(
                "// [cycle {cycle}] measure atom{} ({})",
                atom.0,
                basis_str(*basis)
            ));
        }
        NeutralAtomAction::Reset { atom, .. } => {
            notes.push(format!("// [cycle {cycle}] reset atom{}", atom.0));
        }
        NeutralAtomAction::Reuse { atom, region, .. } => {
            let region = region
                .map(|r| r.0.to_string())
                .unwrap_or_else(|| "none".to_string());
            notes.push(format!(
                "// [cycle {cycle}] reuse atom{} region={region}",
                atom.0
            ));
        }
        NeutralAtomAction::Wait { duration_us } => {
            notes.push(format!("// [cycle {cycle}] wait {duration_us}us"));
        }
    }
}

fn basis_str(basis: MeasurementBasis) -> &'static str {
    match basis {
        MeasurementBasis::X => "X",
        MeasurementBasis::Y => "Y",
        MeasurementBasis::Z => "Z",
    }
}

/// Format an `f64` as a NAViz number: round to 6 decimals (absorbing
/// cross-platform float noise) and emit the shortest decimal representation.
fn num(v: f64) -> String {
    let r = (v * 1e6).round() / 1e6;
    format!("{r}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{AtomBinding, AtomSite, Position};
    use crate::schedule::{
        AtomMove, MovementGroup, NeutralAtomAction, ScheduleLayer, TrapTransfer,
    };
    use backend::{
        AodMovement, AodMovementModel, AodSpeedModel, NeutralAtomCostModel, NeutralAtomFidelity,
        NeutralAtomGrid, NeutralAtomTiming, RydbergInteraction,
    };

    fn layout_two_atoms() -> NeutralAtomLayout {
        NeutralAtomLayout {
            sites: vec![
                AtomSite {
                    id: SiteId(0),
                    position: Position {
                        x_um: 0.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(1),
                    position: Position {
                        x_um: 12.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(2),
                    position: Position {
                        x_um: 24.0,
                        y_um: 0.0,
                    },
                },
            ],
            initial_bindings: vec![
                AtomBinding {
                    atom: AtomId(0),
                    trap: TrapBinding::Slm { site: SiteId(0) },
                },
                AtomBinding {
                    atom: AtomId(1),
                    trap: TrapBinding::Slm { site: SiteId(1) },
                },
            ],
        }
    }

    #[test]
    fn naviz_emits_target_setup_atoms_and_cz() {
        let layout = layout_two_atoms();
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [AtomId(0), AtomId(1)],
                duration_us: 360,
            }],
        }];
        let out = schedule_to_naviz(&layers, &layout, "demo");
        insta::assert_snapshot!("naviz_cz", out);
    }

    #[test]
    fn naviz_emits_move_transfer_and_local_gate() {
        let layout = layout_two_atoms();
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: AtomId(0),
                        direction: TransferDirection::SlmToAod,
                        site: SiteId(0),
                        aod: crate::layout::AodTrapRef {
                            aod_id: 0,
                            row: 0,
                            col: 0,
                        },
                        duration_us: 15,
                    }),
                    NeutralAtomAction::Move(MovementGroup {
                        moves: vec![AtomMove {
                            atom: AtomId(0),
                            from: SiteId(0),
                            to: SiteId(2),
                        }],
                        duration_us: 10,
                    }),
                    NeutralAtomAction::LocalGate {
                        atom: AtomId(1),
                        gate: LocalGateKind::Rz(std::f64::consts::FRAC_PI_2),
                        duration_us: 6,
                    },
                ],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![
                    NeutralAtomAction::Entangle2 {
                        atoms: [AtomId(0), AtomId(1)],
                        duration_us: 360,
                    },
                    NeutralAtomAction::Measure {
                        atom: AtomId(0),
                        basis: MeasurementBasis::Z,
                        duration_us: 1500,
                    },
                ],
            },
        ];
        let out = schedule_to_naviz(&layers, &layout, "demo");
        insta::assert_snapshot!("naviz_move_gate_measure", out);
    }

    #[test]
    fn namachine_emits_zones_traps_time_distance() {
        let layout = layout_two_atoms();
        let target = NeutralAtomTarget {
            grid: NeutralAtomGrid {
                width_um: 420.0,
                height_um: 500.0,
            },
            zones: vec![
                backend::NeutralAtomZone {
                    zone_id: 0,
                    kind: ZoneKind::Storage,
                    rows: 2,
                    cols: 2,
                    origin_um: (0.0, 0.0),
                    site_pitch_um: (12.0, 12.0),
                    pair_gap_um: None,
                },
                backend::NeutralAtomZone {
                    zone_id: 1,
                    kind: ZoneKind::Entanglement,
                    rows: 1,
                    cols: 3,
                    origin_um: (0.0, 310.0),
                    site_pitch_um: (12.0, 10.0),
                    pair_gap_um: Some(2.0),
                },
            ],
            movement: AodMovement {
                model: AodMovementModel::RowColumnCoupled,
                aod_rows: 100,
                aod_cols: 100,
                num_aods: 1,
                min_row_col_separation_um: 2.0,
                speed_model: AodSpeedModel {
                    kind: AodSpeedModelKind::Sqrt,
                    acceleration_m_s2: 2750.0,
                    jerk_m_s3: 0.0,
                    max_velocity_m_s: 0.0,
                },
                trap_transfer_us: 15.0,
            },
            interaction: RydbergInteraction {
                rydberg_range_um: 7.5,
                min_rydberg_spacing_um: 18.75,
                max_parallel_entangling_pairs: 340,
            },
            native_gates: vec!["cz".to_string(), "rz".to_string()],
            timing: NeutralAtomTiming {
                cz_us: 0.36,
                single_qubit_us: 0.625,
                measurement_us: 1500.0,
                reset_us: 1500.0,
            },
            fidelity: NeutralAtomFidelity {
                cz: 0.995,
                single_qubit: 0.9997,
                atom_transfer: 0.999,
                coherence_time_us: 1500000.0,
            },
            error_model: None,
            cost_model: NeutralAtomCostModel {
                rydberg_stage_weight: 1.0,
                movement_time_weight: 1.0,
                trap_transfer_weight: 1.0,
                idle_time_weight: 1e-6,
            },
        };
        let out = target_to_namachine(&target, &layout, "demo_machine");
        insta::assert_snapshot!("namachine_demo", out);
    }

    #[test]
    fn namachine_uses_configured_cruise_velocity_for_jerk_limited() {
        let layout = NeutralAtomLayout::default();
        let mut target = base_target();
        target.movement.speed_model.kind = AodSpeedModelKind::JerkLimited;
        target.movement.speed_model.max_velocity_m_s = 0.42;
        let out = target_to_namachine(&target, &layout, "jerk");
        assert!(out.contains("max_speed: 0.42"), "{out}");
    }

    fn base_target() -> NeutralAtomTarget {
        NeutralAtomTarget {
            grid: NeutralAtomGrid {
                width_um: 100.0,
                height_um: 100.0,
            },
            zones: vec![],
            movement: AodMovement {
                model: AodMovementModel::RowColumnCoupled,
                aod_rows: 1,
                aod_cols: 1,
                num_aods: 1,
                min_row_col_separation_um: 2.0,
                speed_model: AodSpeedModel {
                    kind: AodSpeedModelKind::Sqrt,
                    acceleration_m_s2: 2750.0,
                    jerk_m_s3: 0.0,
                    max_velocity_m_s: 0.0,
                },
                trap_transfer_us: 15.0,
            },
            interaction: RydbergInteraction {
                rydberg_range_um: 7.5,
                min_rydberg_spacing_um: 18.75,
                max_parallel_entangling_pairs: 1,
            },
            native_gates: vec!["cz".to_string()],
            timing: NeutralAtomTiming {
                cz_us: 0.36,
                single_qubit_us: 0.625,
                measurement_us: 1500.0,
                reset_us: 1500.0,
            },
            fidelity: NeutralAtomFidelity {
                cz: 0.995,
                single_qubit: 0.9997,
                atom_transfer: 0.999,
                coherence_time_us: 1500000.0,
            },
            error_model: None,
            cost_model: NeutralAtomCostModel {
                rydberg_stage_weight: 1.0,
                movement_time_weight: 1.0,
                trap_transfer_weight: 1.0,
                idle_time_weight: 1e-6,
            },
        }
    }
}
