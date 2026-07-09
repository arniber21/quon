//! Property tests: gate_cancellation preserves statevectors (issue #118).

mod equiv_harness;

use equiv_harness::circuit_spec::circuit_strategy;
use equiv_harness::extract::extract_func_circuit;
use equiv_harness::lower::{
    context, lower_func_module, prop_config, read_func_op, seed_adversarial,
};
use equiv_harness::statevector::{TOL, circuits_equiv};

use mlir_bridge::passes::gate_cancellation;
use proptest::prelude::*;

fn assert_pass_preserves(spec: &equiv_harness::CircuitSpec) -> Result<(), TestCaseError> {
    let ctx = context();
    let module = lower_func_module(&ctx, spec);
    gate_cancellation::run_on_module(&ctx, &module);
    let post = extract_func_circuit(read_func_op(&module));
    circuits_equiv(spec, &post, TOL).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(prop_config())]

    #[test]
    fn gate_cancellation_preserves_statevector(spec in circuit_strategy()) {
        assert_pass_preserves(&spec)?;
    }
}

#[test]
fn seeded_adversarial_gate_cancellation() {
    for spec in seed_adversarial() {
        assert_pass_preserves(&spec).unwrap_or_else(|e| {
            panic!("adversarial failure on {spec:?}: {e}");
        });
    }
}

#[test]
fn hh_cancels_and_matches_identity() {
    use equiv_harness::{CircuitSpec, GateInst, GateKind};
    let spec = CircuitSpec::new(
        1,
        vec![
            GateInst::new(GateKind::H, vec![0]),
            GateInst::new(GateKind::H, vec![0]),
        ],
    );
    assert_pass_preserves(&spec).expect("HH");
}
