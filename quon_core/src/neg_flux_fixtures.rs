//! Negative compile fixtures for the scalar Flux predicates (issue #414).
//!
//! Each function below *intentionally violates* a Flux-refined invariant
//! kernel from this crate. Marked `#[should_fail]`, they prove the predicates
//! are **load-bearing**: Flux rejects the violation at compile time, and the
//! `#[should_fail]` marker records that rejection as the expected outcome (a
//! function that *passes* while marked `#[should_fail]` would itself be an
//! error). So a future edit that weakened a predicate enough to accept the
//! violating body would fail this check.
//!
//! One fixture per invariant family — bounds, arity, linearity, and ordering
//! — the four scalar predicate families made load-bearing by #414:
//!
//! | Fixture | Kernel violated | Load-bearing caller |
//! |--------|-----------------|---------------------|
//! | [`neg_bounds_index_out_of_range`] | [`index_in_bounds`](crate::index_in_bounds) | `QubitId`/`BitId` construction |
//! | [`neg_arity_mismatch`] | [`operand_arity_ok`](crate::operand_arity_ok) | `from_gate_info` |
//! | [`neg_linearity_non_linear_use`] | [`is_linear_use_count`](crate::is_linear_use_count) | `linearity_verifier` / `dynamic_linearity_verifier` |
//! | [`neg_ordering_depth_increases`] | [`depth_after_removal`](crate::depth_after_removal) | gate-cancellation / rotation-merging passes |
//!
//! The schedule ordering family (`cycle_is_monotonic` / `wait_barrier_ok` in
//! `quon_na`) is load-bearing at the `verify_schedule_ordering` call site; its
//! scalar predicate is isomorphic to the depth-monotonicity fixture below
//! (a non-decreasing / non-increasing comparison), and is pinned by the
//! `quon_na` smoke test in `flux_verify`. The `quon_na` crate itself cannot
//! host a `#[should_fail]` fixture today because `flux-infer` ICEs on its
//! iterator-closure-heavy bodies (ADR-0027) before reaching the schedule
//! invariants module.
//!
//! Verified by the gated `cargo flux -p quon_core --features flux` CI step.

use flux_rs::attrs::*;

/// Bounds violation: claims an index is in range but returns it *equal* to the
/// bound (not strictly less). `index_in_bounds` refines the result to
/// `idx < bound`; this body returns `idx == bound`, so Flux rejects the
/// postcondition.
#[should_fail]
#[spec(fn(idx: usize, bound: usize) -> bool[idx < bound])]
pub fn neg_bounds_index_out_of_range(idx: usize, bound: usize) -> bool {
    idx == bound
}

/// Arity violation: claims operand counts match but returns a mismatch.
/// `operand_arity_ok` refines the result to `expected == actual`; this body
/// returns `expected != actual`, so Flux rejects the postcondition.
#[should_fail]
#[spec(fn(expected: usize, actual: usize) -> bool[expected == actual])]
pub fn neg_arity_mismatch(expected: usize, actual: usize) -> bool {
    expected != actual
}

/// Linearity violation: claims a use count is linear (exactly 1) but returns a
/// count of 2. `is_linear_use_count` refines the result to `count == 1`; this
/// body returns `count == 2`, so Flux rejects the postcondition.
#[should_fail]
#[spec(fn(count: usize) -> bool[count == 1])]
pub fn neg_linearity_non_linear_use(count: usize) -> bool {
    count == 2
}

/// Ordering violation: claims depth is non-increasing under removal but
/// returns a depth that grows. `depth_after_removal` refines the result to
/// `v <= current`; this body returns `current + 1` (strictly greater), so Flux
/// rejects the postcondition. This is the monotonicity/ordering invariant the
/// gate-cancellation and rotation-merging passes rely on (the schedule analogue
/// is `cycle_is_monotonic` in `quon_na`).
#[should_fail]
#[spec(fn(current: u64, removed: u64) -> u64{v: v <= current})]
pub fn neg_ordering_depth_increases(current: u64, removed: u64) -> u64 {
    let _ = removed;
    current + 1
}
