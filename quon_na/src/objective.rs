//! Linear schedule objective from the target's `cost_model` weights.
//!
//! The canonical combination is architecture_model.md §9:
//!
//! ```text
//! cost = w_stage · n_rydberg_stages
//!      + w_move  · Σ_steps t_move(d_max(step))
//!      + w_xfer  · n_trap_transfers
//!      + w_idle  · Σ_atoms t_idle(atom)
//! ```
//!
//! `t_move` is the duration already stamped on each emitted move (microseconds).
//! Idle time is the per-atom gap between the schedule's layer-max wall clock
//! and the layers in which that atom appears, summed over atoms. The verified
//! report reads those counts off the emitted `ScheduleSpec` after verification,
//! not off planner-internal counters.

use std::collections::{BTreeMap, BTreeSet};

use backend::NeutralAtomCostModel;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::geometry::{SpeedModel, movement_duration_for_model};

/// Placeholder weights shipped on the checked-in neutral-atom targets.
///
/// architecture_model.md §8.6 / §9: these are tuning knobs, not measurements.
/// The time-shaped placer keeps its existing choice when a target still
/// carries exactly this vector. Any other vector is scored with the formula
/// above.
pub const PLACEHOLDER_COST_WEIGHTS: NeutralAtomCostModel = NeutralAtomCostModel {
    rydberg_stage_weight: 1.0,
    movement_time_weight: 1.0,
    trap_transfer_weight: 1.0,
    idle_time_weight: 0.000001,
};

/// One weighted objective, with every term that went into the total.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleObjective {
    pub rydberg_stages: u64,
    pub movement_time_us: u64,
    pub trap_transfers: u64,
    pub idle_time_us: u64,
    pub rydberg_stage_weight: f64,
    pub movement_time_weight: f64,
    pub trap_transfer_weight: f64,
    pub idle_time_weight: f64,
    /// `w_stage · stages + w_move · movement_time_us + w_xfer · transfers + w_idle · idle_time_us`.
    pub total: f64,
}

/// The target weights cannot be used as an objective.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ObjectiveError {
    #[error(
        "cost_model weights must be finite and non-negative (rydberg_stage_weight, movement_time_weight, trap_transfer_weight, idle_time_weight)"
    )]
    InvalidWeights,
}

/// Movement-time and transfer cost of one gate orientation.
///
/// One rearrangement group whose duration is `t(max(dist_a, dist_b))`, plus
/// two trap transfers per atom that actually moves. The Rydberg stage is the
/// same for both orientations of a gate, so it is left to the schedule-level
/// total.
pub fn orientation_cost(
    dist_a_um: f64,
    dist_b_um: f64,
    weights: &NeutralAtomCostModel,
    speed: &SpeedModel,
) -> f64 {
    let n_moves = u64::from(dist_a_um >= 1e-9) + u64::from(dist_b_um >= 1e-9);
    let d_max = dist_a_um.max(dist_b_um);
    group_weighted_cost(n_moves as usize, d_max, weights, speed)
}

/// Weighted movement-time and transfer cost of one AOD movement group.
pub fn group_weighted_cost(
    n_moves: usize,
    d_max_um: f64,
    weights: &NeutralAtomCostModel,
    speed: &SpeedModel,
) -> f64 {
    let movement_us = if d_max_um >= 1e-9 {
        movement_duration_for_model(d_max_um, speed) as f64
    } else {
        0.0
    };
    weights.movement_time_weight * movement_us
        + weights.trap_transfer_weight * (2.0 * n_moves as f64)
}

/// §9 total from already-counted components.
pub fn weighted_total(
    rydberg_stages: u64,
    movement_time_us: u64,
    trap_transfers: u64,
    idle_time_us: u64,
    weights: &NeutralAtomCostModel,
) -> Result<f64, ObjectiveError> {
    if !weights_usable(weights) {
        return Err(ObjectiveError::InvalidWeights);
    }
    Ok(weights.rydberg_stage_weight * rydberg_stages as f64
        + weights.movement_time_weight * movement_time_us as f64
        + weights.trap_transfer_weight * trap_transfers as f64
        + weights.idle_time_weight * idle_time_us as f64)
}

