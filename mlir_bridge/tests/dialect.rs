//! Dialect tests for issue #4: registration, op builders, the verifier
//! rejection matrix, and MLIR-text round-trips.

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use quon_core::DepthExpr;

use support::{bool_attr, depth_attr, generic_op, i64_attr, str_attr};

// --- Registration ----------------------------------------------------------

#[test]
fn registration_is_idempotent_and_panic_free() {
    let context = support::context();
    qc::register_dialect(&context);
    qc::register_dialect(&context);
    assert!(context.allow_unregistered_dialects());
    assert_eq!(qc::OPS.len(), 7);
}

#[test]
fn type_helpers_print_canonically() {
    let context = support::context();
    assert_eq!(qc::qubit_type(&context).to_string(), qc::QUBIT_TYPE);
    assert_eq!(qc::circuit_type(&context).to_string(), qc::CIRCUIT_TYPE);
}

// --- Builders all succeed --------------------------------------------------

#[test]
fn all_builders_produce_verifiable_ops() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);
    let depth = DepthExpr::Nat(1);

    let block = support::scratch_block(&[qubit, qubit, circuit, circuit], location);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let c0 = Value::from(block.argument(2).unwrap());
    let c1 = Value::from(block.argument(3).unwrap());

    // Single- and multi-qubit gates.
    qc::gate(&context, "H", 1, true, &[q0], location).expect("1-qubit gate");
    qc::gate(&context, "CX", 1, true, &[q0, q1], location).expect("2-qubit gate");

    qc::compose(&context, c0, c1, &depth, location).expect("compose");
    qc::tensor(&context, c0, c1, &depth, location).expect("tensor");
    qc::adjoint(&context, c0, &depth, location).expect("adjoint");
    qc::controlled(&context, c0, q0, &depth, location).expect("controlled");

    let body = Region::new();
    body.append_block(Block::new(&[]));
    qc::borrow(&context, 2, &depth, body, location).expect("borrow");

    qc::r#return(&[q0, q1], location).expect("return");

    // func is exercised by bell_like_module.
    let _ = support::bell_like_module(&context);
}

// --- Round-trips -----------------------------------------------------------

#[test]
fn func_module_round_trips() {
    let context = support::context();
    let module = support::bell_like_module(&context);

    let text = module.as_operation().to_string();
    assert!(text.contains("quantum.circ.func"));
    assert!(text.contains("quantum.circ.gate"));
    assert!(text.contains(qc::QUBIT_TYPE));

    let reparsed = Module::parse(&context, &text).expect("re-parse");
    assert_eq!(text, reparsed.as_operation().to_string());
}

