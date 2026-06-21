//! Unit tests for linear qubit SSA invariant kernels.

use quon_core::linearity::{
    UseCountViolation, barrier_identity_ok, classify_use_count, if_qubit_threading_ok,
    is_linear_use_count, is_reuse_after_measure, unitary_region_boundary_ok,
};

#[test]
fn linear_use_count_is_one() {
    assert!(is_linear_use_count(1));
    assert!(!is_linear_use_count(0));
    assert!(!is_linear_use_count(2));
}

#[test]
fn classify_accepts_single_use() {
    assert_eq!(classify_use_count(1, false, false), None);
    assert_eq!(classify_use_count(1, true, false), None);
}

#[test]
fn classify_detects_unused() {
    assert_eq!(
        classify_use_count(0, false, false),
        Some(UseCountViolation::Unused)
    );
}

#[test]
fn classify_detects_multi_use() {
    assert_eq!(
        classify_use_count(2, false, false),
        Some(UseCountViolation::MultiUse { count: 2 })
    );
}

#[test]
fn classify_detects_reuse_after_measure() {
    assert!(is_reuse_after_measure(2, true, true));
    assert!(!is_reuse_after_measure(2, true, false));
    assert!(!is_reuse_after_measure(2, false, true));
    assert_eq!(
        classify_use_count(2, true, true),
        Some(UseCountViolation::ReuseAfterMeasure)
    );
}

#[test]
fn barrier_identity_requires_matching_arity() {
    assert!(barrier_identity_ok(1, 1));
    assert!(barrier_identity_ok(3, 3));
    assert!(!barrier_identity_ok(0, 0));
    assert!(!barrier_identity_ok(2, 1));
}

#[test]
fn if_qubit_threading_requires_equal_counts() {
    assert!(if_qubit_threading_ok(0, 0));
    assert!(if_qubit_threading_ok(2, 2));
    assert!(!if_qubit_threading_ok(1, 0));
}

#[test]
fn unitary_region_boundary_requires_all_counts_equal() {
    assert!(unitary_region_boundary_ok(2, 2, 2, 2));
    assert!(!unitary_region_boundary_ok(2, 1, 2, 2));
    assert!(!unitary_region_boundary_ok(2, 2, 1, 2));
    assert!(!unitary_region_boundary_ok(2, 2, 2, 1));
}
