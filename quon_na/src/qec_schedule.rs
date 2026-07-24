//! Hybrid QEC → NA schedule: expand workload rounds, plan inside each round,
//! barrier across rounds (ADR-0016 / issue #248).
//!
//! Flow:
//! 1. [`quon_qec::expand_workload`] → physical CNOT / measure / reset rounds
//! 2. Per round: Misra–Gries entangling → place→move (or zoned) → terminals
//! 3. Append a durable [`NeutralAtomAction::Wait`] after each memory round
//! 4. Concatenate rounds; compact with Barrier cuts at Wait markers (fail-closed)
//! 5. Resource report sized from code blocks + memory-round metadata

use std::time::Instant;

use quon_qec::{
    ExpandedWorkload, PhysicalAtomId, PhysicalCnot, PhysicalRound, QecWorkload, RoundTerminal,
    expand_workload,
};

use crate::compaction::{
    CompactionError, CompactionOptions, ScheduleDependency, ScheduleDependencyKind,
    compact_schedule, feed_forward_dependencies, infer_atom_dependencies,
};
use crate::entangling_schedule::schedule_entangling_layers;
use crate::graph::{
    AtomVertexId, DEFAULT_GAMMA, Interaction, InteractionEdge, InteractionGraph, InteractionId,
    InteractionSegment, LogicalQubitId, SegmentKind,
};
use crate::layout::{AtomId, NeutralAtomLayout};
use crate::pipeline::{
    NaPipelineError, NaScheduleArtifacts, NaScheduleOptions, validate_speed_model,
};
use crate::plan::{QecStageAccumulator, plan_backend};
use crate::qec::code_blocks_from_expanded;
use crate::report::{attach_qec_error_budget, build_resource_report};
use crate::schedule::{LocalGateKind, MeasurementBasis, NeutralAtomAction, ScheduleLayer};
use crate::schedule_entry::{GraphScheduleRequest, schedule_from_graph};
use crate::stats::{CompactionConfig, EffectiveConfig, NaStats, StageTimingsUs};
use backend::NeutralAtomTarget;

/// Default duration (µs) for synthetic measure/reset/local actions from QEC expansion.
const QEC_TERMINAL_DURATION_US: u64 = 1;
const QEC_LOCAL_GATE_DURATION_US: u64 = 1;

/// Durable round-barrier Wait duration (µs). Survives [`crate::lower::lower_schedule`].
const QEC_ROUND_BARRIER_WAIT_US: u64 = 1;

