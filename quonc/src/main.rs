use anyhow::{Context as _, Result, anyhow, bail};
use clap::Parser;

use backend::{BackendTarget, generic_openqasm};
use mlir_bridge::emit::openqasm3;
use mlir_bridge::passes::monadic_lowering;

#[derive(Parser)]
#[command(name = "quonc", about = "Quon quantum compiler")]
struct Cli {
    /// Source file to compile (.qn)
    source: std::path::PathBuf,

    /// Emit OpenQASM 3.0 to stdout
    #[arg(long)]
    emit_qasm: bool,

    /// Backend target descriptor (JSON). Defaults to generic_openqasm.
    #[arg(long)]
    target: Option<std::path::PathBuf>,

    /// Dump MLIR after each pass (debug)
    #[arg(long)]
    dump_ir: bool,

    /// Run the linearity verifier pass (debug)
    #[arg(long)]
    verify_linear: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let source = std::fs::read_to_string(&cli.source)
        .with_context(|| format!("reading {}", cli.source.display()))?;

    // The emitter reads only the target's native gate set and id, not its
    // topology, so the qubit width here is immaterial; routing/decomposition
    // (#24/#25/#26) are not yet wired and `generic_openqasm` is all-to-all.
    let target = match &cli.target {
        Some(path) => backend::json::load(path)
            .map_err(|e| anyhow!("loading target {}: {e}", path.display()))?,
        None => generic_openqasm::target(64),
    };

    let qasm = compile_to_qasm(&source, &target, cli.dump_ir)?;

    if cli.emit_qasm {
        print!("{qasm}");
    } else {
        // Without --emit-qasm there is no other output mode yet; emitting is the
        // only terminal stage, so make that explicit rather than silently exiting.
        eprintln!("(compiled successfully; pass --emit-qasm to print OpenQASM 3.0)");
    }
    Ok(())
}

/// Front-to-back compile: source → quantum.circ → quantum.dynamic → OpenQASM 3.0.
fn compile_to_qasm(source: &str, target: &BackendTarget, dump_ir: bool) -> Result<String> {
    let context = melior::Context::new();

    // 1. Parse, type-check, lower circuit funcs + run blocks to quantum.circ /
    //    monadic_staging.
    let module = frontend::lower::lower_program(&context, source).map_err(|diags| {
        let rendered = diags
            .iter()
            .map(|d| format!("  - {}", d.message))
            .collect::<Vec<_>>()
            .join("\n");
        anyhow!("compilation failed:\n{rendered}")
    })?;
    if dump_ir {
        eprintln!("--- after lowering ---\n{}", module.as_operation());
    }

    // 2. Monadic lowering: quantum.circ.run staging → quantum.dynamic.
    monadic_lowering::run_on_module(&context, &module)
        .map_err(|e| anyhow!("monadic lowering failed: {e}"))?;
    if dump_ir {
        eprintln!("--- after monadic lowering ---\n{}", module.as_operation());
    }

    // 3. Emit OpenQASM 3.0 (reify boundary + total render).
    match openqasm3::emit(&module, target) {
        Ok(qasm) => Ok(qasm),
        Err(e) => bail!("OpenQASM emission failed: {e}"),
    }
}
