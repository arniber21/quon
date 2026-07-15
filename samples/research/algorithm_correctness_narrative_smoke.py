#!/usr/bin/env python3
"""Smoke twin for `algorithm_correctness_narrative.ipynb` (issue #190).

Regenerates the notebook's two headline claims — Bernstein-Vazirani's
single-shot secret recovery and Grover's N=4/M=1 exact success probability —
by compiling the *linked, canonical* `test/verify/*.qn` fixtures (owned by
`test/verify/`, not forked into `samples/`) and comparing the sampled Aer
distribution against the closed-form theoretical prediction derived in the
notebook's prose.

This is a **research narrative regeneration script**, not a `test/verify/`
correctness oracle: `test/verify/bernstein_vazirani.py` and
`test/verify/grover.py` already gate compiler correctness for these fixtures
in `just ci-rust`. This script instead answers the notebook's question —
"does the *theoretical* probability I derived on paper match what Aer
actually samples?" — and is wired into that same CI loop's Aer/python
verify list, running alongside (not instead of) those two correctness
oracles (see `samples/research/README.md`'s narrative-vs-verifier split).

Run:
    python samples/research/algorithm_correctness_narrative_smoke.py

Requires `qiskit`, `qiskit-aer`, `qiskit-qasm3-import` (see
`python/requirements.txt`); skips (exit 0) if unavailable, matching
`python/test_quon_aer.py`'s optional-dependency handling.
"""

from __future__ import annotations

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

SHOTS = 4096
SEED = 190  # pinned so the sampled numbers printed below are reproducible

BV_SOURCE = os.path.join(REPO_ROOT, "test", "verify", "bernstein_vazirani.qn")
BV_SECRET = (1, 1, 0)  # (c0, c1, c2); see bernstein_vazirani.qn's header comment
# Closed form: one oracle query recovers an n-bit secret with probability 1
# (Bernstein & Vazirani 1997) — there is no distribution to sample, only a
# constant across every shot.
BV_THEORETICAL_P = 1.0

GROVER_SOURCE = os.path.join(REPO_ROOT, "test", "verify", "grover.qn")
GROVER_MARKED = "11"
# Closed form for N=4 (n=2), M=1 marked item: theta = asin(1/sqrt(N)) =
# asin(1/2) = pi/6; after r=1 iteration, P(marked) = sin((2r+1)*theta)^2 =
# sin(pi/2)^2 = 1.0 exactly — Grover's exact special case (see grover.qn's
# header comment). Import math lazily only where used, to keep the skip path
# dependency-free.
import math

GROVER_THEORETICAL_P = math.sin((2 * 1 + 1) * math.asin(1 / math.sqrt(4))) ** 2

# N=4, M=1, r=1 is Grover's *exact* special case (see the derivation above):
# a noiseless Aer simulation has no distribution to approximate, so the
# notebook's literal claim — "all 4096 sampled shots land on |11>", fidelity
# `1.0000` — is checked exactly below (both via this near-1.0 floor and the
# explicit shot-count equality check in `main`), not just above a
# sampling-noise-tolerant threshold.
MIN_FIDELITY = 0.999


def main() -> int:
    try:
        import quon_aer
    except ImportError as exc:
        print(f"SKIP: quon_aer unavailable ({exc})")
        return 0

    try:
        import qiskit  # noqa: F401
        import qiskit_aer  # noqa: F401
    except ImportError as exc:
        print(
            "SKIP: qiskit/qiskit-aer not installed — "
            f"pip install -r python/requirements.txt ({exc})"
        )
        return 0

    print("=== Bernstein-Vazirani (n=3, s=110) ===")
    print(f"theoretical P(single shot recovers s) = {BV_THEORETICAL_P}")
    try:
        qasm = quon_aer.compile_to_qasm(BV_SOURCE)
    except quon_aer.VerificationError as exc:
        print(f"FAIL: could not compile {BV_SOURCE}: {exc}")
        return 1
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    nbits = len(quon_aer.normalize_key(next(iter(counts))))
    recovered = {tuple(quon_aer.clbit(key, i, nbits) for i in range(3)) for key in counts}
    print(f"sampled distinct (c0,c1,c2) across {SHOTS} shots: {recovered}")
    bv_ok = recovered == {BV_SECRET}
    print(
        f"{'PASS' if bv_ok else 'FAIL'}: sampled recovery rate = "
        f"{'1.0 (constant)' if bv_ok else 'not constant at secret'} "
        f"vs theoretical {BV_THEORETICAL_P}"
    )

    print()
    print("=== Grover (N=4, M=1 marked |11>, r=1) ===")
    print(f"theoretical P(marked) = sin(pi/2)^2 = {GROVER_THEORETICAL_P:.6f}")
    try:
        grover_result = quon_aer.verify_distribution(
            GROVER_SOURCE,
            expected={GROVER_MARKED: GROVER_THEORETICAL_P},
            shots=SHOTS,
            seed=SEED,
            min_fidelity=MIN_FIDELITY,
        )
    except quon_aer.VerificationError as exc:
        print(f"FAIL: could not compile {GROVER_SOURCE}: {exc}")
        return 1
    print(f"sampled counts: {grover_result.counts}")
    print(
        f"{grover_result.message} "
        f"(theoretical target {GROVER_THEORETICAL_P:.6f}, "
        f"min_fidelity {MIN_FIDELITY})"
    )

    # Exact-number check: the notebook claims *every* one of the 4096 shots
    # lands on the marked state (not just "most of them, within tolerance").
    # Assert that literally, rather than trusting the fidelity floor alone
    # to stand in for it.
    grover_exact = grover_result.counts == {GROVER_MARKED: SHOTS}
    if not grover_exact:
        print(
            f"FAIL: expected every shot on {GROVER_MARKED!r} "
            f"({{{GROVER_MARKED!r}: {SHOTS}}}), got {grover_result.counts} — "
            "update algorithm_correctness_narrative.ipynb's Grover claim if "
            "this is no longer exact"
        )
        return 1

    if not bv_ok or not grover_result or not grover_exact:
        print("\nFAIL: sampled distribution did not match the theoretical narrative")
        return 1
    print(
        "\nPASS: both sampled distributions match their theoretical prediction "
        "exactly (BV: 4096/4096 recover the secret; Grover: 4096/4096 land on "
        "the marked state)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
