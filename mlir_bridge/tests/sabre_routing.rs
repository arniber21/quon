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

/// Depth-sensitive fixture: CNOT(0,2) on a linear chain (needs one SWAP), then
/// CNOT(1,2). Both edges (0,1) and (1,2) equally reduce front-layer distance,
/// but only (1,2) preserves q1–q2 adjacency for the lookahead gate. High β
/// must therefore prefer a different SWAP than β = 0.
fn depth_sensitive_module(context: &melior::Context) -> Module<'_> {
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
    let cx0 =
        block.append_operation(qc::gate(context, "CNOT", 1, true, &[q0, q2], location).unwrap());
    let q0 = Value::from(cx0.result(0).unwrap());
    let q2 = Value::from(cx0.result(1).unwrap());
    let cx1 =
        block.append_operation(qc::gate(context, "CNOT", 1, true, &[q1, q2], location).unwrap());
    let q1 = Value::from(cx1.result(0).unwrap());
    let q2 = Value::from(cx1.result(1).unwrap());
    block.append_operation(qc::r#return(&[q0, q1, q2], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "depth_sensitive",
        3,
        3,
        &quon_core::DepthExpr::Nat(5),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

fn route_text(cost: SabreCost) -> String {
    let context = context();
    let module = depth_sensitive_module(&context);
    sabre_routing::run_on_module(&context, &linear_5q(), cost, &module);
    module.as_operation().to_string()
}

/// First SWAP in program order: operands are SSA results of the H gates on
/// physical qubits assigned in order (H@0 → %0, H@1 → %1, H@2 → %2), so
/// `(%0, %1)` means physical edge (0,1) and `(%1, %2)` means (1,2).
fn first_swap_operand_snippet(ir: &str) -> &str {
    let marker = "gate_name = \"SWAP\"";
    let swap_attr = ir.find(marker).expect("expected a SWAP in routed IR");
    let line_start = ir[..swap_attr].rfind("quantum.circ.gate").expect("SWAP op");
    let line_end = ir[swap_attr..]
        .find('\n')
        .map(|i| swap_attr + i)
        .unwrap_or(ir.len());
    &ir[line_start..line_end]
}

fn first_cnot_phys_qubit(ir: &str) -> i32 {
    let cnot_marker = "gate_name = \"CNOT\"";
    let mut from = 0usize;
    while let Some(rel) = ir[from..].find(cnot_marker) {
        let abs = from + rel;
        let line_start = ir[..abs].rfind("quantum.circ.gate").expect("CNOT op");
        let line_end = ir[abs..].find('\n').map(|i| abs + i).unwrap_or(ir.len());
        let line = &ir[line_start..line_end];
        if let Some(phys_rel) = line.find("phys_qubit = ") {
            let rest = &line[phys_rel + "phys_qubit = ".len()..];
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return num.parse().expect("phys_qubit i32");
        }
        from = abs + cnot_marker.len();
    }
    panic!("no CNOT with phys_qubit in:\n{ir}");
}

#[test]
fn default_sabre_beta_and_lookahead_match_spec() {
    let cost = SabreCost::default();
    assert!((cost.beta - 0.5).abs() < f64::EPSILON);
    assert_eq!(cost.lookahead, 20);
}

#[test]
fn high_beta_changes_swap_choice_on_depth_sensitive_circuit() {
    let no_beta = SabreCost {
        alpha: 1.0,
        beta: 0.0,
        gamma: 0.0,
        lookahead: 20,
    };
    let high_beta = SabreCost {
        alpha: 1.0,
        beta: 10.0,
        gamma: 0.0,
        lookahead: 20,
    };
    let without = route_text(no_beta);
    let with = route_text(high_beta);

    // β = 0: both (0,1) and (1,2) equally reduce front distance; edge order
    // picks physical SWAP (0,1). After that swap, q0 lives at phys 1.
    let swap0 = first_swap_operand_snippet(&without);
    assert!(
        swap0.contains("(%0, %1)"),
        "β=0 must SWAP physical endpoints 0–1 (H results %0,%1), got:\n{swap0}\nfull:\n{without}"
    );
    assert_eq!(
        first_cnot_phys_qubit(&without),
        1,
        "β=0: after SWAP(0,1), first CNOT's control q0 is at phys 1:\n{without}"
    );

    // High β: SWAP(1,2) preserves q1–q2 adjacency for the lookahead CNOT.
    // After that swap, q0 remains at phys 0; only one SWAP is needed.
    let swap1 = first_swap_operand_snippet(&with);
    assert!(
        swap1.contains("(%1, %2)"),
        "high β must SWAP physical endpoints 1–2 (H results %1,%2), got:\n{swap1}\nfull:\n{with}"
    );
    assert_eq!(
        first_cnot_phys_qubit(&with),
        0,
        "high β: after SWAP(1,2), first CNOT's control q0 stays at phys 0:\n{with}"
    );
    assert_eq!(
        with.matches("gate_name = \"SWAP\"").count(),
        1,
        "high β lookahead should need only one SWAP:\n{with}"
    );
    assert_ne!(
        without.matches("gate_name = \"SWAP\"").count(),
        1,
        "β=0 path should insert an extra SWAP for the follow-on CNOT:\n{without}"
    );
}

#[test]
fn zero_lookahead_disables_beta_effect() {
    let high_beta_no_window = SabreCost {
        alpha: 1.0,
        beta: 10.0,
        gamma: 0.0,
        lookahead: 0,
    };
    let no_beta = SabreCost {
        alpha: 1.0,
        beta: 0.0,
        gamma: 0.0,
        lookahead: 20,
    };
    let high_beta_with_window = SabreCost {
        alpha: 1.0,
        beta: 10.0,
        gamma: 0.0,
        lookahead: 20,
    };
    let zero_look = route_text(high_beta_no_window);
    let beta_off = route_text(no_beta);
    let full = route_text(high_beta_with_window);
    assert_eq!(
        first_swap_operand_snippet(&zero_look),
        first_swap_operand_snippet(&beta_off),
        "lookahead=0 must zero out β (empty W), matching β=0 SWAP endpoints"
    );
    assert_eq!(
        first_cnot_phys_qubit(&zero_look),
        first_cnot_phys_qubit(&beta_off)
    );
    assert!(
        first_swap_operand_snippet(&full).contains("(%1, %2)"),
        "non-zero lookahead must let β select SWAP(1,2):\n{full}"
    );
    assert_eq!(first_cnot_phys_qubit(&full), 0);
}
