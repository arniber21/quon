use anyhow::{Context as _, Result, anyhow, bail};
use clap::Parser;
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, RegionLike};

use backend::{BackendTarget, TargetKind, generic_openqasm};
use mlir_bridge::emit::openqasm3;
use mlir_bridge::passes::{
    classical_region_fusion, clifford_t_opt, compiler_uncomputation, depth_scheduling,
    dynamic_linearity_verifier, gate_cancellation, linearity_verifier, measurement_deferral,
    monadic_lowering, native_gate_decomp, rotation_merging,
    sabre_routing::{self, SabreCost},
    zx_simplification,
};

#[derive(Parser)]
#[command(name = "quonc", about = "Quon quantum compiler", version)]
struct Cli {
    /// Source file to compile (.qn). Optional when using --print-target.
    source: Option<std::path::PathBuf>,

    /// Emit OpenQASM 3.0 to stdout
    #[arg(long)]
    emit_qasm: bool,

    /// Backend target descriptor (JSON). Defaults to generic_openqasm.
    #[arg(long)]
    target: Option<std::path::PathBuf>,

    /// Print the loaded backend target summary and exit.
    #[arg(long)]
    print_target: bool,

    /// Dump MLIR after each pass (debug)
    #[arg(long)]
    dump_ir: bool,

    /// Run the linearity verifier pass (debug)
    #[arg(long)]
    verify_linear: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // The emitter reads only the target's native gate set and id, not its
    // topology, so the qubit width here is immaterial; `generic_openqasm` is
    // an all-to-all target used when the caller supplies no device JSON.
    let target = match &cli.target {
        Some(path) => backend::json::load(path)
            .map_err(|e| anyhow!("loading target {}: {e}", path.display()))?,
        None => generic_openqasm::target(64),
    };

    if cli.print_target {
        print!("{}", render_target_summary(&target));
        return Ok(());
    }

    if target.fixed_target().is_none() {
        bail!(
            "target `{}` has kind `{}`; the OpenQASM pipeline currently supports only fixed targets (use --print-target to inspect this target)",
            target.id,
            target.kind_name()
        );
    }

    let source_path = cli
        .source
        .as_ref()
        .ok_or_else(|| anyhow!("missing source file; pass a .qn source or use --print-target"))?;
    let source = std::fs::read_to_string(source_path)
        .with_context(|| format!("reading {}", source_path.display()))?;

    let qasm = compile_to_qasm(
        source_path,
        &source,
        &target,
        cli.dump_ir,
        cli.verify_linear,
    )?;

    if cli.emit_qasm {
        print!("{qasm}");
    } else {
        // Without --emit-qasm there is no other output mode yet; emitting is the
        // only terminal stage, so make that explicit rather than silently exiting.
        eprintln!("(compiled successfully; pass --emit-qasm to print OpenQASM 3.0)");
    }
    Ok(())
}

