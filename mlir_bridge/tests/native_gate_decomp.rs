//! Native gate decomposition integration tests (issue #24).

mod support;

use backend::json;
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::native_gate_decomp;

use support::context;

fn device_5q() -> backend::BackendTarget {
    json::from_str(include_str!("../../backend/tests/fixtures/device_5q.json"))
        .expect("load device_5q")
}

fn bell_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);

    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());

    let h = block.append_operation(qc::gate(context, "H", 1, true, &[q0], location).unwrap());
    let h_out = Value::from(h.result(0).unwrap());
    let cx = block
        .append_operation(qc::gate(context, "CNOT", 1, true, &[h_out, q1], location).unwrap());
    let out0 = Value::from(cx.result(0).unwrap());
    let out1 = Value::from(cx.result(1).unwrap());
    block.append_operation(qc::r#return(&[out0, out1], location).unwrap());

    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "bell_state",
        2,
        2,
        &quon_core::DepthExpr::Nat(2),
        true,
        region,
        location,
    )
    .expect("func");

    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

fn gate_names(module: &Module<'_>) -> Vec<String> {
    module
        .as_operation()
        .to_string()
        .split("gate_name = \"")
        .skip(1)
        .map(|chunk| chunk.split('"').next().unwrap_or("").to_string())
        .collect()
}

#[test]
fn hadamard_decomposes_on_rz_sx_target() {
    let context = context();
    let target = backend::BackendTarget {
        id: "test".into(),
        num_qubits: 2,
        topology: backend::ConnectivityGraph::all_to_all(2),
        native_gates: vec![
            backend::NativeGate::passthrough("rz", 1),
            backend::NativeGate::passthrough("sx", 1),
        ],
        noise: backend::NoiseModel::default(),
        meas_latency_us: 0.0,
        supports_mid_circuit_meas: true,
        supports_feed_forward: true,
    };
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location)]);
    let q = Value::from(block.argument(0).unwrap());
    block.append_operation(qc::gate(&context, "H", 1, true, &[q], location).unwrap());
    block.append_operation(qc::r#return(&[q], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &quon_core::DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);

    native_gate_decomp::run_on_module(&context, &target, &module);
    let names = gate_names(&module);
    assert!(names.iter().all(|n| n == "rz" || n == "sx"));
    assert!(!names.contains(&"H".to_string()));
}

#[test]
fn bell_state_uses_only_native_gates_on_device_5q() {
    let context = context();
    let module = bell_module(&context);
    native_gate_decomp::run_on_module(&context, &device_5q(), &module);
    let names = gate_names(&module);
    for name in &names {
        assert!(
            matches!(name.as_str(), "cx" | "rz" | "sx" | "x"),
            "unexpected gate `{name}`"
        );
    }
    let text = module.as_operation().to_string();
    assert!(text.contains("native_gate = true"));
    assert!(!text.contains("native_gate = false"));
}

#[test]
fn cnot_stays_native_on_device_5q() {
    let context = context();
    let module = bell_module(&context);
    native_gate_decomp::run_on_module(&context, &device_5q(), &module);
    let text = module.as_operation().to_string();
    assert!(text.contains("gate_name = \"cx\""));
}
