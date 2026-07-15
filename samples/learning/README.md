# Learning samples

Small, self-contained programs for someone new to Quon: one concept per
file, narrated with comments, no dependencies on other samples.

## Status

No dedicated pack — contributions are welcome directly against this
category. Keep new seeds small (one core concept) and self-contained.

## Seeds

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
