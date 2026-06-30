//! Depth scheduling integration tests (issue #26).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::depth_scheduling;

use support::{context, generic_op, i32_attr};

fn gate_with_phys<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c melior::Context,
    block: &B,
    name: &str,
    qubit: Value<'c, '_>,
    phys: i32,
    location: Location<'c>,
) {
    block.append_operation(generic_op(
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
}

fn parallel_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    gate_with_phys(context, &block, "X", q0, 0, location);
    gate_with_phys(context, &block, "X", q1, 1, location);
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

#[test]
fn asap_parallel_gates_share_minimum_depth() {
    let context = context();
    let module = parallel_module(&context);
    depth_scheduling::run_on_module(&context, &backend::generic_openqasm::target(2), &module);
    let text = module.as_operation().to_string();
    assert!(text.contains("schedule_time = 0"));
    assert_eq!(text.matches("schedule_time = 0").count(), 2);
}

#[test]
fn short_t1_selects_alap_mode() {
    let mut noise = backend::NoiseModel::default();
    noise.t1_us.insert(0, 0.01);
    let target = backend::BackendTarget {
        id: "short".into(),
        num_qubits: 2,
        topology: backend::ConnectivityGraph::all_to_all(2),
        native_gates: vec![backend::NativeGate::passthrough("x", 1)],
        noise,
        meas_latency_us: 0.0,
        supports_mid_circuit_meas: true,
        supports_feed_forward: true,
    };
    let context = context();
    let module = parallel_module(&context);
    depth_scheduling::run_on_module(&context, &target, &module);
    assert!(module.as_operation().to_string().contains("schedule_time"));
}
