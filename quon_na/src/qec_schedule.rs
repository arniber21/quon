//! Hybrid QEC → NA schedule: expand workload rounds, plan inside each round,
//! barrier across rounds (ADR-0016 / issue #248).
//!
//! Flow:
//! 1. [`quon_qec::expand_workload`] → physical CNOT / measure / reset rounds
//! 2. Per round: Misra–Gries entangling → place→move (or zoned) → terminals
//! 3. Append a durable [`NeutralAtomAction::Wait`] after each memory round
//! 4. Concatenate rounds; compact with Barrier cuts at Wait markers (fail-closed)
//! 5. Resource report sized from code blocks + memory-round metadata

use quon_qec::{
    ExpandedBlock, ExpandedWorkload, PhysicalAtomId, PhysicalCnot, PhysicalRound, QecWorkload,
    RoundKind, RoundTerminal, expand_workload,
};

use crate::compaction::{
    CompactionError, CompactionOptions, ScheduleDependency, ScheduleDependencyKind,
    compact_schedule, infer_atom_dependencies,
};
use crate::entangling_schedule::schedule_entangling_layers;
use crate::graph::{
    DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
    LogicalQubitId, SegmentKind,
};
use crate::layout::{AtomId, NeutralAtomLayout};
use crate::movement::plan_aod_movement;
use crate::pipeline::{
    NaBackendKind, NaPipelineError, NaScheduleArtifacts, NaScheduleOptions, movement_params,
    validate_speed_model, zoned_architecture,
};
use crate::placement::place;
use crate::qec::{CodeBlock, CodeBlockId};
use crate::report::{attach_qec_error_budget, build_resource_report};
use crate::schedule::{MeasurementBasis, NeutralAtomAction, ScheduleLayer};
use crate::schedule_entry::{GraphScheduleRequest, schedule_from_graph};
use crate::zoned::schedule_zoned;
use backend::NeutralAtomTarget;

/// Default duration (µs) for synthetic measure/reset actions from QEC expansion.
const QEC_TERMINAL_DURATION_US: u64 = 1;

/// Durable round-barrier Wait duration (µs). Survives [`crate::lower::lower_schedule`].
const QEC_ROUND_BARRIER_WAIT_US: u64 = 1;

/// Expand a [`QecWorkload`] and schedule it with per-round planners + barriers.
pub fn run_from_qec_workload(
    workload: &QecWorkload,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let expanded = expand_workload(workload)?;
    schedule_expanded(&expanded, na, opts)
}

fn schedule_expanded(
    expanded: &ExpandedWorkload,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    let code_blocks = code_blocks_from_expanded(expanded);
    let all_atoms = all_physical_atoms(expanded);
    let logical_qubits = expanded.blocks.len() as u64;

    let mut all_layers: Vec<ScheduleLayer> = Vec::new();
    let mut combined_interactions: Vec<Interaction> = Vec::new();
    let mut combined_segments: Vec<InteractionSegment> = Vec::new();
    let mut next_interaction_id = 0u32;
    let mut shared_layout: Option<NeutralAtomLayout> = None;

    for round in &expanded.rounds {
        let (mut round_layers, round_interactions, round_segment) = schedule_round(
            round,
            &all_atoms,
            na,
            opts,
            &mut next_interaction_id,
            &mut shared_layout,
        )?;

        if let Some(segment) = round_segment {
            combined_segments.push(segment);
            combined_interactions.extend(round_interactions);
        }

        if round.kind == RoundKind::MemoryRound {
            let wait_cycle = round_layers
                .last()
                .map(|l| l.cycle.saturating_add(1))
                .unwrap_or(0);
            round_layers.push(ScheduleLayer {
                cycle: wait_cycle,
                actions: vec![NeutralAtomAction::Wait {
                    duration_us: QEC_ROUND_BARRIER_WAIT_US,
                }],
            });
        }

        if round_layers.is_empty() {
            continue;
        }
        all_layers.extend(round_layers);
    }

    for (i, layer) in all_layers.iter_mut().enumerate() {
        layer.cycle = i as u32;
    }

    let vertices: Vec<LogicalQubitId> = all_atoms.iter().map(|a| LogicalQubitId(a.0)).collect();
    let graph = InteractionGraph::from_interactions(
        vertices,
        combined_interactions,
        combined_segments,
        DEFAULT_GAMMA,
    )
    .map_err(|e| NaPipelineError::ScheduleFromGraph(e.into()))?;

    let mut req = GraphScheduleRequest {
        graph,
        layers: all_layers,
        layout: shared_layout,
    };

    if opts.compact && !req.layers.is_empty() {
        let mut deps = infer_atom_dependencies(&req.layers);
        let cuts = round_barrier_cuts(&req.layers);
        if cuts.is_empty() && expanded.memory_round_count() > 0 {
            return Err(NaPipelineError::Compaction(CompactionError::Conflict(
                "QEC hybrid schedule missing durable round-barrier cuts".into(),
            )));
        }
        for &(before, after) in &cuts {
            deps.push(ScheduleDependency {
                before,
                after,
                kind: ScheduleDependencyKind::Barrier,
            });
        }
        // Preserve planner Move/Entangle order: exclusive-cycle ASAP may otherwise
        // reorder independent layers and break occupancy replay at lower_schedule.
        // Round Barrier cuts still forbid cross-round same-cycle merges.
        for i in 0..req.layers.len().saturating_sub(1) {
            deps.push(ScheduleDependency {
                before: i as u32,
                after: (i + 1) as u32,
                kind: ScheduleDependencyKind::Barrier,
            });
        }
        let compact_opts = CompactionOptions {
            arch: None,
            legality: None,
            greedy: false,
        };
        let compacted = compact_schedule(req.clone(), &deps, &compact_opts)?;
        if opts.dump_ir {
            eprintln!(
                "--- QEC after compaction ---\nlayers={}",
                compacted.request.layers.len()
            );
        }
        req = compacted.request;
    }

    let mut report = build_resource_report(&req.layers, Some(&code_blocks), None)?;
    // Distance ownership: `with_code_blocks` via `CodeFamily::distance()`.
    report.memory_rounds = Some(expanded.memory_round_count() as u64);
    let report = match na.error_model.as_ref() {
        Some(model) => attach_qec_error_budget(report, Some(model))?,
        None => report,
    };

    Ok(NaScheduleArtifacts {
        layers: req.layers.clone(),
        resource_report: report,
        logical_qubits: logical_qubits.max(1),
        request: req,
    })
}

