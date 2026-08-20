# Reconfigurable neutral-atom QEC benchmark suite (issue #284)

A compact benchmark suite for reconfigurable neutral-atom QEC compiler work.
The suite makes compiler tradeoffs visible across QEC memory, logical
measurement, logical entanglement, and non-Clifford (magic-state T/CCZ)
workloads.

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
| Surface d=3 T | `samples/neutral-atom/benchmarks/surface_d3_t.qn` | Magic-state-consuming logical T on one surface-code block (`t_count`) |
| Surface d=3 CCZ | `samples/neutral-atom/benchmarks/surface_d3_ccz.qn` | Magic-state-consuming logical CCZ on three surface-code blocks (`ccz_count`) |

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

# Non-Clifford (magic-state) benchmark
quonc samples/neutral-atom/benchmarks/surface_d3_t.qn \
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
- **Magic-state counts** (T/CCZ only): `t_count`, `tdag_count`, `ccz_count`, `magic_state_demand`

## Interpreting results

Resource reports expose **analytic** schedule metrics. Two benchmarks with
the same code distance but different workload structure (e.g. memory vs CX)
will show different Rydberg stage counts, rearrangement steps, and estimated
cycles — these are the compiler tradeoffs the suite makes visible.

The non-Clifford (T/CCZ) benchmarks are magic-state-*consuming* operations:
`logical_t` / `logical_tdag` / `logical_ccz` bind as source identifiers in the
frontend prelude (issue #311, landed via #354) and compile end-to-end through
`quonc`. They are a compiler model of magic-state consumption — not a validated
distillation factory or threshold claim. T/CCZ are recorded as resource-report
metadata (`t_count` / `ccz_count` / `magic_state_demand`) rather than expanded
to physical CNOT/measure/reset rounds; see
[`docs/neutral_atom/magic_state_operations.md`](./magic_state_operations.md).
