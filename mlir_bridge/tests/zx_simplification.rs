//! ZX simplification pass tests (issue #20).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::zx_simplification;
use quon_core::DepthExpr;
use zx::simplify;

use support::context;

/// Builds a single-qubit `quantum.circ.func` named `main` from `(axis, angle)`
/// rotation gates and returns the module plus a ref to the func.
fn rotation_func_module<'c>(context: &'c melior::Context, gates: &[(&str, f64)]) -> Module<'c> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for (axis, angle) in gates {
        let op = block.append_operation(
            qc::rotation_gate(context, axis, *angle, 1, false, wire, location).expect("rz"),
        );
        wire = Value::from(op.result(0).expect("out"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "main",
        1,
        1,
        &DepthExpr::Nat(gates.len() as u64),
        false,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

fn count_gate(module: &Module<'_>, gate_name: &str) -> usize {
    module
        .as_operation()
        .to_string()
        .matches(&format!("gate_name = \"{gate_name}\""))
        .count()
}

#[test]
fn double_rz_is_fused_in_the_module() {
    let context = context();
    let module = rotation_func_module(&context, &[("Rz", 0.5), ("Rz", 0.3)]);
    assert_eq!(count_gate(&module, "Rz"), 2);

    zx_simplification::run_on_module(&context, &module);

    // The pass must actually rewrite the IR: two Rz collapse to one whose angle
    // is the sum. The old pass dropped its result and left this unchanged.
    assert_eq!(count_gate(&module, "Rz"), 1);
    let text = module.as_operation().to_string();
    assert!(text.contains("8.000000e-01"), "{text}");
}

#[test]
fn distinct_axes_are_left_untouched() {
    let context = context();
    let func_ref = {
        let module = rotation_func_module(&context, &[("Rz", 0.5), ("Rx", 0.3)]);
        let func = module.body().first_operation().expect("func");
        // Rz then Rx do not fuse, so the gate count must not shrink.
        assert!(!zx_simplification::simplify_func(&context, func));
        module.as_operation().to_string()
    };
    assert_eq!(func_ref.matches("gate_name = \"Rz\"").count(), 1);
    assert_eq!(func_ref.matches("gate_name = \"Rx\"").count(), 1);
}

#[test]
fn multi_qubit_func_is_declined() {
    // A two-wire func is outside the sound single-wire regime (operand position
    // is not the wire index, and entangling diagrams cannot be extracted), so
    // the pass must decline and leave every gate in place.
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let mut q0 = Value::from(block.argument(0).expect("q0"));
    let q1 = Value::from(block.argument(1).expect("q1"));
    q0 = Value::from(
        block
            .append_operation(
                qc::rotation_gate(&context, "Rz", 0.5, 1, false, q0, location).expect("rz"),
            )
            .result(0)
            .expect("out"),
    );
    let cnot = block
        .append_operation(qc::gate(&context, "CNOT", 1, true, &[q0, q1], location).expect("cnot"));
    let c0 = Value::from(cnot.result(0).expect("c0"));
    let c1 = Value::from(cnot.result(1).expect("c1"));
    block.append_operation(qc::r#return(&[c0, c1], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        2,
        2,
        &DepthExpr::Nat(2),
        false,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    let func_ref = module.body().append_operation(func);

    assert!(!zx_simplification::simplify_func(&context, func_ref));
    assert_eq!(count_gate(&module, "Rz"), 1);
    assert_eq!(count_gate(&module, "CNOT"), 1);
}

#[test]
fn extracted_diagram_fuses_in_the_zx_layer() {
    let context = context();
    let module = rotation_func_module(&context, &[("Rz", 0.5), ("Rz", 0.3)]);
    let func_ref = module.body().first_operation().expect("func");
    let gates = zx_simplification::extract_gates(func_ref);
    assert_eq!(gates.len(), 2);
    let mut zx = zx::circuit_to_zx(&gates);
    assert!(simplify(&mut zx) > 0);
    assert_eq!(zx::zx_to_circuit(&zx).len(), 1);
}
