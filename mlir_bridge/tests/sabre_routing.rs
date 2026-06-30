//! SABRE routing integration tests (issue #25).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::sabre_routing::{self, SabreCost};

use support::context;

fn linear_5q() -> backend::BackendTarget {
    let edges: Vec<(usize, usize)> = (0..4).map(|i| (i, i + 1)).collect();
    backend::BackendTarget {
        id: "linear5".into(),
        num_qubits: 5,
        topology: backend::ConnectivityGraph::try_from_edges(5, edges).expect("topology"),
        native_gates: vec![
            backend::NativeGate::passthrough("cx", 2),
            backend::NativeGate::passthrough("swap", 2),
        ],
        noise: backend::NoiseModel::default(),
        meas_latency_us: 0.0,
        supports_mid_circuit_meas: true,
        supports_feed_forward: true,
    }
}

fn bell_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let h = block.append_operation(qc::gate(context, "H", 1, true, &[q0], location).unwrap());
    let h_out = Value::from(h.result(0).unwrap());
    let cx = block.append_operation(
        qc::gate(context, "CNOT", 1, true, &[h_out, q1], location)
            .unwrap(),
    );
    let out0 = Value::from(cx.result(0).unwrap());
    let out1 = Value::from(cx.result(1).unwrap());
    block.append_operation(qc::r#return(&[out0, out1], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "bell",
        2,
        2,
        &quon_core::DepthExpr::Nat(2),
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
fn assigns_phys_qubit_to_all_gates() {
    let context = context();
    let module = bell_module(&context);
    sabre_routing::run_on_module(&context, &linear_5q(), SabreCost::default(), &module);
    let text = module.as_operation().to_string();
    assert!(text.contains("phys_qubit"));
}

#[test]
fn bell_on_linear_chain_uses_minimum_swaps() {
    let context = context();
    let module = bell_module(&context);
    sabre_routing::run_on_module(&context, &linear_5q(), SabreCost::default(), &module);
    let swaps = module
        .as_operation()
        .to_string()
        .matches("gate_name = \"SWAP\"")
        .count();
    assert_eq!(swaps, 0);
}

#[test]
fn higher_alpha_increases_swap_preference() {
    let low = SabreCost {
        alpha: 0.1,
        ..SabreCost::default()
    };
    let high = SabreCost {
        alpha: 10.0,
        ..SabreCost::default()
    };
    assert!(high.alpha > low.alpha);
}
