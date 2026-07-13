//! Front-to-back compile adapter: frontend lower → library pipelines → metrics.
//!
//! Pass order and Fixed/NA orchestration live in `mlir_bridge::pipeline` and
//! `quon_na::pipeline`. This module collects CLI-facing request/report types and
//! toolchain metadata.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, RegionLike};
use quon_core::{
    CircuitMetrics, CompileStatus, MetricsSnapshot, ProgramInfo, TargetInfo, ToolchainInfo,
};
use sha2::{Digest, Sha256};

use backend::{BackendTarget, TargetKind};
use mlir_bridge::emit::openqasm3;
use mlir_bridge::metrics;
use mlir_bridge::passes::{
    dynamic_linearity_verifier, linearity_verifier, monadic_lowering, sabre_routing::SabreCost,
};
use mlir_bridge::pipeline::{
    dump_ir_stage, emit_openqasm, run_circ_passes_to_fixpoint, run_dynamic_passes,
    run_fixed_physical,
};
use quon_na::{
    GraphScheduleRequest, NaBackendKind, NaScheduleOptions, PlacementStrategy, PlacerMode,
    ResourceReport, ScheduleLayer, run_from_graph, run_from_module,
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
    /// SABRE noise-weight coefficient γ (SPEC §7.4). Default 0.3.
    pub sabre_gamma: f64,
    /// Neutral-atom movement backend (ignored for fixed targets).
    pub na_backend: NaBackendKind,
    /// Zoned placer mode (ignored unless `na_backend` is zoned).
    pub na_placer: PlacerMode,
    /// Run schedule compaction (#108) after movement/zoned scheduling.
    pub na_compact: bool,
    /// Flat AOD placement strategy (ignored unless `na_backend` is flat).
    pub na_placement: PlacementStrategy,
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
            sabre_gamma: 0.3,
            na_backend: NaBackendKind::Zoned,
            na_placer: PlacerMode::RoutingAgnostic,
            na_compact: true,
            na_placement: PlacementStrategy::RowMajor,
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
    /// Neutral-atom resource report when the NA path ran.
    pub resource_report: Option<ResourceReport>,
    /// Interaction-graph vertex count (logical qubits) for NA compiles.
    pub na_logical_qubits: Option<u64>,
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
            CompileReport {
                qasm: artifacts.qasm,
                na_schedule: artifacts.na_schedule,
                resource_report: artifacts.resource_report,
                na_logical_qubits: artifacts.na_logical_qubits,
                snapshot: MetricsSnapshot::ok(
                    program,
                    target_info,
                    toolchain,
                    compile_ms,
                    artifacts.circuit_metrics,
                ),
            }
        }
        Err(message) => {
            let compile_ms = started.elapsed().as_millis() as u64;
            CompileReport {
                qasm: None,
                na_schedule: None,
                resource_report: None,
                na_logical_qubits: None,
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
    resource_report: Option<ResourceReport>,
    na_logical_qubits: Option<u64>,
    circuit_metrics: CircuitMetrics,
}

fn compile_inner(request: &CompileRequest) -> Result<CompileArtifacts, String> {
    let context = melior::Context::new();

    let module = frontend::lower::lower_program(&context, &request.source).map_err(|diags| {
        print_diagnostics(&request.source_path, &request.source, &diags);
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

    monadic_lowering::run_on_module(&context, &module)
        .map_err(|e| format!("monadic lowering failed: {e}"))?;
    dump_ir_stage(request.dump_ir, "after monadic lowering", &module);

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
            };
            let artifacts = run_from_module(&module, na, opts).map_err(|e| e.to_string())?;
            let circuit_metrics = CircuitMetrics {
                depth: artifacts.resource_report.estimated_cycles,
                depth_bound: Some(artifacts.resource_report.estimated_cycles.to_string()),
                gate_count: artifacts.resource_report.entangle2_count
                    + artifacts.resource_report.entangle_n_count,
                t_count: 0,
                qubit_count: artifacts.logical_qubits,
                swap_count: 0,
            };
            Ok(CompileArtifacts {
                qasm: None,
                na_schedule: Some(artifacts.layers),
                resource_report: Some(artifacts.resource_report),
                na_logical_qubits: Some(artifacts.logical_qubits),
                circuit_metrics,
            })
        }
        TargetKind::Fixed(_) => compile_fixed(request, &context, &module),
    }
}

fn compile_fixed(
    request: &CompileRequest,
    context: &melior::Context,
    module: &melior::ir::Module<'_>,
) -> Result<CompileArtifacts, String> {
    let sabre_cost = SabreCost {
        gamma: request.sabre_gamma,
        ..SabreCost::default()
    };
    let physical = run_fixed_physical(context, &request.target, sabre_cost, module);
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

    let qasm = emit_openqasm(module, &request.target)
        .map_err(|e| format!("OpenQASM emission failed: {e}"))?;

    Ok(CompileArtifacts {
        qasm: Some(qasm),
        na_schedule: None,
        resource_report: None,
        na_logical_qubits: None,
        circuit_metrics,
    })
}

/// Serialize schedule layers to pretty JSON.
pub fn schedule_to_json(layers: &[ScheduleLayer]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(layers)
}

/// Renders frontend diagnostics with a caret at the offending source span.
pub fn print_diagnostics(
    source_path: &Path,
    source: &str,
    diags: &[frontend::diagnostics::Diagnostic],
) {
    let id = source_path.display().to_string();
    for diag in diags {
        let span = diag.span.start..diag.span.end;
        let report =
            ariadne::Report::build(ariadne::ReportKind::Error, id.clone(), diag.span.start)
                .with_message(&diag.message)
                .with_label(
                    ariadne::Label::new((id.clone(), span))
                        .with_message(&diag.message)
                        .with_color(ariadne::Color::Red),
                )
                .finish();
        let _ = report.eprint((id.clone(), ariadne::Source::from(source)));
    }
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
    };
    let artifacts = run_from_graph(graph, na, opts, None)?;
    Ok((artifacts.request, artifacts.resource_report))
}
