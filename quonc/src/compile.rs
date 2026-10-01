//! Front-to-back compile adapter: frontend lower → library pipelines → metrics.
//!
//! Pass order and Fixed/NA orchestration live in `mlir_bridge::pipeline` and
//! `quon_na::pipeline`. After lowering, QEC-backed programs
//! (`collect_qec_workload` non-empty) take the hybrid round-expansion path
//! (ADR-0016 / #248); bare-qubit NA programs keep `run_from_module`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, RegionLike};
use quon_core::{
    CircuitMetrics, CompileStatus, MappingStageMetrics, MappingTrace, MappingTraceParts,
    MetricsSnapshot, ProgramInfo, TargetInfo, ToolchainInfo, assemble_mapping_trace,
};
use sha2::{Digest, Sha256};

use crate::include::SourceChunk;
use backend::{BackendTarget, TargetKind};
use mlir_bridge::collect_qec_workload;
use mlir_bridge::emit::openqasm3;
use mlir_bridge::metrics;
use mlir_bridge::metrics::CircuitMetricsRaw;
use mlir_bridge::passes::{
    dynamic_linearity_verifier, linearity_verifier, sabre_routing::SabreCost,
};
use mlir_bridge::pipeline::{
    MappingCapture, dump_ir_stage, emit_openqasm, run_circ_passes_to_fixpoint, run_dynamic_passes,
    run_fixed_physical, run_fixed_physical_with_mapping,
};
use quon_na::{
    DeclaredArchitecture, GraphScheduleRequest, InteractionGraph, NaBackendKind, NaScheduleOptions,
    NaScheduleView, NaScheduleViewMeta, NaStats, NeutralAtomLayout, PlacementStrategy, PlacerMode,
    ResourceReport, ScheduleLayer, ScheduleLowerParams, ScheduleSpec, ScheduleViewZone,
    dump_schedule_text, lower_schedule, objective_from_verified_schedule, run_from_graph,
    run_from_module, run_from_qec_workload, verify_emitted_schedule,
};

/// Inputs for one compile invocation.
#[derive(Debug)]
pub struct CompileRequest {
    pub source_path: PathBuf,
    pub source: String,
    pub target: BackendTarget,
    pub target_descriptor_path: Option<PathBuf>,
    pub dump_ir: bool,
    pub verify_linear: bool,
    /// Run `quantum.na` schedule verification after NA lowering (ADR-0021).
    /// QEC-backed programs always verify regardless of this flag.
    pub verify_na: bool,
    /// SABRE noise-weight coefficient γ (SPEC §7.4). Default 0.3.
    pub sabre_gamma: f64,
    /// SABRE critical-path coefficient β (SPEC §7.4). Default 0.5.
    pub sabre_beta: f64,
    /// SABRE lookahead window size (SPEC §7.4). Default 20.
    pub sabre_lookahead: usize,
    /// Neutral-atom movement backend (ignored for fixed targets).
    pub na_backend: NaBackendKind,
    /// Zoned placer mode (ignored unless `na_backend` is zoned).
    pub na_placer: PlacerMode,
    /// Run schedule compaction (#108) after movement/zoned scheduling.
    pub na_compact: bool,
    /// Flat AOD placement strategy (ignored unless `na_backend` is flat).
    pub na_placement: PlacementStrategy,
    /// State-preparation scheduling mode (issue #302): `Heuristic` (default)
    /// or `Exact` (SMT-optimal, requires `solver` feature).
    pub na_state_prep: quon_na::pipeline::StatePrepMode,
    /// Placement/routing objective (issue #309): `Time` (default) or
    /// `ErrorBudget` (requires target `error_model`; fail-closed otherwise).
    pub na_objective: quon_na::pipeline::NaObjective,
    /// Parse the source as OpenQASM 2/3 (#304) instead of Quon (`.qn`).
    /// Set by the CLI when the input is `.qasm` or `--from-qasm` is passed;
    /// bypasses frontend lowering and enters the NA pipeline directly from
    /// the QASM-derived interaction graph + captured 1-qubit gates.
    pub from_qasm: bool,
    /// Byte ranges of each file inside `source` when `--include` concatenated
    /// several Quon sources (issue #195). Empty means `source` is exactly
    /// `source_path` and diagnostics use that path.
    pub source_chunks: Vec<SourceChunk>,
    /// Record a [`quon_core::MappingTrace`] during SABRE (fixed targets only).
    pub record_mapping: bool,
}

