// Native-gate registry — see issue #3.
//
// Maps OpenQASM 3.0 standard gate names to a `NativeGate` carrying the gate's
// qubit arity and a (currently identity) decomposition. This is how a JSON
// descriptor's `Vec<String>` of gate names becomes a `Vec<NativeGate>`.

use crate::error::BackendError;
use crate::target::NativeGate;

/// Arity of every gate in the OpenQASM 3.0 standard library we recognize.
///
/// Single source of truth for both name resolution and the `generic_openqasm`
/// native set, so the two cannot drift apart.
pub const STD_GATES: &[(&str, usize)] = &[
    ("h", 1),
    ("x", 1),
    ("y", 1),
    ("z", 1),
    ("s", 1),
    ("sdg", 1),
    ("sx", 1),
    ("t", 1),
    ("tdg", 1),
    ("rx", 1),
    ("ry", 1),
    ("rz", 1),
    ("u1", 1),
    ("u2", 1),
    ("u3", 1),
    ("cx", 2),
    ("cy", 2),
    ("cz", 2),
    ("swap", 2),
    ("ccx", 3),
];

/// Resolve a gate name to a [`NativeGate`] with an identity decomposition.
/// Returns [`BackendError::UnknownGate`] for names not in [`STD_GATES`].
pub fn native_gate(name: &str) -> Result<NativeGate, BackendError> {
    STD_GATES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, arity)| NativeGate::passthrough(*n, *arity))
        .ok_or_else(|| BackendError::UnknownGate(name.to_string()))
}
