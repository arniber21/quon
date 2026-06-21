//! Dialect tests for issue #6: registration, op builders, verifier
//! rejection matrix, and MLIR-text round-trips.

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use quon_core::DepthExpr;

use support::{bool_attr, dynamic_context, f64_attr, generic_op, i32_attr};

// --- Registration ----------------------------------------------------------

#[test]
fn registration_is_idempotent_and_panic_free() {
    let context = dynamic_context();
    qd::register_dialect(&context);
    qd::register_dialect(&context);
    assert!(context.allow_unregistered_dialects());
    assert_eq!(qd::OPS.len(), 5);
}

#[test]
fn type_helpers_print_canonically() {
    let context = dynamic_context();
    assert_eq!(qd::bit_type(&context).to_string(), qd::BIT_TYPE);
    assert_eq!(qc::qubit_type(&context).to_string(), qc::QUBIT_TYPE);
}

// --- Builders --------------------------------------------------------------

#[test]
fn all_builders_produce_verifiable_ops() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let bit = qd::bit_type(&context);
    let depth = DepthExpr::Nat(1);

    let block = support::scratch_block(&[qubit, bit], location);
    let q = Value::from(block.argument(0).unwrap());
    let b = Value::from(block.argument(1).unwrap());

    qd::measure(&context, q, location).expect("measure");
    qd::reset(&context, q, location).expect("reset");
    qd::barrier(&context, &[q], location).expect("barrier");

    let inner = Region::new();
    let inner_block = Block::new(&[(qubit, location)]);
    let arg = Value::from(inner_block.argument(0).unwrap());
    let gate =
        inner_block.append_operation(qc::gate(&context, "H", 1, true, &[arg], location).unwrap());
    let out = Value::from(gate.result(0).unwrap());
    inner_block.append_operation(qc::r#return(&[out], location).unwrap());
    inner.append_block(inner_block);
    qd::unitary_region(&context, &[q], &depth, true, inner, location).expect("unitary_region");

    let then_region = Region::new();
    let then_block = Block::new(&[(qubit, location)]);
    let then_arg = Value::from(then_block.argument(0).unwrap());
    then_block.append_operation(qd::r#yield(&[then_arg], location).expect("then yield"));
    then_region.append_block(then_block);

    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location)]);
    let else_arg = Value::from(else_block.argument(0).unwrap());
    else_block.append_operation(qd::r#yield(&[else_arg], location).expect("else yield"));
    else_region.append_block(else_block);

    qd::r#if(&context, b, &[q], then_region, else_region, location).expect("if");
}

#[test]
fn physical_attributes_accepted_when_present() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = support::scratch_block(&[qubit], location);
    let q = Value::from(block.argument(0).unwrap());

    let op = generic_op(
        &context,
        qd::op::MEASURE,
        &[q],
        &[qd::bit_type(&context)],
        &[
            ("phys_qubit", i32_attr(&context, 0)),
            ("native_gate", bool_attr(&context, true)),
            ("fidelity", f64_attr(&context, 0.99)),
        ],
        vec![],
        location,
    );
    assert!(qd::verify(&op).is_ok());
}

// --- Round-trips -----------------------------------------------------------

#[test]
fn minimal_measure_module_round_trips() {
    let context = dynamic_context();
    let location = Location::unknown(&context);

    let module = Module::new(location);
    let body = module.body();
    let q = support::append_foreign_qubit(&context, &body, location);
    body.append_operation(qd::measure(&context, q, location).unwrap());

    let text = module.as_operation().to_string();
    assert!(text.contains(qd::op::MEASURE));
    assert!(text.contains(qd::BIT_TYPE));

    let reparsed = Module::parse(&context, &text).expect("re-parse");
    assert_eq!(text, reparsed.as_operation().to_string());
}

#[test]
fn every_op_round_trips_in_one_module() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let depth = DepthExpr::Nat(1);

    let module = Module::new(location);
    let body = module.body();

    let q0 = Value::from(
        body.append_operation(generic_op(
            &context,
            "test.qubit",
            &[],
            &[qubit],
            &[],
            vec![],
            location,
        ))
        .result(0)
        .unwrap(),
    );
    let q1 = Value::from(
        body.append_operation(generic_op(
            &context,
            "test.qubit",
            &[],
            &[qubit],
            &[],
            vec![],
            location,
        ))
        .result(0)
        .unwrap(),
    );

    let bit_val = Value::from(
        body.append_operation(qd::measure(&context, q0, location).unwrap())
            .result(0)
            .unwrap(),
    );
    let reset = body.append_operation(qd::reset(&context, q1, location).unwrap());
    let q2 = Value::from(reset.result(0).unwrap());

    body.append_operation(qd::barrier(&context, &[q2], location).unwrap());

    let inner = Region::new();
    let inner_block = Block::new(&[(qubit, location)]);
    let arg = Value::from(inner_block.argument(0).unwrap());
    let gate =
        inner_block.append_operation(qc::gate(&context, "H", 1, true, &[arg], location).unwrap());
    let out = Value::from(gate.result(0).unwrap());
    inner_block.append_operation(qc::r#return(&[out], location).unwrap());
    inner.append_block(inner_block);
    let region = body.append_operation(
        qd::unitary_region(&context, &[q2], &depth, true, inner, location).unwrap(),
    );
    let q3 = Value::from(region.result(0).unwrap());

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

    body.append_operation(
        qd::r#if(&context, bit_val, &[q3], then_region, else_region, location).unwrap(),
    );

    let text = module.as_operation().to_string();
    for op in qd::OPS {
        assert!(text.contains(op), "round-trip text missing {op}");
    }

    let reparsed = Module::parse(&context, &text).expect("re-parse kitchen-sink module");
    assert_eq!(text, reparsed.as_operation().to_string());
}

