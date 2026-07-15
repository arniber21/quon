//! Hybrid QEC → NA schedule: expand workload rounds, plan inside each round,
//! barrier across rounds (ADR-0016 / issue #248).
//!
//! Flow:
//! 1. [`quon_qec::expand_workload`] → physical CZ / measure / reset rounds
//! 2. Per round: Misra–Gries / ASAP entangling layers only, then terminal layers
//! 3. Concatenate rounds; record Barrier cuts between non-empty rounds
//! 4. One zoned (or flat AOD) pass on the full schedule for coherent layout
//! 5. Compaction with atom hazards + round Barriers
//! 6. Resource report sized from code blocks + memory-round metadata

use quon_qec::{
    ExpandedBlock, ExpandedWorkload, PhysicalAtomId, PhysicalCz, PhysicalRound, QecWorkload,
    RoundTerminal, expand_workload,
};

use crate::compaction::{
    ScheduleDependency, ScheduleDependencyKind, compact_schedule, infer_atom_dependencies,
};
use crate::entangling_schedule::schedule_entangling_layers;
use crate::graph::{
    DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
    LogicalQubitId, SegmentKind,
};
use crate::layout::AtomId;
use crate::movement::plan_aod_movement;
use crate::pipeline::{
    NaBackendKind, NaPipelineError, NaScheduleArtifacts, NaScheduleOptions, compaction_options,
    movement_params, validate_speed_model, zoned_architecture,
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

    for round in &expanded.rounds {
        // Fresh id space per round for the local Misra–Gries pass.
        let mut round_id = 0u32;
        let round_layers = schedule_round_entangling(round, &all_atoms, na, &mut round_id)?;

        if !round.entangling.is_empty() {
            let (interactions, segment) =
                czs_to_interactions(&round.entangling, &mut next_interaction_id);
            combined_segments.push(segment);
            combined_interactions.extend(interactions);
        }

        if round_layers.is_empty() {
            continue;
        }
        all_layers.extend(round_layers);
    }

    // Dense cycle renumber before movement / compaction.
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
        layout: None,
    };

    // One coherent place/move pass over the full hybrid schedule.
    if !req.layers.is_empty() {
        req = match opts.backend {
            NaBackendKind::Zoned => {
                let arch = zoned_architecture(na);
                schedule_zoned(req, &arch, opts.placer)?.request
            }
            NaBackendKind::FlatAod => {
                let placed = place(req, opts.placement)?;
                let params = movement_params(na);
                plan_aod_movement(placed.request, &params)?.request
            }
        };
        if opts.dump_ir {
            eprintln!("--- QEC after movement ---\nlayers={}", req.layers.len());
        }
    }

    if opts.compact && !req.layers.is_empty() {
        let mut deps = infer_atom_dependencies(&req.layers);
        // Round barriers: Reset (end of memory round) must precede the next
        // Entangle2 / logical Measure — blocks cross-round compaction.
        for &(before, after) in &round_barrier_cuts(&req.layers) {
            deps.push(ScheduleDependency {
                before,
                after,
                kind: ScheduleDependencyKind::Barrier,
            });
        }
        let compact_opts = compaction_options(na, true);
        match compact_schedule(req.clone(), &deps, &compact_opts) {
            Ok(compacted) => {
                if opts.dump_ir {
                    eprintln!(
                        "--- QEC after compaction ---\nlayers={}",
                        compacted.request.layers.len()
                    );
                }
                req = compacted.request;
            }
            Err(e) => {
                if opts.dump_ir {
                    eprintln!("--- QEC compaction skipped ({e}) ---");
                }
            }
        }
    }

    let mut report = build_resource_report(&req.layers, Some(&code_blocks), None)?;
    apply_qec_round_meta(&mut report, expanded);
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

/// Entangling layers for one round (graph planners only — no movement yet).
fn schedule_round_entangling(
    round: &PhysicalRound,
    all_atoms: &[PhysicalAtomId],
    na: &NeutralAtomTarget,
    next_interaction_id: &mut u32,
) -> Result<Vec<ScheduleLayer>, NaPipelineError> {
    let mut layers = Vec::new();

    if !round.entangling.is_empty() {
        let graph = cz_graph(all_atoms, &round.entangling, next_interaction_id)?;
        let req = schedule_from_graph(graph)?;
        let max_pairs = na.interaction.max_parallel_entangling_pairs;
        let scheduled = schedule_entangling_layers(req, max_pairs)?;
        layers.extend(scheduled.request.layers);
    }

    append_terminal_layers(&mut layers, &round.terminal);
    Ok(layers)
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

fn cz_graph(
    all_atoms: &[PhysicalAtomId],
    czs: &[PhysicalCz],
    next_interaction_id: &mut u32,
) -> Result<InteractionGraph, NaPipelineError> {
    let (interactions, segment) = czs_to_interactions(czs, next_interaction_id);
    let vertices: Vec<LogicalQubitId> = all_atoms.iter().map(|a| LogicalQubitId(a.0)).collect();
    InteractionGraph::from_interactions(vertices, interactions, vec![segment], DEFAULT_GAMMA)
        .map_err(|e| NaPipelineError::ScheduleFromGraph(e.into()))
}

fn czs_to_interactions(
    czs: &[PhysicalCz],
    next_interaction_id: &mut u32,
) -> (Vec<Interaction>, InteractionSegment) {
    let mut interactions = Vec::with_capacity(czs.len());
    let mut ids = Vec::with_capacity(czs.len());
    for cz in czs {
        let id = InteractionId(*next_interaction_id);
        *next_interaction_id = next_interaction_id.saturating_add(1);
        let mut qubits = vec![LogicalQubitId(cz.a.0), LogicalQubitId(cz.b.0)];
        qubits.sort();
        interactions.push(Interaction {
            id,
            qubits,
            gate_name: "cz".into(),
            dag_layer: 0,
            on_critical_path: false,
        });
        ids.push(id);
    }
    let segment = InteractionSegment {
        kind: SegmentKind::CommutationGroup,
        interactions: ids,
    };
    (interactions, segment)
}

/// Barrier cuts at memory-round boundaries: each Reset layer must complete
/// before the next Entangle2 / Measure layer (next round or logical measure).
fn round_barrier_cuts(layers: &[ScheduleLayer]) -> Vec<(u32, u32)> {
    let mut cuts = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let is_reset = layer
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Reset { .. }));
        if !is_reset {
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

fn apply_qec_round_meta(report: &mut crate::report::ResourceReport, expanded: &ExpandedWorkload) {
    if let Some(block) = expanded.blocks.first() {
        let homogeneous = expanded
            .blocks
            .iter()
            .all(|b| b.distance == block.distance && b.code_family == block.code_family);
        if homogeneous {
            report.distance = Some(u64::from(block.distance));
        }
    }
    report.memory_rounds = Some(expanded.memory_round_count() as u64);
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
            "two rounds × 4 CZs ⇒ ≥8 Entangle2, got {}",
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
        assert!(has_measure && has_reset && has_entangle);
    }
}
