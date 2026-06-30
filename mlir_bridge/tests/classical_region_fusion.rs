//! Classical region fusion integration tests (issue #23).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::passes::classical_region_fusion;

use support::{append_foreign_qubit, dynamic_context};

fn branch_region<'c>(
    context: &'c melior::Context,
    location: Location<'c>,
    gate_name: Option<&str>,
) -> Region<'c> {
    let block = Block::new(&[(qc::qubit_type(context), location)]);
    let arg = Value::from(block.argument(0).unwrap());
    let out = if let Some(name) = gate_name {
        let gate =
            block.append_operation(qc::gate(context, name, 1, true, &[arg], location).unwrap());
        Value::from(gate.result(0).unwrap())
    } else {
        arg
    };
    block.append_operation(qd::r#yield(&[out], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    region
}

fn append_if<'c>(
    context: &'c melior::Context,
    body: &melior::ir::BlockRef<'c, 'c>,
    bit: Value<'c, '_>,
    qubit: Value<'c, '_>,
    then_gate: Option<&str>,
    else_gate: Option<&str>,
    location: Location<'c>,
) {
    body.append_operation(
        qd::r#if(
            context,
            bit,
            &[qubit],
            branch_region(context, location, then_gate),
            branch_region(context, location, else_gate),
            location,
        )
        .unwrap(),
    );
}

fn count_ifs(module: &Module<'_>) -> usize {
    module
        .as_operation()
        .to_string()
        .matches(qd::op::IF)
        .count()
}

#[test]
fn fuses_adjacent_ifs_on_disjoint_qubits_with_same_condition() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = append_foreign_qubit(&context, &body, location);
    let q1 = append_foreign_qubit(&context, &body, location);
    let q2 = append_foreign_qubit(&context, &body, location);
    let measure = body.append_operation(qd::measure(&context, q0, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    append_if(&context, &body, bit, q1, Some("X"), None, location);
    append_if(&context, &body, bit, q2, Some("Z"), None, location);

    classical_region_fusion::run_on_module(&context, &module);
    assert_eq!(count_ifs(&module), 1);
    assert!(module
        .as_operation()
        .to_string()
        .contains(qd::op::UNITARY_REGION));
}

#[test]
fn does_not_fuse_when_qubits_overlap() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = append_foreign_qubit(&context, &body, location);
    let q1 = append_foreign_qubit(&context, &body, location);
    let measure = body.append_operation(qd::measure(&context, q0, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    append_if(&context, &body, bit, q1, Some("X"), None, location);
    append_if(&context, &body, bit, q1, Some("Z"), None, location);

    classical_region_fusion::run_on_module(&context, &module);
    assert_eq!(count_ifs(&module), 2);
}

#[test]
fn does_not_fuse_when_second_condition_depends_on_first() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = append_foreign_qubit(&context, &body, location);
    let q1 = append_foreign_qubit(&context, &body, location);
    let q2 = append_foreign_qubit(&context, &body, location);

    let measure0 = body.append_operation(qd::measure(&context, q0, location).unwrap());
    let bit0 = Value::from(measure0.result(0).unwrap());

    let then_with_measure = {
        let block = Block::new(&[(qc::qubit_type(&context), location)]);
        let arg = Value::from(block.argument(0).unwrap());
        let inner_measure = block.append_operation(qd::measure(&context, arg, location).unwrap());
        let inner_bit = Value::from(inner_measure.result(0).unwrap());
        block.append_operation(qd::r#yield(&[arg], location).unwrap());
        let region = Region::new();
        region.append_block(block);
        (region, inner_bit)
    };
    let else_region = branch_region(&context, location, None);

    body.append_operation(
        qd::r#if(
            &context,
            bit0,
            &[q1],
            then_with_measure.0,
            else_region,
            location,
        )
        .unwrap(),
    );

    append_if(
        &context,
        &body,
        then_with_measure.1,
        q2,
        Some("X"),
        None,
        location,
    );

    classical_region_fusion::run_on_module(&context, &module);
    assert_eq!(count_ifs(&module), 2);
}
