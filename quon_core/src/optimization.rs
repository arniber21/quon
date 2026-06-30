//! Refinement-checked invariants for peephole optimization passes (SPEC §7.2).
//!
//! Pure, MLIR-free kernels shared by the `mlir_bridge` optimization passes —
//! gate cancellation (#18), rotation merging (#19), ZX simplification (#20), and
//! Clifford+T (#21) — and verified with `cargo flux -p quon_core --features flux`.
//!
//! However a peephole fires, a `quantum.circ.func` rewrite must respect two
//! safety invariants, which these kernels make machine-checked rather than
//! merely tested:
//!
//! 1. **Depth is non-increasing.** Removing or merging gates can only shrink the
//!    static depth bound carried in the `depth` attribute, never grow it.
//! 2. **Qubit arity is preserved.** A rewrite threads exactly as many wires out
//!    as in, so the func's `in_qubits` / `out_qubits` stay consistent and the
//!    dialect verifier still accepts the body.

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// The static depth of a func body after a peephole removes `removed` units of
/// depth. Saturates at zero and is refinement-guaranteed to never exceed
/// `current` — the proof that an optimization pass cannot *deepen* a circuit.
///
/// Used by gate cancellation and rotation merging when they fold the depth a
/// cancelled / merged pair contributed back out of the func's `depth` attribute.
#[cfg_attr(
    feature = "flux",
    spec(fn(current: u64, removed: u64) -> u64{v: v <= current})
)]
// Written as an explicit guard rather than `saturating_sub` so Flux can
// discharge the `{v: v <= current}` postcondition — Flux does not model the
// `saturating_sub` intrinsic, and the saturating form fails refinement
// checking. Both forms have identical runtime behavior; the lint is silenced
// only here because the proof obligation takes precedence.
#[allow(clippy::implicit_saturating_sub)]
pub fn depth_after_removal(current: u64, removed: u64) -> u64 {
    if removed >= current {
        0
    } else {
        current - removed
    }
}

/// Sequential composition depth (`a |> b`): depths add. The result bounds each
/// operand from below — composing circuits never shrinks depth. Depths are gate
/// counts, far below `u64::MAX`, so the addition cannot realistically overflow.
#[cfg_attr(
    feature = "flux",
    spec(fn(a: u64, b: u64) -> u64{v: a <= v && b <= v})
)]
pub fn seq_depth(a: u64, b: u64) -> u64 {
    a + b
}

/// Parallel composition depth (`a ∥ b`): the maximum. An upper bound on both
/// operands that is itself equal to one of them.
#[cfg_attr(
    feature = "flux",
    spec(fn(a: u64, b: u64) -> u64{v: a <= v && b <= v && (v == a || v == b)})
)]
pub fn par_depth(a: u64, b: u64) -> u64 {
    if a >= b { a } else { b }
}

/// True iff a rewrite preserves a func's qubit arity (`in == out`) — the
/// precondition the `quantum.circ.func` verifier re-checks after a peephole has
/// rewired the body.
#[cfg_attr(
    feature = "flux",
    spec(fn(before: usize, after: usize) -> bool[before == after])
)]
pub fn arity_preserved(before: usize, after: usize) -> bool {
    before == after
}

/// True iff two ops are a fusible single-qubit pair: both act on exactly one
/// wire, so rotation merging (#19) can replace them with one single-qubit gate
/// without changing the func's arity.
#[cfg_attr(
    feature = "flux",
    spec(fn(lhs_arity: usize, rhs_arity: usize) -> bool[lhs_arity == 1 && rhs_arity == 1])
)]
pub fn single_qubit_pair(lhs_arity: usize, rhs_arity: usize) -> bool {
    lhs_arity == 1 && rhs_arity == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_never_increases_depth() {
        assert_eq!(depth_after_removal(5, 2), 3);
        assert_eq!(depth_after_removal(2, 5), 0); // saturates
        for current in 0..16u64 {
            for removed in 0..16u64 {
                assert!(depth_after_removal(current, removed) <= current);
            }
        }
    }

    #[test]
    fn seq_depth_bounds_each_operand() {
        for a in 0..16u64 {
            for b in 0..16u64 {
                let v = seq_depth(a, b);
                assert!(a <= v && b <= v);
                assert_eq!(v, a + b);
            }
        }
    }

    #[test]
    fn par_depth_is_the_max() {
        assert_eq!(par_depth(3, 7), 7);
        assert_eq!(par_depth(7, 3), 7);
        for a in 0..16u64 {
            for b in 0..16u64 {
                let v = par_depth(a, b);
                assert!(a <= v && b <= v && (v == a || v == b));
            }
        }
    }

    #[test]
    fn arity_and_pairing() {
        assert!(arity_preserved(2, 2));
        assert!(!arity_preserved(2, 1));
        assert!(single_qubit_pair(1, 1));
        assert!(!single_qubit_pair(1, 2));
    }
}