#[test]
fn every_op_round_trips_in_one_module() {
    let context = support::context();
    let location = Location::unknown(&context);
    let circuit = qc::circuit_type(&context);
    let qubit = qc::qubit_type(&context);
    let depth = DepthExpr::Nat(3);

    let module = Module::new(location);
    let body = module.body();

    // func + gate + return, built inline.
    let func_region = Region::new();
    let func_block = Block::new(&[(qubit, location)]);
    let arg = Value::from(func_block.argument(0).unwrap());
    let gate =
        func_block.append_operation(qc::gate(&context, "H", 1, true, &[arg], location).unwrap());
    let gate_out = Value::from(gate.result(0).unwrap());
    func_block.append_operation(qc::r#return(&[gate_out], location).unwrap());
    func_region.append_block(func_block);
    body.append_operation(
        qc::func(&context, "main", 1, 1, &depth, true, func_region, location).unwrap(),
    );

    // Sources for the combinator ops (foreign ops producing circuit/qubit SSA).
    let c0 = Value::from(
        body.append_operation(generic_op(
            &context,
            "test.circuit",
            &[],
            &[circuit],
            &[],
            vec![],
            location,
        ))
        .result(0)
        .unwrap(),
    );
    let c1 = Value::from(
        body.append_operation(generic_op(
            &context,
            "test.circuit",
            &[],
            &[circuit],
            &[],
            vec![],
            location,
        ))
        .result(0)
        .unwrap(),
    );
    let ctrl = Value::from(
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

    body.append_operation(qc::compose(&context, c0, c1, &depth, location).unwrap());
    body.append_operation(qc::tensor(&context, c0, c1, &depth, location).unwrap());
    body.append_operation(qc::adjoint(&context, c0, &depth, location).unwrap());
    body.append_operation(qc::controlled(&context, c0, ctrl, &depth, location).unwrap());

    let borrow_body = Region::new();
    borrow_body.append_block(Block::new(&[]));
    body.append_operation(qc::borrow(&context, 1, &depth, borrow_body, location).unwrap());

    let text = module.as_operation().to_string();
    for op in qc::OPS {
        assert!(text.contains(op), "round-trip text missing {op}");
    }

    let reparsed = Module::parse(&context, &text).expect("re-parse kitchen-sink module");
    assert_eq!(text, reparsed.as_operation().to_string());
}

#[test]
fn depth_attribute_serializes_as_sexpr() {
    let context = support::context();
    let location = Location::unknown(&context);
    let circuit = qc::circuit_type(&context);

    let block = support::scratch_block(&[circuit, circuit], location);
    let c0 = Value::from(block.argument(0).unwrap());
    let c1 = Value::from(block.argument(1).unwrap());

    let depth = DepthExpr::Nat(1).seq(DepthExpr::Var("n".into()));
    let compose = qc::compose(&context, c0, c1, &depth, location).unwrap();
    assert!(compose.to_string().contains(r#"depth = "(+ 1 n)""#));
}

// --- Verifier rejection matrix --------------------------------------------

#[test]
fn gate_verifier_rejections() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);
    let block = support::scratch_block(&[qubit, circuit], location);
    let q = Value::from(block.argument(0).unwrap());
    let c = Value::from(block.argument(1).unwrap());

    let name = ("gate_name", str_attr(&context, "H"));
    let contribution = ("depth_contribution", i64_attr(&context, 1));
    let clifford = ("clifford", bool_attr(&context, true));

    // Missing gate_name.
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[q],
        &[qubit],
        &[contribution, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingAttribute {
            attr: "gate_name",
            ..
        })
    ));

    // Missing depth_contribution.
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[q],
        &[qubit],
        &[name, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingAttribute {
            attr: "depth_contribution",
            ..
        })
    ));

    // Missing clifford.
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[q],
        &[qubit],
        &[name, contribution],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingAttribute {
            attr: "clifford",
            ..
        })
    ));

    // Negative depth_contribution.
    let neg = ("depth_contribution", i64_attr(&context, -2));
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[q],
        &[qubit],
        &[name, neg, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::NegativeCount { .. })
    ));

    // Zero operands.
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[],
        &[],
        &[name, contribution, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::Arity {
            role: "operand",
            ..
        })
    ));

    // Operand/result count mismatch.
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[q],
        &[qubit, qubit],
        &[name, contribution, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::Arity { role: "result", .. })
    ));

    // Operand is not a qubit.
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[c],
        &[qubit],
        &[name, contribution, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType {
            role: "operand",
            ..
        })
    ));

    // Result is not a qubit (operand fine).
    let op = generic_op(
        &context,
        qc::op::GATE,
        &[q],
        &[circuit],
        &[name, contribution, clifford],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType { role: "result", .. })
    ));
}

