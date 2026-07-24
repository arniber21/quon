#!/usr/bin/env python3
"""QAOA depth scaling on C5: p=1 vs p=2 -- seeded Aer comparison (#191).

Compiles BOTH `samples/applications/maxcut_c5_p1.qn` (one cost+mixer layer)
and `samples/applications/maxcut_c5_p2.qn` (two layers) on the SAME graph
(the 5-cycle C5, MaxCut = 4) and checks that the deeper p=2 ansatz reaches at
least as high an expected cut value as the p=1 baseline -- the empirical
content of "more QAOA depth buys more solution quality". It also reports the
compile-time depth bounds the typechecker proves for each (read straight off
the `Circuit<Q,N,D,Family>` indices in the source), making the depth-vs-quality
tradeoff explicit.

Run:  QUONC=target/debug/quonc python test/verify/maxcut_depth.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 8192
SEED = 1234
N = 5
C5 = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]
MAXCUT = 4
P1 = os.path.join(REPO_ROOT, "samples", "applications", "maxcut_c5_p1.qn")
P2 = os.path.join(REPO_ROOT, "samples", "applications", "maxcut_c5_p2.qn")
# Depth bounds the typechecker proves (hadamard:1 |>(cost:5 + mixer:1) per layer).
P1_DEPTH = 1 + (5 + 1)         # = 7
P2_DEPTH = 1 + 2 * (5 + 1)     # = 13


def cut_value(bs):
    return sum(1 for i, j in C5 if bs[N - 1 - i] != bs[N - 1 - j])


def expected_cut(source):
    qasm = quon_aer.compile_to_qasm(source)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    norm = {quon_aer.normalize_key(k): n for k, n in counts.items()}
    return sum(cut_value(bs) * n for bs, n in norm.items()) / SHOTS, norm


def main() -> int:
    ec1, c1 = expected_cut(P1)
    ec2, c2 = expected_cut(P2)
    print(f"p=1 (depth {P1_DEPTH}): expected cut = {ec1:.4f} of {MAXCUT}")
    print(f"  counts: {c1}")
    print(f"p=2 (depth {P2_DEPTH}): expected cut = {ec2:.4f} of {MAXCUT}")
    print(f"  counts: {c2}")
    print(f"depth ratio {P2_DEPTH}/{P1_DEPTH} = {P2_DEPTH / P1_DEPTH:.2f}x; "
          f"quality {ec2:.4f}/{ec1:.4f} = {ec2 / ec1:.3f}x")

    failures = []
    if ec1 < 0.85 * MAXCUT:
        failures.append(f"p=1 expected cut {ec1:.3f} < {0.85 * MAXCUT:.1f}")
    if ec2 < 0.85 * MAXCUT:
        failures.append(f"p=2 expected cut {ec2:.3f} < {0.85 * MAXCUT:.1f}")
    if ec2 < ec1:
        failures.append(f"p=2 expected cut {ec2:.4f} < p=1 {ec1:.4f} (depth did not help)")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    print(f"PASS: p=2 ({ec2:.4f}) >= p=1 ({ec1:.4f}); depth scales {P1_DEPTH}->{P2_DEPTH}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
