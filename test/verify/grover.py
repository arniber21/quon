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
    # A point-mass expected distribution reduces the Hellinger-fidelity
    # oracle to exactly P(marked) — see quon_aer.hellinger_fidelity's
    # docstring — so this is the same acceptance threshold as before #204's
    # refactor, just expressed via the shared oracle instead of a hand-rolled
    # `count / SHOTS` check.
    result = quon_aer.verify_distribution(
        SOURCE,
        expected={MARKED: 1.0},
        shots=SHOTS,
        seed=SEED,
        min_fidelity=0.9,
    )

    print(f"counts: {result.counts}")
    print(f"P(marked={MARKED}) = {result.fidelity}")
    if not result:
        print(f"FAIL: Grover success probability {result.fidelity} <= 0.9")
        return 1
    print(f"PASS: Grover recovered the marked state |{MARKED}> with P = {result.fidelity}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
