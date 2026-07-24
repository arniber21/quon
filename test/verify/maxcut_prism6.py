#!/usr/bin/env python3
"""MaxCut QAOA p=1 on the 6-vertex triangular prism -- seeded Aer check (#191).

Compiles `samples/applications/maxcut_prism6.qn` (p=1 QAOA, angles baked in
by an offline statevector sweep) and runs it on Aer, then checks two
quantitative properties of the cut distribution:

  1. The expected cut value of the sampled bitstrings is at least 0.80 of the
     MaxCut (7) -- i.e. >= 5.6 -- so the variational state genuinely biases
     toward good cuts, not just "compiles".
  2. The single most-probable bitstring is an optimal MaxCut cut (one of the
     6 bitstrings achieving cut = 7), so the circuit's mode is a real solution.

This is the "seeded Aer checker verifying the cut value" the issue asks for.

Run:  QUONC=target/debug/quonc python test/verify/maxcut_prism6.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 8192
SEED = 1234
SOURCE = os.path.join(REPO_ROOT, "samples", "applications", "maxcut_prism6.qn")
N = 6

# Triangular prism: two triangles (0,1,2),(3,4,5) plus matching (0,3),(1,4),(2,5).
EDGES = [(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (0, 3), (1, 4), (2, 5)]
MAXCUT = 7  # 7 of 9 edges (the two triangles force two uncut edges)


def cut_value(bs):
    s = 0
    for i, j in EDGES:
        if bs[N - 1 - i] != bs[N - 1 - j]:  # qubit i is bit at position N-1-i
            s += 1
    return s


def optimal_bitstrings():
    opt = set()
    for z in range(2 ** N):
        bs = format(z, f"0{N}b")
        if cut_value(bs) == MAXCUT:
            opt.add(bs)
    return opt


def main() -> int:
    optimal = optimal_bitstrings()
    assert len(optimal) == 6, f"expected 6 optimal cuts, found {len(optimal)}"

    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    normalized = {quon_aer.normalize_key(k): n for k, n in counts.items()}

    expected_cut = sum(cut_value(bs) * n for bs, n in normalized.items()) / SHOTS
    mode = max(normalized, key=normalized.get)
    mode_cut = cut_value(mode)
    threshold = 0.80 * MAXCUT

    print(f"counts: {normalized}")
    print(f"expected cut = {expected_cut:.3f} (MaxCut={MAXCUT}, threshold={threshold:.1f})")
    print(f"mode = {mode} (cut={mode_cut}, optimal={mode in optimal})")

    failures = []
    if expected_cut < threshold:
        failures.append(f"expected cut {expected_cut:.3f} < {threshold:.1f}")
    if mode not in optimal:
        failures.append(f"mode {mode} is not an optimal cut (cut={mode_cut} < {MAXCUT})")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    print(
        f"PASS: p=1 QAOA on the triangular prism reaches expected cut {expected_cut:.3f} "
        f">= {threshold:.1f} and its mode is an optimal (MaxCut={MAXCUT}) bitstring"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
