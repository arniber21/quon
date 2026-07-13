//! Clifford+T and uncomputation tests (issue #21).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::{compiler_uncomputation, gate_cancellation};
use quon_core::DepthExpr;

use support::context;

fn count_gate(module: &Module<'_>, gate_name: &str) -> usize {
    module
        .as_operation()
        .to_string()
        .matches(&format!("gate_name = \"{gate_name}\""))
        .count()
}

#[test]
fn t_t_dagger_cancels_via_gate_cancellation() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for name in ["T", "T†"] {
        let op = block
            .append_operation(qc::gate(&context, name, 1, false, &[wire], location).expect("gate"));
        wire = Value::from(op.result(0).expect("out"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &DepthExpr::Nat(2),
        false,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    gate_cancellation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "T"), 0);
    assert_eq!(count_gate(&module, "T†"), 0);
}

#[test]
fn pauli_xx_cancels_via_gate_cancellation() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for _ in 0..2 {
        let op =
            block.append_operation(qc::gate(&context, "X", 1, true, &[wire], location).expect("x"));
        wire = Value::from(op.result(0).expect("out"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &DepthExpr::Nat(2),
        true,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    gate_cancellation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "X"), 0);
}

#[test]
fn uncomputation_appends_adjoint_to_reversible_borrow() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let body = Block::new(&[(qubit, location)]);
    let wire = Value::from(body.argument(0).expect("arg"));
    let h = body.append_operation(qc::gate(&context, "H", 1, true, &[wire], location).expect("h"));
    let out = Value::from(h.result(0).expect("out"));
    body.append_operation(qc::r#return(&[out], location).expect("return"));
    let region = Region::new();
    region.append_block(body);
    let borrow = qc::borrow(&context, 1, &DepthExpr::Nat(1), region, location).expect("borrow");
    let module = Module::new(location);
    module.body().append_operation(borrow);
    compiler_uncomputation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "H"), 2);
}

#[test]
fn uncomputation_skips_borrow_with_unknown_gate() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let body = Block::new(&[(qubit, location)]);
    let wire = Value::from(body.argument(0).expect("arg"));
    body.append_operation(
        qc::rotation_gate(&context, "Rz", 0.5, 1, false, wire, location).expect("rz"),
    );
    body.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(body);
    let borrow = qc::borrow(&context, 1, &DepthExpr::Nat(1), region, location).expect("borrow");
    let module = Module::new(location);
    module.body().append_operation(borrow);
    compiler_uncomputation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "Rz"), 1);
}