fn weights_usable(weights: &NeutralAtomCostModel) -> bool {
    [
        weights.rydberg_stage_weight,
        weights.movement_time_weight,
        weights.trap_transfer_weight,
        weights.idle_time_weight,
    ]
    .into_iter()
    .all(|weight| weight.is_finite() && weight >= 0.0)
}

/// Objective of a schedule that has already passed verification.
///
/// Counts come from `spec` alone: Rydberg stages are entangle layers, movement
/// time is the sum of move durations, transfers are transfer ops, and idle
/// time is the sum over atoms of wall-clock time spent outside layers that
/// name that atom.
#[cfg(feature = "mlir")]
pub fn objective_from_verified_schedule(
    spec: &crate::dialect::ScheduleSpec,
    weights: &NeutralAtomCostModel,
) -> Result<ScheduleObjective, ObjectiveError> {
    let (rydberg_stages, movement_time_us, trap_transfers, idle_time_us) =
        components_from_spec(spec);
    let total = weighted_total(
        rydberg_stages,
        movement_time_us,
        trap_transfers,
        idle_time_us,
        weights,
    )?;
    Ok(ScheduleObjective {
        rydberg_stages,
        movement_time_us,
        trap_transfers,
        idle_time_us,
        rydberg_stage_weight: weights.rydberg_stage_weight,
        movement_time_weight: weights.movement_time_weight,
        trap_transfer_weight: weights.trap_transfer_weight,
        idle_time_weight: weights.idle_time_weight,
        total,
    })
}

#[cfg(feature = "mlir")]
fn components_from_spec(spec: &crate::dialect::ScheduleSpec) -> (u64, u64, u64, u64) {
    use crate::dialect::ActionSpec;

    let mut universe = BTreeSet::new();
    for layer in &spec.layers {
        for action in &layer.actions {
            collect_atoms(action, &mut universe);
        }
    }

    let mut rydberg_stages = 0u64;
    let mut movement_time_us = 0u64;
    let mut trap_transfers = 0u64;
    let mut total_time_us = 0u64;
    let mut active_us: BTreeMap<u32, u64> = BTreeMap::new();

    for layer in &spec.layers {
        let mut layer_max = 0u64;
        let mut seen = BTreeSet::new();
        let mut has_global_ry = false;
        let mut has_entangle = false;
        for action in &layer.actions {
            layer_max = layer_max.max(action_duration(action));
            match action {
                ActionSpec::Move { duration_us, .. } => movement_time_us += *duration_us,
                ActionSpec::Transfer(_) => trap_transfers += 1,
                ActionSpec::Entangle { .. } => has_entangle = true,
                ActionSpec::GlobalRy { .. } => has_global_ry = true,
                _ => {}
            }
            collect_atoms(action, &mut seen);
        }
        if has_entangle {
            rydberg_stages += 1;
        }
        if has_global_ry {
            seen.clone_from(&universe);
        }
        total_time_us += layer_max;
        for atom in seen {
            *active_us.entry(atom).or_default() += layer_max;
        }
    }

    let idle_time_us = universe
        .iter()
        .map(|atom| {
            let active = active_us.get(atom).copied().unwrap_or(0);
            total_time_us.saturating_sub(active)
        })
        .sum();
    (
        rydberg_stages,
        movement_time_us,
        trap_transfers,
        idle_time_us,
    )
}

#[cfg(feature = "mlir")]
fn action_duration(action: &crate::dialect::ActionSpec) -> u64 {
    use crate::dialect::ActionSpec;
    match action {
        ActionSpec::Move { duration_us, .. }
        | ActionSpec::Entangle { duration_us, .. }
        | ActionSpec::LocalGate { duration_us, .. }
        | ActionSpec::GlobalRy { duration_us, .. }
        | ActionSpec::Measure { duration_us, .. }
        | ActionSpec::Reset { duration_us, .. }
        | ActionSpec::Reuse { duration_us, .. }
        | ActionSpec::Wait { duration_us } => *duration_us,
        ActionSpec::Transfer(transfer) => transfer.duration_us,
    }
}

