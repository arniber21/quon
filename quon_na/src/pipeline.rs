//! Neutral-atom schedule pipeline (post–`quantum.dynamic`).
//!
//! # Stages
//!
//! Callable without `quonc`. After lowering + dynamic passes:
//!
//! 1. **Extract** interaction graph from the MLIR module (`mlir` feature)
//! 2. **`schedule_from_graph`** → **`schedule_entangling_layers`**
//! 3. **Zoned** (`schedule_zoned`) **or** flat AOD (`place` + `plan_aod_movement`)
//! 4. **Optional compaction** ([`compact_schedule`], fail-closed)
//! 5. **Resource report** ([`build_resource_report`])
//!
//! Fixed (OpenQASM) physical passes live in `mlir_bridge::pipeline`.

use std::time::Instant;

use backend::{AodSpeedModelKind, NeutralAtomTarget, ZoneKind as BackendZoneKind};
use thiserror::Error;

use crate::compaction::{
    CompactionError, CompactionOptions, LegalityLimits, compact_schedule, infer_atom_dependencies,
};
use crate::entangling_schedule::schedule_entangling_layers;
use crate::graph::{InteractionGraph, LogicalQubitId};
use crate::movement::MovementParams;
use crate::placement::PlacementStrategy;
use crate::report::{ResourceReport, attach_qec_error_budget, build_resource_report};
use crate::schedule::ScheduleLayer;
use crate::schedule_entry::{GraphScheduleRequest, schedule_from_graph};
use crate::stats::{CompactionConfig, EffectiveConfig, NaStats, StageTimingsUs};
use crate::zoned::{AwareSearchParams, PlacerMode, ZoneKind, ZoneSpec, ZonedArchitecture};

/// Wall-clock elapsed microseconds since `start` (saturating on overflow —
/// not reachable in practice, but keeps this instrumentation infallible).
fn elapsed_us(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

#[cfg(feature = "mlir")]
use melior::ir::Module;

#[cfg(feature = "mlir")]
use crate::extract::{ExtractError, LocalGateExtract, extract_interaction_graph_and_local_gates};
#[cfg(feature = "mlir")]
use crate::graph::InteractionId;
#[cfg(feature = "mlir")]
use crate::layout::AtomId;
#[cfg(feature = "mlir")]
use crate::native_gate_decomp::{
    DecomposedLocalGate, NaDecompError, NaLocalOp, decompose_local_gates,
};
#[cfg(feature = "mlir")]
use crate::schedule::{LocalGateKind, NeutralAtomAction};
#[cfg(feature = "mlir")]
use std::collections::BTreeMap;

/// Which movement/placement backend to run after entangling-layer scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaBackendKind {
    /// Zoned RAP / ZAC path (#107) — default for multi-zone targets.
    #[default]
    Zoned,
    /// Flat AOD pair-bank planner (#106).
    FlatAod,
}

/// State-preparation scheduling mode (issue #302, Deliverable A).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatePrepMode {
    /// Heuristic zoned/flat scheduler (default).
    #[default]
    Heuristic,
    /// Exact SMT state-prep scheduling via z3 (requires `solver` feature).
    /// The standalone solver (`exact::state_prep::schedule_exact`) is
    /// implemented, but pipeline wiring into the QEC per-CNOT-phase
    /// scheduler is not yet done, so requesting this currently logs a
    /// fallback to the heuristic scheduler (issue #302, Deliverable A).
    Exact,
}

/// Placement/routing objective (issue #309).
///
/// `Time` (default) minimizes the time-shaped RAP Eq. (1) cost `Σ √(d_max)`.
/// `ErrorBudget` minimizes analytic error-model contributions instead —
/// `rate × count` (ADR-0017/0020), not logical error rates or thresholds.
///
/// Requesting `ErrorBudget` on a target without an `error_model` is a hard
/// error (fail-closed, mirroring `--emit-resource-report` discipline).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaObjective {
    #[default]
    Time,
    ErrorBudget,
}

