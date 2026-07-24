#!/usr/bin/env python3
"""Quantum Phase Estimation verification on Qiskit Aer (issue #186, textbook
algorithm pack).

Compiles `samples/algorithms/phase_estimation.qn` (t=1 counting qubit, U =
Rz(2*pi) = -I, eigenstate |1>) and checks that every shot yields (counting,
eigenstate) = (1, 1), confirming the estimated phase phi = 1/2 (binary 0.1).

This is the smallest non-trivial QPE: H creates a superposition, CZ kicks
the eigenvalue phase (-1) back onto the counting qubit, and a final H
converts it to a deterministic computational-basis measurement. The "inverse
QFT" for t=1 is just a single H gate.

Inspired by the Qiskit textbook "Quantum Phase Estimation" chapter:
  https://qiskit.org/textbook/ch-algorithms/quantum-phase-estimation.html

Run:  QUONC=target/release/quonc python test/verify/phase_estimation.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "samples", "algorithms", "phase_estimation.qn")
# (counting_bit, eigenstate_bit) -- phi = 0.1 = 1/2
EXPECTED = (1, 1)


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    nbits = len(quon_aer.normalize_key(next(iter(counts))))

    recovered = {tuple(quon_aer.clbit(key, i, nbits) for i in range(nbits)) for key in counts}

    print(f"counts: {counts}")
    print(f"distinct outcomes: {recovered}")
    if recovered != {EXPECTED}:
        print(f"FAIL: outcomes are not the constant {EXPECTED}")
        return 1
    counting_bit, _ = EXPECTED
    print(f"PASS: QPE estimated phase phi = 0.{counting_bit} = 1/2 "
          f"(counting bit = {counting_bit}, eigenstate confirmed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
