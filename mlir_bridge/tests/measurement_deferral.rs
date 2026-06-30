//! Measurement deferral integration tests (issue #22).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::passes::{measurement_deferral, monadic_lowering};
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
    use mlir_bridge::dialect::monadic_staging as staging;

    let location = Location::unknown(context);
    let module = Module::new(location);
    let body = module.body();

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

    let qubit = qc::qubit_type(context);
    let msg = append_foreign_qubit(context, &body, location);
    let alice = append_foreign_qubit(context, &body, location);
    let bob = append_foreign_qubit(context, &body, location);

    let run_block = Block::new(&[(qubit, location), (qubit, location), (qubit, location)]);
    let r_msg = Value::from(run_block.argument(0).unwrap());
    let r_alice = Value::from(run_block.argument(1).unwrap());
    let r_bob = Value::from(run_block.argument(2).unwrap());

    let entangle = run_block.append_operation(staging::apply(
        context,
        "bell_state",
        &[r_alice, r_bob],
        location,
    ));
    let a = Value::from(entangle.result(0).unwrap());
    let b = Value::from(entangle.result(1).unwrap());

    let unentangle = run_block.append_operation(staging::apply(
        context,
        "adjoint_bell",
        &[r_msg, a],
        location,
    ));
    let m2 = Value::from(unentangle.result(0).unwrap());

    let x_measure = run_block.append_operation(staging::measure(context, m2, location));
    let x_bit = Value::from(x_measure.result(0).unwrap());

    let z_measure = run_block.append_operation(staging::measure(context, a, location));
    let z_bit = Value::from(z_measure.result(0).unwrap());

    let x_correct = run_block.append_operation(staging::cond_apply(
        context,
        x_bit,
        "X_1",
        "identity_1",
        &[b],
        location,
    ));
    let b2 = Value::from(x_correct.result(0).unwrap());

    let z_correct = run_block.append_operation(staging::cond_apply(
        context,
        z_bit,
        "Z_1",
        "identity_1",
        &[b2],
        location,
    ));
    let b3 = Value::from(z_correct.result(0).unwrap());

    run_block.append_operation(staging::r#yield(&[b3], location));

    let run_region = Region::new();
    run_region.append_block(run_block);
    body.append_operation(staging::run(
        context,
        &[msg, alice, bob],
        run_region,
        location,
    ));

    monadic_lowering::run_on_module(context, &module).expect("lower teleport");
    module
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
