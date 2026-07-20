# Reconfigurable neutral-atom QEC benchmark suite (issue #284)

A compact benchmark suite for reconfigurable neutral-atom QEC compiler work.
The suite makes compiler tradeoffs visible across QEC memory, logical
measurement, logical entanglement, and (placeholder) non-Clifford workloads.

## Assumptions

All benchmarks use **generic public assumptions** from the
`targets/neutral_atom/generic_rna_v0.json` target descriptor. No proprietary
hardware parameters are used. Resource reports are **analytic compiler metrics
only** — not sampled data and not threshold claims (ADR-0020).

## Benchmarks

| Benchmark | Source | Description |
|-----------|--------|-------------|
| Surface d=3 memory | `samples/neutral-atom/benchmarks/surface_d3_memory.qn` | Two syndrome-extraction rounds then logical Z measure |
| Surface d=3 measure | `samples/neutral-atom/benchmarks/surface_d3_measure.qn` | One memory round then logical X measure |
| Surface d=3 CX | `samples/neutral-atom/benchmarks/surface_d3_cx.qn` | Lattice-surgery logical CX between two surface-code blocks |
| Surface d=3 GHZ | `samples/neutral-atom/benchmarks/surface_d3_ghz.qn` | Three-block GHZ-style prep-measure with two logical CX gates |
| Non-Clifford (placeholder) | `samples/neutral-atom/benchmarks/surface_d3_t_placeholder.qn` | Still a placeholder — #283 landed magic-state-consuming `logical_t`/`logical_ccz` at the QEC-workload IR / resource-report layer, but source-level `.qn` syntax for these ops isn't wired into the frontend yet (#311), so a source-driven T/CCZ benchmark can't compile through `quonc` yet |

## Compile commands

Each benchmark has a documented compile command in `samples/catalog.yaml`:

```bash
# Memory benchmark
quonc samples/neutral-atom/benchmarks/surface_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-resource-report -

# CX benchmark
quonc samples/neutral-atom/benchmarks/surface_d3_cx.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-resource-report -
```

## Emitted artifacts

Each benchmark produces a resource report with comparable metrics:

- **Atoms**: physical atom count, atoms per logical
- **Zones**: storage, entanglement, readout zone capacity
- **Moves**: rearrangement steps, rearrangement time
- **Transfers**: trap transfers, transfer time
- **Rydberg operations**: entangle2 count, entangle_n count, rydberg stages
- **Measurement rounds**: per-layer measurement count
- **Idle time**: wait time
- **Estimated cycles**: schedule layer count
- **QEC-specific counts**: code family, distance, memory rounds, error budget

## Interpreting results

Resource reports expose **analytic** schedule metrics. Two benchmarks with
the same code distance but different workload structure (e.g. memory vs CX)
will show different Rydberg stage counts, rearrangement steps, and estimated
cycles — these are the compiler tradeoffs the suite makes visible.

Non-Clifford benchmarks remain a placeholder. Issue #283 landed
magic-state-*consuming* logical T/CCZ as a QEC-workload IR / resource-report
model — see
[`docs/neutral_atom/magic_state_operations.md`](./magic_state_operations.md)
and `examples/na_qec/surface_d3_t.qn` / `surface_d3_ccz.qn` — but that work
did not wire `logical_t` / `logical_tdag` / `logical_ccz` into the frontend
typechecker, so those example programs do not yet compile through `quonc`
(`error: type checking failed: unbound variable`). A surface d=3 T/CCZ
benchmark will be added to this suite once #311 lands that frontend wiring.
