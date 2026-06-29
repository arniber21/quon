//! Monadic lowering integration tests (issue #17).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::monadic_staging as staging;
use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::passes::monadic_lowering;
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

fn run_pass(context: &melior::Context, module: &Module<'_>) {
    monadic_lowering::run_on_module(context, module).expect("monadic lowering pass should succeed");
}

fn module_text(module: &Module<'_>) -> String {
    module.as_operation().to_string()
}

fn count_op(text: &str, op: &str) -> usize {
    text.matches(op).count()
}

#[test]
fn staging_module_constructs_without_pass() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let mut module = Module::new(location);
    let body = module.body();
    body.append_operation(gate_func(
        &context,
        "bell_state",
        &[("H", &[0]), ("CNOT", &[0, 1])],
        2,
        &DepthExpr::Nat(2),
        location,
    ));
    let run_block = Block::new(&[]);
    run_block.append_operation(staging::qreg(&context, 2, location));
    run_block.append_operation(staging::r#yield(&[], location));
    let run_region = Region::new();
    run_region.append_block(run_block);
    body.append_operation(staging::run(&context, &[], run_region, location));
    let text = module_text(&module);
    assert!(text.contains(staging::op::RUN));
}

#[test]
fn qreg_only_lowering_inserts_test_qubits() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let mut module = Module::new(location);
    let body = module.body();
    let run_block = Block::new(&[]);
    run_block.append_operation(staging::qreg(&context, 2, location));
    run_block.append_operation(staging::r#yield(&[], location));
    let run_region = Region::new();
    run_region.append_block(run_block);
    body.append_operation(staging::run(&context, &[], run_region, location));
    monadic_lowering::run_on_module(&context, &module).expect("lower qreg");
    let text = module_text(&module);
    assert!(text.contains("test.qubit"));
}

#[test]
fn hello_bell_lowers_to_two_measures() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let depth = DepthExpr::Nat(2);

    let mut module = Module::new(location);
    let body = module.body();

    body.append_operation(gate_func(
        &context,
        "bell_state",
        &[("H", &[0]), ("CNOT", &[0, 1])],
        2,
        &depth,
        location,
    ));

    let run_block = Block::new(&[]);
    let qreg = run_block.append_operation(staging::qreg(&context, 2, location));
    let q0 = Value::from(qreg.result(0).unwrap());
    let q1 = Value::from(qreg.result(1).unwrap());
    let apply =
        run_block.append_operation(staging::apply(&context, "bell_state", &[q0, q1], location));
    let q0m = Value::from(apply.result(0).unwrap());
    let q1m = Value::from(apply.result(1).unwrap());
    let m0 = run_block.append_operation(staging::measure(&context, q0m, location));
    let b0 = Value::from(m0.result(0).unwrap());
    let m1 = run_block.append_operation(staging::measure(&context, q1m, location));
    let b1 = Value::from(m1.result(0).unwrap());
    run_block.append_operation(staging::r#yield(&[b0, b1], location));

    let run_region = Region::new();
    run_region.append_block(run_block);
    body.append_operation(staging::run(&context, &[], run_region, location));

    run_pass(&context, &module);
    let text = module_text(&module);

    assert_eq!(count_op(&text, qd::op::MEASURE), 2);
    assert_eq!(count_op(&text, qd::op::UNITARY_REGION), 1);
    assert!(!text.contains(staging::op::MEASURE));
    assert!(text.contains(qd::op::MEASURE));
}