// --- Verifier rejection matrix --------------------------------------------

#[test]
fn measure_verifier_rejections() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let bit = qd::bit_type(&context);
    let block = support::scratch_block(&[qubit, bit], location);
    let q = Value::from(block.argument(0).unwrap());
    let b = Value::from(block.argument(1).unwrap());

    let op = generic_op(
        &context,
        qd::op::MEASURE,
        &[],
        &[bit],
        &[],
        vec![],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::Arity {
            role: "operand",
            ..
        })
    ));

    let op = generic_op(
        &context,
        qd::op::MEASURE,
        &[b],
        &[bit],
        &[],
        vec![],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::WrongValueType {
            role: "operand",
            ..
        })
    ));

    let op = generic_op(
        &context,
        qd::op::MEASURE,
        &[q],
        &[qubit],
        &[],
        vec![],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::WrongValueType { role: "result", .. })
    ));

    let op = generic_op(
        &context,
        qd::op::MEASURE,
        &[q],
        &[bit],
        &[("phys_qubit", support::str_attr(&context, "nope"))],
        vec![],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::WrongAttributeType {
            attr: "phys_qubit",
            ..
        })
    ));
}

#[test]
fn unitary_region_verifier_rejections() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let depth = DepthExpr::Nat(1);

    let circ_body = |gate_only: bool| {
        let region = Region::new();
        let block = Block::new(&[(qubit, location)]);
        let arg = Value::from(block.argument(0).unwrap());
        if gate_only {
            block.append_operation(
                qc::gate(&context, "H", 1, true, &[arg], location).expect("gate"),
            );
        } else {
            block.append_operation(qd::measure(&context, arg, location).expect("measure"));
        }
        region.append_block(block);
        region
    };

    let block = support::scratch_block(&[qubit], location);
    let q = Value::from(block.argument(0).unwrap());

    let op = generic_op(
        &context,
        qd::op::UNITARY_REGION,
        &[q],
        &[qubit],
        &[
            ("depth", support::depth_attr(&context, &depth)),
            ("clifford", bool_attr(&context, true)),
        ],
        vec![circ_body(false)],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::ForbiddenOpInUnitaryRegion { .. })
    ));

    let foreign = Region::new();
    foreign.append_block(Block::new(&[(qubit, location)]));
    foreign.first_block().unwrap().append_operation(generic_op(
        &context,
        "test.foreign",
        &[],
        &[],
        &[],
        vec![],
        location,
    ));
    let op = generic_op(
        &context,
        qd::op::UNITARY_REGION,
        &[q],
        &[qubit],
        &[
            ("depth", support::depth_attr(&context, &depth)),
            ("clifford", bool_attr(&context, true)),
        ],
        vec![foreign],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::ForbiddenOpInUnitaryRegion { .. })
    ));

    let op = generic_op(
        &context,
        qd::op::UNITARY_REGION,
        &[q],
        &[qubit],
        &[
            ("depth", support::depth_attr(&context, &depth)),
            ("clifford", bool_attr(&context, true)),
        ],
        vec![circ_body(true)],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::MissingRegion { .. })
    ));
}

#[test]
fn if_verifier_rejections() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let bit = qd::bit_type(&context);
    let block = support::scratch_block(&[qubit, bit], location);
    let q = Value::from(block.argument(0).unwrap());
    let b = Value::from(block.argument(1).unwrap());

    let then_region = Region::new();
    then_region.append_block(Block::new(&[(qubit, location)]));
    let else_region = Region::new();
    else_region.append_block(Block::new(&[(qubit, location)]));

    let op = generic_op(
        &context,
        qd::op::IF,
        &[q, q],
        &[qubit],
        &[],
        vec![then_region, else_region],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::WrongValueType {
            role: "operand",
            index: 0,
            ..
        })
    ));

    let then_region = Region::new();
    let then_block = Block::new(&[(qubit, location)]);
    then_region.append_block(then_block);
    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location)]);
    else_region.append_block(else_block);
    let op = generic_op(
        &context,
        qd::op::IF,
        &[b, q],
        &[qubit],
        &[],
        vec![then_region, else_region],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::MissingYieldTerminator { .. })
    ));
}

#[test]
fn barrier_verifier_rejections() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = support::scratch_block(&[qubit], location);
    let q = Value::from(block.argument(0).unwrap());

    let op = generic_op(&context, qd::op::BARRIER, &[], &[], &[], vec![], location);
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::Arity {
            role: "operand",
            ..
        })
    ));

    let op = generic_op(
        &context,
        qd::op::BARRIER,
        &[q],
        &[qubit, qubit],
        &[],
        vec![],
        location,
    );
    assert!(matches!(
        qd::verify(&op),
        Err(qd::VerifyError::Arity { role: "result", .. })
    ));
}

#[test]
fn verify_ignores_foreign_ops() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let bit = qd::bit_type(&context);

    let op = generic_op(&context, "test.dynamic", &[], &[bit], &[], vec![], location);
    assert!(qd::verify(&op).is_ok());
}
