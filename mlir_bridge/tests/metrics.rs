//! Circuit metrics collector tests (issue #48).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::metrics::collect_module_metrics;

fn append_gate<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c melior::Context,
    block: &B,
    name: &str,
    q: Value<'c, 'a>,
    location: Location<'c>,
) -> Value<'c, 'a> {
    let op = block.append_operation(qc::gate(context, name, 1, true, &[q], location).unwrap());
    Value::from(op.result(0).unwrap())
}

#[test]
fn counts_gates_in_linear_module() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);

    let module = Module::new(location);
    let body = module.body();
    let q0 = support::append_foreign_qubit(&context, &body, location);
    let q1 = append_gate(&context, &body, "H", q0, location);
    let q2 = append_gate(&context, &body, "X", q1, location);
    let _q3 = append_gate(&context, &body, "Z", q2, location);

    let target = backend::BackendTarget::fixed(
        "test",
        backend::FixedTarget {
            num_qubits: 1,
            topology: backend::ConnectivityGraph::all_to_all(1),
            native_gates: vec![
                backend::NativeGate::passthrough("H", 1),
                backend::NativeGate::passthrough("X", 1),
                backend::NativeGate::passthrough("Z", 1),
            ],
            noise: backend::NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    );
    let metrics = collect_module_metrics(&module, &target);
    assert_eq!(metrics.gate_count, 3);
}

#[test]
fn dynamic_if_counts_both_branches() {
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
    let then_out = append_gate(&context, &then_block, "H", then_arg, location);
    then_block.append_operation(qd::r#yield(&[then_out], location).unwrap());
    then_region.append_block(then_block);

    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location)]);
    let else_arg = Value::from(else_block.argument(0).unwrap());
    let else_out = append_gate(&context, &else_block, "T", else_arg, location);
    else_block.append_operation(qd::r#yield(&[else_out], location).unwrap());
    else_region.append_block(else_block);

    body.append_operation(
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

    let target = backend::BackendTarget::fixed(
        "test",
        backend::FixedTarget {
            num_qubits: 2,
            topology: backend::ConnectivityGraph::all_to_all(2),
            native_gates: vec![
                backend::NativeGate::passthrough("H", 1),
                backend::NativeGate::passthrough("T", 1),
            ],
            noise: backend::NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    );
    let metrics = collect_module_metrics(&module, &target);
    // Conservative upper bound: both branches contribute to gate/t counts.
    assert_eq!(
        metrics.gate_count, 2,
        "expected both if branches to be counted"
    );
    assert_eq!(metrics.t_count, 1, "T gate only appears in else branch");
}
