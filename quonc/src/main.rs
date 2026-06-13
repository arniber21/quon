use anyhow::Result;
use clap::Parser;

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
    let _cli = Cli::parse();
    todo!("compiler pipeline not yet implemented — see issues #4–#27")
}