impl Default for CompileRequest {
    fn default() -> Self {
        Self {
            source_path: PathBuf::new(),
            source: String::new(),
            target: backend::generic_openqasm::target(64),
            target_descriptor_path: None,
            dump_ir: false,
            verify_linear: false,
            verify_na: false,
            sabre_gamma: 0.3,
            sabre_beta: 0.5,
            sabre_lookahead: 20,
            na_backend: NaBackendKind::Zoned,
            na_placer: PlacerMode::RoutingAgnostic,
            na_compact: true,
            na_placement: PlacementStrategy::RowMajor,
            na_state_prep: quon_na::pipeline::StatePrepMode::Heuristic,
            na_objective: quon_na::pipeline::NaObjective::default(),
            from_qasm: false,
            source_chunks: Vec::new(),
            record_mapping: false,
        }
    }
}

/// Outcome of one compile invocation.
#[derive(Clone, Debug)]
pub struct CompileReport {
    /// OpenQASM 3.0 text when the fixed-target path ran.
    pub qasm: Option<String>,
    /// Neutral-atom schedule layers when the NA path ran.
    pub na_schedule: Option<Vec<ScheduleLayer>>,
    /// Final NA layout (sites + bindings) when the NA path ran.
    pub na_layout: Option<NeutralAtomLayout>,
    /// Interaction graph when the NA path ran (DOT / JSON tooling).
    pub na_graph: Option<InteractionGraph>,
    /// Canonical `quantum.na` schedule spec (ADR-0011) when the NA path ran.
    pub na_schedule_spec: Option<ScheduleSpec>,
    /// Neutral-atom resource report when the NA path ran.
    pub resource_report: Option<ResourceReport>,
    /// Compiler-internals stats (timings / search diagnostics / config echo,
    /// issue #307) when the NA path ran. Populated for both bare-qubit
    /// (`run_from_graph` / `run_from_module`) and QEC hybrid
    /// (`run_from_qec_workload`) paths via the shared `plan_backend` stage
    /// (#317). `None` for fixed targets.
    pub na_stats: Option<NaStats>,
    /// Interaction-graph vertex count (logical qubits) for NA compiles.
    pub na_logical_qubits: Option<u64>,
    /// True when the entrypoint used QEC builtins (ADR-0021 auto `--verify-na`).
    pub qec_backed: bool,
    /// QEC workload IR when the compile took the hybrid path (ADR-0016 / #255).
    pub qec_workload: Option<quon_qec::QecWorkload>,
    /// SABRE mapping capture when `CompileRequest::record_mapping` is set on a
    /// fixed target. `None` for neutral-atom compiles and when the flag is off.
    pub mapping: Option<MappingCapture>,
    pub snapshot: MetricsSnapshot,
}

/// Compile Quon source through the full pipeline.
pub fn compile(request: &CompileRequest) -> CompileReport {
    let started = Instant::now();
    let toolchain = probe_toolchain();
    let program = ProgramInfo {
        source: request.source_path.display().to_string(),
        source_sha256: sha256_hex(&request.source),
        entry: "main".to_string(),
    };
    let target_info = TargetInfo {
        id: request.target.id.clone(),
        descriptor_path: request
            .target_descriptor_path
            .as_ref()
            .map(|p| p.display().to_string()),
    };

    match compile_inner(request) {
        Ok(artifacts) => {
            let compile_ms = started.elapsed().as_millis() as u64;
            let mut snapshot = MetricsSnapshot::ok(
                program,
                target_info,
                toolchain,
                compile_ms,
                artifacts.circuit_metrics,
            );
            if let Some(message) = artifacts.openqasm_error {
                snapshot.compile.status = quon_core::CompileStatus::Error;
                snapshot.compile.error = Some(message);
            }
            CompileReport {
                qasm: artifacts.qasm,
                na_schedule: artifacts.na_schedule,
                na_layout: artifacts.na_layout,
                na_graph: artifacts.na_graph,
                na_schedule_spec: artifacts.na_schedule_spec,
                resource_report: artifacts.resource_report,
                na_stats: artifacts.na_stats,
                na_logical_qubits: artifacts.na_logical_qubits,
                qec_backed: artifacts.qec_backed,
                qec_workload: artifacts.qec_workload,
                mapping: artifacts.mapping,
                snapshot,
            }
        }
        Err(message) => {
            let compile_ms = started.elapsed().as_millis() as u64;
            CompileReport {
                qasm: None,
                na_schedule: None,
                na_layout: None,
                na_graph: None,
                na_schedule_spec: None,
                resource_report: None,
                na_stats: None,
                na_logical_qubits: None,
                qec_backed: false,
                qec_workload: None,
                mapping: None,
                snapshot: MetricsSnapshot {
                    schema_version: quon_core::SCHEMA_VERSION,
                    program,
                    target: target_info,
                    toolchain,
                    compile: quon_core::CompileInfo {
                        status: CompileStatus::Error,
                        compile_ms,
                        error: Some(message),
                    },
                    metrics: None,
                    simulation: None,
                },
            }
        }
    }
}

