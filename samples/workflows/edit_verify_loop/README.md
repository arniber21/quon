# Edit → verify loop

The core algorithm-development loop: change one line of `.qn`, typecheck it,
emit QASM, and re-verify on Aer — repeat. This walkthrough also doubles as
the "add an Aer checker patterned on `test/verify`" story for pack #188.

## Status

Owned by pack [#188](https://github.com/arniber21/quon/issues/188). Catalog
id: `workflows/edit-verify-loop`.

## The loop

`edit_verify_loop.qn` prepares a fixed single-qubit state with `I @0` — no
entanglement, so the loop stays about the *process*, not the physics. Run it
as checked in:

```bash
cargo build -p quonc
export QUONC=$PWD/target/debug/quonc

# 1. typecheck + lower + emit QASM
$QUONC --emit-qasm samples/workflows/edit_verify_loop/edit_verify_loop.qn

# 2. Aer-verify the emitted circuit against the expected fixed point
pip install -r python/requirements.txt   # once, for qiskit + qiskit-aer
python samples/workflows/edit_verify_loop/verify_edit_verify_loop.py
```

Expect QASM ending in `c[0] = measure q[0];` with no gate before it, and:

```text
counts: {'0': 1024}
PASS: Hellinger fidelity 1.0000 >= 0.99 vs expected {'0': 1.0}
PASS: edit_verify_loop.qn verified on Aer
```

Now make the edit: change `I @0` to `X @0` in `edit_verify_loop.qn` and
re-run the same two commands, passing `--expect 1` to the checker:

```bash
$QUONC --emit-qasm samples/workflows/edit_verify_loop/edit_verify_loop.qn
python samples/workflows/edit_verify_loop/verify_edit_verify_loop.py --expect 1
```

The emitted QASM gains an `x q[0];` line, and the verified distribution
flips to `{'1': 1024}` — that's the whole loop: an edit, a typecheck, and a
number that changes in exactly the way you predicted. Revert the edit before
committing (this file's catalog row is `ci: smoke`, pinned to the `I @0`
fixed point).

## Adding an Aer checker (the `test/verify` pattern)

`verify_edit_verify_loop.py` is a from-scratch, minimal instance of the same
seam every `test/verify/*.py` compiler fixture uses — compare it side by side
with [`test/verify/bell.py`](../../../test/verify/bell.py):

1. **Compile**: `quon_aer.compile_to_qasm(source)` (or, as here,
   `quon_aer.verify_distribution` does this internally) invokes `quonc
   --emit-qasm` via the `QUONC` env var / `PATH`.
2. **Simulate**: `quon_aer.run_on_aer(qasm, shots, seed)` runs the QASM on
   `AerSimulator`, pinning `seed` for a reproducible shot distribution.
3. **Compare**: `quon_aer.verify_distribution(source, expected, ...)` folds
   both steps together and passes when the Hellinger fidelity between the
   observed and expected distributions clears `min_fidelity` (default 0.99).

The only workflow-specific code is the `SOURCE` path and the expected
point-mass distribution (`{"0": 1.0}` or `{"1": 1.0}`) — everything else is
`python/quon_aer.py`'s shared adapter stack. This is the template to copy
when adding a checker for your own workflow or sample: point `SOURCE` at
your `.qn` file, describe the distribution you expect, and let
`verify_distribution` do the compile/simulate/compare.

`verify_edit_verify_loop.py` runs in CI: `just ci-rust` appends it to the
same Aer verify list as `test/verify/*.py` (no new Python deps — it's the
`python/requirements.txt` already installed by `setup-python` for that
job), checked in at the default `--expect 0` fixed point.

## See also

- [`quonc` CLI reference](../../../website/src/content/docs/reference/quonc.md) —
  `--emit-qasm` and the full flag list.
- [Developer tooling guide](../../../website/src/content/docs/guides/tooling.md) —
  `quonfmt`/`quonlint` for the "edit" half of the loop.
- [`test/verify/`](../../../test/verify/) — the compiler's own canonical
  Aer-verified fixtures (Bell, GHZ-adjacent algorithms, QFT, Grover, …), the
  pattern this checker is patterned on.
