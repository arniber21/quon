#!/usr/bin/env python3
"""SABRE routing (#25) end-to-end verification on Qiskit Aer.

Compiles `bernstein_vazirani.qn` against two topology-constrained targets
whose native gate sets lack `swap`, forcing real SWAP insertion + native gate
decomposition for the oracle's `CNOT @(0, 3)`:

  * `device_5q.json`      — needs exactly one swap (0-2-3 path, dist 2)
  * `device_linear_chain.json` — a bare 0-1-2-3 chain, needs cascading swaps

Regression lock for three correctness bugs found wiring the MVP pass pipeline
(issue #1, milestone M1) into `sabre_routing.rs`: a bystander qubit displaced
by an unrelated swap kept a stale SSA operand on both its next gate and the
block terminator, and `wires[logical]` was never updated to a gate's own
result. Both bugs silently routed a later gate (or a measurement) onto the
wrong physical qubit while still round-tripping the linearity verifier (each
value was still used exactly once — just the wrong one). The routed circuit
must recover the same secret as the unrouted `generic_openqasm` baseline
(bernstein_vazirani.py).

Run:  QUONC=target/debug/quonc python test/verify/routing.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "bernstein_vazirani.qn")
SECRET = (1, 1, 0)  # (c0, c1, c2) — same secret as bernstein_vazirani.py
TARGETS = ("device_5q.json", "device_linear_chain.json")


def clbit(key: str, k: int, nbits: int) -> int:
    """Value of classical bit c[k]; Qiskit prints bits high-index-first."""
    return int(key.replace(" ", "")[nbits - 1 - k])


def verify_target(target_name: str) -> bool:
    target_path = os.path.join(REPO_ROOT, "backend", "tests", "fixtures", target_name)
    qasm = quon_aer.compile_to_qasm(SOURCE, target=target_path)
    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    nbits = len(next(iter(counts)).replace(" ", ""))

    recovered = {tuple(clbit(key, i, nbits) for i in range(3)) for key in counts}

    print(f"{target_name} counts: {counts}")
    print(f"{target_name} distinct (c0,c1,c2) across shots: {recovered}")
    if recovered != {SECRET}:
        print(f"FAIL: {target_name} query bits are not the constant secret {SECRET}")
        return False
    return True


def main() -> int:
    results = [verify_target(target) for target in TARGETS]
    if not all(results):
        return 1
    print(f"PASS: routing recovered secret {SECRET} on {', '.join(TARGETS)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
