#!/usr/bin/env python3
"""States & measurement verification on Qiskit Aer (Quon learning track, #187).

Compiles `samples/learning/states_measurement.qn` and checks the Born rule on
the two-qubit output: qubit 0 is left in |0> (computational basis) so c[0] is
0 on every shot, while qubit 1 is put into |+> by a Hadamard so c[1] is 0/1
each about half the time. The point of the lesson is that the basis qubit is
deterministic and the superposition qubit is a fair coin.

Run:  QUONC=target/release/quonc python samples/learning/states_measurement.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "samples", "learning", "states_measurement.qn")


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    nbits = len(quon_aer.normalize_key(next(iter(counts))))

    # c[0] is the basis qubit (always 0); c[1] is the superposition qubit (~50/50).
    basis_zero = sum(n for key, n in counts.items() if quon_aer.clbit(key, 0, nbits) == 0)
    super_one = sum(n for key, n in counts.items() if quon_aer.clbit(key, 1, nbits) == 1)

    # The basis qubit must never read 1. The superposition qubit should land near
    # half the shots: tol = 0.08*4096 ~ 328, about 10 sigma for a fair coin over
    # 4096 shots (sigma = sqrt(4096*0.25) = 32).
    tol = 0.08 * SHOTS
    ok = (
        basis_zero == SHOTS
        and abs(super_one - SHOTS / 2) < tol
    )

    print(f"counts: {counts}")
    print(f"basis c[0]==0: {basis_zero}/{SHOTS} (expect {SHOTS})")
    print(f"superposition c[1]==1: {super_one}/{SHOTS} (expect ~{SHOTS // 2})")
    if not ok:
        print("FAIL: basis qubit leaked to 1, or superposition qubit is not ~50/50")
        return 1
    print("PASS: computational basis is deterministic, superposition follows the Born rule")
    return 0


if __name__ == "__main__":
    sys.exit(main())
