//! Flux refinement-type checks for the Quon workspace.
//!
//! Run with `cargo flux -p flux_verify` (requires `cargo-flux` on PATH).
//! This crate uses a nightly toolchain; the rest of the workspace stays on stable.
//!
//! Dialect and pass invariants for `quantum.dynamic` are modeled in
//! [`quon_core::linearity`] and verified with `cargo flux -p quon_core --features flux`.
//!
//! The negative compile fixtures for the scalar Flux predicates (bounds, arity,
//! linearity, ordering) live in [`quon_core`] under its `flux` feature, where
//! the gated `cargo flux -p quon_core --features flux` CI step verifies that
//! Flux rejects each violation (see `quon_core::neg_flux_fixtures`).

use flux_rs::attrs::*;

/// Returns a value strictly greater than the input (issue #411).
///
/// Flux proves `x < x + 1` under the machine-maximum precondition `x < i32::MAX`:
/// without it, `x + 1` could overflow and the postcondition would be
/// unprovable. The precondition is load-bearing — a direct call at
/// `i32::MAX` violates the documented contract rather than wrapping.
#[spec(fn(x: i32{v: v < 2147483647}) -> i32{v: x < v})]
pub fn inc(x: i32) -> i32 {
    x + 1
}

/// Negative compile fixture (issue #411): `inc` at the machine maximum must
/// be rejected. The `#[should_fail]` attribute encodes that expectation —
/// flux *must* reject this call (overflow violates the `x < v`
/// postcondition). If someone weakens `inc`'s precondition to allow
/// `i32::MAX`, this function would verify and flux would error on the
/// `should_fail` marker, surfacing the regression.
#[allow(dead_code)]
#[should_fail]
#[sig(fn() -> i32[false])]
fn inc_at_machine_max_is_rejected() -> i32 {
    inc(2147483647)
}

/// Negative compile fixture (issue #411): `seq_depth`'s exact-sum
/// postcondition `v == a + b` is load-bearing. The `#[should_fail]`
/// attribute encodes that flux *must* reject the off-by-one claim. If
/// someone weakens the spec to `v >= a + b` (dropping the equality), this
/// function would verify and flux would error on `should_fail`.
#[allow(dead_code)]
#[should_fail]
#[sig(fn(a: u64, b: u64) -> u64{v: v == a + b + 1})]
fn seq_depth_exact_sum_is_load_bearing(a: u64, b: u64) -> u64 {
    quon_core::seq_depth(a, b)
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
    fn inc_respects_machine_max_precondition() {
        // The Flux precondition x < i32::MAX ensures x + 1 doesn't overflow.
        assert_eq!(inc(0), 1);
        assert_eq!(inc(41), 42);
        assert_eq!(inc(i32::MAX - 1), i32::MAX);
        // At the boundary: inc(i32::MAX) would overflow — the precondition
        // excludes this call. In debug Rust it panics (wraps in release);
        // the Flux spec makes it a contract violation at verified call sites.
    }


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
    /// verified by `cargo flux -p quon_core --features flux`. These kernels are
    /// load-bearing (#414): `depth_after_removal` and `seq_depth` feed the
    /// gate-cancellation / rotation-merging passes' depth folding, and
    /// `arity_preserved` / `single_qubit_pair` guard the rotation-merge rewrite.
    /// (`par_depth` is the one optimization kernel with no production caller, so
    /// it intentionally carries no Flux spec.) These calls pin the same kernels
    /// the `mlir_bridge` passes consume.
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

    /// The OpenQASM emitter (#27) is valid by construction: index-bounds and
    /// gate-arity are Flux-checked at the reify boundary (`index_in_bounds`,
    /// `operand_arity_ok`), so the renderer is total. Pin those kernels and the
    /// bound-enforcing `Program` ID minting here.
    #[test]
    fn quon_core_qasm_kernels_enforce_bounds_and_arity() {
        use quon_core::{Program, index_in_bounds, operand_arity_ok};
        assert!(index_in_bounds(1, 2));
        assert!(!index_in_bounds(2, 2));
        assert!(operand_arity_ok(2, 2));
        assert!(!operand_arity_ok(2, 1));
        // Constructors refuse out-of-range indices, so every id that exists is in range.
        let program = Program::new(2, 2);
        assert!(program.qubit(1).is_some());
        assert!(program.qubit(2).is_none());
        assert!(program.bit(2).is_none());
    }

    /// The `quantum.na` schedule scalar invariants (#115) — cycle monotonicity
    /// and Wait barrier ordering — carry Flux refinement specs verified by
    /// `cargo flux -p quon_na --no-default-features --features flux`. These
    /// kernels are load-bearing (#414): `verify_schedule_ordering` in
    /// `quon_na::dialect` routes its cycle-monotonicity and wait-barrier
    /// checks through `cycle_is_monotonic` / `wait_barrier_ok` rather than
    /// re-implementing the comparisons inline. These calls pin the same
    /// kernels the dialect verifier now consumes.
    #[test]
    fn quon_na_schedule_scalar_invariants_hold() {
        use quon_na::{cycle_is_monotonic, wait_barrier_ok};
        // Cycle monotonicity: non-decreasing cycles.
        assert!(cycle_is_monotonic(0, 0));
        assert!(cycle_is_monotonic(0, 1));
        assert!(!cycle_is_monotonic(1, 0));
        // Wait barrier: strictly later cycle after a Wait.
        assert!(wait_barrier_ok(0, 1));
        assert!(!wait_barrier_ok(5, 5));
        assert!(!wait_barrier_ok(5, 4));
    }
}