#[test]
fn func_verifier_rejections() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);

    let body_with = |args: &[melior::ir::Type<'_>]| {
        let region = Region::new();
        let pairs: Vec<_> = args.iter().map(|t| (*t, location)).collect();
        region.append_block(Block::new(&pairs));
        region
    };
    let full = |skip: &str, in_q: i64, out_q: i64| {
        let mut attrs: Vec<(&str, melior::ir::Attribute)> = vec![
            ("sym_name", str_attr(&context, "main")),
            ("in_qubits", i64_attr(&context, in_q)),
            ("out_qubits", i64_attr(&context, out_q)),
            ("depth", depth_attr(&context, &DepthExpr::Nat(0))),
            ("clifford", bool_attr(&context, true)),
        ];
        attrs.retain(|(k, _)| *k != skip);
        attrs
    };

    // Missing each required attribute.
    for missing in ["sym_name", "in_qubits", "out_qubits", "depth", "clifford"] {
        let op = generic_op(
            &context,
            qc::op::FUNC,
            &[],
            &[],
            &full(missing, 1, 1),
            vec![body_with(&[qubit])],
            location,
        );
        assert!(
            matches!(
                qc::verify(&op),
                Err(qc::VerifyError::MissingAttribute { .. })
            ),
            "missing {missing} should be rejected"
        );
    }

    // Negative qubit counts.
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &full("", -1, 0),
        vec![body_with(&[])],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::NegativeCount {
            attr: "in_qubits",
            ..
        })
    ));
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &full("", 0, -1),
        vec![body_with(&[])],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::NegativeCount {
            attr: "out_qubits",
            ..
        })
    ));

    // in_qubits attribute of the wrong kind (a string).
    let mut attrs = full("in_qubits", 1, 1);
    attrs.push(("in_qubits", str_attr(&context, "nope")));
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &attrs,
        vec![body_with(&[qubit])],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongAttributeType {
            attr: "in_qubits",
            ..
        })
    ));

    // Missing region entirely.
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &full("", 0, 0),
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingRegion { .. })
    ));

    // Region with no block.
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &full("", 0, 0),
        vec![Region::new()],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingRegion { .. })
    ));

    // Entry block arity disagrees with in_qubits.
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &full("", 2, 2),
        vec![body_with(&[qubit])],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::ArgCountMismatch { .. })
    ));

    // Entry block argument is not a qubit.
    let op = generic_op(
        &context,
        qc::op::FUNC,
        &[],
        &[],
        &full("", 1, 1),
        vec![body_with(&[circuit])],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType {
            role: "block argument",
            ..
        })
    ));
}

#[test]
fn binary_circuit_verifier_rejections() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);
    let block = support::scratch_block(&[qubit, circuit, circuit], location);
    let q = Value::from(block.argument(0).unwrap());
    let c0 = Value::from(block.argument(1).unwrap());
    let c1 = Value::from(block.argument(2).unwrap());
    let depth = ("depth", depth_attr(&context, &DepthExpr::Nat(0)));

    for op_name in [qc::op::COMPOSE, qc::op::TENSOR] {
        // Wrong arity (one operand).
        let op = generic_op(
            &context,
            op_name,
            &[c0],
            &[circuit],
            &[depth],
            vec![],
            location,
        );
        assert!(matches!(
            qc::verify(&op),
            Err(qc::VerifyError::Arity {
                role: "operand",
                ..
            })
        ));

        // Operand 0 not a circuit.
        let op = generic_op(
            &context,
            op_name,
            &[q, c1],
            &[circuit],
            &[depth],
            vec![],
            location,
        );
        assert!(matches!(
            qc::verify(&op),
            Err(qc::VerifyError::WrongValueType { index: 0, .. })
        ));

        // Operand 1 not a circuit.
        let op = generic_op(
            &context,
            op_name,
            &[c0, q],
            &[circuit],
            &[depth],
            vec![],
            location,
        );
        assert!(matches!(
            qc::verify(&op),
            Err(qc::VerifyError::WrongValueType { index: 1, .. })
        ));

        // Result not a circuit.
        let op = generic_op(
            &context,
            op_name,
            &[c0, c1],
            &[qubit],
            &[depth],
            vec![],
            location,
        );
        assert!(matches!(
            qc::verify(&op),
            Err(qc::VerifyError::WrongValueType { role: "result", .. })
        ));

        // Missing depth.
        let op = generic_op(
            &context,
            op_name,
            &[c0, c1],
            &[circuit],
            &[],
            vec![],
            location,
        );
        assert!(matches!(
            qc::verify(&op),
            Err(qc::VerifyError::MissingAttribute { attr: "depth", .. })
        ));
    }
}

