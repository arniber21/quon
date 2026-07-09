//! Property tests: compiler_uncomputation borrow oracle (issue #118).
//!
//! When the rewrite applies, the post-pass borrow body must act as identity on
//! `|0…0⟩` (U;U†). Do **not** assert pre ≡ post unitary.

mod equiv_harness;

use equiv_harness::circuit_spec::reversible_circuit_strategy;
use equiv_harness::extract::extract_borrow_gates;
use equiv_harness::lower::{context, lower_borrow_module, prop_config, read_func_op};
use equiv_harness::statevector::{TOL, apply_circuit, basis_state, phase_invariant_distance};
use equiv_harness::{CircuitSpec, GateInst, GateKind};

use mlir_bridge::passes::compiler_uncomputation;
use proptest::prelude::*;

fn post_body_is_identity_on_zero(spec: &CircuitSpec) -> Result<(), TestCaseError> {
    let ctx = context();
    let module = lower_borrow_module(&ctx, spec);
    let before_count = spec.depth();
    compiler_uncomputation::run_on_module(&ctx, &module);
    // Live cone feeding `return` — after the fix this is the full U;U† body.
    let post = extract_borrow_gates(read_func_op(&module));

    prop_assert_eq!(
        post.depth(),
        before_count * 2,
        "expected U;U† (2× gates); before={} live_after={}",
        before_count,
        post.depth()
    );

    let mut state = basis_state(post.width);
    apply_circuit(&mut state, &post).map_err(TestCaseError::fail)?;
    let d = phase_invariant_distance(&state, &basis_state(post.width));
    prop_assert!(
        d <= TOL,
        "post-uncomputation body is not identity on |0⟩: dist={d}\npre={spec:?}\npost={post:?}"
    );
    Ok(())
}

proptest! {
    #![proptest_config(prop_config())]

    #[test]
    fn uncomputation_body_is_identity_on_zero(spec in reversible_circuit_strategy()) {
        post_body_is_identity_on_zero(&spec)?;
    }
}

#[test]
fn seeded_h_borrow_uncomputes_to_identity() {
    let spec = CircuitSpec::new(1, vec![GateInst::new(GateKind::H, vec![0])]);
    post_body_is_identity_on_zero(&spec).expect("H borrow");
}

#[test]
fn seeded_cnot_borrow_uncomputes_to_identity() {
    let spec = CircuitSpec::new(2, vec![GateInst::new(GateKind::CNOT, vec![0, 1])]);
    post_body_is_identity_on_zero(&spec).expect("CNOT borrow");
}