/// Options for the NA schedule pipeline.
#[derive(Clone, Copy, Debug)]
pub struct NaScheduleOptions {
    pub backend: NaBackendKind,
    pub placer: PlacerMode,
    pub compact: bool,
    pub placement: PlacementStrategy,
    /// When true, print stage summaries to stderr (matches `quonc --dump-ir`).
    pub dump_ir: bool,
    /// Zoned backend, [`PlacerMode::RoutingAware`] only: A* search tunables
    /// (issue #297). Ignored otherwise. A per-run option (not a target JSON
    /// field) — see [`AwareSearchParams`]'s doc for why.
    pub aware_search: AwareSearchParams,
    /// State-preparation scheduling mode (issue #302, Deliverable A).
    /// `Heuristic` (default) uses the standard zoned/flat scheduler;
    /// `Exact` uses the z3-backed exact state-prep scheduler when the
    /// `solver` feature is enabled, falling back to heuristic on timeout.
    pub state_prep: StatePrepMode,
    /// Placement/routing objective (issue #309): `Time` (default) minimizes
    /// the RAP Eq. (1) time-shaped cost; `ErrorBudget` minimizes analytic
    /// error-model contributions instead (ADR-0017/0020). Requires the
    /// target's `error_model` — requesting it without one is a hard error.
    pub objective: NaObjective,
}
impl Default for NaScheduleOptions {
    fn default() -> Self {
        Self {
            backend: NaBackendKind::Zoned,
            placer: PlacerMode::RoutingAgnostic,
            compact: true,
            placement: PlacementStrategy::RowMajor,
            dump_ir: false,
            aware_search: AwareSearchParams::default(),
            state_prep: StatePrepMode::default(),
            objective: NaObjective::default(),
        }
    }
}

/// Artifacts from a successful NA schedule run.
#[derive(Clone, Debug)]
pub struct NaScheduleArtifacts {
    pub layers: Vec<ScheduleLayer>,
    pub resource_report: ResourceReport,
    pub logical_qubits: u64,
    pub request: GraphScheduleRequest,
    /// Per-stage timings / search diagnostics / config echo (issue #307).
    /// `Some` from all three entry points ([`run_from_graph`],
    /// [`run_from_module`], [`crate::qec_schedule::run_from_qec_workload`])
    /// — the bare and hybrid paths share the same `plan_backend` stage
    /// (#317), so stage stats are populated uniformly.
    pub stats: Option<NaStats>,
}