struct CompileArtifacts {
    qasm: Option<String>,
    na_schedule: Option<Vec<ScheduleLayer>>,
    na_layout: Option<NeutralAtomLayout>,
    na_graph: Option<InteractionGraph>,
    na_schedule_spec: Option<ScheduleSpec>,
    resource_report: Option<ResourceReport>,
    na_stats: Option<NaStats>,
    na_logical_qubits: Option<u64>,
    qec_backed: bool,
    qec_workload: Option<quon_qec::QecWorkload>,
    mapping: Option<MappingCapture>,
    circuit_metrics: CircuitMetrics,
    /// Set when fixed-target OpenQASM emission rejects the circuit. The
    /// mapping capture is kept so `--emit-mapping-json` can still print the
    /// trace; the compile status stays an error.
    openqasm_error: Option<String>,
}

fn compile_inner(request: &CompileRequest) -> Result<CompileArtifacts, String> {
    if request.from_qasm {
        return compile_qasm(request);
    }
    let context = melior::Context::new();

    let module = frontend::lower::lower_program(&context, &request.source).map_err(|diags| {
        print_mapped_diagnostics(
            &request.source_path,
            &request.source,
            &request.source_chunks,
            &diags,
        );
        format!(
            "compilation failed with {} error{}",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" }
        )
    })?;
    dump_ir_stage(request.dump_ir, "after lowering", &module);

    run_circ_passes_to_fixpoint(&context, &module);
    if request.verify_linear {
        verify_circ_linearity(&module).map_err(|e| e.to_string())?;
    }
    dump_ir_stage(request.dump_ir, "after circ passes", &module);

    // `lower_program` already emits `quantum.dynamic` IR directly (the staging
    // dialect was collapsed in #213 / ADR-0037) — no separate lowering pass.
    run_dynamic_passes(&context, &module);
    if request.verify_linear {
        verify_dynamic_linearity(&module).map_err(|e| e.to_string())?;
    }
    dump_ir_stage(request.dump_ir, "after dynamic passes", &module);

    match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => {
            let opts = NaScheduleOptions {
                backend: request.na_backend,
                placer: request.na_placer,
                compact: request.na_compact,
                placement: request.na_placement,
                dump_ir: request.dump_ir,
                state_prep: request.na_state_prep,
                objective: request.na_objective,
                ..Default::default()
            };
            // ADR-0016 / #248: QEC-backed entrypoints expand via workload IR;
            // bare-qubit NA programs keep the interaction-graph extract path.
            let workload =
                collect_qec_workload(&module).map_err(|e| format!("QEC workload collect: {e}"))?;
            let qec_backed = !workload.blocks().is_empty();
            let artifacts = if qec_backed {
                if request.dump_ir {
                    eprintln!(
                        "--- QEC workload ---\nblocks={} ops={} memory_rounds={}",
                        workload.blocks().len(),
                        workload.ops().len(),
                        workload.memory_round_count()
                    );
                }
                run_from_qec_workload(&workload, na, opts).map_err(|e| e.to_string())?
            } else {
                run_from_module(&module, na, opts).map_err(|e| e.to_string())?
            };
            // `quon_na::pipeline` doesn't know its own target id (only the
            // wrapping `BackendTarget` does) — overlay it here so the emitted
            // stats file is self-describing (issue #307).
            let na_stats = artifacts.stats.clone().map(|mut stats| {
                stats.version.target_id = Some(request.target.id.clone());
                stats
            });
            // ADR-0011: quantum.na is the canonical schedule IR. A planner
            // schedule that cannot lower to it is a compile failure, not a
            // degraded artifact.
            let lower_params = ScheduleLowerParams::from_target(request.target.id.clone(), na);
            let schedule_spec = lower_schedule(&artifacts.request, &lower_params)
                .map_err(|e| format!("lowering schedule to quantum.na failed: {e}"))?;
            // ADR-0021: auto-verify any QEC-backed NA compile; physical NA only
            // when `--verify-na` is set. Feed-forward deps stay compaction-only.
            let declared =
                declared_architecture(na, artifacts.request.layout.as_ref(), request.na_backend)?;
            maybe_verify_na_schedule(
                &schedule_spec,
                Some(&declared),
                request.verify_na,
                qec_backed,
            )?;
            let mut resource_report = artifacts.resource_report;
            if should_verify_na(request.verify_na, qec_backed) {
                resource_report =
                    verified_schedule_objective(resource_report, &schedule_spec, &na.cost_model)?;
            }
            let circuit_metrics = CircuitMetrics {
                depth: resource_report.estimated_cycles,
                depth_bound: Some(resource_report.estimated_cycles.to_string()),
                gate_count: resource_report.entangle2_count + resource_report.entangle_n_count,
                t_count: 0,
                qubit_count: artifacts.logical_qubits,
                swap_count: 0,
            };
            Ok(CompileArtifacts {
                qasm: None,
                na_schedule: Some(artifacts.layers),
                na_layout: artifacts.request.layout.clone(),
                na_graph: Some(artifacts.request.graph.clone()),
                na_schedule_spec: Some(schedule_spec),
                resource_report: Some(resource_report),
                na_stats,
                na_logical_qubits: Some(artifacts.logical_qubits),
                qec_backed,
                qec_workload: qec_backed.then_some(workload),
                mapping: None,
                circuit_metrics,
                openqasm_error: None,
            })
        }
        TargetKind::Fixed(_) => compile_fixed(request, &context, &module),
    }
}

