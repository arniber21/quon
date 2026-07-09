//! `na_roundtrip` — parse a `quantum.na` MLIR module from stdin and reprint it.
//!
//! Used by the lit/FileCheck textual round-trip test for issue #102.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use melior::Context;
use melior::ir::Module;

fn main() -> ExitCode {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("read stdin: {error}");
        return ExitCode::from(1);
    }

    let context = Context::new();
    quon_na::dialect::register_dialect(&context);

    let Some(module) = Module::parse(&context, &input) else {
        eprintln!("failed to parse MLIR module");
        return ExitCode::from(2);
    };

    if let Err(error) = writeln!(io::stdout(), "{}", module.as_operation()) {
        eprintln!("write stdout: {error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