#[cfg(feature = "mlir")]
fn collect_atoms(action: &crate::dialect::ActionSpec, atoms: &mut BTreeSet<u32>) {
    use crate::dialect::ActionSpec;
    match action {
        ActionSpec::Move { moves, .. } => {
            for atom_move in moves {
                atoms.insert(atom_move.atom);
            }
        }
        ActionSpec::Transfer(transfer) => {
            atoms.insert(transfer.atom);
        }
        ActionSpec::Entangle { pairs, .. } => {
            for pair in pairs {
                atoms.insert(pair.lhs.atom);
                atoms.insert(pair.rhs.atom);
            }
        }
        ActionSpec::LocalGate { atom, .. }
        | ActionSpec::Measure { atom, .. }
        | ActionSpec::Reset { atom, .. }
        | ActionSpec::Reuse { atom, .. } => {
            atoms.insert(*atom);
        }
        ActionSpec::GlobalRy { .. } | ActionSpec::Wait { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights(stage: f64, movement: f64, transfer: f64, idle: f64) -> NeutralAtomCostModel {
        NeutralAtomCostModel {
            rydberg_stage_weight: stage,
            movement_time_weight: movement,
            trap_transfer_weight: transfer,
            idle_time_weight: idle,
        }
    }

    #[test]
    fn placeholder_idle_weight_matches_target_json_literal() {
        let parsed: f64 = serde_json::from_str("0.000001").expect("parse");
        assert_eq!(parsed, PLACEHOLDER_COST_WEIGHTS.idle_time_weight);
        assert_eq!(
            PLACEHOLDER_COST_WEIGHTS.rydberg_stage_weight,
            PLACEHOLDER_COST_WEIGHTS.movement_time_weight
        );
        assert_eq!(PLACEHOLDER_COST_WEIGHTS.trap_transfer_weight, 1.0);
    }

    #[test]
    fn weighted_total_is_the_section_9_dot_product() {
        let total = weighted_total(2, 10, 4, 100, &weights(3.0, 0.5, 7.0, 0.01)).expect("weights");
        assert_eq!(total, 3.0 * 2.0 + 0.5 * 10.0 + 7.0 * 4.0 + 0.01 * 100.0);
    }

    #[test]
    fn non_finite_weight_is_rejected() {
        let err = weighted_total(0, 0, 0, 0, &weights(f64::NAN, 1.0, 1.0, 0.0)).expect_err("nan");
        assert_eq!(err, ObjectiveError::InvalidWeights);
        let err = weighted_total(0, 0, 0, 0, &weights(-1.0, 1.0, 1.0, 0.0)).expect_err("neg");
        assert_eq!(err, ObjectiveError::InvalidWeights);
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn verified_spec_objective_counts_each_component() {
        use crate::dialect::{
            ActionSpec, EntanglePairSpec, LayerSpec, MoveSpec, PositionedAtom, ScheduleSpec,
            TransferDirection, TransferSpec,
        };

        let spec = ScheduleSpec {
            target_id: "objective".into(),
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            aod_min_separation_um: 2.0,
            layers: vec![
                LayerSpec {
                    cycle: 0,
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
                            to_x_um: 1.0,
                            to_y_um: 0.0,
                        }],
                        duration_us: 10,
                    }],
                },
                LayerSpec {
                    cycle: 1,
                    actions: vec![
                        ActionSpec::Transfer(TransferSpec {
                            atom: 0,
                            site: 1,
                            aod_id: 0,
                            row: 0,
                            col: 0,
                            direction: TransferDirection::AodToSlm,
                            duration_us: 15,
                        }),
                        ActionSpec::Transfer(TransferSpec {
                            atom: 1,
                            site: 1,
                            aod_id: 0,
                            row: 1,
                            col: 0,
                            direction: TransferDirection::SlmToAod,
                            duration_us: 15,
                        }),
                    ],
                },
                LayerSpec {
                    cycle: 2,
                    actions: vec![ActionSpec::Entangle {
                        pairs: vec![EntanglePairSpec {
                            lhs: PositionedAtom {
                                atom: 0,
                                x_um: 0.0,
                                y_um: 0.0,
                            },
                            rhs: PositionedAtom {
                                atom: 1,
                                x_um: 6.0,
                                y_um: 0.0,
                            },
                        }],
                        duration_us: 1,
                    }],
                },
                LayerSpec {
                    cycle: 3,
                    actions: vec![ActionSpec::Wait { duration_us: 5 }],
                },
            ],
        };
        let objective = objective_from_verified_schedule(&spec, &weights(2.0, 3.0, 4.0, 5.0))
            .expect("objective");
        assert_eq!(objective.rydberg_stages, 1);
        assert_eq!(objective.movement_time_us, 10);
        assert_eq!(objective.trap_transfers, 2);
        // Wall clock is 10+15+1+5 = 31. Atom 0 is active in the move, the
        // transfer layer, and the entangle (10+15+1). Atom 1 is active in the
        // transfer layer and the entangle (15+1). Both are idle during the wait.
        assert_eq!(objective.idle_time_us, (31 - 26) + (31 - 16));
        assert_eq!(
            objective.total,
            weighted_total(
                1,
                10,
                2,
                objective.idle_time_us,
                &weights(2.0, 3.0, 4.0, 5.0)
            )
            .expect("total")
        );
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn changing_target_weights_selects_a_different_verified_schedule() {
        use crate::entangling_schedule::schedule_entangling_layers;
        use crate::geometry::{SpeedModel, SpeedModelKind};
        use crate::graph::{
            DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
            LogicalQubitId, SegmentKind,
        };
        use crate::layout::{AtomBinding, AtomId, NeutralAtomLayout, SiteId, TrapBinding};
        use crate::lower::{ScheduleLowerParams, lower_schedule};
        use crate::replay::{DeclaredArchitecture, verify_emitted_schedule};
        use crate::schedule::NeutralAtomAction;
        use crate::schedule_entry::schedule_from_graph;
        use crate::zoned::{
            AwareSearchParams, PlacementCostModel, PlacerMode, ZoneKind, ZoneSpec,
            ZonedArchitecture, schedule_zoned_with_aware_params,
        };

        // One wide pair. Atom 0 starts on the left site; atom 1 starts 200 µm
        // to the left. Keeping that orientation moves only atom 1 (2 transfers,
        // longer d_max). Swapping the pair moves both atoms (4 transfers, shorter
        // d_max). The two weight vectors pick opposite orientations.
        let arch = ZonedArchitecture {
            zones: vec![
                ZoneSpec {
                    zone_id: 0,
                    kind: ZoneKind::Storage,
                    rows: 2,
                    cols: 2,
                    origin_um: (-200.0, 0.0),
                    site_pitch_um: (4.0, 4.0),
                    pair_gap_um: None,
                },
                ZoneSpec {
                    zone_id: 1,
                    kind: ZoneKind::Entanglement,
                    rows: 1,
                    cols: 2,
                    origin_um: (0.0, 0.0),
                    site_pitch_um: (12.0, 10.0),
                    pair_gap_um: Some(6.0),
                },
            ],
            speed_model: SpeedModel {
                kind: SpeedModelKind::Sqrt,
                acceleration_m_s2: 2750.0,
                jerk_m_s3: 0.0,
                max_velocity_m_s: 0.0,
            },
            trap_transfer_us: 15,
            require_readout_zone: false,
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            aod_min_separation_um: 2.0,
        };
        let prefer_shorter_move = weights(1.0, 1.0, 0.0, 0.0);
        let prefer_fewer_transfers = weights(1.0, 0.0, 1.0, 0.0);

        let (short_spec, short_transfers, short_movement, short_obj) =
            scheduled(&arch, prefer_shorter_move);
        let (transfer_spec, transfer_transfers, transfer_movement, transfer_obj) =
            scheduled(&arch, prefer_fewer_transfers);
        assert_ne!(
            short_transfers, transfer_transfers,
            "weight change must select a different schedule"
        );
        assert!(
            short_transfers > transfer_transfers,
            "shorter-move weights take the two-atom move ({short_transfers} transfers), fewer-transfer weights take {transfer_transfers} transfers"
        );
        assert!(short_movement < transfer_movement);

        let short_under_transfer_weights =
            objective_from_verified_schedule(&short_spec, &prefer_fewer_transfers)
                .expect("rescore");
        let transfer_under_move_weights =
            objective_from_verified_schedule(&transfer_spec, &prefer_shorter_move)
                .expect("rescore");
        assert!(short_obj.total < transfer_under_move_weights.total);
        assert!(transfer_obj.total < short_under_transfer_weights.total);
        assert_eq!(short_obj.movement_time_us, short_movement);
        assert_eq!(short_obj.trap_transfers, short_transfers);
        assert_eq!(transfer_obj.trap_transfers, transfer_transfers);

        fn scheduled(
            arch: &ZonedArchitecture,
            weights: NeutralAtomCostModel,
        ) -> (crate::dialect::ScheduleSpec, u64, u64, ScheduleObjective) {
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
            let mut request =
                schedule_entangling_layers(schedule_from_graph(graph).expect("request"), 340)
                    .expect("layers")
                    .request;
            request.layout = Some(NeutralAtomLayout {
                sites: Vec::new(),
                initial_bindings: vec![
                    AtomBinding {
                        atom: AtomId(0),
                        trap: TrapBinding::Slm { site: SiteId(4) },
                    },
                    AtomBinding {
                        atom: AtomId(1),
                        trap: TrapBinding::Slm { site: SiteId(0) },
                    },
                ],
                declared_initial_bindings: Vec::new(),
            });
            let result = schedule_zoned_with_aware_params(
                request,
                arch,
                PlacerMode::RoutingAgnostic,
                AwareSearchParams::default(),
                PlacementCostModel::Weighted {
                    weights,
                    speed_model: arch.speed_model,
                },
            )
            .expect("zoned");
            let layout = result.request.layout.as_ref().expect("layout");
            let spec = lower_schedule(
                &result.request,
                &ScheduleLowerParams {
                    target_id: "weight-tradeoff".into(),
                    rydberg_range_um: arch.rydberg_range_um,
                    min_rydberg_spacing_um: arch.min_rydberg_spacing_um,
                    aod_min_separation_um: arch.aod_min_separation_um,
                },
            )
            .expect("lower");
            let declared = DeclaredArchitecture {
                sites: layout.sites.clone(),
                initial_bindings: layout.declared_initial_bindings.clone(),
                zones: arch.zones.clone(),
                check_zones: true,
                require_readout_zone: false,
                rydberg_range_um: arch.rydberg_range_um,
                min_rydberg_spacing_um: arch.min_rydberg_spacing_um,
                aod_min_separation_um: arch.aod_min_separation_um,
            };
            verify_emitted_schedule(&spec, &declared).expect("verified schedule");
            let objective = objective_from_verified_schedule(&spec, &weights).expect("objective");
            let trap_transfers = result
                .request
                .layers
                .iter()
                .flat_map(|layer| &layer.actions)
                .filter(|action| matches!(action, NeutralAtomAction::Transfer(_)))
                .count() as u64;
            let movement_time_us = result
                .request
                .layers
                .iter()
                .flat_map(|layer| &layer.actions)
                .filter_map(|action| match action {
                    NeutralAtomAction::Move(group) => Some(group.duration_us),
                    _ => None,
                })
                .sum();
            (spec, trap_transfers, movement_time_us, objective)
        }
    }
}
