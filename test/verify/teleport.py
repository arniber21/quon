#!/usr/bin/env python3
"""Quantum-teleportation verification on Qiskit Aer (issue #29).

Compiles `test/verify/teleport.qn`, which prepares the message qubit in |1>,
teleports it to `bob` through a Bell pair with feed-forward X/Z corrections, and
measures all three qubits. Bob's outcome (the highest-index classical bit) must
be 1 on every shot — fidelity 1.0 for the |1> message — while the two Bell-
measurement bits are random.

Run:  QUONC=target/debug/quonc python test/verify/teleport.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "teleport.qn")


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)

    # Qiskit prints classical bits high-index-first, so Bob (c[2]) is the
    # leftmost character of each outcome string.
    total = sum(counts.values())
    bob_one = sum(n for key, n in counts.items() if key.replace(" ", "")[0] == "1")
    fidelity = bob_one / total

    print(f"counts: {counts}")
    print(f"P(bob=1) = {fidelity}")
    if fidelity < 0.99:
        print("FAIL: teleported |1> not recovered with fidelity > 0.99")
        return 1
    print("PASS: teleportation recovered |1> with feed-forward corrections")
    return 0


if __name__ == "__main__":
    sys.exit(main())
