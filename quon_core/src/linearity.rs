//! Linear qubit SSA invariants (SPEC §6.2–§6.3).
//!
//! This module is the **IR adapter** of the no-cloning / linear-use judgment —
//! the rule that *every qubit resource is consumed exactly once*. It hosts the
//! pure, MLIR-free SSA use-count kernels shared by `mlir_bridge` verifiers /
//! passes and Flux refinement proofs. After lowering, source names are gone: a
//! `!qubit` SSA value must have exactly one use, and a value consumed by
//! `quantum.dynamic.measure` must not appear at any other use site.
//!
//! # Δ ⇄ SSA: two adapters of one judgment
//!
//! The compiler enforces the no-cloning judgment at two distinct stages, with
//! two adapters that share vocabulary but **not** a type:
//!
//! * **Frontend adapter — `Δ` (linear context).** `frontend`'s typing context
//!   `Δ : HashMap<Name, Type>` records named qubit resources and physically
//!   removes a name from `Δ` when it is consumed in the term; a second use is
//!   a *scope* error caught statically at the source language. Lives in the
//!   `frontend` crate — see the `Linear context` glossary entry in CONTEXT.md.
//! * **IR adapter — this module.** Names are gone post-lowering; the judgment
//!   is re-expressed as "every `!qubit` SSA value has exactly
//!   [`LINEAR_USE_COUNT`] use" and checked by region verifier passes against
//!   these kernels.
//!
//! The two adapters deliberately stay as separate types in their owning
//! crates. `Δ` operates on source names before lowering; the SSA use-count
//! kernels operate on post-lowering values. Neither subsumes the other, and
//! there is no plan to unify them into a shared data type — only the
//! *vocabulary* (linear use, no-cloning, exactly-once consumption) is shared.

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
