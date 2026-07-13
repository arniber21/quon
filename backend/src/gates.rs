// Native-gate registry adapter — see issue #3 / #209.
//
// Maps OpenQASM 3.0 standard gate names to a `NativeGate` carrying the gate's
// qubit arity and a (currently identity) decomposition. Metadata comes from
// `quon_core::gates` so typecheck and backend cannot drift apart.

use crate::error::BackendError;
use crate::target::NativeGate;
use quon_core::gates;

/// Arity of every gate in the OpenQASM 3.0 standard library we recognize.
///
/// Derived from [`quon_core::gates::REGISTRY`]; that module is the source of truth
/// when adding gates.
pub fn std_gates() -> &'static [(&'static str, usize)] {
    gates::std_gates_slice()
}

/// Resolve a gate name to a [`NativeGate`] with an identity decomposition.
/// Returns [`BackendError::UnknownGate`] for names not in the OpenQASM std set.
///
/// Resolution is by OpenQASM keyword (lowercase), matching the JSON descriptor
/// wire format — Quon surface names like `"H"` / `"CNOT"` are rejected.
pub fn native_gate(name: &str) -> Result<NativeGate, BackendError> {
    let info = gates::lookup(name).ok_or_else(|| BackendError::UnknownGate(name.to_string()))?;
    let Some(qasm) = info.openqasm else {
        return Err(BackendError::UnknownGate(name.to_string()));
    };
    // Descriptor / audit tests expect exact OpenQASM keyword match (case-sensitive).
    if name != qasm {
        return Err(BackendError::UnknownGate(name.to_string()));
    }
    Ok(NativeGate::passthrough(qasm, info.arity))
}