/// Wall-clock elapsed microseconds since `start` (saturating on overflow).
fn elapsed_us(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Expand a [`QecWorkload`] and schedule it with per-round planners + barriers.
pub fn run_from_qec_workload(
    workload: &QecWorkload,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<NaScheduleArtifacts, NaPipelineError<AtomVertexId>> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let expanded = expand_workload(workload)?;
    schedule_expanded(&expanded, na, opts)
}

fn schedule_expanded(
    expanded: &ExpandedWorkload,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<NaScheduleArtifacts, NaPipelineError<AtomVertexId>> {
    let code_blocks = code_blocks_from_expanded(expanded);
    let all_atoms = all_physical_atoms(expanded);
    let logical_qubits = expanded.blocks.len() as u64;
    let pipeline_started = Instant::now();
    let mut stage_acc = QecStageAccumulator::default();

    // Issue #302 Deliverable A: `--na-state-prep exact` requests SMT-optimal
    // CZ-pair scheduling. The standalone solver
    // (`crate::exact::state_prep::schedule_exact`) is implemented and
    // unit-tested (Steane 7q/9CZ -> 3 stages), but it is NOT yet wired into
    // the QEC pipeline per-CNOT-phase scheduling -- `plan_backend` does
    // not consume `opts.state_prep`, so the heuristic zoned/flat scheduler
    // runs unchanged. Log the fallback so the request is never silently
    // honoured (issue #302: "no silent heuristic-only fallback without
    // logging").
    #[cfg(feature = "solver")]
    if opts.state_prep == crate::pipeline::StatePrepMode::Exact {
        eprintln!(
            "[quon_na] --na-state-prep exact requested for QEC workload \
             (blocks={}, memory_rounds={}), but exact state-prep scheduling \
             is not yet wired into the pipeline; using heuristic",
            expanded.blocks.len(),
            expanded.memory_round_count()
        );
    }
    #[cfg(not(feature = "solver"))]
    if opts.state_prep == crate::pipeline::StatePrepMode::Exact {
        eprintln!(
            "[quon_na] exact state-prep scheduling requires the `solver` feature, \
             using heuristic"
        );
    }

    let mut all_layers: Vec<ScheduleLayer> = Vec::new();
    let mut combined_interactions: Vec<Interaction<AtomVertexId>> = Vec::new();
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
            &mut stage_acc,
        )?;

        if let Some(segment) = round_segment {
            combined_segments.push(segment);
            combined_interactions.extend(round_interactions);
        }

        if round.kind.needs_round_barrier() {
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

    let vertices: Vec<AtomVertexId> = all_atoms.iter().copied().map(AtomVertexId::from).collect();
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

    let mut compaction_config = CompactionConfig {
        requested: opts.compact,
        applied: false,
        greedy: false,
        legality_checked: false,
    };
    let mut compaction_us = None;
    if opts.compact && !req.layers.is_empty() {
        let mut deps = infer_atom_dependencies(&req.layers);
        // Lattice-surgery measure → subsequent phase FeedForward edges
        // (ADR-0019 byproducts / measurement deps; not invented AtomHazard).
        deps.extend(lattice_surgery_feedforward_deps(&req.layers));
        let cuts = round_barrier_cuts(&req.layers);
        if cuts.is_empty() && expanded.barrier_round_count() > 0 {
            return Err(NaPipelineError::Compaction(CompactionError::Conflict(
                "QEC hybrid schedule missing durable round-barrier cuts".into(),
            )));
        }
        // Only durable Wait / round cuts are hard Barriers. Do **not** insert a
        // consecutive-layer total-order Barrier chain: that makes exclusive-cycle
        // ASAP an identity and turns compact into a no-op (ADR-0016: optimize
        // inside the round). AtomHazard cycle-order in `compact_schedule` keeps
        // Move→Entangle occupancy legal without faking a total order.
        for &(before, after) in &cuts {
            deps.push(ScheduleDependency {
                before,
                after,
                kind: ScheduleDependencyKind::Barrier,
            });
        }
        let compact_opts = CompactionOptions {
            arch: None,
            legality: None,
            greedy: true,
        };
        let stage_started = Instant::now();
        let compacted = compact_schedule(req.clone(), &deps, &compact_opts)?;
        compaction_us = Some(elapsed_us(stage_started));
        compaction_config.applied = true;
        compaction_config.greedy = compact_opts.greedy;
        compaction_config.legality_checked = compact_opts.arch.is_some();
        if opts.dump_ir {
            eprintln!(
                "--- QEC after compaction ---\nlayers={}",
                compacted.request.layers.len()
            );
        }
        req = compacted.request;
    }

    let stage_started = Instant::now();
    let mut report = build_resource_report(&req.layers, Some(&code_blocks), None)?;
    // Distance ownership: `with_code_blocks` via `CodeFamily::distance()`.
    report.memory_rounds = Some(expanded.memory_round_count() as u64);
    // Magic-state-consuming logical operation counts (issue #283).
    report.t_count = Some(expanded.t_count() as u64);
    report.tdag_count = Some(expanded.tdag_count() as u64);
    report.ccz_count = Some(expanded.ccz_count() as u64);
    report.magic_state_demand = Some(expanded.magic_state_demand() as u64);
    let report = match na.error_model.as_ref() {
        Some(model) => attach_qec_error_budget(report, Some(model))?,
        None => report,
    };
    // Analytic end-to-end fidelity estimate (Enola Eq. (1), issue #305) —
    // same unconditional overlay as the flat/zoned pipeline
    // (`pipeline::finish_pipeline`); `NeutralAtomTarget::fidelity` is
    // mandatory, so this always applies once a target is available.
    let report = report.with_fidelity_estimate(&req.layers, &na.fidelity);
    // Analytic per-atom movement-heating / atom-loss budget (issue #310,
    // [Atomique] Eqs. (1)–(2)). Optional like `error_model`: attached only
    // when the target carries `atom_loss_model`; distance measured against
    // the zoned schedule's layout (real √-law travel), else zeroed/omitted.
    let report = match na.atom_loss_model.as_ref() {
        Some(model) => report.with_atom_loss_budget(&req.layers, req.layout.as_ref(), model),
        None => report,
    };
    let report = report.with_agnostic_placer_mechanism(stage_acc.agnostic_placer_mechanism);
    let report = match stage_acc.schedule_optimality {
        Some(optimality) => report.with_schedule_optimality(optimality),
        None => report,
    };
    let resource_report_us = elapsed_us(stage_started);

    let req = project_request_to_logical(req);

    // Stage stats — issue #317 / #307. The QEC path now populates NaStats the
    // same way as `run_from_graph`, using the shared `plan_backend` timings
    // accumulated across all CNOT phases.
    let stats = NaStats {
        kind: crate::stats::NA_STATS_KIND.to_string(),
        schema_version: crate::stats::NA_STATS_SCHEMA_VERSION,
        version: Default::default(),
        config: EffectiveConfig {
            backend: opts.backend,
            placer_mode: stage_acc.placer_mode,
            placement_strategy: stage_acc.placement_strategy,
            compaction: compaction_config,
            objective: opts.objective,
        },
        stage_timings_us: StageTimingsUs {
            extract_us: None,
            schedule_from_graph_us: stage_acc.schedule_from_graph_us,
            entangling_layers_us: stage_acc.entangling_layers_us,
            zoned_schedule_us: stage_acc.zoned_schedule_us,
            placement_us: stage_acc.placement_us,
            movement_us: stage_acc.movement_us,
            compaction_us,
            resource_report_us,
            total_us: elapsed_us(pipeline_started),
        },
        search: stage_acc.search_diagnostics,
    };

    Ok(NaScheduleArtifacts {
        layers: req.layers.clone(),
        resource_report: report,
        logical_qubits: logical_qubits.max(1),
        request: req,
        stats: Some(stats),
    })
}

/// Layers, interactions, and the optional trailing segment produced for one round.
type RoundScheduleParts = (
    Vec<ScheduleLayer>,
    Vec<Interaction<AtomVertexId>>,
    Option<InteractionSegment>,
);

/// Layers, interactions, and interaction ids produced for one CNOT phase.
type CnotScheduleParts = (
    Vec<ScheduleLayer>,
    Vec<Interaction<AtomVertexId>>,
    Vec<InteractionId>,
);

/// Place→move (or zoned) *inside* one physical round with Z-then-X phases, then
/// append locals / terminals.
///
/// Phase order (ADR-0016 serial split): `local_before` → Z-CXs → `local_mid`
/// (X-check H) → X-CXs → `local_after` (X-check H) → measure/reset.
fn schedule_round(
    round: &PhysicalRound,
    all_atoms: &[PhysicalAtomId],
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    next_interaction_id: &mut u32,
    shared_layout: &mut Option<NeutralAtomLayout>,
    stage_acc: &mut QecStageAccumulator,
) -> Result<RoundScheduleParts, NaPipelineError<AtomVertexId>> {
    if round.z_cnot_count > round.entangling.len() {
        return Err(NaPipelineError::InvalidZCnotCount {
            z_cnot_count: round.z_cnot_count,
            entangling_len: round.entangling.len(),
        });
    }

    let mut layers = Vec::new();
    let mut interactions = Vec::new();
    let mut interaction_ids = Vec::new();

    append_local_gate_layers(&mut layers, &round.local_before);

    let (z_cnots, x_cnots) = round.entangling.split_at(round.z_cnot_count);

    if !z_cnots.is_empty() {
        let (phase_layers, phase_interactions, phase_ids) = schedule_cnot_phase(
            z_cnots,
            all_atoms,
            na,
            opts,
            next_interaction_id,
            shared_layout,
            stage_acc,
        )?;
        layers.extend(phase_layers);
        interactions.extend(phase_interactions);
        interaction_ids.extend(phase_ids);
    }

    append_local_gate_layers(&mut layers, &round.local_mid);

    if !x_cnots.is_empty() {
        let (phase_layers, phase_interactions, phase_ids) = schedule_cnot_phase(
            x_cnots,
            all_atoms,
            na,
            opts,
            next_interaction_id,
            shared_layout,
            stage_acc,
        )?;
        layers.extend(phase_layers);
        interactions.extend(phase_interactions);
        interaction_ids.extend(phase_ids);
    }

    append_local_gate_layers(&mut layers, &round.local_after);
    append_terminal_layers(&mut layers, &round.terminal);

    let segment = if interaction_ids.is_empty() {
        None
    } else {
        Some(InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: interaction_ids,
        })
    };
    Ok((layers, interactions, segment))
}

fn schedule_cnot_phase(
    cnots: &[PhysicalCnot],
    all_atoms: &[PhysicalAtomId],
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    next_interaction_id: &mut u32,
    shared_layout: &mut Option<NeutralAtomLayout>,
    stage_acc: &mut QecStageAccumulator,
) -> Result<CnotScheduleParts, NaPipelineError<AtomVertexId>> {
    let (round_interactions, round_ids) = cnots_to_interactions(cnots, next_interaction_id)?;
    let segment = InteractionSegment {
        kind: SegmentKind::CommutationGroup,
        interactions: round_ids.clone(),
    };
    let graph = cnot_graph(all_atoms, &round_interactions, segment)?;

    let stage_started = Instant::now();
    let req = schedule_from_graph(graph)?;
    let schedule_from_graph_us = elapsed_us(stage_started);

    let max_pairs = na.interaction.max_parallel_entangling_pairs;
    let stage_started = Instant::now();
    let scheduled = schedule_entangling_layers(req, max_pairs)?;
    let entangling_layers_us = elapsed_us(stage_started);

    let round_req = GraphScheduleRequest {
        graph: scheduled.request.graph,
        layers: scheduled.request.layers,
        layout: shared_layout.clone(),
    };

    // Shared place/AOD (or zoned) backend stage — issue #317. Same entry
    // point as `pipeline::finish_pipeline`.
    let (planned_req, backend_info) = plan_backend(round_req, na, opts)?;

    stage_acc.accumulate_phase(schedule_from_graph_us, entangling_layers_us, &backend_info);

    if opts.dump_ir {
        eprintln!(
            "--- QEC CX phase after movement ---\nlayers={}",
            planned_req.layers.len()
        );
    }
    *shared_layout = planned_req.layout.clone();
    Ok((planned_req.layers, round_interactions, round_ids))
}

fn append_local_gate_layers(layers: &mut Vec<ScheduleLayer>, ops: &[quon_qec::RoundLocalOp]) {
    use quon_qec::RoundLocalOp;
    let mut hs = Vec::new();
    for op in ops {
        match op {
            RoundLocalOp::H { atom } => hs.push(NeutralAtomAction::LocalGate {
                atom: AtomId(atom.0),
                gate: LocalGateKind::H,
                duration_us: QEC_LOCAL_GATE_DURATION_US,
            }),
        }
    }
    if hs.is_empty() {
        return;
    }
    let cycle = layers
        .last()
        .map(|l| l.cycle.saturating_add(1))
        .unwrap_or(0);
    layers.push(ScheduleLayer { cycle, actions: hs });
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

    let mut cycle = layers
        .last()
        .map(|l| l.cycle.saturating_add(1))
        .unwrap_or(0);
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
    interactions: &[Interaction<AtomVertexId>],
    segment: InteractionSegment,
) -> Result<InteractionGraph<AtomVertexId>, NaPipelineError<AtomVertexId>> {
    let vertices: Vec<AtomVertexId> = all_atoms.iter().copied().map(AtomVertexId::from).collect();
    InteractionGraph::from_interactions(
        vertices,
        interactions.to_vec(),
        vec![segment],
        DEFAULT_GAMMA,
    )
    .map_err(|e| NaPipelineError::ScheduleFromGraph(e.into()))
}

/// CNOT interactions built from physical CNOTs, atom-indexed (#318).
type CnotInteractions = (Vec<Interaction<AtomVertexId>>, Vec<InteractionId>);

fn cnots_to_interactions(
    cnots: &[PhysicalCnot],
    next_interaction_id: &mut u32,
) -> Result<CnotInteractions, NaPipelineError<AtomVertexId>> {
    let mut interactions = Vec::with_capacity(cnots.len());
    let mut ids = Vec::with_capacity(cnots.len());
    for cnot in cnots {
        let id = InteractionId(*next_interaction_id);
        *next_interaction_id = next_interaction_id
            .checked_add(1)
            .ok_or(NaPipelineError::InteractionIdOverflow)?;
        let mut qubits = vec![
            AtomVertexId::from_atom(cnot.control),
            AtomVertexId::from_atom(cnot.target),
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
    Ok((interactions, ids))
}

/// Project the atom-indexed hybrid request to the canonical emit representation
/// (`LogicalQubitId` vertices) at the schedule-artifact boundary (#318).
///
/// By this point atom identity is baked into the schedule layers as `AtomId`
/// actions, so the graph vertex id is just a label; relabeling
/// `AtomVertexId -> LogicalQubitId(index)` loses no information.
fn project_request_to_logical(
    req: GraphScheduleRequest<AtomVertexId>,
) -> GraphScheduleRequest<LogicalQubitId> {
    GraphScheduleRequest {
        graph: project_graph_to_logical(req.graph),
        layers: req.layers,
        layout: req.layout,
    }
}

fn project_graph_to_logical(
    graph: InteractionGraph<AtomVertexId>,
) -> InteractionGraph<LogicalQubitId> {
    InteractionGraph {
        vertices: graph
            .vertices
            .iter()
            .map(|v| LogicalQubitId(v.index()))
            .collect(),
        interactions: graph
            .interactions
            .into_iter()
            .map(|i| Interaction {
                id: i.id,
                qubits: i.qubits.iter().map(|q| LogicalQubitId(q.index())).collect(),
                gate_name: i.gate_name,
                dag_layer: i.dag_layer,
                on_critical_path: i.on_critical_path,
            })
            .collect(),
        edges: graph
            .edges
            .into_iter()
            .map(|e| InteractionEdge {
                a: LogicalQubitId(e.a.index()),
                b: LogicalQubitId(e.b.index()),
                weight: e.weight,
                interactions: e.interactions,
            })
            .collect(),
        segments: graph.segments,
        gamma: graph.gamma,
    }
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

/// Explicit FeedForward edges from each Measure layer to the next entangle /
/// measure layer after the following Wait (lattice-surgery measurement deps).
fn lattice_surgery_feedforward_deps(layers: &[ScheduleLayer]) -> Vec<ScheduleDependency> {
    let mut deps = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let has_measure = layer
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Measure { .. }));
        if !has_measure {
            continue;
        }
        // Find next Wait after this measure, then next active layer after Wait.
        let Some(wait_idx) = layers.iter().enumerate().skip(i + 1).find_map(|(j, l)| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
                .then_some(j)
        }) else {
            continue;
        };
        if let Some((after, _)) = layers.iter().enumerate().skip(wait_idx + 1).find(|(_, l)| {
            l.actions.iter().any(|a| {
                matches!(
                    a,
                    NeutralAtomAction::Entangle2 { .. }
                        | NeutralAtomAction::EntangleN { .. }
                        | NeutralAtomAction::Measure { .. }
                        | NeutralAtomAction::LocalGate { .. }
                )
            })
        }) {
            deps.extend(feed_forward_dependencies(i as u32, &[after as u32]));
        }
    }
    deps
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
    use crate::schedule::LocalGateKind;
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

    fn surface_d3_workload() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        b.finish()
    }

    /// Type-level guarantee (#318): the hybrid QEC interaction graph is indexed
    /// by `AtomVertexId` (physical atoms), not `LogicalQubitId`. If someone
    /// reintroduces a `LogicalQubitId(atom.0)` cast in `cnot_graph` /
    /// `cnots_to_interactions`, the explicit `InteractionGraph<AtomVertexId>` /
    /// `Vec<Interaction<AtomVertexId>>` annotations below stop compiling — the
    /// missing seam between "logical qubit" and "atom" must not come back.
    #[test]
    fn hybrid_graph_is_atom_indexed() {
        let atoms = [
            PhysicalAtomId(0),
            PhysicalAtomId(1),
            PhysicalAtomId(2),
            PhysicalAtomId(3),
        ];
        let cnots = [
            PhysicalCnot {
                control: PhysicalAtomId(0),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(3),
            },
        ];
        let mut next_id = 0u32;
        let (interactions, ids): (Vec<Interaction<AtomVertexId>>, Vec<InteractionId>) =
            cnots_to_interactions(&cnots, &mut next_id).expect("interactions");
        // Atoms — not logical qubits — label the interaction qubits.
        assert_eq!(
            interactions[0].qubits,
            vec![AtomVertexId(0), AtomVertexId(1)]
        );
        assert_eq!(
            interactions[1].qubits,
            vec![AtomVertexId(2), AtomVertexId(3)]
        );

        let segment = InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: ids,
        };
        let graph: InteractionGraph<AtomVertexId> =
            cnot_graph(&atoms, &interactions, segment).expect("graph");
        // Vertex set is the atom set, relabeled AtomVertexId 1:1 with atoms.
        assert_eq!(
            graph.vertices,
            vec![
                AtomVertexId(0),
                AtomVertexId(1),
                AtomVertexId(2),
                AtomVertexId(3)
            ]
        );
        // AtomVertexId is a distinct type from LogicalQubitId — the cast that
        // previously hid the missing seam must not collapse the two.
        assert_ne!(
            std::any::TypeId::of::<AtomVertexId>(),
            std::any::TypeId::of::<LogicalQubitId>()
        );
    }

    /// The hybrid schedule artifact projects to the canonical emit
    /// representation (`LogicalQubitId` vertices) at the boundary — atom identity
    /// is already baked into the schedule layers as `AtomId` actions, so the
    /// graph vertex id is just a label there (#318).
    #[test]
    fn hybrid_artifact_request_is_logical_qubit_indexed() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: false,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts = run_from_qec_workload(&d3_workload(), &na, opts).expect("schedule");
        // The emit-boundary graph is LogicalQubitId-typed (the projection).
        let _: &InteractionGraph<LogicalQubitId> = &artifacts.request.graph;
        // Resource report logical-qubit count stays block-level, not atom-level.
        assert_eq!(artifacts.resource_report.logical_qubits, 1);
    }

    #[test]
    fn surface_d3_memory_round_is_z_then_h_then_x_then_h() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: false,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts = run_from_qec_workload(&surface_d3_workload(), &na, opts).expect("schedule");

        // Within the first memory round (before first Wait): find the phase order.
        let wait_idx = artifacts
            .layers
            .iter()
            .position(|l| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
            })
            .expect("wait");
        let first_round = &artifacts.layers[..wait_idx];

        #[derive(Debug, PartialEq)]
        enum Phase {
            ZEntangle,
            MidH,
            XEntangle,
            AfterH,
            Measure,
        }
        let mut phases = Vec::new();
        for layer in first_round {
            let has_h = layer.actions.iter().any(|a| {
                matches!(
                    a,
                    NeutralAtomAction::LocalGate {
                        gate: LocalGateKind::H,
                        ..
                    }
                )
            });
            let has_e = layer.actions.iter().any(|a| {
                matches!(
                    a,
                    NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
                )
            });
            let has_m = layer
                .actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Measure { .. }));
            if has_h {
                let h_atoms: Vec<u32> = layer
                    .actions
                    .iter()
                    .filter_map(|a| match a {
                        NeutralAtomAction::LocalGate {
                            atom,
                            gate: LocalGateKind::H,
                            ..
                        } => Some(atom.0),
                        _ => None,
                    })
                    .collect();
                // X-check ancillas are 9,11,14,16 for d=3.
                assert_eq!(h_atoms, vec![9, 11, 14, 16]);
                if phases.last() == Some(&Phase::ZEntangle) {
                    phases.push(Phase::MidH);
                } else if phases.last() == Some(&Phase::XEntangle) {
                    phases.push(Phase::AfterH);
                } else {
                    phases.push(Phase::MidH);
                }
            } else if has_e {
                if phases.contains(&Phase::MidH) {
                    phases.push(Phase::XEntangle);
                } else {
                    phases.push(Phase::ZEntangle);
                }
            } else if has_m {
                phases.push(Phase::Measure);
            }
        }
        // Deduplicate consecutive identical phase tags from multi-layer Misra–Gries.
        phases.dedup();
        assert_eq!(
            phases,
            vec![
                Phase::ZEntangle,
                Phase::MidH,
                Phase::XEntangle,
                Phase::AfterH,
                Phase::Measure,
            ],
            "first memory round must be Z → mid H → X → after H → measure"
        );
    }

    #[test]
    fn surface_code_x_prep_emits_data_hadamards() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::X, LogicalQubitId(0))
            .unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::X)
            .unwrap();
        let na = load_na();
        let artifacts = run_from_qec_workload(
            &b.finish(),
            &na,
            NaScheduleOptions {
                compact: false,
                dump_ir: false,
                ..Default::default()
            },
        )
        .expect("schedule");
        let data_hs: Vec<u32> = artifacts
            .layers
            .iter()
            .flat_map(|l| l.actions.iter())
            .filter_map(|a| match a {
                NeutralAtomAction::LocalGate {
                    atom,
                    gate: LocalGateKind::H,
                    ..
                } if atom.0 < 9 => Some(atom.0),
                _ => None,
            })
            .collect();
        assert_eq!(
            data_hs,
            (0..9).collect::<Vec<_>>(),
            "X-init must Hadamard all data atoms before memory rounds"
        );
    }

    #[test]
    fn surface_d3_schedules_with_qec_report_fields() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts = run_from_qec_workload(&surface_d3_workload(), &na, opts).expect("schedule");
        let report = &artifacts.resource_report;

        assert_eq!(report.logical_qubits, 1);
        assert_eq!(report.physical_atoms, 17);
        assert_eq!(report.atoms_per_logical, Some(17));
        assert_eq!(report.code_family.as_deref(), Some("surface_code_like"));
        assert_eq!(report.distance, Some(3));
        assert_eq!(report.memory_rounds, Some(2));
        assert_eq!(report.entangle2_count, 48);
        assert_eq!(report.measurement_rounds, 3);
        assert_eq!(report.reset_rounds, 2);

        let waits = artifacts
            .layers
            .iter()
            .filter(|l| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
            })
            .count();
        assert_eq!(waits, 2, "one durable Wait per memory round");
    }

    /// Wait/round cuts are causal: compact with cuts vs without must differ for
    /// atom-disjoint cross-Wait E0. Deleting only `round_barrier_cuts` fails this.
    ///
    /// Also see `wait_barrier_cuts_are_causal_for_cross_round_e0` in
    /// `tests/compaction.rs`.
    #[test]
    fn round_barriers_constrain_compaction() {
        use crate::graph::{DEFAULT_GAMMA, InteractionGraph};
        use crate::layout::AtomId;
        use crate::schedule_entry::GraphScheduleRequest;

        let na = load_na();
        let uncompacted = run_from_qec_workload(
            &d3_workload(),
            &na,
            NaScheduleOptions {
                compact: false,
                dump_ir: false,
                ..Default::default()
            },
        )
        .expect("uncompacted");
        let cuts = round_barrier_cuts(&uncompacted.layers);
        assert!(
            !cuts.is_empty(),
            "hybrid schedule must expose Wait/round cuts"
        );
        let wait_idxs: Vec<u32> = uncompacted
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
            })
            .map(|(i, _)| i as u32)
            .collect();
        assert!(wait_idxs.len() >= 2, "expected ≥2 Wait layers");
        for &w in &wait_idxs {
            assert!(
                cuts.iter().any(|(b, _)| *b == w),
                "Wait index {w} must be a cut predecessor (cuts are derived from Wait)"
            );
        }

        // Causal compact: same three layers, with vs without Wait cuts.
        let vertices: Vec<LogicalQubitId> = (0..4).map(LogicalQubitId).collect();
        let graph = InteractionGraph::from_interactions(vertices, vec![], vec![], DEFAULT_GAMMA)
            .expect("graph");
        let synthetic = GraphScheduleRequest {
            graph,
            layers: vec![
                ScheduleLayer {
                    cycle: 0,
                    actions: vec![NeutralAtomAction::Entangle2 {
                        atoms: [AtomId(0), AtomId(1)],
                        duration_us: 1,
                    }],
                },
                ScheduleLayer {
                    cycle: 1,
                    actions: vec![NeutralAtomAction::Wait { duration_us: 1 }],
                },
                ScheduleLayer {
                    cycle: 2,
                    actions: vec![NeutralAtomAction::Entangle2 {
                        atoms: [AtomId(2), AtomId(3)],
                        duration_us: 1,
                    }],
                },
            ],
            layout: None,
        };
        let syn_cuts = round_barrier_cuts(&synthetic.layers);
        assert_eq!(syn_cuts, vec![(1, 2)]);
        let opts = CompactionOptions {
            arch: None,
            legality: None,
            greedy: true,
        };
        let without = compact_schedule(synthetic.clone(), &[], &opts).expect("without cuts");
        let mut with_deps = Vec::new();
        for &(before, after) in &syn_cuts {
            with_deps.push(ScheduleDependency {
                before,
                after,
                kind: ScheduleDependencyKind::Barrier,
            });
        }
        let with = compact_schedule(synthetic, &with_deps, &opts).expect("with cuts");
        assert_eq!(
            without.request.layers.len(),
            2,
            "without Wait cuts, atom-disjoint E2∥E2 across Wait must merge"
        );
        assert_eq!(
            with.request.layers.len(),
            3,
            "with Wait cuts, cross-Wait E2∥E2 must not merge"
        );

        let compacted = run_from_qec_workload(
            &d3_workload(),
            &na,
            NaScheduleOptions {
                compact: true,
                dump_ir: false,
                ..Default::default()
            },
        )
        .expect("compact schedule");
        let last_memory_reset_cycle = compacted
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
        let logical_measure_cycle = compacted
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
        for layer in &compacted.layers {
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
            let artifacts = run_from_qec_workload(&d3_workload(), &na, opts).unwrap_or_else(|e| {
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

    fn surface_d3_cx_workload() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.memory_round(LogicalQubitId(1)).unwrap();
        b.logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        b.measure_logical(LogicalQubitId(1), LogicalBasis::Z)
            .unwrap();
        b.finish()
    }

    #[test]
    fn surface_d3_logical_cx_schedules_with_barriers_and_feedforward() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts =
            run_from_qec_workload(&surface_d3_cx_workload(), &na, opts).expect("schedule cx");

        assert!(artifacts.resource_report.physical_atoms >= 51);
        assert_eq!(artifacts.logical_qubits, 3); // control + target + ancilla

        let wait_count = artifacts
            .layers
            .iter()
            .filter(|l| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Wait { .. }))
            })
            .count();
        // rough merge, rough split, smooth merge, smooth split, measure ancilla
        assert!(
            wait_count >= 5,
            "expected ≥5 Wait barriers for surgery phases, got {wait_count}"
        );

        let has_entangle = artifacts.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
        });
        let has_measure = artifacts.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Measure { .. }))
        });
        assert!(has_entangle && has_measure);

        let cuts = round_barrier_cuts(&artifacts.layers);
        assert!(
            !cuts.is_empty(),
            "lattice-surgery schedule must expose barrier cuts"
        );
    }

    #[cfg(feature = "mlir")]
    #[test]
    fn surface_d3_logical_cx_lowers_to_verifiable_quantum_na() {
        use crate::dialect::{self as qna, schedule_module};
        use crate::lower::{ScheduleLowerParams, lower_schedule};
        use melior::ir::BlockLike;

        let na = load_na();
        let opts = NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts =
            run_from_qec_workload(&surface_d3_cx_workload(), &na, opts).expect("schedule");
        let params =
            ScheduleLowerParams::from_target("generic_reconfigurable_neutral_atom_v0", &na);
        let spec = lower_schedule(&artifacts.request, &params).expect("lower");
        let context = melior::Context::new();
        let module = schedule_module(&context, &spec).unwrap_or_else(|e| match e {
            qna::BuildError::Verify(err) => panic!("verify: {err}"),
            other => panic!("build: {other}"),
        });
        let schedule = module.body().first_operation().expect("schedule op");
        qna::verify(&schedule).expect("verify na");
    }

    /// Both the bare-qubit path (`run_from_graph` → `finish_pipeline`) and the
    /// hybrid QEC path (`schedule_cnot_phase`) call the same [`plan_backend`]
    /// for the place/AOD/zoned step (issue #317). This test verifies the
    /// structural guarantee functionally: a direct `plan_backend` call produces
    /// the same layers as `run_from_graph` (which calls it internally).
    #[test]
    fn both_paths_share_plan_backend_entry_point() {
        use crate::entangling_schedule::schedule_entangling_layers;
        use crate::graph::{
            DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
            LogicalQubitId, SegmentKind,
        };
        use crate::pipeline::run_from_graph;
        use crate::plan::plan_backend;
        use crate::schedule_entry::{GraphScheduleRequest, schedule_from_graph};

        let na = load_na();
        let opts = NaScheduleOptions {
            compact: false,
            dump_ir: false,
            ..Default::default()
        };

        // Build a simple 2-qubit interaction graph.
        let vertices = vec![LogicalQubitId(0), LogicalQubitId(1)];
        let interaction = Interaction {
            id: InteractionId(0),
            qubits: vec![LogicalQubitId(0), LogicalQubitId(1)],
            gate_name: "cx".into(),
            dag_layer: 0,
            on_critical_path: false,
        };
        let segment = InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: vec![InteractionId(0)],
        };
        let graph = InteractionGraph::from_interactions(
            vertices,
            vec![interaction],
            vec![segment],
            DEFAULT_GAMMA,
        )
        .expect("graph");

        // Direct call to the shared plan_backend (the entry point both pipelines use).
        let req = schedule_from_graph(graph.clone()).expect("schedule_from_graph");
        let max_pairs = na.interaction.max_parallel_entangling_pairs;
        let scheduled = schedule_entangling_layers(req, max_pairs).expect("entangling");
        let req_for_backend = GraphScheduleRequest {
            graph: scheduled.request.graph.clone(),
            layers: scheduled.request.layers.clone(),
            layout: None,
        };
        let (planned_req, _info) = plan_backend(req_for_backend, &na, opts).expect("plan_backend");

        // Bare path: run_from_graph → finish_pipeline → plan_backend (no compaction).
        let bare_artifacts = run_from_graph(graph, &na, opts, None).expect("run_from_graph");

        // Both go through the same plan_backend → layers must be identical.
        assert_eq!(
            planned_req.layers, bare_artifacts.layers,
            "bare path (run_from_graph) and direct plan_backend call must produce \
             identical layers — they share the same place/AOD entry point (#317)"
        );
    }

    /// The hybrid QEC path now populates stage stats (`NaStats`) the same way
    /// as `run_from_graph` — not `None` (issue #317 / #307).
    #[test]
    fn qec_path_populates_stage_stats() {
        let na = load_na();
        let opts = NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        };
        let artifacts = run_from_qec_workload(&d3_workload(), &na, opts).expect("schedule");
        let stats = artifacts
            .stats
            .as_ref()
            .expect("QEC path must populate NaStats (not None) — #317/#307");

        // Stage timings are populated (not all zero) — the per-round entangle +
        // plan_backend calls were timed.
        assert_eq!(stats.kind, crate::stats::NA_STATS_KIND);
        assert_eq!(stats.schema_version, crate::stats::NA_STATS_SCHEMA_VERSION);
        assert!(
            stats.stage_timings_us.total_us > 0,
            "total_us must be > 0 (QEC path timed the pipeline)"
        );
        // The zoned backend (default) should have zoned_schedule_us populated.
        assert!(
            stats.stage_timings_us.zoned_schedule_us.is_some(),
            "zoned backend must report zoned_schedule_us"
        );
        // Effective config echoes the backend.
        assert_eq!(
            stats.config.backend,
            crate::pipeline::NaBackendKind::Zoned,
            "config must echo the zoned backend"
        );
        // Compaction was requested and applied.
        assert!(stats.config.compaction.applied, "compaction was applied");
    }
}
