#!/usr/bin/env python3
"""Transverse-field Ising on a ring (periodic boundary) -- seeded Aer check
(#191).

Compiles `samples/applications/ising_ring.qn` (a 6-qubit ring with the closing
bond (5,0), first-order Trotterized) at t=0, where every Rzz/Rx rotation angle
is exactly 0 (tau = t/n_steps = 0) so the whole evolution is the identity,
and measuring the default |000000> initial state must give all zeros -- the
same t=0 boundary oracle as `ising.py`, applied to the new ring topology.

Run:  QUONC=target/debug/quonc python test/verify/ising_ring.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 1024
SEED = 1234
SOURCE = os.path.join(REPO_ROOT, "samples", "applications", "ising_ring.qn")
EXPECTED = "000000"


def main() -> int:
    result = quon_aer.verify_distribution(
        SOURCE,
        expected={EXPECTED: 1.0},
        shots=SHOTS,
        seed=SEED,
        min_fidelity=0.99,
    )
    print(f"counts: {result.counts}")
    print(f"P(result={EXPECTED}) = {result.fidelity}")
    if not result:
        print(f"FAIL: Ising ring t=0 fidelity {result.fidelity} <= 0.99")
        return 1
    print(
        f"PASS: Ising ring t=0 evolution is the identity, recovered |{EXPECTED}> "
        f"with P = {result.fidelity}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
