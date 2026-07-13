#!/usr/bin/env python3
"""Bernstein-Vazirani verification on Qiskit Aer (issue #29).

Compiles `test/verify/bernstein_vazirani.qn` (secret s = 110 over 3 query qubits
+ 1 ancilla) and checks that a single shot recovers the secret exactly: the three
query bits (c[0], c[1], c[2]) are constant across every shot and equal s, while
the ancilla bit is irrelevant.

Run:  QUONC=target/debug/quonc python test/verify/bernstein_vazirani.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "bernstein_vazirani.qn")
SECRET = (1, 1, 0)  # (c0, c1, c2)


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    nbits = len(quon_aer.normalize_key(next(iter(counts))))

    recovered = {
        tuple(quon_aer.clbit(key, i, nbits) for i in range(3)) for key in counts
    }

    print(f"counts: {counts}")
    print(f"distinct (c0,c1,c2) across shots: {recovered}")
    if recovered != {SECRET}:
        print(f"FAIL: query bits are not the constant secret {SECRET}")
        return 1
    print(f"PASS: single-shot recovery of secret {SECRET}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
