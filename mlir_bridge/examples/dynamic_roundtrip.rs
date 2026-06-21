//! `dynamic_roundtrip` — parse a `quantum.dynamic` MLIR module from stdin and
//! reprint it to stdout.
//!
//! Used as the round-trip oracle for FileCheck tests (issue #6):
//!
//! ```text
//!   dynamic_roundtrip < module.mlir | FileCheck module.mlir
//! ```

use std::io::{self, Read, Write};
use std::process::ExitCode;

use melior::Context;
use melior::ir::Module;

use mlir_bridge::dialect;

fn main() -> ExitCode {
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("error: failed to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    let context = Context::new();
    dialect::register_all(&context);

    let Some(module) = Module::parse(&context, &source) else {
        eprintln!("error: failed to parse quantum.dynamic module");
        return ExitCode::FAILURE;
    };

    let text = module.as_operation().to_string();
    if let Err(error) = io::stdout().write_all(text.as_bytes()) {
        eprintln!("error: failed to write stdout: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
