# Algorithm samples

Narrative walkthroughs of named quantum algorithms and canonical circuit
constructions — building on `learning/`'s primitives, but each still a
complete, runnable program.

## Status

No dedicated pack — contributions are welcome directly against this
category. Prefer a new algorithm/construction over duplicating one that
already has a canonical home in `test/verify/` (link it from your README
section instead of forking the `.qn`).

## Seeds

| Catalog id | Path | Concept |
| --- | --- | --- |
| `algorithms/ghz-state` | [`ghz_state.qn`](./ghz_state.qn) | n-qubit GHZ entanglement (generalizes `learning/hello-bell`'s Bell pair) |

For deeper algorithm fixtures already verified end-to-end on Aer, see
[`test/verify/`](../../test/verify/) (Grover, QFT, Bernstein–Vazirani, Shor,
Ising, QAOA) and the [website cookbook](../../website/src/content/docs/cookbook/).

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). Every new file here needs a
matching row in [`../catalog.yaml`](../catalog.yaml).