fn render_target_summary(target: &BackendTarget) -> String {
    let mut out = String::new();
    out.push_str(&format!("target: {}\n", target.id));
    out.push_str(&format!("kind: {}\n", target.kind_name()));

    match &target.kind {
        TargetKind::Fixed(fixed) => {
            out.push_str(&format!("num_qubits: {}\n", fixed.num_qubits));
            out.push_str(&format!("topology_edges: {}\n", fixed.topology.edges.len()));
            out.push_str(&format!(
                "native_gates: {}\n",
                fixed
                    .native_gates
                    .iter()
                    .map(|gate| gate.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            out.push_str(&format!(
                "measurement_latency_us: {}\n",
                fixed.meas_latency_us
            ));
            out.push_str(&format!(
                "supports_mid_circuit_meas: {}\n",
                fixed.supports_mid_circuit_meas
            ));
            out.push_str(&format!(
                "supports_feed_forward: {}\n",
                fixed.supports_feed_forward
            ));
        }
        TargetKind::NeutralAtomReconfigurable(na) => {
            out.push_str(&format!(
                "grid_um: {} x {}\n",
                na.grid.width_um, na.grid.height_um
            ));
            out.push_str(&format!("zones: {}\n", na.zones.len()));
            out.push_str(&format!(
                "zone_capacity: storage={}, entanglement={}, readout={}\n",
                na.zone_capacity(backend::ZoneKind::Storage),
                na.zone_capacity(backend::ZoneKind::Entanglement),
                na.zone_capacity(backend::ZoneKind::Readout)
            ));
            out.push_str(&format!(
                "movement: aod_row_column_coupled, rows={}, cols={}, aods={}, trap_transfer_us={}\n",
                na.movement.aod_rows,
                na.movement.aod_cols,
                na.movement.num_aods,
                na.movement.trap_transfer_us
            ));
            out.push_str(&format!(
                "rydberg: range_um={}, min_spacing_um={}, max_parallel_pairs={}\n",
                na.interaction.rydberg_range_um,
                na.interaction.min_rydberg_spacing_um,
                na.interaction.max_parallel_entangling_pairs
            ));
            out.push_str(&format!(
                "timing_us: cz={}, single_qubit={}, measurement={}, reset={}\n",
                na.timing.cz_us,
                na.timing.single_qubit_us,
                na.timing.measurement_us,
                na.timing.reset_us
            ));
            out.push_str(&format!("native_gates: {}\n", na.native_gates.join(", ")));
        }
    }

    out
}

/// Front-to-back compile: source → quantum.circ → quantum.dynamic →
/// quantum.physical → OpenQASM 3.0. Pass order follows SPEC.md §7.1 (see
/// `mlir_bridge::passes` module docs).
fn compile_to_qasm(
    source_path: &std::path::Path,
    source: &str,
    target: &BackendTarget,
    dump_ir: bool,
    verify_linear: bool,
) -> Result<String> {
    let context = melior::Context::new();

    // 1. Parse, type-check, lower circuit funcs + run blocks to quantum.circ /
    //    monadic_staging.
    let module = frontend::lower::lower_program(&context, source).map_err(|diags| {
        print_diagnostics(source_path, source, &diags);
        anyhow!(
            "compilation failed with {} error{}",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" }
        )
    })?;
    if dump_ir {
        eprintln!("--- after lowering ---\n{}", module.as_operation());
    }

    // 2. quantum.circ passes, iterated to fixpoint.
    run_circ_passes_to_fixpoint(&context, &module);
    if verify_linear {
        verify_circ_linearity(&module)?;
    }
    if dump_ir {
        eprintln!("--- after circ passes ---\n{}", module.as_operation());
    }

    // 3. Monadic lowering: quantum.circ.run staging → quantum.dynamic.
    monadic_lowering::run_on_module(&context, &module)
        .map_err(|e| anyhow!("monadic lowering failed: {e}"))?;
    if dump_ir {
        eprintln!("--- after monadic lowering ---\n{}", module.as_operation());
    }

    // 4. quantum.dynamic passes.
    measurement_deferral::run_on_module(&context, &module);
    classical_region_fusion::run_on_module(&context, &module);
    if verify_linear {
        verify_dynamic_linearity(&module)?;
    }
    if dump_ir {
        eprintln!("--- after dynamic passes ---\n{}", module.as_operation());
    }

    // 5. quantum.physical passes, strict order (native_gate_decomp assigns
    //    native_gate=true; sabre_routing assigns phys_qubit + inserts SWAPs;
    //    depth_scheduling reads phys_qubit opportunistically). native_gate_decomp
    //    runs a second time after routing: on a target whose native set lacks
    //    `swap`, SABRE's inserted SWAP gates are themselves non-native and must
    //    be decomposed before emission. The pass is a no-op on anything already
    //    marked `native_gate = true`, so re-running it is not wasted work.
    native_gate_decomp::run_on_module(&context, target, &module);
    sabre_routing::run_on_module(&context, target, SabreCost::default(), &module);
    native_gate_decomp::run_on_module(&context, target, &module);
    depth_scheduling::run_on_module(&context, target, &module);
    if dump_ir {
        eprintln!("--- after physical passes ---\n{}", module.as_operation());
    }

    // 6. Emit OpenQASM 3.0 (reify boundary + total render).
    match openqasm3::emit(&module, target) {
        Ok(qasm) => Ok(qasm),
        Err(e) => bail!("OpenQASM emission failed: {e}"),
    }
}

/// Renders frontend diagnostics with a caret at the offending source span
/// (issue #9: "span-accurate errors") instead of a bare message list.
fn print_diagnostics(
    source_path: &std::path::Path,
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

/// Runs the `quantum.circ` optimization passes to fixpoint (SPEC §7.1 step 1):
/// gate_cancellation, rotation_merging, compiler_uncomputation,
/// zx_simplification, clifford_t_opt. A round that leaves the module's textual
/// form unchanged has reached fixpoint; capped so a pass bug that oscillates
/// cannot hang the compiler.
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

/// Runs the circ-dialect linearity verifier (`--verify-linear`) over every
/// top-level `quantum.circ.func` and fails fast on the first violation.
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

/// Runs the dynamic-dialect linearity verifier (`--verify-linear`) over the
/// whole module and fails fast on the first violation.
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