/// Compile an OpenQASM 2/3 source through the NA pipeline (#304).
///
/// Bypasses frontend lowering (no `.qn`): `quonc::qasm::parse_to_graph`
/// builds the interaction graph + captured 1-qubit gates (the same contract
/// `quon_na::extract` produces for `.qn`), then `run_from_graph_with_local_gates`
/// enters the existing NA schedule/lower/verify stages unchanged. QASM
/// ingestion is bare-qubit only — QEC-backed (`collect_qec_workload`) entry
/// stays a `.qn` concern — so this path is never QEC-backed and requires a
/// neutral-atom target.
fn compile_qasm(request: &CompileRequest) -> Result<CompileArtifacts, String> {
    let na = match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => na,
        TargetKind::Fixed(_) => {
            return Err(
                "OpenQASM ingestion (#304) targets the neutral-atom pipeline — \
                 pass a neutral-atom target via --target."
                    .to_string(),
            );
        }
    };
    let (graph, local_gates) = crate::qasm::parse_to_graph(&request.source)
        .map_err(|e| format!("OpenQASM ingestion failed: {e}"))?;
    let logical_qubits = graph.vertices.len() as u64;
    if request.dump_ir {
        eprintln!(
            "--- QASM interaction graph ---\nvertices={} interactions={} local_1q_gates={}",
            graph.vertices.len(),
            graph.interactions.len(),
            local_gates.len(),
        );
    }
    let opts = NaScheduleOptions {
        backend: request.na_backend,
        placer: request.na_placer,
        compact: request.na_compact,
        placement: request.na_placement,
        dump_ir: request.dump_ir,
        state_prep: request.na_state_prep,
        objective: request.na_objective,
        ..Default::default()
    };
    // QASM ingestion rejects `measure`/`reset` at parse time (#405) —
    // measurement is orchestrated by the source `.qn`'s `measure_all`,
    // not by benchmark QASM — so there is no measured-qubit set to pass
    // here.
    let artifacts = quon_na::pipeline::run_from_graph_with_local_gates(
        graph,
        local_gates,
        Vec::new(),
        na,
        opts,
        Some(logical_qubits),
    )
    .map_err(|e| e.to_string())?;
    let na_stats = artifacts.stats.clone().map(|mut stats| {
        stats.version.target_id = Some(request.target.id.clone());
        stats
    });
    let lower_params = ScheduleLowerParams::from_target(request.target.id.clone(), na);
    let schedule_spec = lower_schedule(&artifacts.request, &lower_params)
        .map_err(|e| format!("lowering schedule to quantum.na failed: {e}"))?;
    let declared =
        declared_architecture(na, artifacts.request.layout.as_ref(), request.na_backend)?;
    maybe_verify_na_schedule(&schedule_spec, Some(&declared), request.verify_na, false)?;
    let mut resource_report = artifacts.resource_report;
    if should_verify_na(request.verify_na, false) {
        resource_report =
            verified_schedule_objective(resource_report, &schedule_spec, &na.cost_model)?;
    }
    let circuit_metrics = CircuitMetrics {
        depth: resource_report.estimated_cycles,
        depth_bound: Some(resource_report.estimated_cycles.to_string()),
        gate_count: resource_report.entangle2_count + resource_report.entangle_n_count,
        t_count: 0,
        qubit_count: artifacts.logical_qubits,
        swap_count: 0,
    };
    Ok(CompileArtifacts {
        qasm: None,
        na_schedule: Some(artifacts.layers),
        na_layout: artifacts.request.layout.clone(),
        na_graph: Some(artifacts.request.graph.clone()),
        na_schedule_spec: Some(schedule_spec),
        resource_report: Some(resource_report),
        na_stats,
        na_logical_qubits: Some(artifacts.logical_qubits),
        qec_backed: false,
        qec_workload: None,
        mapping: None,
        circuit_metrics,
        openqasm_error: None,
    })
}

