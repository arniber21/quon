//! Quon compiler driver library.
//!
//! Exposes the compile pipeline for integration tests, watch mode, and metrics.
//! Pass orchestration lives in `mlir_bridge::pipeline` / `quon_na::pipeline`;
//! this crate is the thin CLI/test adapter.

pub mod compile;
pub mod na_target;
pub mod watch;

pub use compile::{
    CompileReport, CompileRequest, build_na_schedule_view, compile, print_diagnostics,
    schedule_to_json, schedule_to_mlir,
};
pub use na_target::{NaBackendKind, parse_na_backend, parse_placer_mode};
