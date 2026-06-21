//! Shared, MLIR-free core types for the Quon workspace.
//!
//! Both `frontend` and `mlir_bridge` depend on this crate, so it must never pull
//! in `melior`/LLVM. It is the single home for domain types that cross the
//! frontend↔IR seam — starting with [`DepthExpr`], the symbolic gate-depth bound
//! carried by `Circuit<n, m, d, C>` types in the frontend and by `quantum.circ`
//! op attributes downstream.

pub mod depth;

pub use depth::{DepthExpr, DepthParseError};
