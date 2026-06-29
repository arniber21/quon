//! `monadic_lower` — parse a module with `quantum.circ.run` staging ops, run the
//! monadic lowering pass, and print the resulting `quantum.dynamic` MLIR.
//!
//! ```text
//!   monadic_lower < input.mlir | FileCheck input.mlir
//! ```

use std::io::{self, Read, Write};
use std::process::ExitCode;

use melior::Context;
use melior::ir::Module;

use mlir_bridge::dialect;
use mlir_bridge::passes::monadic_lowering;

fn main() -> ExitCode {
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("error: failed to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    let context = Context::new();
    dialect::register_all(&context);

    let Some(mut module) = Module::parse(&context, &source) else {
        eprintln!("error: failed to parse module");
        return ExitCode::FAILURE;
    };

    if monadic_lowering::run_on_module(&context, &module).is_err() {
        eprintln!("error: monadic lowering pass failed");
        return ExitCode::FAILURE;
    }

    let text = module.as_operation().to_string();
    if let Err(error) = io::stdout().write_all(text.as_bytes()) {
        eprintln!("error: failed to write stdout: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
