#!/usr/bin/env python3
"""Quantum dice verification on Qiskit Aer (creative pack, #200).

Compiles `samples/creative/quantum_dice.qn` (3 qubits, one Hadamard each in a
depth-1 parallel layer) and checks the 3-bit output is a fair 8-sided die:
every face 000..111 appears, each near 1/8 of the shots. The point is that a
uniform superposition measured on the computational basis yields a uniform
distribution -- the Born rule with equal amplitudes -- and that the `for`
loop's depth-1 layer rolls all three "coins" at once.

Run:  QUONC=target/release/quonc python test/verify/quantum_dice.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 8192
SEED = 20200
SOURCE = os.path.join(REPO_ROOT, "samples", "creative", "quantum_dice.qn")
NBITS = 3
FACES = 2**NBITS


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    normalized = {quon_aer.normalize_key(k): n for k, n in counts.items()}

    # Every one of the 8 faces must show up, and each must land near 1/8.
    # tol = 0.06*SHOTS ~= 492, about 11 sigma for a uniform multinomial cell
    # over 8192 shots (sigma = sqrt(8192 * 1/8 * 7/8) ~= 30).
    tol = 0.06 * SHOTS
    expected = SHOTS / FACES
    missing = [f for f in (format(i, f"0{NBITS}b") for i in range(FACES)) if f not in normalized]
    bad = [f for f, n in normalized.items() if abs(n - expected) > tol]

    print(f"counts: {normalized}")
    print(f"faces seen: {len(normalized)}/{FACES}, missing: {missing}")
    print(f"expected ~{expected:.0f} per face; tol {tol:.0f}; off-band: {bad}")
    ok = not missing and not bad and len(normalized) == FACES
    if not ok:
        print("FAIL: distribution is not a uniform 8-sided die")
        return 1
    print("PASS: every face appears, each ~1/8 -- a fair quantum die")
    return 0


if __name__ == "__main__":
    sys.exit(main())
