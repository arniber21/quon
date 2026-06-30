#!/usr/bin/env python3
"""Bell-state verification on Qiskit Aer (issue #29).

Compiles `test/verify/bell.qn` with quonc, runs it on AerSimulator, and asserts
the output distribution is the maximally-entangled {"00", "11"} with ~50/50
weight and (essentially) no "01"/"10" leakage.

Run:  QUONC=target/debug/quonc python test/verify/bell.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "bell.qn")


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)

    correlated = counts.get("00", 0) + counts.get("11", 0)
    leakage = counts.get("01", 0) + counts.get("10", 0)

    # The two correlated outcomes should each land near half the shots; allow a
    # generous statistical band. tol = 0.08*4096 ≈ 328, about 10σ for a fair
    # coin over 4096 shots (σ = sqrt(4096*0.25) = 32).
    tol = 0.08 * SHOTS
    ok = (
        leakage == 0
        and abs(counts.get("00", 0) - SHOTS / 2) < tol
        and abs(counts.get("11", 0) - SHOTS / 2) < tol
        and correlated == SHOTS
    )

    print(f"counts: {counts}")
    print(f"correlated(00+11)={correlated}/{SHOTS}, leakage(01+10)={leakage}")
    if not ok:
        print("FAIL: distribution is not a 50/50 Bell state")
        return 1
    print("PASS: Bell state verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
