#!/usr/bin/env python3
"""Shor's algorithm quantum kernel verification on Qiskit Aer (issue #1, MVP
milestone M3f).

`shor.qn` composes recursion, controlled(Rz), tensored, split, and
adjoint(qft(n)) together — every individual feature is already Aer-verified
elsewhere (test/verify/qft.py), so this checks the composition holds: the
circuit compiles, executes, and produces a reproducible (fixed-seed-stable)
distribution confined to the two outcomes the schematic modmul (see the
fixture's docstring for why it isn't a real modular exponentiation, hence no
"period-finding peaks" to check) actually produces — not an error, and not a
distribution spread arbitrarily over all 4 two-bit outcomes, which is what a
linearity/wiring bug corrupting tensored/split would look like.

Run:  QUONC=target/debug/quonc python test/verify/shor.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "shor.qn")
EXPECTED_OUTCOMES = {"00", "01"}


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts_a = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    counts_b = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    print(f"run 1: {counts_a}")
    print(f"run 2: {counts_b}")

    if counts_a != counts_b:
        print("FAIL: not reproducible with a fixed sampler seed")
        return 1

    seen = {key.replace(" ", "") for key in counts_a}
    if not seen <= EXPECTED_OUTCOMES:
        print(f"FAIL: saw outcomes {seen}, expected a subset of {EXPECTED_OUTCOMES}")
        return 1

    print(f"PASS: shor.qn compiles, runs reproducibly, and stays within {EXPECTED_OUTCOMES}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
