//! Property tests: zx_simplification preserves statevectors (issue #118).
//!
//! Restricted to width-1 circuits — the pass declines multi-qubit funcs.

mod equiv_harness;

use equiv_harness::circuit_spec::{width1_rotation_chain_strategy, width1_strategy};
use equiv_harness::extract::extract_func_circuit;
use equiv_harness::lower::{context, lower_func_module, prop_config, read_func_op};
use equiv_harness::statevector::{TOL, circuits_equiv};
use equiv_harness::{CircuitSpec, GateInst, GateKind};

use mlir_bridge::passes::zx_simplification;
use proptest::prelude::*;

fn assert_pass_preserves(spec: &CircuitSpec) -> Result<(), TestCaseError> {
    prop_assert_eq!(spec.width, 1);
    let ctx = context();
    let module = lower_func_module(&ctx, spec);
    zx_simplification::run_on_module(&ctx, &module);
    let post = extract_func_circuit(read_func_op(&module));
    circuits_equiv(spec, &post, TOL).map_err(TestCaseError::fail)
}

proptest! {
    #![proptest_config(prop_config())]

    #[test]
    fn zx_simplification_preserves_width1(spec in width1_strategy()) {
        assert_pass_preserves(&spec)?;
    }

    #[test]
    fn zx_simplification_fusable_rotations(spec in width1_rotation_chain_strategy()) {
        assert_pass_preserves(&spec)?;
    }
}

#[test]
fn double_rz_seed() {
    let spec = CircuitSpec::new(
        1,
        vec![
            GateInst::rotation(GateKind::Rz, 0, 0.5),
            GateInst::rotation(GateKind::Rz, 0, 0.3),
        ],
    );
    assert_pass_preserves(&spec).expect("Rz+Rz");
}