fn compile_fixed(
    request: &CompileRequest,
    context: &melior::Context,
    module: &melior::ir::Module<'_>,
) -> Result<CompileArtifacts, String> {
    let sabre_cost = SabreCost {
        beta: request.sabre_beta,
        gamma: request.sabre_gamma,
        lookahead: request.sabre_lookahead,
        ..SabreCost::default()
    };
    let (physical, mapping) = if request.record_mapping {
        let (physical, capture) =
            run_fixed_physical_with_mapping(context, &request.target, sabre_cost, module);
        (physical, Some(capture))
    } else {
        (
            run_fixed_physical(context, &request.target, sabre_cost, module),
            None,
        )
    };
    dump_ir_stage(request.dump_ir, "after physical passes", module);

    let raw = metrics::collect_module_metrics(module, &request.target);
    let qubit_count = openqasm3::reify(module, &request.target)
        .map(|program| program.num_qubits() as u64)
        .unwrap_or(raw.qubit_count);

    let circuit_metrics = CircuitMetrics {
        depth: raw.depth,
        depth_bound: raw.depth_bound,
        gate_count: raw.gate_count,
        t_count: physical.t_count,
        qubit_count,
        swap_count: raw.swap_count,
    };

    // Hardware rejection is a failed compile, but the SABRE log was already
    // captured. Keep it so a mapping trace can still describe the circuit
    // that emission refused.
    let (qasm, openqasm_error) = match emit_openqasm(module, &request.target) {
        Ok(qasm) => (Some(qasm), None),
        Err(error) => (None, Some(format!("OpenQASM emission failed: {error}"))),
    };

    Ok(CompileArtifacts {
        qasm,
        na_schedule: None,
        na_layout: None,
        na_graph: None,
        na_schedule_spec: None,
        resource_report: None,
        na_stats: None,
        na_logical_qubits: None,
        qec_backed: false,
        qec_workload: None,
        mapping,
        circuit_metrics,
        openqasm_error,
    })
}

