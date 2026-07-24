//! Classical region fusion integration tests (issue #23).

mod support;

use melior::ir::operation::OperationLike;
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
) -> melior::ir::OperationRef<'c, 'c> {
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
    )
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
fn does_not_fuse_threaded_versions_of_same_qubit() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = append_foreign_qubit(&context, &body, location);
    let q1 = append_foreign_qubit(&context, &body, location);
    let measure = body.append_operation(qd::measure(&context, q0, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let first = append_if(&context, &body, bit, q1, Some("X"), None, location);
    let threaded_q1 = Value::from(first.result(0).unwrap());
    append_if(&context, &body, bit, threaded_q1, Some("Z"), None, location);

    classical_region_fusion::run_on_module(&context, &module);
    assert_eq!(count_ifs(&module), 2);
}

#[test]
fn does_not_drop_non_gate_branch_ops() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = append_foreign_qubit(&context, &body, location);
    let q1 = append_foreign_qubit(&context, &body, location);
    let q2 = append_foreign_qubit(&context, &body, location);
    let measure = body.append_operation(qd::measure(&context, q0, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let branch_with_measure = {
        let block = Block::new(&[(qc::qubit_type(&context), location)]);
        let arg = Value::from(block.argument(0).unwrap());
        block.append_operation(qd::measure(&context, arg, location).unwrap());
        block.append_operation(qd::r#yield(&[arg], location).unwrap());
        let region = Region::new();
        region.append_block(block);
        region
    };
    body.append_operation(
        qd::r#if(
            &context,
            bit,
            &[q1],
            branch_with_measure,
            branch_region(&context, location, None),
            location,
        )
        .unwrap(),
    );
    append_if(&context, &body, bit, q2, Some("Z"), None, location);

    classical_region_fusion::run_on_module(&context, &module);
    let text = module.as_operation().to_string();
    assert_eq!(count_ifs(&module), 2, "{text}");
    assert!(text.contains(qd::op::MEASURE), "{text}");
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

/// Counts `quantum.dynamic.if` ops that are direct children of the module body
/// — i.e. the classical/quantum boundary crossings. Nested `if`s inside a
/// region are not counted.
fn count_top_level_ifs(module: &Module<'_>) -> usize {
    let body = module.body();
    let mut op = body.first_operation();
    let mut count = 0;
    while let Some(current) = op {
        let name = current
            .name()
            .as_string_ref()
            .as_str()
            .unwrap_or("")
            .to_string();
        if name == qd::op::IF {
            count += 1;
        }
        op = current.next_in_block();
    }
    count
}

#[test]
fn fuses_adjacent_ifs_on_disjoint_qubits_with_independent_conditions() {
    // Issue #97: two `if`s on *different*, independent condition bits with
    // disjoint target qubits fuse via the nested-if shape. Each condition is
    // its own measurement (neither bit is produced by the other's body), so the
    // pair is independent and eligible for fusion.
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let module = Module::new(location);
    let body = module.body();

    let m0 = append_foreign_qubit(&context, &body, location);
    let m1 = append_foreign_qubit(&context, &body, location);
    let q0 = append_foreign_qubit(&context, &body, location);
    let q1 = append_foreign_qubit(&context, &body, location);
    let measure0 = body.append_operation(qd::measure(&context, m0, location).unwrap());
    let b1 = Value::from(measure0.result(0).unwrap());
    let measure1 = body.append_operation(qd::measure(&context, m1, location).unwrap());
    let b2 = Value::from(measure1.result(0).unwrap());

    append_if(&context, &body, b1, q0, Some("X"), None, location);
    append_if(&context, &body, b2, q1, Some("Z"), None, location);

    classical_region_fusion::run_on_module(&context, &module);
    let text = module.as_operation().to_string();

    // Exactly one top-level `if` remains: the outer fused `if`. The
    // classical/quantum boundary-crossing count drops from two to one.
    assert_eq!(count_top_level_ifs(&module), 1, "{text}");
    // The nested-if shape adds one inner `if` per outer branch (both branches
    // need it to keep all four condition combinations reachable), so three
    // `if`s in total — but only the outer one is a boundary crossing.
    assert_eq!(count_ifs(&module), 3, "{text}");
    // Both correction bodies survive, each guarded by its own condition.
    assert!(text.contains("gate_name = \"X\""), "{text}");
    assert!(text.contains("gate_name = \"Z\""), "{text}");
}