/// Place→move (or zoned) *inside* one physical round, then append terminals.
fn schedule_round(
    round: &PhysicalRound,
    all_atoms: &[PhysicalAtomId],
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    next_interaction_id: &mut u32,
    shared_layout: &mut Option<NeutralAtomLayout>,
) -> Result<
    (
        Vec<ScheduleLayer>,
        Vec<Interaction>,
        Option<InteractionSegment>,
    ),
    NaPipelineError,
> {
    let mut layers = Vec::new();
    let mut interactions = Vec::new();
    let mut segment = None;

    if !round.entangling.is_empty() {
        let (round_interactions, round_segment) =
            cnots_to_interactions(&round.entangling, next_interaction_id)?;
        let graph = cnot_graph(all_atoms, &round_interactions, round_segment.clone())?;
        let req = schedule_from_graph(graph)?;
        let max_pairs = na.interaction.max_parallel_entangling_pairs;
        let scheduled = schedule_entangling_layers(req, max_pairs)?;
        let mut round_req = GraphScheduleRequest {
            graph: scheduled.request.graph,
            layers: scheduled.request.layers,
            layout: shared_layout.clone(),
        };

        round_req = match opts.backend {
            NaBackendKind::Zoned => {
                let arch = zoned_architecture(na);
                schedule_zoned(round_req, &arch, opts.placer)?.request
            }
            NaBackendKind::FlatAod => {
                if round_req.layout.is_none() {
                    round_req = place(round_req, opts.placement)?.request;
                }
                let params = movement_params(na);
                plan_aod_movement(round_req, &params)?.request
            }
        };
        if opts.dump_ir {
            eprintln!(
                "--- QEC round {:?} after movement ---\nlayers={}",
                round.kind,
                round_req.layers.len()
            );
        }
        *shared_layout = round_req.layout.clone();
        layers = round_req.layers;
        interactions = round_interactions;
        segment = Some(round_segment);
    }

    append_terminal_layers(&mut layers, &round.terminal);
    Ok((layers, interactions, segment))
}

fn append_terminal_layers(layers: &mut Vec<ScheduleLayer>, terminal: &[RoundTerminal]) {
    let measures: Vec<NeutralAtomAction> = terminal
        .iter()
        .filter_map(|t| match t {
            RoundTerminal::Measure { atom, basis } => Some(NeutralAtomAction::Measure {
                atom: AtomId(atom.0),
                basis: map_basis(*basis),
                duration_us: QEC_TERMINAL_DURATION_US,
            }),
            RoundTerminal::Reset { .. } => None,
        })
        .collect();
    let resets: Vec<NeutralAtomAction> = terminal
        .iter()
        .filter_map(|t| match t {
            RoundTerminal::Reset { atom } => Some(NeutralAtomAction::Reset {
                atom: AtomId(atom.0),
                duration_us: QEC_TERMINAL_DURATION_US,
            }),
            RoundTerminal::Measure { .. } => None,
        })
        .collect();

    let mut cycle = layers.last().map(|l| l.cycle.saturating_add(1)).unwrap_or(0);
    if !measures.is_empty() {
        layers.push(ScheduleLayer {
            cycle,
            actions: measures,
        });
        cycle = cycle.saturating_add(1);
    }
    if !resets.is_empty() {
        layers.push(ScheduleLayer {
            cycle,
            actions: resets,
        });
    }
}

