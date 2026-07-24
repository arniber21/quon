#!/usr/bin/env python3
"""VQE ansatz energy -- exact statevector check + seeded Aer cross-check (#191).

Compiles `samples/applications/vqe_ansatz.qn` (a hardware-efficient Ry/CNOT
ansatz with baked-in optimized angles) and plays the role of the classical
VQE outer loop:

  1. Extracts the trial state's statevector from the compiled circuit
     (measurements stripped) and computes the exact expectation <H> for the
     model Hamiltonian H = 0.5 Z0 + 0.3 Z1 + 0.6 X0 X1 - 0.4 Z0 Z1, asserting
     it equals the exact ground energy E0 = -1.400 (the off-diagonal X0 X1
     term makes the ground state a genuine superposition, so a non-trivial
     ansatz is genuinely required). This is the quantitative check.
  2. Runs a seeded Aer shot of the SAME circuit and checks the resulting
     Z-basis measurement distribution is consistent with the statevector
     (Hellinger fidelity >= 0.99) -- the compiled circuit's measurements
     agree with its prepared state.

The classical optimizer that found the angles is external to Quon; this script
only evaluates the baked optimum, exactly as a VQE driver would at convergence.

Run:  QUONC=target/debug/quonc python test/verify/vqe_ansatz.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402
from qiskit.quantum_info import Statevector, SparsePauliOp  # noqa: E402

SHOTS = 8192
SEED = 1234
SOURCE = os.path.join(REPO_ROOT, "samples", "applications", "vqe_ansatz.qn")

# Model Hamiltonian and its exact ground energy (computed once, checked here).
H = SparsePauliOp.from_list([("ZI", 0.5), ("IZ", 0.3), ("XX", 0.6), ("ZZ", -0.4)])
E0 = -1.4


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)

    # 1. Exact energy from the compiled circuit's statevector.
    circuit = quon_aer.load_circuit(qasm)
    circuit.remove_final_measurements(inplace=True)
    sv = Statevector.from_instruction(circuit)
    energy = float(sv.expectation_value(H).real)
    print(f"VQE energy <H> = {energy:.6f} (exact ground E0 = {E0})")

    failures = []
    if abs(energy - E0) > 1e-3:
        failures.append(f"energy {energy:.6f} != ground {E0} (|diff|={abs(energy - E0):.2e})")

    # 2. Seeded Aer cross-check: the measurement distribution matches the state.
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    normalized = {quon_aer.normalize_key(k): n / SHOTS for k, n in counts.items()}
    # Statevector probabilities keyed on the same bitstring convention (q0 LSB).
    sv_probs = {format(i, "02b"): float(abs(sv.data[i]) ** 2) for i in range(4)}
    fidelity = quon_aer.hellinger_fidelity(sv_probs, normalized)
    print(f"Aer vs statevector Hellinger fidelity = {fidelity:.4f}")
    if fidelity < 0.99:
        failures.append(f"Aer/statevector fidelity {fidelity:.4f} < 0.99")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    print(f"PASS: baked VQE ansatz reaches ground energy {E0} and Aer matches the state")
    return 0


if __name__ == "__main__":
    sys.exit(main())
