//! Integration tests for the circ_extract seam (issue #320).
//!
//! Property tests verify extract→rebuild identity (structural and semantic)
//! on random supported gate sequences, including multi-qubit circuits.

mod equiv_harness;

use equiv_harness::circuit_spec::{circuit_strategy, GateInst, GateKind};
use equiv_harness::extract::extract_func_circuit;
use equiv_harness::lower::{context, lower_func_module, prop_config, read_func_op};
use equiv_harness::statevector::{TOL, circuits_equiv};
use equiv_harness::CircuitSpec;

use mlir_bridge::circ_extract;
use proptest::prelude::*;

/// Extract → rebuild → extract must be the identity on CircIr.
fn assert_circir_identity(spec: &CircuitSpec) -> Result<(), TestCaseError> {
    let ctx = context();
    let module = lower_func_module(&ctx, spec);
    let func = read_func_op(&module);

    let circ1 = circ_extract::extract(func).expect("extract should succeed on supported circuit");
    circ_extract::rebuild(&ctx, func, &circ1).expect("rebuild should succeed");
    let circ2 = circ_extract::extract(func).expect("second extract should succeed");

    prop_assert_eq!(circ1, circ2);
    Ok(())
}

/// Extract → rebuild must preserve the circuit's statevector.
fn assert_statevector_preserved(spec: &CircuitSpec) -> Result<(), TestCaseError> {
    let ctx = context();
    let module = lower_func_module(&ctx, spec);
    let func = read_func_op(&module);

    let circ = circ_extract::extract(func).expect("extract should succeed");
    circ_extract::rebuild(&ctx, func, &circ).expect("rebuild should succeed");

    let post = extract_func_circuit(func);
    circuits_equiv(spec, &post, TOL).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(prop_config())]

    #[test]
    fn extract_rebuild_is_circir_identity(spec in circuit_strategy()) {
        assert_circir_identity(&spec)?;
    }

    #[test]
    fn extract_rebuild_preserves_statevector(spec in circuit_strategy()) {
        assert_statevector_preserved(&spec)?;
    }
}

// ── seeded cases ──────────────────────────────────────────────────────────

#[test]
fn single_qubit_hxz_identity() {
    let spec = CircuitSpec::new(
        1,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::X, vec![0]),
            GateInst::new(GateKind::Z, vec![0]),
        ],
    );
    assert_circir_identity(&spec).expect("H-X-Z");
}

#[test]
fn two_qubit_cnot_h_identity() {
    let spec = CircuitSpec::new(
        2,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::CNOT, vec![0, 1]),
            GateInst::new(GateKind::H, vec![1]),
        ],
    );
    assert_circir_identity(&spec).expect("H-CNOT-H");
}

#[test]
fn two_qubit_cz_swap_identity() {
    let spec = CircuitSpec::new(
        2,
        vec![
            GateInst::new(GateKind::CZ, vec![0, 1]),
            GateInst::new(GateKind::SWAP, vec![0, 1]),
            GateInst::new(GateKind::CZ, vec![1, 0]),
        ],
    );
    assert_circir_identity(&spec).expect("CZ-SWAP-CZ");
}

#[test]
fn three_qubit_chain_identity() {
    let spec = CircuitSpec::new(
        3,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::CNOT, vec![0, 1]),
            GateInst::new(GateKind::CNOT, vec![1, 2]),
            GateInst::new(GateKind::H, vec![2]),
            GateInst::new(GateKind::S, vec![1]),
        ],
    );
    assert_circir_identity(&spec).expect("3-qubit chain");
}

#[test]
fn rotation_chain_identity() {
    let spec = CircuitSpec::new(
        1,
        vec![
            GateInst::rotation(GateKind::Rz, 0, 0.5),
            GateInst::rotation(GateKind::Rx, 0, 0.3),
            GateInst::rotation(GateKind::Rz, 0, 0.7),
        ],
    );
    assert_circir_identity(&spec).expect("Rz-Rx-Rz");
}

#[test]
fn mixed_clifford_t_identity() {
    let spec = CircuitSpec::new(
        2,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::T, vec![0]),
            GateInst::new(GateKind::CNOT, vec![0, 1]),
            GateInst::new(GateKind::S, vec![1]),
            GateInst::new(GateKind::T, vec![1]),
        ],
    );
    assert_circir_identity(&spec).expect("H-T-CNOT-S-T");
}

#[test]
fn extract_preserves_statevector_seeded() {
    let spec = CircuitSpec::new(
        2,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::CNOT, vec![0, 1]),
            GateInst::rotation(GateKind::Rz, 1, 0.5),
        ],
    );
    assert_statevector_preserved(&spec).expect("statevector preserved");
}
