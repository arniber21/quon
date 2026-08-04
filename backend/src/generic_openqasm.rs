// generic_openqasm built-in target — see issue #3, SPEC.md §8.2.
//
// All-to-all connectivity, all OpenQASM 3.0 standard gates native, ASAP
// scheduling, no noise model. Used for all emission and IR tests in Phases 1–5.

use crate::gates::std_gates;
use crate::target::{BackendTarget, ConnectivityGraph, FixedTarget, NativeGate, NoiseModel};

/// The built-in `generic_openqasm` target over `num_qubits` qubits.
pub fn target(num_qubits: usize) -> BackendTarget {
    BackendTarget::fixed(
        "generic_openqasm",
        FixedTarget::new(
            ConnectivityGraph::all_to_all(num_qubits),
            std_gates()
                .iter()
                .map(|(name, arity)| NativeGate::passthrough(*name, *arity))
                .collect(),
            NoiseModel::default(),
            0.0,
            true,
            true,
        ),
    )
}
