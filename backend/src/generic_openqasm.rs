// generic_openqasm built-in target — see issue #3, SPEC.md §8.2.
//
// All-to-all connectivity, all OpenQASM 3.0 standard gates native, ASAP
// scheduling, no noise model. Used for all emission and IR tests in Phases 1–5.

use crate::gates::STD_GATES;
use crate::target::{BackendTarget, ConnectivityGraph, FixedTarget, NativeGate, NoiseModel};

/// The built-in `generic_openqasm` target over `num_qubits` qubits.
pub fn target(num_qubits: usize) -> BackendTarget {
    BackendTarget::fixed(
        "generic_openqasm",
        FixedTarget {
            num_qubits,
            // All-to-all is valid by construction, so no fallible path here.
            topology: ConnectivityGraph::all_to_all(num_qubits),
            native_gates: STD_GATES
                .iter()
                .map(|(name, arity)| NativeGate::passthrough(*name, *arity))
                .collect(),
            noise: NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    )
}
