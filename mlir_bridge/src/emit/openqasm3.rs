// OpenQASM 3.0 emitter — see issue #27, SPEC.md §9.1
//
// Performs a linear traversal of the quantum.dynamic IR via Melior's walk API
// and generates OpenQASM 3.0 source text.
//
// A quantum.circ.gate with native_gate=false is an emitter error —
// the native gate decomposition pass must have run first.

use thiserror::Error;

/// Errors raised while emitting OpenQASM 3.0 from quantum.dynamic IR.
#[derive(Debug, Error)]
pub enum EmitError {
    /// A gate was not lowered to native form before emission.
    #[error("gate `{name}` is not native — run native gate decomposition first")]
    NonNativeGate { name: String },
}

pub fn emit(_module: &melior::ir::Module) -> Result<String, EmitError> {
    todo!("OpenQASM 3.0 emitter — see issue #27")
}