fn stage_metrics(raw: &CircuitMetricsRaw) -> MappingStageMetrics {
    MappingStageMetrics {
        gate_count: raw.gate_count,
        depth: raw.depth,
        swap_count: raw.swap_count,
        t_count: raw.t_count,
    }
}

/// Build the fixed-target mapping trace (issue #135).
///
/// Requires [`CompileRequest::record_mapping`] on a fixed target. SWAP events
/// come from SABRE before native decomposition; the `native_decomp` stage is
/// the final scheduled circuit, whose `swap_count` may be 0 after SWAP→CX
/// decomposition.
pub fn build_mapping_trace(
    report: &CompileReport,
    request: &CompileRequest,
) -> Result<MappingTrace, anyhow::Error> {
    let fixed = request.target.fixed_target().ok_or_else(|| {
        anyhow!(
            "--emit-mapping-json requires a fixed target (neutral-atom schedules use --emit-na-schedule)"
        )
    })?;
    let capture = report.mapping.as_ref().ok_or_else(|| {
        anyhow!("no mapping trace was recorded (fixed-target compile did not capture SABRE)")
    })?;
    let final_metrics = report.snapshot.metrics.clone().unwrap_or_default();
    let edges = fixed
        .topology
        .edges()
        .iter()
        .map(|(left, right)| (*left as u64, *right as u64))
        .collect();
    Ok(assemble_mapping_trace(MappingTraceParts {
        target_id: request.target.id.clone(),
        edges,
        log: capture.log.clone(),
        before_routing: stage_metrics(&capture.before_routing),
        after_routing: stage_metrics(&capture.after_routing),
        final_metrics: MappingStageMetrics {
            gate_count: final_metrics.gate_count,
            depth: final_metrics.depth,
            swap_count: final_metrics.swap_count,
            t_count: final_metrics.t_count,
        },
    }))
}

/// Build the debug/visualization schedule envelope (#113).
pub fn build_na_schedule_view(
    report: &CompileReport,
    request: &CompileRequest,
) -> Result<NaScheduleView, anyhow::Error> {
    let layers = report
        .na_schedule
        .as_ref()
        .ok_or_else(|| anyhow!("no NA schedule available (compile with a neutral-atom target)"))?;
    let metrics = report.resource_report.clone().ok_or_else(|| {
        anyhow!("no resource report available (compile with a neutral-atom target)")
    })?;
    let zones = match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => na
            .zones
            .iter()
            .map(|z| ScheduleViewZone {
                zone_id: z.zone_id,
                kind: z.kind,
                origin_um: [z.origin_um.0, z.origin_um.1],
                width_um: z.width_um(),
                height_um: z.height_um(),
                rows: z.rows,
                cols: z.cols,
                site_pitch_um: [z.site_pitch_um.0, z.site_pitch_um.1],
            })
            .collect(),
        TargetKind::Fixed(_) => {
            bail!("NA schedule view requires a neutral-atom target")
        }
    };
    Ok(NaScheduleView::new(
        NaScheduleViewMeta {
            target_id: request.target.id.clone(),
            na_backend: request.na_backend,
            na_placer: request.na_placer,
        },
        metrics,
        zones,
        report.na_layout.clone(),
        layers.clone(),
    ))
}

/// Serialize a schedule visualization envelope to pretty JSON
/// (`quantum.na` MLIR is the canonical schedule artifact, ADR-0011).
pub fn schedule_to_json(view: &NaScheduleView) -> Result<String, serde_json::Error> {
    view.to_json_string_pretty()
}

/// Render the canonical `quantum.na` schedule as generic-form textual MLIR.
pub fn schedule_to_mlir(spec: &ScheduleSpec) -> Result<String> {
    dump_schedule_text(spec).map_err(|e| anyhow!("emitting quantum.na MLIR failed: {e}"))
}

/// Whether NA schedule verification should run (ADR-0021 / #256).
///
/// QEC-backed compiles always verify; physical NA only when `--verify-na`.
pub fn should_verify_na(verify_na: bool, qec_backed: bool) -> bool {
    verify_na || qec_backed
}

