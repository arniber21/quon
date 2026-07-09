//! Quon compiler driver library.
//!
//! Exposes the compile pipeline for integration tests, watch mode, and metrics.

pub mod compile;
pub mod watch;

pub use compile::{CompileReport, CompileRequest, compile, print_diagnostics};
