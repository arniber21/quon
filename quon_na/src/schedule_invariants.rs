//! Flux refinement-typed schedule invariants (issue #115).
//!
//! Pure, MLIR-free scalar kernels for the `quantum.na` schedule invariants that
//! are expressible as Flux refinement types: cycle monotonicity and Wait barrier
//! ordering. These are the only `quantum.na` verifier invariants that map to
//! pure integer comparisons — the rest require `f64` geometry or collection
//! operations that Flux cannot reason about (see ADR-0026).
//!
//! Verified with `cargo flux -p quon_na --no-default-features --features flux`.
//! The runtime verifier in [`crate::dialect`] remains authoritative; these
//! kernels are compile-time proofs that document and pin the scalar invariants.

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// True iff `curr` is non-decreasing relative to `prev` — the per-layer cycle
/// monotonicity invariant from `verify_schedule_ordering` in `dialect.rs`.
///
/// The schedule's `cycle` attributes must form a non-decreasing sequence in
/// layer order. Flux proves `prev <= curr` from the function body.
#[cfg_attr(
    feature = "flux",
    spec(fn(prev: u32, curr: u32) -> bool[prev <= curr])
)]
pub fn cycle_is_monotonic(prev: u32, curr: u32) -> bool {
    prev <= curr
}

/// True iff `after_cycle` is strictly later than `wait_cycle` — the Wait
/// hard-barrier invariant: any layer after a `quantum.na.wait` must advance the
/// cycle.
///
/// Flux proves `wait_cycle < after_cycle` from the function body.
#[cfg_attr(
    feature = "flux",
    spec(fn(wait_cycle: u32, after_cycle: u32) -> bool[wait_cycle < after_cycle])
)]
pub fn wait_barrier_ok(wait_cycle: u32, after_cycle: u32) -> bool {
    wait_cycle < after_cycle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_accepts_equal_and_increasing() {
        assert!(cycle_is_monotonic(0, 0));
        assert!(cycle_is_monotonic(0, 1));
        assert!(cycle_is_monotonic(5, 10));
    }

    #[test]
    fn monotonic_rejects_decreasing() {
        assert!(!cycle_is_monotonic(1, 0));
        assert!(!cycle_is_monotonic(10, 5));
    }

    #[test]
    fn wait_barrier_accepts_strictly_later() {
        assert!(wait_barrier_ok(0, 1));
        assert!(wait_barrier_ok(5, 6));
        assert!(wait_barrier_ok(5, 100));
    }

    #[test]
    fn wait_barrier_rejects_equal_or_earlier() {
        assert!(!wait_barrier_ok(5, 5));
        assert!(!wait_barrier_ok(5, 4));
        assert!(!wait_barrier_ok(5, 0));
    }
}