fn map_basis(basis: quon_qec::LogicalBasis) -> MeasurementBasis {
    match basis {
        quon_qec::LogicalBasis::X => MeasurementBasis::X,
        quon_qec::LogicalBasis::Z => MeasurementBasis::Z,
    }
}

fn cnot_graph(
    all_atoms: &[PhysicalAtomId],
    interactions: &[Interaction],
    segment: InteractionSegment,
) -> Result<InteractionGraph, NaPipelineError> {
    let vertices: Vec<LogicalQubitId> = all_atoms.iter().map(|a| LogicalQubitId(a.0)).collect();
    InteractionGraph::from_interactions(
        vertices,
        interactions.to_vec(),
        vec![segment],
        DEFAULT_GAMMA,
    )
    .map_err(|e| NaPipelineError::ScheduleFromGraph(e.into()))
}

fn cnots_to_interactions(
    cnots: &[PhysicalCnot],
    next_interaction_id: &mut u32,
) -> Result<(Vec<Interaction>, InteractionSegment), NaPipelineError> {
    let mut interactions = Vec::with_capacity(cnots.len());
    let mut ids = Vec::with_capacity(cnots.len());
    for cnot in cnots {
        let id = InteractionId(*next_interaction_id);
        *next_interaction_id = next_interaction_id
            .checked_add(1)
            .ok_or(NaPipelineError::InteractionIdOverflow)?;
        let mut qubits = vec![
            LogicalQubitId(cnot.control.0),
            LogicalQubitId(cnot.target.0),
        ];
        qubits.sort();
        interactions.push(Interaction {
            id,
            qubits,
            gate_name: "cx".into(),
            dag_layer: 0,
            on_critical_path: false,
        });
        ids.push(id);
    }
    let segment = InteractionSegment {
        kind: SegmentKind::CommutationGroup,
        interactions: ids,
    };
    Ok((interactions, segment))
}

/// Barrier cuts at durable Wait markers (and Reset→next as a safety net).
pub fn round_barrier_cuts(layers: &[ScheduleLayer]) -> Vec<(u32, u32)> {
    let mut cuts = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let is_wait = layer
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Wait { .. }));
        let is_reset = layer
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Reset { .. }));
        if !is_wait && !is_reset {
            continue;
        }
        if is_reset
            && !is_wait
            && layers.get(i + 1).is_some_and(|next| {
                next.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
            })
        {
            continue;
        }
        if let Some((j, _)) = layers.iter().enumerate().skip(i + 1).find(|(_, l)| {
            l.actions.iter().any(|a| {
                matches!(
                    a,
                    NeutralAtomAction::Entangle2 { .. }
                        | NeutralAtomAction::EntangleN { .. }
                        | NeutralAtomAction::Measure { .. }
                )
            })
        }) {
            cuts.push((i as u32, j as u32));
        }
    }
    cuts
}

fn code_blocks_from_expanded(expanded: &ExpandedWorkload) -> Vec<CodeBlock> {
    expanded
        .blocks
        .iter()
        .map(|b: &ExpandedBlock| CodeBlock {
            id: CodeBlockId(b.logical_id.0),
            family: b.code_family.clone(),
            logical_qubits: vec![b.logical_id],
            atoms: b.atoms.iter().map(|a| AtomId(a.0)).collect(),
        })
        .collect()
}

fn all_physical_atoms(expanded: &ExpandedWorkload) -> Vec<PhysicalAtomId> {
    let mut atoms = Vec::new();
    for b in &expanded.blocks {
        atoms.extend(b.atoms.iter().copied());
    }
    atoms
}

#[cfg(test)]
mod tests {
    use super::*;
    use quon_qec::{LogicalBasis, SourceFamily, WorkloadBuilder};

