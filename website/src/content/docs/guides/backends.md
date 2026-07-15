---
title: Backends and verification
description: Compile Quon programs for fixed gate-model targets, neutral-atom targets, and Qiskit Aer verification.
---

Quon separates the shared frontend from target-specific artifact generation.
Fixed gate-model targets produce OpenQASM 3. Reconfigurable neutral-atom
targets produce schedule JSON and resource reports. The Qiskit Aer bridge
verifies the OpenQASM path locally without a hardware account.

## Fixed gate-model targets

A fixed `BackendTarget` JSON file records the gate-model constraints associated
with a compilation:

- `id` names the target in diagnostics and metrics.
- `num_qubits` sets the available physical qubits.
- `topology.edges` lists directly connected qubit pairs. Routing inserts swaps
  when a two-qubit operation is not adjacent.
- `native_gates` lists the OpenQASM gate names the target accepts. The compiler
  decomposes unsupported operations before emission and rejects unknown gate
  names.
- `noise` can record gate fidelity, T1/T2 times, and readout error. T1 values
  can inform scheduling; the values are target metadata rather than an Aer
  simulator noise model.
- `meas_latency_us`, `supports_mid_circuit_meas`, and
  `supports_feed_forward` record measurement and dynamic-circuit capabilities.

The optional top-level `"kind": "fixed"` makes the architecture family
explicit. A descriptor without `kind` is read as a fixed target for backward
compatibility. See
[`backend/tests/fixtures/device_5q.json`](https://github.com/arniber21/quon/blob/main/backend/tests/fixtures/device_5q.json)
for a complete example.

Inspect a descriptor:

```bash
cargo run -p quonc -- \
  --target backend/tests/fixtures/device_5q.json \
  --print-target
```

## Emit OpenQASM 3

Compile a Bell-state program for the five-qubit fixture:

```bash
cargo run -p quonc -- \
  test/verify/bell.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm
```

With no `--target`, `quonc` uses the built-in `generic_openqasm` target: 64
all-to-all qubits, the standard OpenQASM gate set, and no device noise data.

## Neutral-atom targets

A `neutral_atom_reconfigurable` descriptor models a different architecture
family: zones, array geometry, AOD movement, Rydberg interactions, timing,
fidelity, and a cost model. Quon compiles these targets to schedule and
resource artifacts rather than OpenQASM.

```bash
cargo run -p quonc -- test/na/qaoa_graph.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json \
  --emit-na-graph graph.dot \
  --emit-resource-report report.md \
  --resource-report-format markdown
```

The neutral-atom path extracts the interaction graph, schedules entangling
layers, chooses a movement backend, optionally compacts the result, and reports
timing/resource estimates.

`--emit-na-schedule` writes a versioned visualization envelope
(`kind: na_schedule_view`) with zones, layout, metrics, and schedule layers —
a debug view, not the canonical schedule IR (`--emit-na-mlir`).
`--emit-na-graph` writes Graphviz DOT for the interaction graph.

Render frames / the graph with matplotlib + Graphviz (no HTML):

```bash
pip install -r python/requirements-viz.txt
python python/visualize_na_schedule.py schedule.json --graph graph.dot \
  -o /tmp/na-viz --format svg
```

`meta.na_placer` / `meta.na_backend` in the schedule JSON are reserved so a
future before/after (routing-agnostic vs routing-aware) comparison can share
axes without a schema bump.

Useful neutral-atom options:

- `--na-backend zoned` uses zoned RAP scheduling.
- `--na-backend flat` uses the flat AOD movement path.
- `--na-placer routing-agnostic` or `--na-placer routing-aware` selects the
  zoned placement mode.
- `--na-placement row-major` selects the flat AOD placement strategy.
- `--no-na-compact` leaves the schedule uncompacted for inspection.

See the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md)
for the target schema, assumptions, and citations.

## Run OpenQASM output on Aer

Install the optional verification dependencies and build the compiler:

```bash
just setup-python
source .venv/bin/activate
cargo build -p quonc
export QUONC="$PWD/target/debug/quonc"
```

`python/quon_aer.py` accepts either Quon source or OpenQASM on standard input:

```bash
python python/quon_aer.py test/verify/bell.qn --shots 4096 --seed 1234

cargo run -p quonc -- test/verify/bell.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm |
  python python/quon_aer.py --shots 4096 --seed 1234
```

The bridge imports the emitted OpenQASM and runs an ideal `AerSimulator`.
`--seed` makes sampling reproducible. The printed counts are raw simulation
results, not live-hardware performance estimates.

## Run reference verifiers

The scripts in `test/verify/` add assertions to compilation and simulation.
Run all same-stem `.qn`/`.py` cases:

```bash
QUONC="$PWD/target/debug/quonc" bash test/verify/run_e2e.sh
```

Or run one case by stem:

```bash
QUONC="$PWD/target/debug/quonc" bash test/verify/run_e2e.sh bell
```

The reference oracles cover Bell, teleportation, Bernstein-Vazirani, Grover,
QFT, Ising, QAOA, dense spin-glass QAOA, and Shor's quantum kernel.

The routing verifier checks constrained fixed targets against the all-to-all
baseline:

```bash
QUONC="$PWD/target/debug/quonc" python test/verify/routing.py
```
