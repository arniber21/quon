//! `circ_lower` — lower a Quon source program to `quantum.circ` MLIR on stdout.
//!
//! Used as the lowering oracle for FileCheck tests (issue #16):
//!
//! ```text
//!   circ_lower < program.qn | FileCheck program.qn
//! ```

use std::io::{self, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("error: failed to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    match frontend::lower_program_to_mlir(&source) {
        Ok(text) => {
            if let Err(error) = io::stdout().write_all(text.as_bytes()) {
                eprintln!("error: failed to write stdout: {error}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for diag in diags {
                eprintln!("error: {}", diag.message);
            }
            ExitCode::FAILURE
        }
    }
}
