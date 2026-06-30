//! Flux refinement-type checks for the Quon workspace.
//!
//! Run with `cargo flux -p flux_verify` (requires `cargo-flux` on PATH).
//! This crate uses a nightly toolchain; the rest of the workspace stays on stable.
//!
//! Dialect and pass invariants for `quantum.dynamic` are modeled in
//! [`quon_core::linearity`] and verified with `cargo flux -p quon_core --features flux`.

use flux_rs::attrs::*;

/// Returns a value strictly greater than the input.
#[spec(fn(x: i32) -> i32{v: x < v})]
pub fn inc(x: i32) -> i32 {
    x + 1
}

/// Natural numbers are non-negative.
#[spec(fn(n: usize) -> usize{v: 0 <= v})]
pub fn id_usize(n: usize) -> usize {
    n
}

/// Stable smoke test: `mlir_bridge` and Flux share the same linearity kernels.
#[cfg(test)]
mod smoke {
    use quon_core::linearity::{
        UseCountViolation, classify_use_count, is_linear_use_count, is_reuse_after_measure,
    };

    #[test]
    fn quon_core_linearity_kernels_match_issue6() {
        assert!(is_linear_use_count(1));
        assert!(is_reuse_after_measure(2, true, true));
        assert_eq!(
            classify_use_count(2, true, true),
            Some(UseCountViolation::ReuseAfterMeasure)
        );
    }

    /// The optimization-pass invariants (#18–#21) carry Flux postconditions
    /// verified by `cargo flux -p quon_core --features flux`; these calls pin the
    /// same kernels the `mlir_bridge` passes use so the proofs stay load-bearing.
    #[test]
    fn quon_core_optimization_kernels_are_safe() {
        use quon_core::optimization::{
            arity_preserved, depth_after_removal, par_depth, seq_depth, single_qubit_pair,
        };
        // Depth is non-increasing under gate removal / rotation merging.
        assert!(depth_after_removal(5, 2) <= 5);
        assert_eq!(depth_after_removal(2, 5), 0);
        // Composition bounds.
        assert!(seq_depth(3, 4) >= 4);
        assert_eq!(par_depth(3, 7), 7);
        // Arity preservation guards.
        assert!(arity_preserved(2, 2));
        assert!(single_qubit_pair(1, 1));
    }
}
