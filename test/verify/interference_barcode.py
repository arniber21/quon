#!/usr/bin/env python3
"""Interference barcode verification on Qiskit Aer (creative pack, #200).

Compiles `samples/creative/interference_barcode.qn` (3 qubits, each H |> T |>
H) and checks two things:

  1. The output histogram is monotone in Hamming weight: P(000) > P(one-1) >
     P(two-1s) > P(111). That is the signature of single-qubit H-T-H
     interference -- the T phase biases each qubit toward 0, so denser bitstrings
     are rarer.
  2. The shape is recognizably a "barcode": it then renders the measured
     histogram as ASCII bars so the interference fringe is visible, not just
     asserted.

P(0) = cos^2(pi/8) ~= 0.854 per qubit, so the fringe descends roughly
0.62 / 0.11 / 0.018 / 0.003 by popcount.

Run:  QUONC=target/release/quonc python test/verify/interference_barcode.py
"""

import os
import sys
from collections import defaultdict

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 8192
SEED = 20200
SOURCE = os.path.join(REPO_ROOT, "samples", "creative", "interference_barcode.qn")
NBITS = 3


def render(counts: dict[str, int]) -> str:
    """ASCII bar chart of each bitstring, sorted by bitstring (binary order)."""
    width = max(counts.values()) if counts else 1
    rows = []
    for face in (format(i, f"0{NBITS}b") for i in range(2**NBITS)):
        n = counts.get(face, 0)
        bar = "#" * round(40 * n / width)
        rows.append(f"  {face} |{bar:<40} {n}")
    return "\n".join(rows)


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    normalized = {quon_aer.normalize_key(k): n for k, n in counts.items()}

    by_pop = defaultdict(int)
    for k, n in normalized.items():
        by_pop[k.count("1")] += n
    weights = [by_pop[w] for w in range(NBITS + 1)]

    print("interference barcode (H |> T |> H per qubit):")
    print(render(normalized))
    print(f"\nby Hamming weight: {dict(enumerate(weights))}")

    # Monotone strictly descending by popcount (the H-T-H fringe signature).
    monotone = all(weights[i] > weights[i + 1] for i in range(NBITS))
    # 000 must dominate, 111 must be the rarest.
    dom = normalized.get("000", 0) == max(normalized.values())
    rare = normalized.get("111", 0) == min(normalized.values())

    print(f"monotone descending by weight: {monotone}")
    print(f"000 dominates / 111 rarest: {dom} / {rare}")
    ok = monotone and dom and rare
    if not ok:
        print("FAIL: histogram is not the H-T-H interference fringe")
        return 1
    print("PASS: fringe descends by Hamming weight -- interference, drawn as a barcode")
    return 0


if __name__ == "__main__":
    sys.exit(main())