/// Errors from the NA schedule pipeline.
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); the vertex
/// sub-errors carry `V`. The bare Quon-program path uses the default; the
/// hybrid QEC path (#318) uses `NaPipelineError<AtomVertexId>`.
#[derive(Debug, Error)]
pub enum NaPipelineError<V = LogicalQubitId> {
    #[error("{0}")]
    InvalidTarget(String),
    #[cfg(feature = "mlir")]
    #[error("interaction-graph extraction failed: {0}")]
    Extract(#[from] ExtractError),
    #[error("QEC hybrid expansion failed: {0}")]
    QecExpand(#[from] quon_qec::ExpandError),
    #[error("schedule_from_graph failed: {0}")]
    ScheduleFromGraph(#[from] crate::schedule_entry::ScheduleFromGraphError<V>),
    #[error("entangling-layer scheduling failed: {0}")]
    Entangling(#[from] crate::entangling_schedule::EntanglingScheduleError<V>),
    #[error("zoned scheduling failed: {0}")]
    Zoned(#[from] crate::zoned::ZonedScheduleError),
    #[error("placement failed: {0}")]
    Placement(#[from] crate::placement::PlacementError<V>),
    #[error("AOD movement planning failed: {0}")]
    Movement(#[from] crate::movement::MovementPlanError),
    #[error("schedule compaction failed: {0}")]
    Compaction(#[from] CompactionError),
    #[cfg(feature = "mlir")]
    #[error("single-qubit gate decomposition failed: {0}")]
    LocalGateDecomp(#[from] NaDecompError),
    /// Internal invariant violation (issue #298 review finding #2): a
    /// captured 1-qubit gate's `after` anchor named an interaction id that
    /// `schedule_entangling_layers` never assigned a cycle to.
    /// `schedule_entangling_layers` always populates `interaction_cycle` for
    /// every interaction it schedules, so this should be unreachable — it is
    /// a hard error rather than a silent fallback (which would have
    /// mis-scheduled the gate to the very start of the program, hiding a
    /// real bug) on purpose.
    #[cfg(feature = "mlir")]
    #[error(
        "internal error: 1-qubit gate anchored to interaction {interaction:?}, which \
         schedule_entangling_layers never assigned a cycle to"
    )]
    LocalGateAnchorMissing { interaction: InteractionId },
    #[error("interaction id overflow while building QEC hybrid schedule")]
    InteractionIdOverflow,
    #[error(
        "QEC round z_cnot_count ({z_cnot_count}) exceeds entangling.len() ({entangling_len}); \
         refusing silent clamp"
    )]
    InvalidZCnotCount {
        z_cnot_count: usize,
        entangling_len: usize,
    },
    /// Error-budget placement objective requested on a target without an
    /// `error_model` (issue #309). Mirrors `--emit-resource-report`
    /// fail-closed discipline (ADR-0017): never derive from fidelity.
    #[error(
        "error-budget placement objective (--na-objective error-budget) requires a \
         target error_model; set error_model on the target (do not derive from \
         fidelity) — mirrors --emit-resource-report fail-closed discipline (ADR-0017)"
    )]
    MissingErrorModelForObjective,
    #[error("resource report failed: {0}")]
    Report(#[from] crate::report::ReportError),
}

/// Convert a loaded neutral-atom target into the zoned architecture RAP expects.
pub fn zoned_architecture(na: &NeutralAtomTarget) -> ZonedArchitecture {
    ZonedArchitecture {
        zones: na
            .zones
            .iter()
            .map(|z| ZoneSpec {
                zone_id: z.zone_id,
                kind: map_zone_kind(z.kind),
                rows: z.rows,
                cols: z.cols,
                origin_um: z.origin_um,
                site_pitch_um: z.site_pitch_um,
                pair_gap_um: z.pair_gap_um,
            })
            .collect(),
        speed_model: crate::geometry::SpeedModel::from(&na.movement.speed_model),
        trap_transfer_us: na.movement.trap_transfer_us.round() as u64,
        require_readout_zone: false,
        rydberg_range_um: na.interaction.rydberg_range_um,
        min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
        aod_min_separation_um: na.movement.min_row_col_separation_um,
    }
}

/// Flat AOD movement parameters derived from the same target JSON.
pub fn movement_params(na: &NeutralAtomTarget) -> MovementParams {
    let pair_gap_um = na
        .zones
        .iter()
        .find(|z| z.kind == BackendZoneKind::Entanglement)
        .and_then(|z| z.pair_gap_um)
        .unwrap_or(2.0);
    MovementParams {
        speed_model: crate::geometry::SpeedModel::from(&na.movement.speed_model),
        trap_transfer_us: na.movement.trap_transfer_us.round() as u64,
        rydberg_range_um: na.interaction.rydberg_range_um,
        min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
        min_row_col_separation_um: na.movement.min_row_col_separation_um,
        aod_rows: na.movement.aod_rows,
        aod_cols: na.movement.aod_cols,
        num_aods: na.movement.num_aods,
        pair_gap_um,
        pair_pitch_um: na.interaction.min_rydberg_spacing_um,
        return_home: false,
    }
}

/// Compaction options with optional geometry legality from the target.
pub fn compaction_options(na: &NeutralAtomTarget, greedy: bool) -> CompactionOptions {
    CompactionOptions {
        arch: Some(zoned_architecture(na)),
        legality: Some(LegalityLimits {
            rydberg_range_um: na.interaction.rydberg_range_um,
            min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
            aod_min_separation_um: na.movement.min_row_col_separation_um,
        }),
        greedy,
    }
}

/// Sanity-check that the target's speed model is the one `quon_na` implements.
///
/// Both `Sqrt` (default) and `JerkLimited` (issue #308) are implemented. The
/// jerk-limited variant additionally requires `jerk_m_s3 > 0` and
/// `max_velocity_m_s >= 0` (the backend descriptor loader enforces the same on
/// JSON load; this re-checks for in-memory `NeutralAtomTarget`s).
pub fn validate_speed_model(na: &NeutralAtomTarget) -> Result<(), String> {
    match na.movement.speed_model.kind {
        AodSpeedModelKind::Sqrt => Ok(()),
        AodSpeedModelKind::JerkLimited => {
            if na.movement.speed_model.jerk_m_s3 <= 0.0 {
                return Err(format!(
                    "movement.speed_model.jerk_m_s3 must be positive for jerk_limited, got {}",
                    na.movement.speed_model.jerk_m_s3
                ));
            }
            if na.movement.speed_model.max_velocity_m_s < 0.0 {
                return Err(format!(
                    "movement.speed_model.max_velocity_m_s must be non-negative for \
                     jerk_limited, got {}",
                    na.movement.speed_model.max_velocity_m_s
                ));
            }
            Ok(())
        }
    }
}

fn map_zone_kind(kind: BackendZoneKind) -> ZoneKind {
    match kind {
        BackendZoneKind::Storage => ZoneKind::Storage,
        BackendZoneKind::Entanglement => ZoneKind::Entanglement,
        BackendZoneKind::Readout => ZoneKind::Readout,
    }
}

/// Schedule from a post-dynamic MLIR module.
///
/// Uses [`extract_interaction_graph_and_local_gates`] (not the bare-graph
/// extractor) so 1-qubit gates are preserved end to end (issue #298) — the
/// production `quonc` path always goes through here, never through
/// [`run_from_graph`] directly.
#[cfg(feature = "mlir")]
pub fn run_from_module<'c>(
    module: &Module<'c>,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let extract_started = Instant::now();
    let (graph, local_gates) = extract_interaction_graph_and_local_gates(module)?;
    let extract_us = elapsed_us(extract_started);
    let logical_qubits = graph.vertices.len() as u64;
    if opts.dump_ir {
        eprintln!(
            "--- interaction graph ---\nvertices={} interactions={} local_1q_gates={}",
            graph.vertices.len(),
            graph.interactions.len(),
            local_gates.len(),
        );
    }
    let mut artifacts =
        run_from_graph_with_local_gates(graph, local_gates, na, opts, Some(logical_qubits))?;
    if let Some(stats) = artifacts.stats.as_mut() {
        stats.stage_timings_us.extract_us = Some(extract_us);
        stats.stage_timings_us.total_us =
            stats.stage_timings_us.total_us.saturating_add(extract_us);
    }
    Ok(artifacts)
}

/// Schedule from a raw interaction graph (debug / stress entry) — no 1-qubit
/// gates (there is no MLIR to extract them from). Use [`run_from_module`] for
/// the full-fidelity, 1-qubit-gate-preserving production path.
pub fn run_from_graph(
    graph: InteractionGraph,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    logical_qubits_override: Option<u64>,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let logical_qubits = logical_qubits_override.unwrap_or(graph.vertices.len() as u64);
    let pipeline_started = Instant::now();

    let stage_started = Instant::now();
    let req = schedule_from_graph(graph)?;
    let schedule_from_graph_us = elapsed_us(stage_started);

    let stage_started = Instant::now();
    let max_pairs = na.interaction.max_parallel_entangling_pairs;
    let scheduled = schedule_entangling_layers(req, max_pairs)?;
    let entangling_layers_us = elapsed_us(stage_started);

    finish_pipeline(
        scheduled.request,
        na,
        opts,
        logical_qubits,
        pipeline_started,
        schedule_from_graph_us,
        entangling_layers_us,
    )
}

/// Schedule from a raw interaction graph plus captured 1-qubit gates
/// (issue #298): decomposes each gate to `na.native_gates` (local `rz`,
/// global `ry`, or a `u3` escape hatch — see [`crate::native_gate_decomp`])
/// and splices the results into the entangling-scheduled layers at the
/// gate's extraction-time anchor, before zoned/flat-AOD placement (neither
/// backend's movement planning needs `LocalGate`/`GlobalRy` site info — see
/// [`interleave_local_gates`]).
#[cfg(feature = "mlir")]
pub fn run_from_graph_with_local_gates(
    graph: InteractionGraph,
    local_gates: Vec<LocalGateExtract>,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    logical_qubits_override: Option<u64>,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    validate_speed_model(na).map_err(NaPipelineError::InvalidTarget)?;
    let logical_qubits = logical_qubits_override.unwrap_or(graph.vertices.len() as u64);
    let pipeline_started = Instant::now();

    let decomposed = decompose_local_gates(&local_gates, &na.native_gates)?;

    let stage_started = Instant::now();
    let req = schedule_from_graph(graph)?;
    let schedule_from_graph_us = elapsed_us(stage_started);

    let stage_started = Instant::now();
    let max_pairs = na.interaction.max_parallel_entangling_pairs;
    let scheduled = schedule_entangling_layers(req, max_pairs)?;
    let entangling_layers_us = elapsed_us(stage_started);

    let mut req = scheduled.request;
    req.layers = interleave_local_gates(
        req.layers,
        &scheduled.interaction_cycle,
        decomposed,
        na,
        logical_qubits,
    )?;

    finish_pipeline(
        req,
        na,
        opts,
        logical_qubits,
        pipeline_started,
        schedule_from_graph_us,
        entangling_layers_us,
    )
}

/// Splice decomposed 1-qubit gate actions into `layers` at their
/// extraction-time anchor (issue #298).
///
/// Every decomposed step becomes its **own** dedicated [`ScheduleLayer`]
/// (never sharing a layer with another local step, an entangling action, or
/// another `GlobalRy`). This is a deliberate correctness-first simplification
/// over layer-packing multiple independent-atom local gates together: it
/// trivially guarantees no two `LocalGate`s on the same atom are ever
/// reordered relative to each other or to the interaction they're anchored
/// to. Packing independent local gates into shared layers for fewer cycles
/// is a follow-up optimization, not a correctness requirement — see the PR
/// description for issue #298.
///
/// A gate with `after: None` (no ≥2-qubit interaction ever touched its
/// qubit) is scheduled before every entangling layer; a gate anchored to
/// interaction `id` is scheduled immediately after the layer that hosted
/// `id` (any position strictly between that layer and the qubit's *next*
/// interaction, if any, is causally equivalent — nothing else touches the
/// atom in between).
///
/// **`GlobalRy` bystander isolation (issue #298 review finding — CRITICAL):**
/// every logical atom is bound to a site from `layout.initial_bindings` at
/// schedule start (this compiler has no zone-based spatial isolation of the
/// raster's footprint), so a bare `GlobalRy` physically hits *every* trapped
/// atom, not just the one it was decomposed for. Emitting one raw `GlobalRy`
/// per decomposed gate — this function's first cut — silently applied an
/// unwanted rotation to every bystander atom, composing across successive
/// rasters into schedule output that did not implement the source circuit's
/// unitary. Fixed via [`push_global_ry_with_refocus`]: a Hahn-echo-style
/// composite-pulse sequence (split the raster into two half-angle pulses;
/// sandwich a local `Rz(pi)`/`Rz(-pi)` pair around the second half for every
/// atom that should *not* receive the rotation) that is exact — see that
/// function's doc comment for the derivation and this module's tests for a
/// mechanical check.
#[cfg(feature = "mlir")]
fn interleave_local_gates(
    layers: Vec<ScheduleLayer>,
    interaction_cycle: &BTreeMap<InteractionId, u32>,
    decomposed: Vec<DecomposedLocalGate>,
    na: &NeutralAtomTarget,
    total_atoms: u64,
) -> Result<Vec<ScheduleLayer>, NaPipelineError> {
    let single_qubit_us = na.timing.single_qubit_us.max(0.0).round() as u64;
    let all_atoms: Vec<AtomId> = (0..total_atoms).map(|i| AtomId(i as u32)).collect();

    let mut by_anchor: BTreeMap<Option<u32>, Vec<NeutralAtomAction>> = BTreeMap::new();
    for gate in decomposed {
        // Issue #298 review finding #2: distinguish "no anchor" (`None`,
        // legitimate — schedule before everything) from "anchor named an
        // interaction id `schedule_entangling_layers` never assigned a
        // cycle to" (an internal invariant violation) — the previous
        // `.and_then` silently collapsed both to `None`, which would have
        // mis-scheduled the gate to the very start of the program instead
        // of surfacing the bug.
        let anchor_cycle = match gate.after {
            None => None,
            Some(id) => Some(
                *interaction_cycle
                    .get(&id)
                    .ok_or(NaPipelineError::LocalGateAnchorMissing { interaction: id })?,
            ),
        };
        let atom = AtomId(gate.qubit.0);
        let bucket = by_anchor.entry(anchor_cycle).or_default();
        for op in gate.ops {
            match op {
                NaLocalOp::Local(kind) => {
                    bucket.push(NeutralAtomAction::LocalGate {
                        atom,
                        gate: kind,
                        duration_us: single_qubit_us,
                    });
                }
                NaLocalOp::GlobalRy(theta) => {
                    push_global_ry_with_refocus(bucket, atom, theta, &all_atoms, single_qubit_us);
                }
            }
        }
    }

    let mut out = Vec::with_capacity(layers.len());
    if let Some(actions) = by_anchor.remove(&None) {
        for action in actions {
            out.push(ScheduleLayer {
                cycle: 0,
                actions: vec![action],
            });
        }
    }
    for layer in layers {
        let cycle = layer.cycle;
        out.push(layer);
        if let Some(actions) = by_anchor.remove(&Some(cycle)) {
            for action in actions {
                out.push(ScheduleLayer {
                    cycle: 0,
                    actions: vec![action],
                });
            }
        }
    }
    // Defensive: an anchor cycle that didn't match any layer shouldn't
    // happen (`interaction_cycle` values always come from `layers.cycle`),
    // but append rather than silently drop if it ever does.
    for (_, actions) in by_anchor {
        for action in actions {
            out.push(ScheduleLayer {
                cycle: 0,
                actions: vec![action],
            });
        }
    }

    for (i, layer) in out.iter_mut().enumerate() {
        layer.cycle = i as u32;
    }
    Ok(out)
}

/// Push a `GlobalRy(theta)` intended for `atom` into `bucket`, correctly
/// isolating every other atom in `all_atoms` from the raster (issue #298
/// review finding — see [`interleave_local_gates`]'s doc comment for why a
/// bare `GlobalRy` is unsafe).
///
/// When `atom` is the only trapped atom, no other atom can be corrupted, so
/// this pushes a single plain `GlobalRy(theta)` — no need for the composite
/// sequence. Otherwise it pushes, in order:
/// 1. `GlobalRy(theta / 2)`
/// 2. `LocalGate { gate: Rz(pi) }` for every bystander atom
/// 3. `GlobalRy(theta / 2)`
/// 4. `LocalGate { gate: Rz(-pi) }` for every bystander atom
///
/// This is a Hahn-echo-style composite-pulse sequence, standard in quantum
/// control for realizing an effectively-local operation from a globally
/// shared drive. Derivation (all rotations as 2x2 `SU(2)`; `.` = matrix
/// product, left = applied last):
///
/// - Bystander net effect: `Rz(-pi) . Ry(theta/2) . Rz(pi) . Ry(theta/2)`.
///   Using `Rz(pi) . Ry(a) = Ry(-a) . Rz(pi)` (conjugating the `Y` generator
///   by a `pi` `Z`-rotation flips its sign): the middle three terms collapse
///   to `Ry(theta/2) . Ry(-theta/2) . Rz(pi) = Rz(pi)`, so the whole product
///   is `Rz(-pi) . Rz(pi) = I` — **exactly** identity, not an approximation.
/// - `atom`'s net effect from just this function's output is
///   `Ry(theta/2) . Ry(theta/2) = Ry(theta)` — the plain `ry` component
///   `native_gate_decomp` decomposed it to. The surrounding `Rz(phi)` /
///   `Rz(lambda)` from that same decomposition are separate `NaLocalOp`
///   entries already emitted immediately before/after this one by the
///   caller's loop, so `atom`'s full realized unitary is
///   `Rz(phi) . Ry(theta) . Rz(lambda)` — its intended ZYZ step.
///
/// Verified both symbolically (above) and numerically against
/// `backend::unitary` in this module's tests.
#[cfg(feature = "mlir")]
fn push_global_ry_with_refocus(
    bucket: &mut Vec<NeutralAtomAction>,
    atom: AtomId,
    theta: f64,
    all_atoms: &[AtomId],
    duration_us: u64,
) {
    let bystanders: Vec<AtomId> = all_atoms.iter().copied().filter(|&a| a != atom).collect();
    if bystanders.is_empty() {
        bucket.push(NeutralAtomAction::GlobalRy {
            theta_rad: theta,
            duration_us,
        });
        return;
    }

    let half = theta / 2.0;
    bucket.push(NeutralAtomAction::GlobalRy {
        theta_rad: half,
        duration_us,
    });
    for &bystander in &bystanders {
        bucket.push(NeutralAtomAction::LocalGate {
            atom: bystander,
            gate: LocalGateKind::Rz(std::f64::consts::PI),
            duration_us,
        });
    }
    bucket.push(NeutralAtomAction::GlobalRy {
        theta_rad: half,
        duration_us,
    });
    for &bystander in &bystanders {
        bucket.push(NeutralAtomAction::LocalGate {
            atom: bystander,
            gate: LocalGateKind::Rz(-std::f64::consts::PI),
            duration_us,
        });
    }
}

/// Shared tail of the pipeline: placement/movement, optional compaction, and
/// the resource report. `req.layers` must already be fully populated
/// (entangling actions, plus any interleaved local-gate actions).
fn finish_pipeline(
    mut req: GraphScheduleRequest,
    na: &NeutralAtomTarget,
    opts: NaScheduleOptions,
    logical_qubits: u64,
    pipeline_started: Instant,
    schedule_from_graph_us: u64,
    entangling_layers_us: u64,
) -> Result<NaScheduleArtifacts, NaPipelineError> {
    // Shared place/AOD (or zoned) backend stage — issue #317. Both the bare
    // path (here, via `finish_pipeline`) and the hybrid QEC path
    // (`qec_schedule::schedule_cnot_phase`) call `plan_backend`.
    let (planned_req, backend_info) = crate::plan::plan_backend(req, na, opts)?;
    req = planned_req;
    let aware_search_status = backend_info.aware_search_status;
    let agnostic_placer_mechanism = backend_info.agnostic_placer_mechanism;
    let search_diagnostics = backend_info.search_diagnostics;
    let zoned_schedule_us = backend_info.zoned_schedule_us;
    let placement_us = backend_info.placement_us;
    let movement_us = backend_info.movement_us;
    let placer_mode = backend_info.placer_mode;
    let placement_strategy = backend_info.placement_strategy;
    let schedule_optimality = backend_info.schedule_optimality;

    let mut compaction_config = CompactionConfig {
        requested: opts.compact,
        applied: false,
        greedy: false,
        legality_checked: false,
    };
    let mut compaction_us = None;
    if opts.compact && !req.layers.is_empty() {
        let deps = infer_atom_dependencies(&req.layers);
        // After movement, layout bindings are final occupancy — static R2/R3
        // against that layout is not meaningful for earlier layers. Order-only
        // compaction still fail-closes on software/dependency errors.
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
                "--- after compaction ---\nlayers={} (was {})",
                compacted.request.layers.len(),
                req.layers.len()
            );
        }
        req = compacted.request;
    }

    let stage_started = Instant::now();
    let report = build_resource_report(&req.layers, None, Some(logical_qubits.max(1)))?;
    let report = match aware_search_status {
        Some((completed, fell_back)) => report.with_aware_search_status(completed, fell_back),
        None => report,
    };
    let report = match schedule_optimality {
        Some(optimality) => report.with_schedule_optimality(optimality),
        None => report,
    };
    let report = report.with_agnostic_placer_mechanism(agnostic_placer_mechanism);
    // Production path: attach analytic error_budget whenever the target carries
    // an error_model (ADR-0017). `--emit-resource-report` in quonc additionally
    // hard-requires the model so missing budgets fail at emit time.
    let report = match na.error_model.as_ref() {
        Some(model) => attach_qec_error_budget(report, Some(model))?,
        None => report,
    };
    // Analytic end-to-end fidelity estimate (Enola Eq. (1), issue #305).
    // Unlike `error_model`, `NeutralAtomTarget::fidelity` is a mandatory
    // field, so this overlay always applies once a target is available —
    // `gate_fidelity_product`/`estimated_fidelity` are `Some` on every
    // production report from this pipeline.
    let report = report.with_fidelity_estimate(&req.layers, &na.fidelity);
    let resource_report_us = elapsed_us(stage_started);

    let stats = NaStats {
        kind: crate::stats::NA_STATS_KIND.to_string(),
        schema_version: crate::stats::NA_STATS_SCHEMA_VERSION,
        version: Default::default(),
        config: EffectiveConfig {
            backend: opts.backend,
            placer_mode,
            placement_strategy,
            compaction: compaction_config,
            objective: opts.objective,
        },
        stage_timings_us: StageTimingsUs {
            extract_us: None,
            schedule_from_graph_us,
            entangling_layers_us,
            zoned_schedule_us,
            placement_us,
            movement_us,
            compaction_us,
            resource_report_us,
            total_us: elapsed_us(pipeline_started),
        },
        search: search_diagnostics,
    };

    Ok(NaScheduleArtifacts {
        layers: req.layers.clone(),
        resource_report: report,
        logical_qubits,
        request: req,
        stats: Some(stats),
    })
}

#[cfg(all(test, feature = "mlir"))]
mod tests {
    use super::*;
    use backend::unitary::{Complex, M2, mul2, rotation_unitary, unitary_distance2};