/// Architecture and placement-time bindings for the post-compaction replay.
///
/// Fails closed when the layout never recorded a declared start. The
/// planner's rewritten `initial_bindings` are not a substitute.
fn declared_architecture(
    na: &backend::NeutralAtomTarget,
    layout: Option<&NeutralAtomLayout>,
    backend: NaBackendKind,
) -> Result<DeclaredArchitecture, String> {
    let layout = layout.ok_or_else(|| {
        "quantum.na verification requires a layout with declared initial bindings".to_string()
    })?;
    DeclaredArchitecture::from_compiled(na, layout, backend == NaBackendKind::Zoned)
        .map_err(|error| format!("quantum.na verification failed: {error}"))
}

/// Run [`verify_emitted_schedule`] when [`should_verify_na`] is true.
///
/// Extracted so tests fail if the auto-verify gate stops calling the verifier.
/// The replay starts from `declared`, not from planner-final occupancy.
pub fn maybe_verify_na_schedule(
    spec: &ScheduleSpec,
    declared: Option<&DeclaredArchitecture>,
    verify_na: bool,
    qec_backed: bool,
) -> Result<(), String> {
    if !should_verify_na(verify_na, qec_backed) {
        return Ok(());
    }
    let declared = declared.ok_or_else(|| {
        "quantum.na verification requires the declared architecture and initial bindings"
            .to_string()
    })?;
    verify_emitted_schedule(spec, declared)
        .map_err(|error| format!("quantum.na verification failed: {error}"))
}

/// Recompute the §9 objective from a schedule that just passed verification.
fn verified_schedule_objective(
    report: ResourceReport,
    spec: &ScheduleSpec,
    weights: &backend::NeutralAtomCostModel,
) -> Result<ResourceReport, String> {
    let objective = objective_from_verified_schedule(spec, weights)
        .map_err(|error| format!("quantum.na schedule objective: {error}"))?;
    Ok(report.with_schedule_objective(objective))
}

/// Renders frontend diagnostics with a caret at the offending source span.
pub fn print_diagnostics(
    source_path: &Path,
    source: &str,
    diags: &[frontend::diagnostics::Diagnostic],
) {
    print_mapped_diagnostics(source_path, source, &[], diags);
}

/// Like [`print_diagnostics`], but attributes each span to the `--include`
/// chunk that contains it when `chunks` is non-empty.
pub fn print_mapped_diagnostics(
    source_path: &Path,
    source: &str,
    chunks: &[SourceChunk],
    diags: &[frontend::diagnostics::Diagnostic],
) {
    for diag in diags {
        let (id, local_source, start, end) =
            map_diag_span(source_path, source, chunks, diag.span.start, diag.span.end);
        let report = ariadne::Report::build(ariadne::ReportKind::Error, id.clone(), start)
            .with_message(&diag.message)
            .with_label(
                ariadne::Label::new((id.clone(), start..end))
                    .with_message(&diag.message)
                    .with_color(ariadne::Color::Red),
            )
            .finish();
        let _ = report.eprint((id, ariadne::Source::from(local_source)));
    }
}

