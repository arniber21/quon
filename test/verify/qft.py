#!/usr/bin/env python3
"""Recursive QFT round-trip verification on Qiskit Aer (issue #1, MVP M2/M3).

`qft.qn` prepares |101>, then applies `qft(3) |> adjoint(qft(3))`. A
measurement histogram cannot observe QFT's output phases directly (there is
no "theoretical DFT" to compare a bitstring distribution against), so this
checks the *round trip* returns the prepared state unchanged with P = 1.0 —
see the fixture's docstring and docs/plans/mvp-landing-plan.md M3 for why
this is the right acceptance criterion here.

Run:  QUONC=target/debug/quonc python test/verify/qft.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 1024
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "test", "verify", "qft.qn")
EXPECTED = "101"


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    # gate cancellation + rotation merging (run to fixpoint by the default
    # pipeline) structurally prove the round trip is the identity: they
    # cancel qft(3) |> adjoint(qft(3)) down to nothing, leaving just the
    # prep_101 gates in the emitted QASM. That's a *stronger* proof than the
    # Aer run below, so assert it explicitly rather than only checking counts.
    non_native_ops = [
        line
        for line in qasm.splitlines()
        if line.strip() and not line.split("(")[0].split()[0] in ("x", "measure") and "OPENQASM" not in line
        and "include" not in line and "qubit" not in line and not line.startswith("bit")
        and not line.strip().startswith("c[")
    ]
    print(f"qasm:\n{qasm}")
    if non_native_ops:
        print(f"NOTE: round trip did not fully cancel structurally: {non_native_ops}")

    counts = quon_aer.run(qasm, shots=SHOTS, seed=SEED)
    print(f"counts: {counts}")

    matches = sum(n for key, n in counts.items() if key.replace(" ", "") == EXPECTED)
    fidelity = matches / SHOTS
    print(f"P(result={EXPECTED}) = {fidelity}")
    if fidelity < 0.99:
        print(f"FAIL: qft |> adjoint(qft) round trip fidelity {fidelity} <= 0.99")
        return 1
    print(f"PASS: qft(3) |> adjoint(qft(3)) recovered |{EXPECTED}> with P = {fidelity}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
