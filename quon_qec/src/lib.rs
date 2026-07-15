//! Shared MLIR-free QEC layer (ADR-0015).
//!
//! Owns code-family sizing formulas and the QEC workload IR collected from
//! typed builtins after `quantum.dynamic` lowering. Neutral-atom scheduling
//! (`quon_na`) and experiment emit consume these types; this crate has no
//! Melior / MLIR dependency.

pub mod family;
pub mod workload;

pub use family::{
    CodeFamily, NetRate, QecError, SourceFamily, atoms_per_logical, ceil_div, repetition_n,
    surface_n,
};
pub use workload::{
    LogicalBasis, LogicalQubitId, QecWorkload, WorkloadBlock, WorkloadBuilder, WorkloadError,
    WorkloadOp,
};
