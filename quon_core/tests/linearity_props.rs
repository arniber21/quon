//! Property-based tests for linear qubit SSA invariant kernels.

use proptest::prelude::*;
use quon_core::linearity::{
    UseCountViolation, barrier_identity_ok, classify_use_count, if_qubit_threading_ok,
    is_linear_use_count, is_reuse_after_measure, unitary_region_boundary_ok,
};

proptest! {
    #[test]
    fn linear_use_count_iff_one(count in 0usize..8) {
        prop_assert_eq!(is_linear_use_count(count), count == 1);
    }

    #[test]
    fn classify_none_iff_linear(count in 0usize..8, has_measure: bool, has_other: bool) {
        let violation = classify_use_count(count, has_measure, has_other);
        prop_assert_eq!(violation.is_none(), count == 1);
    }

    #[test]
    fn reuse_after_measure_characterization(
        count in 2usize..8,
        has_measure: bool,
        has_other: bool,
    ) {
        prop_assert_eq!(
            is_reuse_after_measure(count, has_measure, has_other),
            has_measure && has_other
        );
        if has_measure && has_other {
            prop_assert_eq!(
                classify_use_count(count, has_measure, has_other),
                Some(UseCountViolation::ReuseAfterMeasure)
            );
        }
    }

    #[test]
    fn barrier_identity_matches_definition(operands in 0usize..8, results in 0usize..8) {
        prop_assert_eq!(
            barrier_identity_ok(operands, results),
            operands >= 1 && operands == results
        );
    }

    #[test]
    fn if_threading_matches_definition(qubits in 0usize..8, results in 0usize..8) {
        prop_assert_eq!(if_qubit_threading_ok(qubits, results), qubits == results);
    }

    #[test]
    fn unitary_boundary_matches_definition(
        outer in 0usize..8,
        inner in 0usize..8,
        ret in 0usize..8,
        results in 0usize..8,
    ) {
        prop_assert_eq!(
            unitary_region_boundary_ok(outer, inner, ret, results),
            outer == inner && outer == ret && outer == results
        );
    }
}
