// quantum.dynamic dialect registration — see issue #6, SPEC.md §6.3
//
// Ops: measure (!qubit → !bit), reset (!qubit → !qubit),
//      unitary_region (embedded quantum.circ block),
//      if (!bit, two regions), barrier (variadic !qubit)
//
// Physical hardware attributes (phys_qubit, native_gate, fidelity) are
// accepted as optional — populated later by the physical lowering pass.

pub fn register_dialect(_ctx: &melior::Context) {
    todo!("quantum.dynamic dialect registration — see issue #6")
}
