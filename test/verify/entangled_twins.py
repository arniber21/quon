#!/usr/bin/env python3
"""Entangled-twins verification on Qiskit Aer (creative pack, #200).

Compiles `samples/creative/entangled_twins.qn` (two Bell pairs with opposite
correlations) and checks the magic-trick claim: pair (b0, b1) is always
correlated (|Phi+>, outcomes 00/11) and pair (b2, b3) is always
anti-correlated (|Psi+>, outcomes 01/10). The lesson is that entanglement is a
*determined relation* -- same or opposite -- set by state preparation, not a
runtime signal between the parties.

Run:  QUONC=target/release/quonc python test/verify/entangled_twins.py
"""

import os
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 8192
SEED = 20200
SOURCE = os.path.join(REPO_ROOT, "samples", "creative", "entangled_twins.qn")
NBITS = 4


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    normalized = {quon_aer.normalize_key(k): n for k, n in counts.items()}

    same = sum(n for k, n in normalized.items() if quon_aer.clbit(k, 0, NBITS) == quon_aer.clbit(k, 1, NBITS))
    opp = sum(n for k, n in normalized.items() if quon_aer.clbit(k, 2, NBITS) != quon_aer.clbit(k, 3, NBITS))

    # An outcome is legal iff it respects *both* correlations (b0==b1 and
    # b2!=b3). Derive legality from the predicates rather than a hardcoded set:
    # Qiskit prints bits high-index-first (b3 b2 b1 b0), so the four legal
    # strings depend on that ordering and are easy to transcribe wrong.
    leakage = {
        k: n
        for k, n in normalized.items()
        if quon_aer.clbit(k, 0, NBITS) != quon_aer.clbit(k, 1, NBITS)
        or quon_aer.clbit(k, 2, NBITS) == quon_aer.clbit(k, 3, NBITS)
    }

    print(f"counts: {normalized}")
    print(f"pair0 b0==b1 (correlated): {same}/{SHOTS}")
    print(f"pair1 b2!=b3 (anti-correlated): {opp}/{SHOTS}")
    print(f"leakage (illegal outcomes): {leakage}")
    ok = same == SHOTS and opp == SHOTS and not leakage
    if not ok:
        print("FAIL: correlations do not match |Phi+> (same) and |Psi+> (opposite)")
        return 1
    print("PASS: pair0 always agrees, pair1 always disagrees -- entanglement as a determined relation")
    return 0


if __name__ == "__main__":
    sys.exit(main())
