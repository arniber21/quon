//! SABRE permutation helpers: track SWAP layout and reorder amplitudes.

use backend::unitary::Complex;

use super::circuit_spec::{CircuitSpec, GateInst, GateKind};
use super::statevector::State;

/// After simulating a post-SABRE circuit in **wire-slot** bit order (SSA extract
/// without crossing SWAP results), recover the map `perm[logical] = slot` so
/// [`permute_statevector`] yields a logical-ordered statevector.
///
/// Each `SWAP(a, b)` exchanges which logical qubit currently occupies slots
/// `a` and `b`. Non-SWAP gates do not change the layout.
pub fn slot_of_logical_after_swaps(width: u8, gates: &[GateInst]) -> Vec<u8> {
    let n = width as usize;
    let mut slot_to_logical: Vec<u8> = (0..width).collect();
    for gate in gates {
        if gate.kind == GateKind::SWAP && gate.qubits.len() == 2 {
            let a = gate.qubits[0] as usize;
            let b = gate.qubits[1] as usize;
            if a < n && b < n {
                slot_to_logical.swap(a, b);
            }
        }
    }
    let mut perm = vec![0u8; n];
    for (slot, &logical) in slot_to_logical.iter().enumerate() {
        perm[logical as usize] = slot as u8;
    }
    perm
}

/// Convenience: layout permutation for a full post-pass [`CircuitSpec`].
pub fn return_permutation(spec: &CircuitSpec) -> Vec<u8> {
    slot_of_logical_after_swaps(spec.width, &spec.gates)
}

/// Permute amplitudes so bit `i` of the output index is bit `perm[i]` of the input.
///
/// `perm[out_bit] = in_bit` — after permutation, output qubit `out_bit` holds
/// what was previously in input bit `in_bit`.
pub fn permute_statevector(state: &State, perm: &[u8]) -> State {
    let n = state.n as usize;
    assert_eq!(perm.len(), n);
    let dim = state.amps.len();
    let mut amps = vec![Complex::new(0.0, 0.0); dim];
    for old_idx in 0..dim {
        let mut new_idx = 0usize;
        for (out_bit, &in_bit) in perm.iter().enumerate() {
            if old_idx & (1 << in_bit as usize) != 0 {
                new_idx |= 1 << out_bit;
            }
        }
        amps[new_idx] = state.amps[old_idx];
    }
    State { n: state.n, amps }
}

/// Identity permutation `0..n`.
pub fn identity_perm(n: u8) -> Vec<u8> {
    (0..n).collect()
}
