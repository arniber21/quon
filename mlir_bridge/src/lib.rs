//! MLIR bridge — Melior wrappers, dialect registration, passes, and emitter.
//!
//! Stub implementations in this crate are expanded in issues #4–#27.

#![allow(dead_code)]

pub mod circ_extract;
pub mod diagnostics;
pub mod dialect;
pub mod dynamic_walk;
pub mod emit;
pub mod metrics;
pub mod passes;
pub mod pipeline;
pub mod qec_collect;

pub use qec_collect::{CollectError, collect_qec_workload};
