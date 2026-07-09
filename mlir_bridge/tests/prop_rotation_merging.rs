//! Property tests: rotation_merging preserves statevectors (issue #118).

mod equiv_harness;

use equiv_harness::circuit_spec::{circuit_strategy, rotation_biased_strategy};
use equiv_harness::extract::extract_func_circuit;
use equiv_harness::lower::{
    context, lower_func_module, prop_config, read_func_op, seed_adversarial,
};
use equiv_harness::statevector::{TOL, circuits_equiv};

use mlir_bridge::passes::rotation_merging;
use proptest::prelude::*;

fn assert_pass_preserves(spec: &equiv_harness::CircuitSpec) -> Result<(), TestCaseError> {
    let ctx = context();
    let module = lower_func_module(&ctx, spec);
    rotation_merging::run_on_module(&ctx, &module);
    let post = extract_func_circuit(read_func_op(&module));
    circuits_equiv(spec, &post, TOL).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(prop_config())]

    #[test]
    fn rotation_merging_preserves_statevector(spec in rotation_biased_strategy()) {
        assert_pass_preserves(&spec)?;
    }

    #[test]
    fn rotation_merging_on_general_circuits(spec in circuit_strategy()) {
        assert_pass_preserves(&spec)?;
    }
}

#[test]
fn seeded_rz_rz_merge() {
    for spec in seed_adversarial() {
        assert_pass_preserves(&spec).unwrap_or_else(|e| {
            panic!("adversarial failure on {spec:?}: {e}");
        });
    }
}
