# Learning samples

Small, self-contained programs for someone new to Quon: one concept per
file, narrated with comments, no dependencies on other samples.

## Status

No dedicated pack — contributions are welcome directly against this
category. Keep new seeds small (one core concept) and self-contained.

## Seeds

### Learning track (#187)

A six-lesson progressive track: states -> gates -> entanglement -> algorithms.
Each lesson is a short `.qn` a beginner can edit without reading SPEC.md, with
explicit teaching of Quon-specific concepts (linearity, the `Circuit<Q,N,D,F>`
indices, `|>` vs parallel layers). Read them in order; each links to the next.
The website presents the same track in order under the **Learning track**
section of the docs.

| # | Catalog id | Path | Concept |
| --- | --- | --- | --- |
| 1 | `learning/hello-quon` | [`hello_quon.qn`](./hello_quon.qn) | End-to-end: `circuit {}` value, `run {}` monad, `@` application, `measure`, the four `Circuit` indices |
| 2 | `learning/states-measurement` | [`states_measurement.qn`](./states_measurement.qn) | Computational basis vs superposition, the Born rule (Aer checker [`states_measurement.py`](./states_measurement.py)) |
| 3 | `learning/gates-composition` | [`gates_composition.qn`](./gates_composition.qn) | `\|>` (depth adds) vs a parallel `for` layer (depth = max), depth as a verified bound |
| 4 | `learning/linearity-borrow` | [`linearity_borrow.qn`](./linearity_borrow.qn) | No-cloning, linear consumption, ancilla discipline (`qreg` + `measure`); `borrow` blocks referenced |
| 5 | `learning/entanglement` | [`entanglement.qn`](./entanglement.qn) | Bell-pair correlation vs separable independence, measured in one shot |
| 6 | `learning/oracles-algorithms` | [`oracles_algorithms.qn`](./oracles_algorithms.qn) | Phase-kickback oracle seed; hand-off into the #186 textbook algorithms |

### Other seeds

| Catalog id | Path | Concept |
| --- | --- | --- |
| `learning/hello-bell` | [`hello_bell.qn`](./hello_bell.qn) | Circuits, sequential composition (`\|>`), circuit application (`@`), measurement |

For the genuine mid-circuit-measurement ("dynamic circuit") story that
builds on this pair's measurement step, see
[`neutral-atom/README.md`](../neutral-atom/README.md#3-dynamic-circuit--mid-circuit-measurement),
which walks through the real interleaved measure/entangle schedule on
[`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn),
and the [teleportation cookbook page](../../website/src/content/docs/cookbook/teleportation.mdx),
which shows the fixed/QASM-path feed-forward-correction version of a
related shape.

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). Every new file here needs a
matching row in [`../catalog.yaml`](../catalog.yaml).
