//! Linear qubit SSA invariants (SPEC §6.2–§6.3).
//!
//! Pure, MLIR-free kernels shared by `mlir_bridge` verifiers/passes and Flux
//! refinement proofs. Every `!qubit` SSA value must have exactly one use; a value
//! consumed by `quantum.dynamic.measure` must not appear at any other use site.

use std::fmt;

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// The unique legal use count for a linear qubit SSA value.
pub const LINEAR_USE_COUNT: usize = 1;

/// How a qubit use-count check failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseCountViolation {
    /// Zero uses — an implicit drop.
    Unused,
    /// Two or more uses without a measure-and-reuse pattern.
    MultiUse {
        /// Observed use count.
        count: usize,
    },
    /// Measured and also used by another op (issue #6).
    ReuseAfterMeasure,
}

impl fmt::Display for UseCountViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unused => write!(f, "unused qubit (0 uses)"),
            Self::MultiUse { count } => write!(f, "{count} uses"),
            Self::ReuseAfterMeasure => write!(f, "reuse after measure"),
        }
    }
}

/// True iff `count` is exactly [`LINEAR_USE_COUNT`].
#[cfg_attr(
    feature = "flux",
    spec(fn(count: usize) -> bool[count == 1])
)]
pub fn is_linear_use_count(count: usize) -> bool {
    count == LINEAR_USE_COUNT
}

/// True iff a measure op shares a qubit SSA value with another consumer.
#[cfg_attr(
    feature = "flux",
    spec(fn(count: usize, has_measure: bool, has_other: bool) -> bool[
        count > 1 && has_measure && has_other
    ])
)]
pub fn is_reuse_after_measure(count: usize, has_measure: bool, has_other: bool) -> bool {
    count > LINEAR_USE_COUNT && has_measure && has_other
}

/// Classify a qubit use count. Returns `None` when linear.
pub fn classify_use_count(
    count: usize,
    has_measure: bool,
    has_other: bool,
) -> Option<UseCountViolation> {
    if is_linear_use_count(count) {
        None
    } else if count == 0 {
        Some(UseCountViolation::Unused)
    } else if is_reuse_after_measure(count, has_measure, has_other) {
        Some(UseCountViolation::ReuseAfterMeasure)
    } else {
        Some(UseCountViolation::MultiUse { count })
    }
}

/// `quantum.dynamic.barrier` identity threading: ≥1 operands, equal results.
#[cfg_attr(
    feature = "flux",
    spec(fn(operands: usize, results: usize) -> bool[operands >= 1 && operands == results])
)]
pub fn barrier_identity_ok(operands: usize, results: usize) -> bool {
    operands >= 1 && operands == results
}

/// `quantum.dynamic.if` qubit threading: qubit operand count equals results.
#[cfg_attr(
    feature = "flux",
    spec(fn(qubit_operands: usize, qubit_results: usize) -> bool[qubit_operands == qubit_results])
)]
pub fn if_qubit_threading_ok(qubit_operands: usize, qubit_results: usize) -> bool {
    qubit_operands == qubit_results
}

/// `quantum.dynamic.unitary_region` boundary counts all agree.
#[cfg_attr(
    feature = "flux",
    spec(fn(outer: usize, inner_args: usize, return_operands: usize, outer_results: usize) -> bool[
        outer == inner_args && outer == return_operands && outer == outer_results
    ])
)]
pub fn unitary_region_boundary_ok(
    outer_operands: usize,
    inner_args: usize,
    return_operands: usize,
    outer_results: usize,
) -> bool {
    outer_operands == inner_args
        && outer_operands == return_operands
        && outer_operands == outer_results
}