    fn identity() -> M2 {
        M2([
            [Complex::new(1.0, 0.0), Complex::new(0.0, 0.0)],
            [Complex::new(0.0, 0.0), Complex::new(1.0, 0.0)],
        ])
    }

    fn load_na_target() -> NeutralAtomTarget {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../targets/neutral_atom/generic_rna_v0.json"
        ));
        let loaded = backend::json::load(path).expect("load target");
        loaded.neutral_atom_target().expect("na target").clone()
    }

    /// Interpret a flat list of `NeutralAtomAction`s (as produced by
    /// [`push_global_ry_with_refocus`]) into each atom's net unitary, the
    /// same way a real schedule-consuming semantic check would. `LocalGate`
    /// only ever appears as `Rz` here (the refocus echo pulses); `GlobalRy`
    /// applies to every atom in `atoms`.
    fn net_unitaries(actions: &[NeutralAtomAction], atoms: &[AtomId]) -> Vec<M2> {
        let mut net: std::collections::BTreeMap<AtomId, M2> =
            atoms.iter().map(|&a| (a, identity())).collect();
        for action in actions {
            match action {
                NeutralAtomAction::LocalGate {
                    atom,
                    gate: LocalGateKind::Rz(theta),
                    ..
                } => {
                    let u = rotation_unitary("rz", *theta).expect("rz unitary");
                    let entry = net.get_mut(atom).expect("known atom");
                    *entry = mul2(u, *entry);
                }
                NeutralAtomAction::GlobalRy { theta_rad, .. } => {
                    let u = rotation_unitary("ry", *theta_rad).expect("ry unitary");
                    for entry in net.values_mut() {
                        *entry = mul2(u, *entry);
                    }
                }
                other => panic!("unexpected action in refocus-only test: {other:?}"),
            }
        }
        atoms.iter().map(|a| net[a]).collect()
    }

    /// Issue #298 review Finding 1: a bystander atom's net effect from a
    /// refocused `GlobalRy` must be exactly identity (not approximately —
    /// this is an algebraic identity, see `push_global_ry_with_refocus`'s
    /// doc comment), for a range of raster angles including edge cases.
    #[test]
    fn global_ry_refocus_is_exact_identity_for_bystanders() {
        let wanted = AtomId(0);
        let bystander = AtomId(1);
        let atoms = [wanted, bystander];
        for &theta in &[0.1, 0.6, 1.5, std::f64::consts::PI, -0.9, 6.0] {
            let mut actions = Vec::new();
            push_global_ry_with_refocus(&mut actions, wanted, theta, &atoms, 1);
            let nets = net_unitaries(&actions, &atoms);
            let bystander_net = nets[1];
            assert!(
                unitary_distance2(bystander_net, identity()) < 1e-9,
                "theta={theta}: bystander net should be identity, got {bystander_net:?}"
            );
        }
    }

    /// The wanted atom's net effect from just the `GlobalRy` component is
    /// `Ry(theta)` — matching what a plain (unrefocused) raster would have
    /// realized for it, so combining this with its surrounding `Rz`
    /// decomposition steps (emitted separately by the caller) reproduces its
    /// intended ZYZ unitary exactly.
    #[test]
    fn global_ry_refocus_realizes_wanted_atom_rotation() {
        let wanted = AtomId(0);
        let atoms = [wanted, AtomId(1), AtomId(2)];
        for &theta in &[0.1, 0.6, 1.5, -2.2] {
            let mut actions = Vec::new();
            push_global_ry_with_refocus(&mut actions, wanted, theta, &atoms, 1);
            let nets = net_unitaries(&actions, &atoms);
            let expected = rotation_unitary("ry", theta).expect("ry unitary");
            assert!(
                unitary_distance2(nets[0], expected) < 1e-9,
                "theta={theta}: wanted atom net should be Ry(theta), got {:?}",
                nets[0]
            );
        }
    }

    /// Single-atom case: no bystanders exist, so no refocus overhead is
    /// needed — must fall back to a single plain `GlobalRy`.
    #[test]
    fn global_ry_refocus_skips_echo_when_no_bystanders() {
        let only = AtomId(0);
        let mut actions = Vec::new();
        push_global_ry_with_refocus(&mut actions, only, 0.42, &[only], 1);
        assert_eq!(
            actions,
            vec![NeutralAtomAction::GlobalRy {
                theta_rad: 0.42,
                duration_us: 1,
            }]
        );
    }

    /// Issue #298 review Finding 2: an `after: Some(id)` that fails to
    /// resolve in `interaction_cycle` must be a hard error, not a silent
    /// fallback to `None` (which would mis-schedule the gate to the very
    /// start of the program).
    #[test]
    fn interleave_local_gates_errors_on_unresolved_anchor() {
        use crate::graph::{InteractionId, LogicalQubitId};
        use crate::native_gate_decomp::{DecomposedLocalGate, NaLocalOp};

        let na = load_na_target();
        let decomposed = vec![DecomposedLocalGate {
            qubit: LogicalQubitId(0),
            after: Some(InteractionId(999)), // never present in interaction_cycle
            ops: vec![NaLocalOp::Local(LocalGateKind::Rz(0.5))],
        }];
        let interaction_cycle = BTreeMap::new();
        let result = interleave_local_gates(Vec::new(), &interaction_cycle, decomposed, &na, 1);
        assert!(matches!(
            result,
            Err(NaPipelineError::LocalGateAnchorMissing {
                interaction: InteractionId(999)
            })
        ));
    }
}
