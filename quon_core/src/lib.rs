//! Shared, MLIR-free core types for the Quon workspace.
//!
//! Both `frontend` and `mlir_bridge` depend on this crate, so it must never pull
//! in `melior`/LLVM. It is the single home for domain types that cross the
//! frontend↔IR seam — starting with [`DepthExpr`], the symbolic gate-depth bound
//! carried by `Circuit<n, m, d, C>` types in the frontend and by `quantum.circ`
//! op attributes downstream.

pub mod depth;
pub mod linearity;
pub mod optimization;
pub mod qasm;

pub use depth::{DepthExpr, DepthParseError};
pub use linearity::{
    LINEAR_USE_COUNT, UseCountViolation, barrier_identity_ok, classify_use_count,
    if_qubit_threading_ok, is_linear_use_count, is_reuse_after_measure, unitary_region_boundary_ok,
};
pub use optimization::{
    arity_preserved, depth_after_removal, par_depth, seq_depth, single_qubit_pair,
};
pub use qasm::{
    BitId, Expr, GateDef, OneQubitGate, Program, QasmGate, QubitId, Register, RotationGate, Stmt,
    TwoQubitGate, index_in_bounds, operand_arity_ok, render,
};
