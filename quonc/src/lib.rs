//! Quon compiler driver library.
//!
//! Exposes the compile pipeline for integration tests, watch mode, and metrics.

pub mod compile;
pub mod na_target;
pub mod watch;

pub use compile::{CompileReport, CompileRequest, compile, print_diagnostics, schedule_to_json};
pub use na_target::{NaBackendKind, parse_na_backend, parse_placer_mode};
