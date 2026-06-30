#!/usr/bin/env python3
"""Quantum-teleportation verification on Qiskit Aer (issue #29).

Runs two teleportation oracles:

* `teleport.qn` prepares |1> and measures Bob in the Z basis. This catches a
  broken X correction.
* `teleport_plus.qn` prepares |+> and measures Bob in the X basis. This catches a
  broken Z correction.

Run:  QUONC=target/debug/quonc python test/verify/teleport.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
Z_SOURCE = os.path.join(REPO_ROOT, "test", "verify", "teleport.qn")
X_SOURCE = os.path.join(REPO_ROOT, "test", "verify", "teleport_plus.qn")


def clbit(key: str, k: int, nbits: int) -> int:
    """Value of classical bit c[k]; Qiskit prints bits high-index-first."""
    return int(key.replace(" ", "")[nbits - 1 - k])


def verify_case(source: str, expected: int, label: str) -> bool:
    qasm = quon_aer.compile_to_qasm(source)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    nbits = len(next(iter(counts)).replace(" ", ""))
    result_bit = nbits - 1
    total = sum(counts.values())
    matches = sum(
        n for key, n in counts.items() if clbit(key, result_bit, nbits) == expected
    )
    fidelity = matches / total

    print(f"{label} counts: {counts}")
    print(f"{label} P(result={expected}) = {fidelity}")
    if fidelity < 0.99:
        print(f"FAIL: {label} teleportation fidelity <= 0.99")
        return False
    return True


def main() -> int:
    z_ok = verify_case(Z_SOURCE, expected=1, label="|1> / Z-basis")
    x_ok = verify_case(X_SOURCE, expected=0, label="|+> / X-basis")
    if not (z_ok and x_ok):
        return 1
    print("PASS: teleportation recovered |1> and |+> with feed-forward corrections")
    return 0


if __name__ == "__main__":
    sys.exit(main())
