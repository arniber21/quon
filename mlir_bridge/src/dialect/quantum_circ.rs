// quantum.circ dialect registration — see issue #4, SPEC.md §6.2, ADR-0001
//
// Ops: func, gate, compose, tensor, adjoint, controlled, borrow
// Each op carries: in_qubits I64Attr, out_qubits I64Attr,
//                  depth DepthExprAttr, clifford BoolAttr
//
// Registered via Melior's #[melior::dialect] proc-macro (ADR-0001).
// The linearity verifier pass (checking every !qubit SSA value has exactly
// one use) is a standalone pass registered separately — see passes/mod.rs.

pub fn register_dialect(_ctx: &melior::Context) {
    todo!("quantum.circ dialect registration — see issue #4")
}
