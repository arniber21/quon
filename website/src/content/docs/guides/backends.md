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

`--emit-resource-report` is a compiler **analytic** artifact (schedule metrics,
QEC metadata, `error_budget` = rate × count). Sampled logical failures from
`python/quon_qec_sinter.py` stay in a separate CSV. An optional labeled join CSV
from the #254 ablation harness is allowed for comparisons only — it does not
replace the separate Sinter CSV or mutate the report (ADR-0020). Neither
artifact is a threshold claim.

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

- `--na-backend zoned` uses zoned architecture scheduling (default placer is
  routing-agnostic / ZAC-style; `--na-placer routing-aware` selects the RAP-style
  search).
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
```

No `QUONC` export is needed. After `cargo build`, the Aer bridge
auto-discovers the local compiler binary, probing `target/release/quonc`
then `target/debug/quonc` before falling back to `quonc` on `PATH`
([#375](https://github.com/arniber21/quon/issues/375)). Set `QUONC`
explicitly only to override that order — for example, to pin a binary
built with non-default features.

`python/quon_aer.py` accepts either Quon source or OpenQASM on standard
input — one copy-paste command from source to Aer counts:

```bash
python python/quon_aer.py test/verify/bell.qn --shots 4096 --seed 1234
```

Or pipe emitted QASM through a constrained target:

```bash
cargo run -p quonc -- test/verify/bell.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm |
  python python/quon_aer.py --shots 4096 --seed 1234
```

The bridge imports the emitted OpenQASM and runs an ideal `AerSimulator`.
`--seed` makes sampling reproducible. The printed counts are raw simulation
results, not live-hardware performance estimates. When a required Python
package is missing, the error names the active Python executable and prints
a ready-to-run install command using the project `.venv` when one exists.

## Run reference verifiers

The scripts in `test/verify/` add assertions to compilation and simulation.
Run all same-stem `.qn`/`.py` cases — the compiler binary is auto-discovered,
so no `QUONC` prefix is required after a build:

```bash
bash test/verify/run_e2e.sh
```

Or run one case by stem:

```bash
bash test/verify/run_e2e.sh bell
```

The reference oracles cover Bell, teleportation, Bernstein-Vazirani, Grover,
QFT, Ising, QAOA, dense spin-glass QAOA, and Shor's quantum kernel.

The routing verifier checks constrained fixed targets against the all-to-all
baseline:

```bash
python test/verify/routing.py
```

Set `QUONC` only when you need to point at a specific binary instead of the
auto-discovered one.

## Diagnostic catalog

When a backend target or emission flag is rejected, the
[diagnostic catalog](/reference/diagnostics/#backend-target-and-artifact-emission)
lists every target-descriptor and emission error with its cause and repair.

## OpenQASM export to Qiskit

`quonc --emit-qasm` writes OpenQASM 3 for a fixed target. The Python export
helper is `to_qiskit_circuit` in `python/quon_aer.py` (issue #197).
`load_circuit` calls it. Both normalize `c[i] == 1` to
`c[i] == true` before `qiskit.qasm3.loads`. Calling `qasm3.loads` on the raw
compiler output is unsupported.

```python
from quon_aer import compile_to_qasm, to_qiskit_circuit

qasm = compile_to_qasm("test/verify/bell.qn")
circuit = to_qiskit_circuit(qasm)
```

The emitted subset is the fixed-target statement set:

- `OPENQASM 3.0`, `include "stdgates.inc"`, `qubit[n] q`, and `bit[m] c` when the program measures
- standard gates that are native on the selected target (`h`, `x`, `y`, `z`, `s`, `sdg`, `sx`, `t`, `tdg`, `rx`, `ry`, `rz`, `cx`, `cy`, `cz`, `swap`, `ccx`, and the rotations those names denote)
- `c[i] = measure q[j]`, `reset`, `barrier`, and `if (c[i] == 1) { ... } else { ... }`
- identity gates are omitted

User `gate` definitions, `for`/`while`, `opaque`, and pulse/OpenPulse are not emitted. A gate outside the target native set is an error, not a custom definition.

`just ci-rust` compiles these cookbook programs and checks their Aer distributions, which is the export validation for the current cookbook:

- [Bell](https://github.com/arniber21/quon/blob/main/test/verify/bell.qn)
- [Teleportation](https://github.com/arniber21/quon/blob/main/test/verify/teleport.qn)
- [Bernstein–Vazirani](https://github.com/arniber21/quon/blob/main/test/verify/bernstein_vazirani.qn)
- [Grover](https://github.com/arniber21/quon/blob/main/test/verify/grover.qn)
- [QFT](https://github.com/arniber21/quon/blob/main/test/verify/qft.qn)
- [Ising](https://github.com/arniber21/quon/blob/main/test/verify/ising.qn)
- [QAOA](https://github.com/arniber21/quon/blob/main/test/verify/qaoa.qn)
- [Shor's kernel](https://github.com/arniber21/quon/blob/main/test/verify/shor.qn)

### If you know QuantumCircuit

| Circuit | Qiskit | Quon |
| --- | --- | --- |
| Bell | `qc.h(0); qc.cx(0, 1); qc.measure_all()` | [`bell.qn`](https://github.com/arniber21/quon/blob/main/test/verify/bell.qn): `H @0 \|> CNOT @(0, 1)`, then `measure` each qubit in `run` |
| Bernstein–Vazirani | oracle of CNOTs into an ancilla, Hadamards around it, one shot | [`bernstein_vazirani.qn`](https://github.com/arniber21/quon/blob/main/test/verify/bernstein_vazirani.qn): secret `110` as `CNOT @(0, 3)` and `CNOT @(1, 3)` |
| Grover (n=2) | H, oracle phase on `11`, diffusion, measure | [`grover.qn`](https://github.com/arniber21/quon/blob/main/test/verify/grover.qn): `oracle` is `CZ @(0, 1)`, one `repeat` of oracle then diffusion |

Quon circuits are values of type `Circuit<n, m, d, C>`. Qubits are linear: `measure` consumes them, and there is no implicit reuse. A Qiskit circuit that measures a qubit and then applies another gate needs a `reset` (or a fresh qubit) in Quon. The fixed-target descriptor records that capability as `supports_mid_circuit_meas`.

## OpenQASM import

v1 does not translate OpenQASM or Qiskit into `.qn` source. The limited converter that shipped is neutral-atom ingestion (issue #304): `quonc` parses a benchmark subset and enters the neutral-atom scheduler. It does not produce Quon source.

```bash
cargo run -p quonc -- test/na/ising_n42.qasm \
  --target targets/neutral_atom/rap_table_i.json \
  --emit-resource-report -
```

Accepted on that path: an `OPENQASM` header, `include` (skipped, not expanded), `qreg` / `qubit`, `creg` / `bit` (skipped), gate calls `name(params) reg[i], ...` with at most one angle on a one-qubit gate and no angles on wider gates, and `barrier`.

Rejected with a line number: `measure`, `reset`, `gate` / `opaque` definitions, `if` / `for` / `while`, and multi-parameter gates such as `u2` and `u3`. Fixed-target OpenQASM import is not supported; pass a neutral-atom `--target`.

Source-level migration stays the cheat sheet above. A second QASM-to-`.qn` translator is not part of v1.
