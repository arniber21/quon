//! `sabre_route` — SABRE routing on `quantum.circ` IR.

use std::env;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use melior::Context;
use melior::ir::Module;

use mlir_bridge::dialect;
use mlir_bridge::passes::sabre_routing::{self, SabreCost};

fn main() -> ExitCode {
    let target_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "backend/tests/fixtures/device_5q.json".to_string());

    let target = match std::fs::read_to_string(&target_path)
        .map_err(backend::BackendError::Io)
        .and_then(|text| backend::json::from_str(&text))
    {
        Ok(target) => target,
        Err(error) => {
            eprintln!("error: failed to load target `{target_path}`: {error}");
            return ExitCode::FAILURE;
        }
    };

    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("error: failed to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    let context = Context::new();
    dialect::register_all(&context);

    let Some(module) = Module::parse(&context, &source) else {
        eprintln!("error: failed to parse module");
        return ExitCode::FAILURE;
    };

    sabre_routing::run_on_module(&context, &target, SabreCost::default(), &module);

    let text = module.as_operation().to_string();
    if let Err(error) = io::stdout().write_all(text.as_bytes()) {
        eprintln!("error: failed to write stdout: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
