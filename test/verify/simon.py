#!/usr/bin/env python3
"""Simon's algorithm verification on Qiskit Aer (issue #186, textbook
algorithm pack).

Compiles `samples/algorithms/simon.qn` (hidden string s = 110, n = 3 query +
3 work qubits), runs it on Aer, and recovers s via *classical post-
processing* over GF(2): collect measurement samples y (the query-register
bits) satisfying y . s = 0, build the constraint matrix, compute its null
space, and read off the non-zero solution.

This is the honest boundary described in the .qn file's docstring: Quon is
the quantum *circuit* (Hadamard layers + oracle + Hadamard layer), and the
classical outer loop (GF(2) linear algebra) lives here in Python -- just as
in the Qiskit textbook, but with the circuit itself a typed, depth-verified
Clifford value.

Inspired by the Qiskit textbook "Simon's Algorithm" chapter:
  https://qiskit.org/textbook/ch-algorithms/simon-part.html

Run:  QUONC=target/release/quonc python test/verify/simon.py
"""

import os
import sys

import numpy as np

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SHOTS = 4096
SEED = 1234  # pin the Aer sampler so the run is reproducible
SOURCE = os.path.join(REPO_ROOT, "samples", "algorithms", "simon.qn")
N = 3  # query-register width
SECRET = (1, 1, 0)  # hidden string s = 110


def gf2_null_space_1d(rows: list[tuple[int, ...]], n: int) -> tuple[int, ...] | None:
    """Gaussian elimination over GF(2).

    Given a list of constraint vectors y (each y . s = 0), find the non-zero
    null-space vector s.  Returns s if the null space is exactly 1-dimensional
    (i.e. s is unique up to scaling, which over GF(2) means s itself); returns
    None if the null space has dimension != 1 (under-determined or trivial).
    """
    if not rows:
        return None
    mat = np.array(rows, dtype=np.int64)  # shape (m, n)
    # Row-reduce to RREF over GF(2).
    m = mat.shape[0]
    pivot_cols: list[int] = []
    row = 0
    for col in range(n):
        # Find a pivot in this column at or below `row`.
        pivot = None
        for r in range(row, m):
            if mat[r, col]:
                pivot = r
                break
        if pivot is None:
            continue
        mat[[row, pivot]] = mat[[pivot, row]]
        for r in range(m):
            if r != row and mat[r, col]:
                mat[r] = (mat[r] + mat[row]) % 2
        pivot_cols.append(col)
        row += 1
    rank = len(pivot_cols)
    # Null space dimension = n - rank.  We want exactly 1 (unique non-zero s).
    if n - rank != 1:
        return None
    free_col = next(c for c in range(n) if c not in pivot_cols)
    # Back-substitute: set the free variable to 1, solve for pivots.
    s = np.zeros(n, dtype=np.int64)
    s[free_col] = 1
    for i, pc in enumerate(pivot_cols):
        s[pc] = int(mat[i, free_col])  # RREF: pivot row gives free-var coefficient
    return tuple(int(x) for x in s)


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = quon_aer.run_on_aer(qasm, shots=SHOTS, seed=SEED)
    nbits = len(quon_aer.normalize_key(next(iter(counts))))

    # Collect non-trivial y vectors from the query register (first N bits).
    y_rows: set[tuple[int, ...]] = set()
    for key in counts:
        y = tuple(quon_aer.clbit(key, i, nbits) for i in range(N))
        if y != (0,) * N:
            y_rows.add(y)

    print(f"counts: {counts}")
    print(f"non-trivial y samples: {sorted(y_rows)}")

    if not y_rows:
        print("FAIL: no non-trivial y samples collected (need at least one)")
        return 1

    s = gf2_null_space_1d(sorted(y_rows), N)
    if s is None:
        print("FAIL: null space is not 1-dimensional -- not enough independent samples")
        return 1

    print(f"recovered hidden string s = {s}")
    if s != SECRET:
        print(f"FAIL: expected s = {SECRET}, got {s}")
        return 1
    print(f"PASS: Simon's algorithm recovered hidden string s = {s}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
