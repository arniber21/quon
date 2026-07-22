//! Measurement deferral integration tests (issue #22).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::passes::measurement_deferral;
use quon_core::DepthExpr;

use support::{append_foreign_qubit, dynamic_context};

fn gate_func<'c>(
    context: &'c melior::Context,
    name: &str,
    gates: &[(&str, &[usize])],
    in_qubits: i64,
    depth: &DepthExpr,
    location: Location<'c>,
) -> melior::ir::Operation<'c> {
    let qubit = qc::qubit_type(context);
    let block = Block::new(
        &(0..in_qubits)
            .map(|_| (qubit, location))
            .collect::<Vec<_>>(),
    );
    let mut wires: Vec<Value<'c, '_>> = (0..block.argument_count())
        .map(|index| Value::from(block.argument(index).expect("arg")))
        .collect();
    for (gate_name, targets) in gates {
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|index| wires[*index]).collect();
        let op = block.append_operation(
            qc::gate(context, gate_name, 1, true, &operands, location).expect("gate"),
        );
        for (index, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).expect("result"));
        }
    }
    block.append_operation(qc::r#return(&wires, location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    qc::func(
        context, name, in_qubits, in_qubits, depth, true, region, location,
    )
    .expect("func")
}

fn count_op(text: &str, op: &str) -> usize {
    text.matches(op).count()
}

fn lower_teleport_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let module = Module::new(location);
    let body = module.body();

    // Circuit-function definitions (preserved as dead code after inlining —
    // `lower` inlines their bodies into `unitary_region`/`if` regions now that
    // the staging dialect is gone, #213 / ADR-0037).
    body.append_operation(gate_func(
        context,
        "bell_state",
        &[("H", &[0]), ("CNOT", &[0, 1])],
        2,
        &DepthExpr::Nat(2),
        location,
    ));
    body.append_operation(gate_func(
        context,
        "adjoint_bell",
        &[("CNOT", &[0, 1]), ("H", &[0])],
        2,
        &DepthExpr::Nat(2),
        location,
    ));
    body.append_operation(gate_func(
        context,
        "X_1",
        &[("X", &[0])],
        1,
        &DepthExpr::Nat(1),
        location,
    ));
    body.append_operation(gate_func(
        context,
        "Z_1",
        &[("Z", &[0])],
        1,
        &DepthExpr::Nat(1),
        location,
    ));
    body.append_operation(gate_func(
        context,
        "identity_1",
        &[],
        1,
        &DepthExpr::Nat(0),
        location,
    ));

    let msg = append_foreign_qubit(context, &body, location);
    let alice = append_foreign_qubit(context, &body, location);
    let bob = append_foreign_qubit(context, &body, location);

    // ent = bell_state(alice, bob) inlined into a unitary_region.
    let ent = body.append_operation(
        qd::unitary_region(
            context,
            &[alice, bob],
            &DepthExpr::Nat(2),
            true,
            unitary_body(context, 2, &[("H", &[0]), ("CNOT", &[0, 1])], location),
            location,
        )
        .unwrap(),
    );
    let a = Value::from(ent.result(0).unwrap());
    let b = Value::from(ent.result(1).unwrap());

    // unent = adjoint_bell(msg, a) inlined.
    let unent = body.append_operation(
        qd::unitary_region(
            context,
            &[msg, a],
            &DepthExpr::Nat(2),
            true,
            unitary_body(context, 2, &[("CNOT", &[0, 1]), ("H", &[0])], location),
            location,
        )
        .unwrap(),
    );
    let m2 = Value::from(unent.result(0).unwrap());

    let x_measure = body.append_operation(qd::measure(context, m2, location).unwrap());
    let x_bit = Value::from(x_measure.result(0).unwrap());
    let z_measure = body.append_operation(qd::measure(context, a, location).unwrap());
    let z_bit = Value::from(z_measure.result(0).unwrap());

    // Feed-forward corrections: each measure bit drives one `if`.
    let x_correct = body.append_operation(
        qd::r#if(
            context,
            x_bit,
            &[b],
            branch_body(context, &[("X", &[0])], location),
            branch_body(context, &[], location),
            location,
        )
        .unwrap(),
    );
    let b2 = Value::from(x_correct.result(0).unwrap());

    let z_correct = body.append_operation(
        qd::r#if(
            context,
            z_bit,
            &[b2],
            branch_body(context, &[("Z", &[0])], location),
            branch_body(context, &[], location),
            location,
        )
        .unwrap(),
    );
    let _b3 = Value::from(z_correct.result(0).unwrap());

    module
}

/// A `quantum.dynamic.unitary_region` body region: an `n`-qubit block running
/// `gates` and terminated by `quantum.circ.return` (mirrors `gate_func`'s body,
/// inlined straight into the dynamic IR by `lower`).
fn unitary_body<'c>(
    context: &'c melior::Context,
    n: usize,
    gates: &[(&str, &[usize])],
    location: Location<'c>,
) -> Region<'c> {
    let qubit = qc::qubit_type(context);
    let block = Block::new(&(0..n).map(|_| (qubit, location)).collect::<Vec<_>>());
    let mut wires: Vec<Value<'c, '_>> = (0..n)
        .map(|i| Value::from(block.argument(i).expect("arg")))
        .collect();
    for (gate_name, targets) in gates {
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|idx| wires[*idx]).collect();
        let op = block.append_operation(
            qc::gate(context, gate_name, 1, true, &operands, location).expect("gate"),
        );
        for (i, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(i).expect("result"));
        }
    }
    block.append_operation(qc::r#return(&wires, location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    region
}

