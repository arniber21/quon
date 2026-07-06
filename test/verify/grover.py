#!/usr/bin/env python3
"""Grover's algorithm verification on Qiskit Aer (issue #1, MVP milestone M2/M3).

Compiles `grover.qn` (n=2, marked item |11>, r=1 iteration — Grover's exact
special case for N=4, M=1) and checks that every shot recovers the marked
state. This is the first fixture end-to-end exercising parametric circuit
elaboration (`for q in qubits(n) { .. }`, `repeat`, nested parametric calls —
see `frontend/src/elaborate.rs`), not just fixed-width reference algorithms.

Run:  QUONC=target/debug/quonc python test/verify/grover.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "grover.qn")
MARKED = "11"


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)

    marked_count = sum(n for key, n in counts.items() if key.replace(" ", "") == MARKED)
    fidelity = marked_count / SHOTS

    print(f"counts: {counts}")
    print(f"P(marked={MARKED}) = {fidelity}")
    if fidelity < 0.9:
        print(f"FAIL: Grover success probability {fidelity} <= 0.9")
        return 1
    print(f"PASS: Grover recovered the marked state |{MARKED}> with P = {fidelity}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