    fn load_na() -> NeutralAtomTarget {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../targets/neutral_atom/generic_rna_v0.json"
        ));
        let loaded = backend::json::load(path).expect("load target");
        loaded.neutral_atom_target().expect("na target").clone()
    }

    fn d3_workload() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        b.finish()
    }

    #[test]
    fn repetition_d3_schedules_with_qec_report_fields() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts = run_from_qec_workload(&d3_workload(), &na, opts).expect("schedule");
        let report = &artifacts.resource_report;

        assert_eq!(report.logical_qubits, 1);
        assert_eq!(report.physical_atoms, 5);
        assert_eq!(report.atoms_per_logical, Some(5));
        assert_eq!(report.code_family.as_deref(), Some("repetition_code_toy"));
        assert_eq!(report.distance, Some(3));
        assert_eq!(report.memory_rounds, Some(2));

        assert!(
            report.measurement_rounds >= 3,
            "expected ≥3 measurement rounds, got {}",
            report.measurement_rounds
        );
        assert!(
            report.reset_rounds >= 2,
            "expected ≥2 reset rounds, got {}",
            report.reset_rounds
        );
        assert!(
            report.entangle2_count >= 8,
            "two rounds × 4 CNOTs ⇒ ≥8 Entangle2, got {}",
            report.entangle2_count
        );

        let has_measure = artifacts.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Measure { .. }))
        });
        let has_reset = artifacts.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Reset { .. }))
        });
        let has_entangle = artifacts.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
        });
        let has_wait = artifacts.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
        });
        assert!(has_measure && has_reset && has_entangle && has_wait);
    }

    #[test]
    fn round_barriers_constrain_compaction() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts = run_from_qec_workload(&d3_workload(), &na, opts).expect("schedule");

        let cuts = round_barrier_cuts(&artifacts.layers);
        assert!(
            !cuts.is_empty(),
            "expected non-empty round barrier cuts after compact"
        );

        let wait_count = artifacts
            .layers
            .iter()
            .filter(|l| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
            })
            .count();
        assert!(
            wait_count >= 2,
            "expected ≥2 durable Wait barriers (one per memory round), got {wait_count}"
        );

        let last_memory_reset_cycle = artifacts
            .layers
            .iter()
            .filter_map(|l| {
                let is_check_reset = l.actions.iter().any(|a| match a {
                    NeutralAtomAction::Reset { atom, .. } => atom.0 == 1 || atom.0 == 3,
                    _ => false,
                });
                is_check_reset.then_some(l.cycle)
            })
            .max()
            .expect("memory reset");

        let logical_measure_cycle = artifacts
            .layers
            .iter()
            .filter_map(|l| {
                let is_data_measure = l.actions.iter().any(|a| match a {
                    NeutralAtomAction::Measure { atom, .. } => {
                        atom.0 == 0 || atom.0 == 2 || atom.0 == 4
                    }
                    _ => false,
                });
                is_data_measure.then_some(l.cycle)
            })
            .min()
            .expect("logical measure");

        assert!(
            last_memory_reset_cycle < logical_measure_cycle,
            "last memory Reset cycle {last_memory_reset_cycle} must precede logical Measure cycle {logical_measure_cycle}"
        );

        for layer in &artifacts.layers {
            let has_check_reset = layer.actions.iter().any(|a| match a {
                NeutralAtomAction::Reset { atom, .. } => atom.0 == 1 || atom.0 == 3,
                _ => false,
            });
            let has_data_measure = layer.actions.iter().any(|a| match a {
                NeutralAtomAction::Measure { atom, .. } => {
                    atom.0 == 0 || atom.0 == 2 || atom.0 == 4
                }
                _ => false,
            });
            assert!(
                !(has_check_reset && has_data_measure),
                "cycle {} merged check Reset with data Measure across round barrier",
                layer.cycle
            );
        }
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn hybrid_schedule_lowers_to_verifiable_quantum_na() {
        use crate::dialect::{self as qna, schedule_module};
        use crate::lower::{ScheduleLowerParams, lower_schedule};
        use melior::ir::BlockLike;

        let na = load_na();
        for compact in [false, true] {
            let opts = NaScheduleOptions {
                compact,
                dump_ir: false,
                ..Default::default()
            };
            let artifacts =
                run_from_qec_workload(&d3_workload(), &na, opts).unwrap_or_else(|e| {
                    panic!("schedule compact={compact}: {e}");
                });
            let params =
                ScheduleLowerParams::from_target("generic_reconfigurable_neutral_atom_v0", &na);
            let spec = lower_schedule(&artifacts.request, &params).unwrap_or_else(|e| {
                panic!("lower compact={compact}: {e}");
            });
            let context = melior::Context::new();
            let module = schedule_module(&context, &spec).unwrap_or_else(|e| match e {
                qna::BuildError::Verify(err) => panic!("verify compact={compact}: {err}"),
                other => panic!("build compact={compact}: {other}"),
            });
            let schedule = module.body().first_operation().expect("schedule op");
            qna::verify(&schedule).unwrap_or_else(|e| {
                panic!("verify compact={compact}: {e}");
            });
        }
    }
}
