//! `circ_roundtrip` — parse a `quantum.circ` MLIR module from stdin and reprint
//! it to stdout.
//!
//! This is the round-trip oracle for FileCheck tests (issue #4 acceptance, wired
//! into the lit harness in issue #28):
//!
//! ```text
//!   circ_roundtrip < module.mlir | FileCheck module.mlir
//! ```
//!
//! Because `quantum.circ` is an unregistered dialect (see
//! [`mlir_bridge::dialect::quantum_circ`]), the context must allow unregistered
//! dialects before parsing — that is exactly what `register_dialect` does.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use melior::Context;
use melior::ir::Module;

use mlir_bridge::dialect::quantum_circ;

fn main() -> ExitCode {
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("error: failed to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    let context = Context::new();
    quantum_circ::register_dialect(&context);

    let Some(module) = Module::parse(&context, &source) else {
        eprintln!("error: failed to parse quantum.circ module");
        return ExitCode::FAILURE;
    };

    let text = module.as_operation().to_string();
    if let Err(error) = io::stdout().write_all(text.as_bytes()) {
        eprintln!("error: failed to write stdout: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
