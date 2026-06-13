// generic_openqasm built-in target — see issue #3, SPEC.md §8.2
//
// All-to-all connectivity, all OpenQASM 3.0 standard gates native,
// ASAP scheduling, no noise model. Used for all emission and IR tests
// in Phases 1–5.

use crate::target::{BackendTarget, ConnectivityGraph, NoiseModel};

pub fn target(num_qubits: usize) -> BackendTarget {
    let edges: Vec<(usize, usize)> = (0..num_qubits)
        .flat_map(|i| (0..num_qubits).filter(move |&j| j != i).map(move |j| (i, j)))
        .collect();

    BackendTarget {
        id: "generic_openqasm".into(),
        num_qubits,
        topology: ConnectivityGraph::new(num_qubits, edges),
        native_gates: vec![
            "h", "x", "y", "z", "s", "sdg", "t", "tdg",
            "rx", "ry", "rz", "cx", "cy", "cz", "swap",
            "ccx", "u1", "u2", "u3",
        ].into_iter().map(String::from).collect(),
        noise: NoiseModel::default(),
        meas_latency_us: 0.0,
        supports_mid_circuit_meas: true,
        supports_feed_forward: true,
    }
}
