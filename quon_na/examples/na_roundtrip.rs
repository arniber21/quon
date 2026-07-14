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

    // The module's textual form already ends with a newline; only append one
    // when it does not, so dump → parse → dump is byte-stable.
    let text = module.as_operation().to_string();
    let result = if text.ends_with('\n') {
        write!(io::stdout(), "{text}")
    } else {
        writeln!(io::stdout(), "{text}")
    };
    if let Err(error) = result {
        eprintln!("write stdout: {error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
