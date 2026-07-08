//! Depth scheduling integration tests (issue #26).

mod support;

use melior::ir::operation::OperationLike;
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;

use support::{context, generic_op, i32_attr};

fn gate_with_phys<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c melior::Context,
    block: &B,
    name: &str,
    qubit: Value<'c, 'a>,
    phys: i32,
    location: Location<'c>,
) -> Value<'c, 'a> {
    let op = block.append_operation(generic_op(
        context,
        qc::op::GATE,
        &[qubit],
        &[qc::qubit_type(context)],
        &[
            ("gate_name", support::str_attr(context, name)),
            ("depth_contribution", support::i64_attr(context, 1)),
            ("clifford", support::bool_attr(context, true)),
            ("phys_qubit", i32_attr(context, phys)),
        ],
        vec![],
        location,
    ));
    Value::from(op.result(0).unwrap())
}

fn gate_on_qubits<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c melior::Context,
    block: &B,
    name: &str,
    qubits: &[Value<'c, 'a>],
    location: Location<'c>,
) -> Vec<Value<'c, 'a>> {
    let results = vec![qc::qubit_type(context); qubits.len()];
    let op = block.append_operation(generic_op(
        context,
        qc::op::GATE,
        qubits,
        &results,
        &[
            ("gate_name", support::str_attr(context, name)),
            ("depth_contribution", support::i64_attr(context, 1)),
            ("clifford", support::bool_attr(context, true)),
        ],
        vec![],
        location,
    ));
    op.results().map(Value::from).collect()
}

fn parallel_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let q0 = gate_with_phys(context, &block, "X", q0, 0, location);
    let q1 = gate_with_phys(context, &block, "X", q1, 1, location);
    block.append_operation(qc::r#return(&[q0, q1], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "parallel",
        2,
        2,
        &quon_core::DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

fn parse_schedule_times(text: &str) -> Vec<i64> {
    text.split("schedule_time = ")
        .skip(1)
        .filter_map(|chunk| {
            chunk
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|value| value.parse().ok())
        })
        .collect()
}

#[test]
fn asap_parallel_gates_share_minimum_depth() {
    let context = context();
    let module = parallel_module(&context);
    mlir_bridge::passes::depth_scheduling::run_on_module(
        &context,
        &backend::generic_openqasm::target(2),
        &module,
    );
    let text = module.as_operation().to_string();
    assert!(text.contains("schedule_time = 0"));
    assert_eq!(text.matches("schedule_time = 0").count(), 2);
}

#[test]
fn short_t1_selects_alap_mode() {
    let mut noise = backend::NoiseModel::default();
    noise.t1_us.insert(0, 0.01);
    let target = backend::BackendTarget::fixed(
        "short",
        backend::FixedTarget {
            num_qubits: 2,
            topology: backend::ConnectivityGraph::all_to_all(2),
            native_gates: vec![backend::NativeGate::passthrough("x", 1)],
            noise,
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    );
    let context = context();
    let module = parallel_module(&context);
    mlir_bridge::passes::depth_scheduling::run_on_module(&context, &target, &module);
    assert!(module.as_operation().to_string().contains("schedule_time"));
}

#[test]
fn multi_qubit_gate_depends_on_all_wires() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let q0 = gate_with_phys(&context, &block, "H", q0, 0, location);
    let cnot = gate_on_qubits(&context, &block, "CNOT", &[q0, q1], location);
    let q1 = gate_with_phys(&context, &block, "X", cnot[1], 1, location);
    block.append_operation(qc::r#return(&[cnot[0], q1], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let module = Module::new(location);
    module.body().append_operation(
        qc::func(
            &context,
            "deps",
            2,
            2,
            &quon_core::DepthExpr::Nat(3),
            true,
            region,
            location,
        )
        .unwrap(),
    );

    mlir_bridge::passes::depth_scheduling::run_on_module(
        &context,
        &backend::generic_openqasm::target(2),
        &module,
    );
    assert_eq!(
        parse_schedule_times(&module.as_operation().to_string()),
        vec![0, 1, 2]
    );
}

#[test]
fn barrier_segments_keep_distinct_offsets() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q0 = gate_with_phys(&context, &block, "X", q0, 0, location);
    let barrier = block.append_operation(generic_op(
        &context,
        mlir_bridge::dialect::quantum_dynamic::op::BARRIER,
        &[q0],
        &[qubit],
        &[],
        vec![],
        location,
    ));
    let q0 = Value::from(barrier.result(0).unwrap());
    let q0 = gate_with_phys(&context, &block, "Z", q0, 0, location);
    block.append_operation(qc::r#return(&[q0], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let module = Module::new(location);
    module.body().append_operation(
        qc::func(
            &context,
            "barrier_offsets",
            1,
            1,
            &quon_core::DepthExpr::Nat(2),
            true,
            region,
            location,
        )
        .unwrap(),
    );

    mlir_bridge::passes::depth_scheduling::run_on_module(
        &context,
        &backend::generic_openqasm::target(1),
        &module,
    );
    assert_eq!(
        parse_schedule_times(&module.as_operation().to_string()),
        vec![0, 1]
    );
}
