#!/usr/bin/env python3
"""Transverse-field Ising model verification on Qiskit Aer (issue #1, MVP
milestone M3c).

Compiles `ising.qn` (n=4, first-order Trotterization) at t=0, where every
Rzz/Rx rotation angle is exactly 0 (tau = t/n_steps = 0) regardless of any
sign/angle-convention question, so the whole evolution is the identity and
measuring the default |0000> initial state must give all zeros — the PRD's
stated Ising acceptance criterion.

Run:  QUONC=target/debug/quonc python test/verify/ising.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 1024
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "ising.qn")
EXPECTED = "0000"


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    print(f"counts: {counts}")

    matches = sum(n for key, n in counts.items() if key.replace(" ", "") == EXPECTED)
    fidelity = matches / SHOTS
    print(f"P(result={EXPECTED}) = {fidelity}")
    if fidelity < 0.99:
        print(f"FAIL: Ising t=0 fidelity {fidelity} <= 0.99")
        return 1
    print(f"PASS: Ising t=0 evolution is the identity, recovered |{EXPECTED}> with P = {fidelity}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
