//! Test-only unitary-equivalence harness for `quantum.circ` passes (issue #118).
//!
//! Generate a [`CircuitSpec`], lower to MLIR, run a pass, SSA-extract gates, and
//! compare phase-invariant statevectors. Not for production use.

#![allow(dead_code, unused_imports)]

pub mod circuit_spec;
pub mod extract;
pub mod lower;
pub mod sabre_layout;
pub mod statevector;

pub use circuit_spec::{CircuitSpec, GateInst, GateKind};
pub use extract::{extract_borrow_gates, extract_func_circuit};
pub use lower::{
    context, lower_borrow_module, lower_func_module, prop_config, read_func_op, seed_adversarial,
};
pub use sabre_layout::{
    identity_perm, permute_statevector, return_permutation, slot_of_logical_after_swaps,
};
pub use statevector::{
    TOL, apply_circuit, basis_state, circuits_equiv, phase_invariant_distance, statevectors_equiv,
};