#[test]
fn adjoint_and_controlled_verifier_rejections() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);
    let block = support::scratch_block(&[qubit, circuit, circuit], location);
    let q = Value::from(block.argument(0).unwrap());
    let c0 = Value::from(block.argument(1).unwrap());
    let c1 = Value::from(block.argument(2).unwrap());
    let depth = ("depth", depth_attr(&context, &DepthExpr::Nat(0)));

    // adjoint: wrong arity.
    let op = generic_op(
        &context,
        qc::op::ADJOINT,
        &[c0, c1],
        &[circuit],
        &[depth],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::Arity { .. })
    ));
    // adjoint: operand not a circuit.
    let op = generic_op(
        &context,
        qc::op::ADJOINT,
        &[q],
        &[circuit],
        &[depth],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType {
            role: "operand",
            ..
        })
    ));

    // controlled: control operand must be a qubit.
    let op = generic_op(
        &context,
        qc::op::CONTROLLED,
        &[c0, c1],
        &[circuit],
        &[depth],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType { index: 1, .. })
    ));
    // controlled: missing depth.
    let op = generic_op(
        &context,
        qc::op::CONTROLLED,
        &[c0, q],
        &[circuit],
        &[],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingAttribute { attr: "depth", .. })
    ));
}

#[test]
fn borrow_and_return_verifier_rejections() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);
    let block = support::scratch_block(&[qubit], location);
    let q = Value::from(block.argument(0).unwrap());
    let depth = ("depth", depth_attr(&context, &DepthExpr::Nat(0)));

    // borrow: zero results.
    let op = generic_op(
        &context,
        qc::op::BORROW,
        &[],
        &[],
        &[depth],
        vec![support::empty_body()],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::Arity { role: "result", .. })
    ));
    // borrow: result not a qubit.
    let op = generic_op(
        &context,
        qc::op::BORROW,
        &[],
        &[circuit],
        &[depth],
        vec![support::empty_body()],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType { role: "result", .. })
    ));
    // borrow: missing region.
    let op = generic_op(
        &context,
        qc::op::BORROW,
        &[],
        &[qubit],
        &[depth],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingRegion { .. })
    ));
    // borrow: missing depth.
    let op = generic_op(
        &context,
        qc::op::BORROW,
        &[],
        &[qubit],
        &[],
        vec![support::empty_body()],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::MissingAttribute { attr: "depth", .. })
    ));

    // return: must have zero results.
    let op = generic_op(
        &context,
        qc::op::RETURN,
        &[q],
        &[qubit],
        &[],
        vec![],
        location,
    );
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::Arity { role: "result", .. })
    ));
}

#[test]
fn return_rejects_non_qubit_operand() {
    let context = support::context();
    let location = Location::unknown(&context);
    let circuit = qc::circuit_type(&context);
    let block = support::scratch_block(&[circuit], location);
    let c = Value::from(block.argument(0).unwrap());

    let op = generic_op(&context, qc::op::RETURN, &[c], &[], &[], vec![], location);
    assert!(matches!(
        qc::verify(&op),
        Err(qc::VerifyError::WrongValueType {
            role: "operand",
            ..
        })
    ));
}

#[test]
fn verify_ignores_foreign_ops() {
    let context = support::context();
    let location = Location::unknown(&context);
    let circuit = qc::circuit_type(&context);

    // An op outside the dialect is not our concern: verify returns Ok.
    let op = generic_op(
        &context,
        "test.circuit",
        &[],
        &[circuit],
        &[],
        vec![],
        location,
    );
    assert!(qc::verify(&op).is_ok());
}
