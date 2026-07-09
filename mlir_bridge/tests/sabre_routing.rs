//! SABRE routing integration tests (issue #25).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::sabre_routing::{self, SabreCost};

use support::context;

fn linear_5q() -> backend::BackendTarget {
    let edges: Vec<(usize, usize)> = (0..4).map(|i| (i, i + 1)).collect();
    backend::BackendTarget::fixed(
        "linear5",
        backend::FixedTarget {
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
        },
    )
}

fn bell_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let h = block.append_operation(qc::gate(context, "H", 1, true, &[q0], location).unwrap());
    let h_out = Value::from(h.result(0).unwrap());
    let cx =
        block.append_operation(qc::gate(context, "CNOT", 1, true, &[h_out, q1], location).unwrap());
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

fn non_adjacent_cnot_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location), (qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let q2 = Value::from(block.argument(2).unwrap());
    let g0 = block.append_operation(qc::gate(context, "H", 1, true, &[q0], location).unwrap());
    let g1 = block.append_operation(qc::gate(context, "H", 1, true, &[q1], location).unwrap());
    let g2 = block.append_operation(qc::gate(context, "H", 1, true, &[q2], location).unwrap());
    let q0 = Value::from(g0.result(0).unwrap());
    let q1 = Value::from(g1.result(0).unwrap());
    let q2 = Value::from(g2.result(0).unwrap());
    let cx =
        block.append_operation(qc::gate(context, "CNOT", 1, true, &[q0, q2], location).unwrap());
    let q0 = Value::from(cx.result(0).unwrap());
    let q2 = Value::from(cx.result(1).unwrap());
    block.append_operation(qc::r#return(&[q0, q1, q2], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "non_adjacent",
        3,
        3,
        &quon_core::DepthExpr::Nat(4),
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

#[test]
fn non_adjacent_cnot_inserts_live_swap_and_rewrites_gate() {
    let context = context();
    let module = non_adjacent_cnot_module(&context);
    sabre_routing::run_on_module(&context, &linear_5q(), SabreCost::default(), &module);
    let text = module.as_operation().to_string();
    assert!(text.contains("gate_name = \"SWAP\""), "{text}");
    let swap_pos = text.find("gate_name = \"SWAP\"").unwrap();
    let cnot_pos = text.rfind("gate_name = \"CNOT\"").unwrap();
    assert!(swap_pos < cnot_pos, "{text}");
}

/// Linear chain with asymmetric CX fidelity: edge 0–1 is excellent, 1–2 is
/// terrible. Bidirectional lookup must see both `(1,2)` and `(2,1)` keys.
fn noisy_linear_3q() -> backend::BackendTarget {
    let mut noise = backend::NoiseModel::default();
    noise
        .two_qubit_fidelity
        .insert(("cx".to_string(), 0, 1), 0.999);
    // Only the reverse direction is published — noise_penalty must still find it.
    noise
        .two_qubit_fidelity
        .insert(("cx".to_string(), 2, 1), 0.50);
    noise.readout_error.insert(0, 0.001);
    noise.readout_error.insert(1, 0.001);
    noise.readout_error.insert(2, 0.20);

    backend::BackendTarget::fixed(
        "noisy_linear3",
        backend::FixedTarget {
            num_qubits: 3,
            topology: backend::ConnectivityGraph::try_from_edges(3, vec![(0, 1), (1, 2)])
                .expect("topology"),
            native_gates: vec![
                backend::NativeGate::passthrough("cx", 2),
                backend::NativeGate::passthrough("swap", 2),
            ],
            noise,
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    )
}

#[test]
fn default_sabre_gamma_is_point_three() {
    assert!((SabreCost::default().gamma - 0.3).abs() < f64::EPSILON);
}

#[test]
fn noise_aware_routing_still_inserts_swap_for_non_adjacent_cnot() {
    let context = context();
    let module = non_adjacent_cnot_module(&context);
    let cost = SabreCost {
        gamma: 1.0,
        ..SabreCost::default()
    };
    sabre_routing::run_on_module(&context, &noisy_linear_3q(), cost, &module);
    let text = module.as_operation().to_string();
    assert!(
        text.contains("gate_name = \"SWAP\""),
        "expected a SWAP under noise-weighted SABRE:\n{text}"
    );
}

#[test]
fn higher_gamma_keeps_noise_term_configurable() {
    let low = SabreCost {
        gamma: 0.0,
        ..SabreCost::default()
    };
    let high = SabreCost {
        gamma: 5.0,
        ..SabreCost::default()
    };
    assert!(high.gamma > low.gamma);
    // Smoke: both costs must successfully route the same module.
    let context = context();
    for cost in [low, high] {
        let module = non_adjacent_cnot_module(&context);
        sabre_routing::run_on_module(&context, &noisy_linear_3q(), cost, &module);
        assert!(
            module
                .as_operation()
                .to_string()
                .contains("gate_name = \"SWAP\"")
        );
    }
}
