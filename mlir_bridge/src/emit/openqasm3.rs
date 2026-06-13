// OpenQASM 3.0 emitter — see issue #27, SPEC.md §9.1
//
// Performs a linear traversal of the quantum.dynamic IR via Melior's walk API
// and generates OpenQASM 3.0 source text.
//
// A quantum.circ.gate with native_gate=false is an emitter error —
// the native gate decomposition pass must have run first.

pub fn emit(_module: &melior::ir::Module) -> Result<String, anyhow::Error> {
    todo!("OpenQASM 3.0 emitter — see issue #27")
}