#[test]
fn teleport_lowers_with_two_ifs_and_measures() {
    let context = dynamic_context();
    let location = Location::unknown(&context);

    let mut module = Module::new(location);
    let body = module.body();

    body.append_operation(gate_func(
        &context,
        "bell_state",
        &[("H", &[0]), ("CNOT", &[0, 1])],
        2,
        &DepthExpr::Nat(2),
        location,
    ));
    body.append_operation(gate_func(
        &context,
        "adjoint_bell",
        &[("CNOT", &[0, 1]), ("H", &[0])],
        2,
        &DepthExpr::Nat(2),
        location,
    ));
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
        "Z_1",
        &[("Z", &[0])],
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

    let qubit = qc::qubit_type(&context);
    let msg = append_foreign_qubit(&context, &body, location);
    let alice = append_foreign_qubit(&context, &body, location);
    let bob = append_foreign_qubit(&context, &body, location);

    let run_block = Block::new(&[(qubit, location), (qubit, location), (qubit, location)]);
    let r_msg = Value::from(run_block.argument(0).unwrap());
    let r_alice = Value::from(run_block.argument(1).unwrap());
    let r_bob = Value::from(run_block.argument(2).unwrap());

    let entangle = run_block.append_operation(staging::apply(
        &context,
        "bell_state",
        &[r_alice, r_bob],
        location,
    ));
    let a = Value::from(entangle.result(0).unwrap());
    let b = Value::from(entangle.result(1).unwrap());

    let unentangle = run_block.append_operation(staging::apply(
        &context,
        "adjoint_bell",
        &[r_msg, a],
        location,
    ));
    let m2 = Value::from(unentangle.result(0).unwrap());
    let _a2 = Value::from(unentangle.result(1).unwrap());

    let x_measure = run_block.append_operation(staging::measure(&context, m2, location));
    let x_bit = Value::from(x_measure.result(0).unwrap());

    let z_measure = run_block.append_operation(staging::measure(&context, a, location));
    let z_bit = Value::from(z_measure.result(0).unwrap());

    let x_correct = run_block.append_operation(staging::cond_apply(
        &context,
        x_bit,
        "X_1",
        "identity_1",
        &[b],
        location,
    ));
    let b2 = Value::from(x_correct.result(0).unwrap());

    let z_correct = run_block.append_operation(staging::cond_apply(
        &context,
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
        &context,
        &[msg, alice, bob],
        run_region,
        location,
    ));

    run_pass(&context, &module);
    let text = module_text(&module);

    assert_eq!(count_op(&text, qd::op::MEASURE), 2, "{text}");
    assert_eq!(count_op(&text, qd::op::IF), 2, "{text}");
    assert!(text.contains(qd::op::UNITARY_REGION));
    assert!(!text.contains(staging::op::COND_APPLY));

    // Feed-forward: each measure bit feeds an if condition.
    assert!(text.contains(qd::BIT_TYPE));
}

#[test]
fn lit_teleport_mlir_lowers() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../test/lit/dynamic/monadic_lowering_teleport.mlir"
    ))
    .expect("read lit file");
    let context = dynamic_context();
    let module = Module::parse(&context, &source).expect("parse lit mlir");
    monadic_lowering::run_on_module(&context, &module).expect("lower lit teleport");
    let text = module_text(&module);
    assert!(!text.contains(staging::op::RUN));
    assert_eq!(count_op(&text, qd::op::MEASURE), 2);
    assert_eq!(count_op(&text, qd::op::IF), 2);
}

#[test]
fn unitary_regions_contain_only_circ_ops() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let mut module = Module::new(location);
    let body = module.body();
    body.append_operation(gate_func(
        &context,
        "bell_state",
        &[("H", &[0]), ("CNOT", &[0, 1])],
        2,
        &DepthExpr::Nat(2),
        location,
    ));

    let run_block = Block::new(&[]);
    let qreg = run_block.append_operation(staging::qreg(&context, 2, location));
    let q0 = Value::from(qreg.result(0).unwrap());
    let q1 = Value::from(qreg.result(1).unwrap());
    run_block.append_operation(staging::apply(&context, "bell_state", &[q0, q1], location));
    run_block.append_operation(staging::r#yield(&[], location));
    let run_region = Region::new();
    run_region.append_block(run_block);
    body.append_operation(staging::run(&context, &[], run_region, location));

    run_pass(&context, &module);
    let text = module_text(&module);
    assert!(text.contains(qc::op::GATE));
    assert!(text.contains(qc::op::RETURN));
    assert!(!text.contains(staging::op::APPLY));
}