fn map_diag_span(
    source_path: &Path,
    source: &str,
    chunks: &[SourceChunk],
    start: usize,
    end: usize,
) -> (String, String, usize, usize) {
    if let Some(chunk) = crate::include::chunk_at(chunks, start) {
        let local = source.get(chunk.start..chunk.end).unwrap_or("");
        let rel_start = start.saturating_sub(chunk.start).min(local.len());
        let rel_end = end
            .clamp(chunk.start, chunk.end)
            .saturating_sub(chunk.start);
        let rel_end = rel_end.max(rel_start).min(local.len());
        return (
            chunk.path.display().to_string(),
            local.to_string(),
            rel_start,
            rel_end,
        );
    }
    let end = end.min(source.len()).max(start.min(source.len()));
    let start = start.min(source.len());
    (
        source_path.display().to_string(),
        source.to_string(),
        start,
        end,
    )
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn verify_circ_linearity(module: &melior::ir::Module<'_>) -> Result<()> {
    let Some(body) = module
        .as_operation()
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return Ok(());
    };
    let mut op = body.first_operation();
    while let Some(current) = op {
        if op_name(&current) == mlir_bridge::dialect::quantum_circ::op::FUNC {
            let diagnostics = linearity_verifier::check_linearity(&current);
            if !diagnostics.is_empty() {
                let rendered = diagnostics
                    .iter()
                    .map(|d| format!("  - {d}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!("linearity verification failed:\n{rendered}");
            }
        }
        op = current.next_in_block();
    }
    Ok(())
}

fn verify_dynamic_linearity(module: &melior::ir::Module<'_>) -> Result<()> {
    let region = module
        .as_operation()
        .region(0)
        .map_err(|e| anyhow!("module has no top-level region: {e}"))?;
    let diagnostics = dynamic_linearity_verifier::check_dynamic_linearity(region);
    if !diagnostics.is_empty() {
        let rendered = diagnostics
            .iter()
            .map(|d| format!("  - {d}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!("dynamic linearity verification failed:\n{rendered}");
    }
    Ok(())
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn probe_toolchain() -> ToolchainInfo {
    let git_commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string());

    let git_dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    ToolchainInfo {
        quonc_version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit,
        git_dirty,
    }
}

/// Debug/stress entry: schedule a raw interaction graph without Quon source.
pub fn schedule_raw_graph(
    graph: quon_na::InteractionGraph,
    na: &backend::NeutralAtomTarget,
    backend: NaBackendKind,
    placer: PlacerMode,
    compact: bool,
    placement: PlacementStrategy,
) -> Result<(GraphScheduleRequest, ResourceReport)> {
    let opts = NaScheduleOptions {
        backend,
        placer,
        compact,
        placement,
        dump_ir: false,
        ..Default::default()
    };
    let artifacts = run_from_graph(graph, na, opts, None)?;
    Ok((artifacts.request, artifacts.resource_report))
}

#[cfg(test)]
mod verify_na_gate_tests {
    use super::*;
    use quon_na::dialect::{ActionSpec, LayerSpec, ScheduleSpec};

    fn bad_measure_reuse_spec() -> ScheduleSpec {
        ScheduleSpec {
            target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            aod_min_separation_um: 2.0,
            layers: vec![
                LayerSpec {
                    cycle: 0,
                    actions: vec![ActionSpec::Measure {
                        atom: 0,
                        basis: "z".to_string(),
                        duration_us: 10,
                    }],
                },
                LayerSpec {
                    cycle: 1,
                    actions: vec![ActionSpec::Entangle {
                        pairs: vec![quon_na::dialect::EntanglePairSpec {
                            lhs: quon_na::dialect::PositionedAtom {
                                atom: 0,
                                x_um: 0.0,
                                y_um: 0.0,
                            },
                            rhs: quon_na::dialect::PositionedAtom {
                                atom: 1,
                                x_um: 6.0,
                                y_um: 0.0,
                            },
                        }],
                        duration_us: 1,
                    }],
                },
            ],
        }
    }

    fn declared() -> DeclaredArchitecture {
        DeclaredArchitecture {
            sites: vec![quon_na::AtomSite {
                id: quon_na::SiteId(0),
                position: quon_na::Position {
                    x_um: 0.0,
                    y_um: 0.0,
                },
            }],
            initial_bindings: vec![quon_na::AtomBinding {
                atom: quon_na::AtomId(0),
                trap: quon_na::TrapBinding::Slm {
                    site: quon_na::SiteId(0),
                },
            }],
            zones: vec![],
            check_zones: false,
            require_readout_zone: false,
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            aod_min_separation_um: 2.0,
        }
    }

    #[test]
    fn qec_backed_gate_runs_verify_without_flag() {
        let bad = bad_measure_reuse_spec();
        let declared = declared();
        assert!(
            maybe_verify_na_schedule(&bad, Some(&declared), false, true).is_err(),
            "QEC-backed must verify even when verify_na=false"
        );
        assert!(
            maybe_verify_na_schedule(&bad, None, false, false).is_ok(),
            "physical without flag must skip verify"
        );
        assert!(
            maybe_verify_na_schedule(&bad, Some(&declared), true, false).is_err(),
            "physical with --verify-na must verify"
        );
    }

    #[test]
    fn should_verify_na_matches_adr_0021() {
        assert!(should_verify_na(false, true));
        assert!(should_verify_na(true, false));
        assert!(should_verify_na(true, true));
        assert!(!should_verify_na(false, false));
    }
}