/// A `quantum.dynamic.if` branch body region: a 1-qubit block running `gates`
/// (empty for the identity correction) terminated by `quantum.dynamic.yield`.
fn branch_body<'c>(
    context: &'c melior::Context,
    gates: &[(&str, &[usize])],
    location: Location<'c>,
) -> Region<'c> {
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location)]);
    let mut wires: Vec<Value<'c, '_>> = vec![Value::from(block.argument(0).expect("arg"))];
    for (gate_name, targets) in gates {
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|idx| wires[*idx]).collect();
        let op = block.append_operation(
            qc::gate(context, gate_name, 1, true, &operands, location).expect("gate"),
        );
        for (i, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(i).expect("result"));
        }
    }
    block.append_operation(qd::r#yield(&wires, location).expect("yield"));
    let region = Region::new();
    region.append_block(block);
    region
}

#[test]
fn single_use_measure_if_is_deferred() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    body.append_operation(gate_func(
        &context,
        "X_1",
        &[("X", &[0])],
        1,
        &DepthExpr::Nat(1),
        location,
    ));
    body.append_operation(gate_func(
        &context,
        "identity_1",
        &[],
        1,
        &DepthExpr::Nat(0),
        location,
    ));

    let q_meas = append_foreign_qubit(&context, &body, location);
    let q_target = append_foreign_qubit(&context, &body, location);

    let measure = body.append_operation(qd::measure(&context, q_meas, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let then_region = {
        let block = Block::new(&[(qc::qubit_type(&context), location)]);
        let arg = Value::from(block.argument(0).unwrap());
        let gate =
            block.append_operation(qc::gate(&context, "X", 1, true, &[arg], location).unwrap());
        let out = Value::from(gate.result(0).unwrap());
        block.append_operation(qd::r#yield(&[out], location).unwrap());
        let region = Region::new();
        region.append_block(block);
        region
    };
    let else_region = {
        let block = Block::new(&[(qc::qubit_type(&context), location)]);
        let arg = Value::from(block.argument(0).unwrap());
        block.append_operation(qd::r#yield(&[arg], location).unwrap());
        let region = Region::new();
        region.append_block(block);
        region
    };
    body.append_operation(
        qd::r#if(
            &context,
            bit,
            &[q_target],
            then_region,
            else_region,
            location,
        )
        .unwrap(),
    );

    measurement_deferral::run_on_module(&context, &module);
    let text = module.as_operation().to_string();
    assert_eq!(count_op(&text, qd::op::IF), 0, "{text}");
    assert!(text.contains(qd::op::UNITARY_REGION), "{text}");
    assert!(text.contains("CNOT"), "{text}");
    assert!(
        text.contains("gate_name = \"CNOT\"") && text.matches("!quantum.qubit").count() >= 2,
        "expected CNOT between distinct qubits: {text}"
    );
}

#[test]
fn bit_used_by_two_ifs_is_not_deferred() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q_meas = append_foreign_qubit(&context, &body, location);
    let q_a = append_foreign_qubit(&context, &body, location);
    let q_b = append_foreign_qubit(&context, &body, location);

    let measure = body.append_operation(qd::measure(&context, q_meas, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let identity_region = |ctx: &melior::Context, loc: Location<'_>| {
        let block = Block::new(&[(qc::qubit_type(ctx), loc)]);
        let arg = Value::from(block.argument(0).unwrap());
        block.append_operation(qd::r#yield(&[arg], loc).unwrap());
        let region = Region::new();
        region.append_block(block);
        region
    };

    for target in [q_a, q_b] {
        body.append_operation(
            qd::r#if(
                &context,
                bit,
                &[target],
                identity_region(&context, location),
                identity_region(&context, location),
                location,
            )
            .unwrap(),
        );
    }

    measurement_deferral::run_on_module(&context, &module);
    let text = module.as_operation().to_string();
    assert_eq!(count_op(&text, qd::op::IF), 2, "{text}");
    assert_eq!(count_op(&text, qd::op::MEASURE), 1, "{text}");
}

#[test]
fn non_commuting_branch_correction_uses_u_v_dagger_order() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q_meas = append_foreign_qubit(&context, &body, location);
    let q_target = append_foreign_qubit(&context, &body, location);
    let measure = body.append_operation(qd::measure(&context, q_meas, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    body.append_operation(
        qd::r#if(
            &context,
            bit,
            &[q_target],
            branch_with_gate(&context, location, "X"),
            branch_with_gate(&context, location, "Z"),
            location,
        )
        .unwrap(),
    );

    measurement_deferral::run_on_module(&context, &module);
    let text = module.as_operation().to_string();
    assert_eq!(count_op(&text, qd::op::IF), 0, "{text}");
    let cz = text.find("gate_name = \"CZ\"").expect(&text);
    let cx = text.find("gate_name = \"CNOT\"").expect(&text);
    assert!(
        cz < cx,
        "correction must emit V† before U to implement U·V†: {text}"
    );
}

fn branch_with_gate<'c>(
    context: &'c melior::Context,
    location: Location<'c>,
    gate_name: &str,
) -> Region<'c> {
    let block = Block::new(&[(qc::qubit_type(context), location)]);
    let arg = Value::from(block.argument(0).unwrap());
    let gate =
        block.append_operation(qc::gate(context, gate_name, 1, true, &[arg], location).unwrap());
    let out = Value::from(gate.result(0).unwrap());
    block.append_operation(qd::r#yield(&[out], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    region
}

#[test]
fn teleport_defers_both_measurements() {
    let context = dynamic_context();
    let module = lower_teleport_module(&context);
    measurement_deferral::run_on_module(&context, &module);
    let text = module.as_operation().to_string();
    assert_eq!(count_op(&text, qd::op::IF), 0, "{text}");
    assert_eq!(count_op(&text, qd::op::MEASURE), 2, "{text}");
    assert!(text.contains("CNOT"), "{text}");
    assert!(text.contains("CZ"), "{text}");
}
