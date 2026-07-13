//! Individual MLIR pass implementations.
//!
//! Pass *order* and Fixed vs NA fork live in [`crate::pipeline`] (and
//! `quon_na::pipeline` for the neutral-atom schedule path), not only as comments
//! here. Summary of the Fixed / shared stages:
//!
//! ```text
//! quantum.circ (fixpoint):
//!   1. gate_cancellation
//!   2. rotation_merging
//!   3. compiler_uncomputation
//!   4. zx_simplification
//!   5. clifford_t_opt — RESERVED for #96 (not implemented; do not alias)
//!
//! monadic_lowering → quantum.dynamic:
//!   6. measurement_deferral
//!   7. classical_region_fusion
//!
//! Fixed physical (strict, implemented order):
//!   8. native_gate_decomp
//!   9. sabre_routing
//!  10. native_gate_decomp (post-SWAP)
//!  11. depth_scheduling
//! ```
//!
//! See SPEC.md §7.1 for the normative pipeline description; physical steps 8–10
//! in the SPEC text differ slightly from the implemented pre-route decomp +
//! post-SWAP decomp sequence above — callers must use [`crate::pipeline`].

pub mod classical_region_fusion;
pub mod compiler_uncomputation;
pub mod depth_scheduling;
pub mod dynamic_linearity_verifier;
pub mod gate_cancellation;
pub mod linearity_verifier;
pub mod measurement_deferral;
pub mod monadic_lowering;
pub mod native_gate_decomp;
pub(crate) mod qubit_wiring;
pub mod rotation_merging;
pub mod sabre_routing;
pub mod zx_simplification;
