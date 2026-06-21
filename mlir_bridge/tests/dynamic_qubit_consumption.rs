//! Dynamic qubit linearity tests for issue #6.

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::passes::dynamic_linearity_verifier::check_dynamic_linearity;
use quon_core::linearity::{UseCountViolation, classify_use_count};

#[test]
fn pass_agrees_with_quon_core_kernel_on_reuse_after_measure() {
    assert_eq!(
        classify_use_count(2, true, true),
        Some(UseCountViolation::ReuseAfterMeasure)
    );
}

#[test]
fn accepts_measure_then_uses_bit_only() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let module = Module::new(location);
    let body = module.body();
    let q_meas = support::append_foreign_qubit(&context, &body, location);
    let q_branch = support::append_foreign_qubit(&context, &body, location);

    let measure = body.append_operation(qd::measure(&context, q_meas, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let then_region = Region::new();
    let then_block = Block::new(&[(qubit, location)]);
    let then_arg = Value::from(then_block.argument(0).unwrap());
    then_block.append_operation(qd::r#yield(&[then_arg], location).unwrap());
    then_region.append_block(then_block);

    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location)]);
    let else_arg = Value::from(else_block.argument(0).unwrap());
    else_block.append_operation(qd::r#yield(&[else_arg], location).unwrap());
    else_region.append_block(else_block);

    let if_op = body.append_operation(
        qd::r#if(
            &context,
            bit,
            &[q_branch],
            then_region,
            else_region,
            location,
        )
        .expect("if"),
    );
    let q_out = Value::from(if_op.result(0).unwrap());
    let barrier = body.append_operation(qd::barrier(&context, &[q_out], location).unwrap());
    let q_final = Value::from(barrier.result(0).unwrap());
    body.append_operation(qd::measure(&context, q_final, location).unwrap());

    let diagnostics = check_dynamic_linearity(support::module_region(&module));
    assert!(
        diagnostics.is_empty(),
        "{:?}",
        diagnostics.iter().collect::<Vec<_>>()
    );
}

#[test]
fn rejects_gate_after_measure_on_same_qubit() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);

    let module = Module::new(location);
    let body = module.body();
    let q = support::append_foreign_qubit(&context, &body, location);
    body.append_operation(qd::measure(&context, q, location).unwrap());
    body.append_operation(qc::gate(&context, "H", 1, true, &[q], location).unwrap());

    let diagnostics = check_dynamic_linearity(support::module_region(&module));
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message().contains("quantum.dynamic.measure")),
        "{:?}",
        diagnostics.iter().map(|d| d.message()).collect::<Vec<_>>()
    );
}

#[test]
fn accepts_reset_as_consume_and_produce() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);

    let module = Module::new(location);
    let body = module.body();
    let q0 = support::append_foreign_qubit(&context, &body, location);
    let reset = body.append_operation(qd::reset(&context, q0, location).unwrap());
    let q1 = Value::from(reset.result(0).unwrap());
    let barrier = body.append_operation(qd::barrier(&context, &[q1], location).unwrap());
    let q2 = Value::from(barrier.result(0).unwrap());
    body.append_operation(qd::measure(&context, q2, location).unwrap());

    let diagnostics = check_dynamic_linearity(support::module_region(&module));
    assert!(diagnostics.is_empty());
}

#[test]
fn rejects_use_of_pre_reset_qubit() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);

    let module = Module::new(location);
    let body = module.body();
    let q0 = support::append_foreign_qubit(&context, &body, location);
    let reset = body.append_operation(qd::reset(&context, q0, location).unwrap());
    let q1 = Value::from(reset.result(0).unwrap());
    body.append_operation(qd::barrier(&context, &[q0, q1], location).unwrap());

    let diagnostics = check_dynamic_linearity(support::module_region(&module));
    assert!(
        diagnostics.iter().any(|d| {
            d.message().contains("2 use(s)") || d.message().contains("cannot be reused")
        }),
        "{:?}",
        diagnostics.iter().map(|d| d.message()).collect::<Vec<_>>()
    );
}

#[test]
fn pass_fails_on_reuse_after_measure() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);

    let mut module = Module::new(location);
    let body = module.body();
    let q = support::append_foreign_qubit(&context, &body, location);
    body.append_operation(qd::measure(&context, q, location).unwrap());
    body.append_operation(qc::gate(&context, "H", 1, true, &[q], location).unwrap());

    let pass_manager = melior::pass::PassManager::new(&context);
    pass_manager.add_pass(mlir_bridge::passes::dynamic_linearity_verifier::create_pass());
    assert!(pass_manager.run(&mut module).is_err());
}
