#!/usr/bin/env python3
"""Aer verification for `edit_verify_loop.qn` (issue #188).

Patterned directly on `test/verify/bell.py`'s compile -> simulate -> compare
shape, via the shared `python/quon_aer.py` seam (issue #29/#204): this is the
"add an Aer checker" half of the edit -> verify loop workflow, kept in the
workflow's own directory (not `test/verify/`) since it checks a samples/
narrative fixture, not a compiler correctness oracle.

`prepared_qubit` is checked in with `I @0`, so the expected distribution is
the single point mass {"0": 1.0}. If you've done the README's edit (swapping
`I @0` for `X @0`), pass `--expect 1` to check the flipped state instead.

Run:  QUONC=target/debug/quonc python samples/workflows/edit_verify_loop/verify_edit_verify_loop.py
"""

import argparse
import os
import sys

REPO_ROOT = os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 1024
SEED = 188
SOURCE = os.path.join(REPO_ROOT, "samples", "workflows", "edit_verify_loop", "edit_verify_loop.qn")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--expect",
        choices=["0", "1"],
        default="0",
        help="expected fixed measurement outcome (0 as checked in; 1 after the README's X @0 edit)",
    )
    args = parser.parse_args()

    result = quon_aer.verify_distribution(
        SOURCE, {args.expect: 1.0}, shots=SHOTS, seed=SEED, min_fidelity=0.99
    )
    print(f"counts: {result.counts}")
    print(result.message)
    if not result.ok:
        print(f"FAIL: expected a fixed point at |{args.expect}>")
        return 1
    print("PASS: edit_verify_loop.qn verified on Aer")
    return 0


if __name__ == "__main__":
    sys.exit(main())
