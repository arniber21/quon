//! MLIR bridge — Melior wrappers, dialect registration, passes, and emitter.

pub mod circ_extract;
pub mod diagnostics;
pub mod dialect;
pub mod dynamic_walk;
pub mod emit;
pub mod fixed_physical;
pub mod metrics;
pub mod passes;
pub mod pipeline;
pub mod qec_collect;

pub use qec_collect::{CollectError, collect_qec_workload};
