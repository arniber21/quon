//! Front-to-back compile pipeline and metrics collection.

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

use backend::BackendTarget;
use mlir_bridge::emit::openqasm3;
use mlir_bridge::metrics;
use mlir_bridge::passes::{
    classical_region_fusion, clifford_t_opt, compiler_uncomputation, depth_scheduling,
    dynamic_linearity_verifier, gate_cancellation, linearity_verifier, measurement_deferral,
    monadic_lowering, native_gate_decomp, rotation_merging,
    sabre_routing::{self, SabreCost},
    zx_simplification,
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
}

/// Outcome of one compile invocation.
#[derive(Clone, Debug)]
pub struct CompileReport {
    pub qasm: Option<String>,
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
        Ok((qasm, circuit_metrics)) => {
            let compile_ms = started.elapsed().as_millis() as u64;
            CompileReport {
                qasm: Some(qasm),
                snapshot: MetricsSnapshot::ok(
                    program,
                    target_info,
                    toolchain,
                    compile_ms,
                    circuit_metrics,
                ),
            }
        }
        Err(message) => {
            let compile_ms = started.elapsed().as_millis() as u64;
            CompileReport {
                qasm: None,
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

fn compile_inner(request: &CompileRequest) -> Result<(String, CircuitMetrics), String> {
    let context = melior::Context::new();

    let module = frontend::lower::lower_program(&context, &request.source).map_err(|diags| {
        print_diagnostics(&request.source_path, &request.source, &diags);
        format!(
            "compilation failed with {} error{}",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" }
        )
    })?;
    if request.dump_ir {
        eprintln!("--- after lowering ---\n{}", module.as_operation());
    }

    run_circ_passes_to_fixpoint(&context, &module);
    if request.verify_linear {
        verify_circ_linearity(&module).map_err(|e| e.to_string())?;
    }
    if request.dump_ir {
        eprintln!("--- after circ passes ---\n{}", module.as_operation());
    }

    monadic_lowering::run_on_module(&context, &module)
        .map_err(|e| format!("monadic lowering failed: {e}"))?;
    if request.dump_ir {
        eprintln!("--- after monadic lowering ---\n{}", module.as_operation());
    }

    measurement_deferral::run_on_module(&context, &module);
    classical_region_fusion::run_on_module(&context, &module);
    if request.verify_linear {
        verify_dynamic_linearity(&module).map_err(|e| e.to_string())?;
    }
    if request.dump_ir {
        eprintln!("--- after dynamic passes ---\n{}", module.as_operation());
    }

    native_gate_decomp::run_on_module(&context, &request.target, &module);
    let sabre_cost = SabreCost {
        gamma: request.sabre_gamma,
        ..SabreCost::default()
    };
    sabre_routing::run_on_module(&context, &request.target, sabre_cost, &module);
    let t_count = metrics::count_t_gates(&module);
    native_gate_decomp::run_on_module(&context, &request.target, &module);
    depth_scheduling::run_on_module(&context, &request.target, &module);
    if request.dump_ir {
        eprintln!("--- after physical passes ---\n{}", module.as_operation());
    }

    let raw = metrics::collect_module_metrics(&module, &request.target);
    let qubit_count = openqasm3::reify(&module, &request.target)
        .map(|program| program.num_qubits() as u64)
        .unwrap_or(raw.qubit_count);

    let circuit_metrics = CircuitMetrics {
        depth: raw.depth,
        depth_bound: raw.depth_bound,
        gate_count: raw.gate_count,
        t_count,
        qubit_count,
        swap_count: raw.swap_count,
    };

    let qasm = openqasm3::emit(&module, &request.target)
        .map_err(|e| format!("OpenQASM emission failed: {e}"))?;

    Ok((qasm, circuit_metrics))
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

fn run_circ_passes_to_fixpoint(context: &melior::Context, module: &melior::ir::Module<'_>) {
    const MAX_ROUNDS: usize = 10;
    for _ in 0..MAX_ROUNDS {
        let before = module.as_operation().to_string();
        gate_cancellation::run_on_module(context, module);
        rotation_merging::run_on_module(context, module);
        compiler_uncomputation::run_on_module(context, module);
        zx_simplification::run_on_module(context, module);
        clifford_t_opt::run_on_module(context, module);
        let after = module.as_operation().to_string();
        if before == after {
            break;
        }
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
