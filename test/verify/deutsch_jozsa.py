#!/usr/bin/env python3
"""Deutsch-Jozsa verification on Qiskit Aer (issue #186, textbook algorithm pack).

Compiles `samples/algorithms/deutsch_jozsa.qn` (balanced oracle,
f(x) = x0 + x1 + x2 mod 2, n=3 query qubits + 1 ancilla) and checks that
every shot yields the query bits (1, 1, 1) -- non-zero, confirming f is
*balanced* rather than constant. A constant oracle would read (0, 0, 0).

Inspired by the Qiskit textbook "Deutsch-Jozsa Algorithm" chapter:
  https://qiskit.org/textbook/ch-algorithms/deutsch-jozsa.html

Run:  QUONC=target/release/quonc python test/verify/deutsch_jozsa.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "samples", "algorithms", "deutsch_jozsa.qn")
BALANCED_RESULT = (1, 1, 1)  # (b0, b1, b2) for balanced oracle s = 111


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    nbits = len(quon_aer.normalize_key(next(iter(counts))))

    recovered = {
        tuple(quon_aer.clbit(key, i, nbits) for i in range(3)) for key in counts
    }

    print(f"counts: {counts}")
    print(f"distinct (b0,b1,b2) across shots: {recovered}")
    if recovered != {BALANCED_RESULT}:
        print(f"FAIL: query bits are not the constant {BALANCED_RESULT} (balanced oracle)")
        return 1
    print(f"PASS: Deutsch-Jozsa identified f as balanced (query bits {BALANCED_RESULT})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
