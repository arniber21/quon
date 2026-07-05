#!/usr/bin/env python3
"""QAOA (p=1) MaxCut verification on Qiskit Aer (issue #1, MVP milestone M3d).

Compiles `qaoa.qn` (n=3, MaxCut on the complete graph K3) and checks every
min-energy bitstring (the 6 states with exactly one or two 1s, MaxCut = 2 —
K3 is an odd cycle, so no bipartition cuts all 3 edges) is strictly more
probable than every non-optimal one (000, 111; MaxCut = 0) — the PRD's
stated QAOA acceptance criterion, phrased this way because K3's symmetry
makes the 6 optimal bitstrings an exact tie, not a single winner.

Run:  QUONC=target/debug/quonc python test/verify/qaoa.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "qaoa.qn")
NON_OPTIMAL = {"000", "111"}


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    normalized = {key.replace(" ", ""): n for key, n in counts.items()}
    print(f"counts: {normalized}")

    optimal = {k: v for k, v in normalized.items() if k not in NON_OPTIMAL}
    non_optimal = {k: v for k, v in normalized.items() if k in NON_OPTIMAL}
    min_optimal = min(optimal.values()) if optimal else 0
    max_non_optimal = max(non_optimal.values()) if non_optimal else 0

    print(f"min P(optimal) = {min_optimal / SHOTS:.4f}, max P(non-optimal) = {max_non_optimal / SHOTS:.4f}")
    if not optimal or min_optimal <= max_non_optimal:
        print("FAIL: a MaxCut = 0 bitstring (000/111) is at least as probable as a MaxCut = 2 one")
        return 1
    print("PASS: every MaxCut = 2 bitstring is strictly more probable than 000/111")
    return 0


if __name__ == "__main__":
    sys.exit(main())
