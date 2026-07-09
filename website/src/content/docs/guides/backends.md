---
title: Backends and verification
description: Compile Quon programs for hardware targets — fixed gate-model (OpenQASM intermediary + Aer) and neutral-atom schedules.
---

Quon is a **hardware-target compiler**: `--target` selects an architecture family,
and the pipeline lowers toward that family's artifacts. OpenQASM 3.0 is a
convenient intermediary for **fixed** gate-model targets (Aer verification and
tooling interop), not the definition of the backend surface — see
[ADR-0010](https://github.com/arniber21/quon/blob/main/docs/adr/0010-hardware-targets-primary-openqasm-intermediary.md).

## Fixed backend targets

A fixed `BackendTarget` JSON file records the gate-model constraints and
capabilities associated with a compilation:

- `id` names the target in diagnostics and metrics.
- `num_qubits` sets the available physical qubits.
- `topology.edges` lists directly connected qubit pairs. The routing pass inserts
  swaps when a two-qubit operation is not adjacent.
- `native_gates` lists the OpenQASM gate names the target accepts. The compiler
  decomposes other operations before emission. Unknown gate names are rejected.
- `noise` can record per-qubit and per-edge gate fidelity, T1/T2 times, and
  readout error. It is optional; T1 values can influence scheduling, while the
  other values are currently descriptive. These values do **not** configure the
  Aer verifier's simulator noise model.
- `meas_latency_us`, `supports_mid_circuit_meas`, and
  `supports_feed_forward` record measurement and dynamic-circuit capabilities.
  The current pipeline loads and reports them but does not yet reject programs
  based on those values.

The optional top-level `"kind": "fixed"` makes the architecture family
explicit. A descriptor without `kind` is also read as a fixed target for
backward compatibility. For a complete working example, see
[`backend/tests/fixtures/device_5q.json`](https://github.com/arniber21/quon/blob/main/backend/tests/fixtures/device_5q.json).

Inspect a descriptor before compiling:

```sh
cargo run -p quonc -- \
  --target backend/tests/fixtures/device_5q.json \
  --print-target
```

The loader validates the JSON, including field names, gate names, and topology
indices. A non-zero exit means the descriptor could not be loaded.

## Compile and emit OpenQASM 3 (fixed-path intermediary)

From the repository root, compile a checked-in Bell-state program for the
five-qubit fixture. `--emit-qasm` writes the OpenQASM intermediary used by Aer
and external tooling:

```sh
cargo run -p quonc -- \
  test/verify/bell.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm > /tmp/bell.qasm
```

The output begins with the OpenQASM 3 header and declarations, followed by
target-native gates and explicit measurement statements:

```text
OPENQASM 3.0;
include "stdgates.inc";
qubit[2] q;
bit[2] c;
// target-native operations...
c[0] = measure q[0];
c[1] = measure q[1];
```

With no `--target`, `quonc` uses the built-in `generic_openqasm` fixed target:
64 all-to-all qubits, the standard OpenQASM gate set, and no noise data.

## Run the emitted program on Aer

Install the verification dependencies into a virtual environment and build the
compiler:

```sh
python3 -m venv .venv
source .venv/bin/activate
python3 -m pip install -r python/requirements.txt
cargo build -p quonc
export QUONC="$PWD/target/debug/quonc"
```

`python/quon_aer.py` accepts either Quon source or OpenQASM on standard input:

```sh
python3 python/quon_aer.py test/verify/bell.qn --shots 4096 --seed 1234

"$QUONC" test/verify/bell.qn \
  --target backend/tests/fixtures/device_5q.json \
  --emit-qasm |
  python3 python/quon_aer.py --shots 4096 --seed 1234
```

The bridge imports the emitted OpenQASM and runs an ideal `AerSimulator`.
`--seed` makes sampling reproducible. Its printed counts are raw simulation
results, not a pass/fail judgment and not an estimate of live-hardware
performance.

## Run the reference verifiers

The scripts in `test/verify/` add assertions to compilation and simulation.
Run all same-stem `.qn`/`.py` cases:

```sh
QUONC="$PWD/target/debug/quonc" bash test/verify/run_e2e.sh
```

Or run one case by stem:

```sh
QUONC="$PWD/target/debug/quonc" bash test/verify/run_e2e.sh bell
```

Exit status `0` and a final `PASS:` line mean that the script's oracle held;
any failed assertion exits non-zero. The reference oracles cover:

- Bell: only `00` and `11`, each within the script's statistical tolerance.
- Teleportation: the recovered Z- and X-basis states each have fidelity above
  `0.99`.
- Bernstein–Vazirani: every shot recovers the fixed secret `(1, 1, 0)`.
- Grover: the marked state `11` has probability above `0.9`.
- QFT and Ising: their identity cases recover the expected state with fidelity
  above `0.99`.
- QAOA: every optimal K3 MaxCut bitstring is sampled more often than `000` or
  `111`.
- Dense spin-glass QAOA: the emitted workload has the expected operation counts
  and is parseable as OpenQASM 3.
- Shor kernel: fixed-seed runs agree and only the fixture's two expected
  outcomes appear.

The routing verifier uses the Bernstein–Vazirani source with two fixed target
fixtures, so run it directly:

```sh
QUONC="$PWD/target/debug/quonc" python3 test/verify/routing.py
```

It passes only when both constrained topologies recover the same secret as the
all-to-all baseline after swap insertion and native-gate decomposition.

## Neutral-atom hardware targets

A `neutral_atom_reconfigurable` target describes zones, array geometry, AOD
movement, Rydberg interactions, timing, fidelity, and a cost model. That path
is a first-class hardware backend (not an OpenQASM side quest):

```sh
cargo run -p quonc -- \
  test/na/bell.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule \
  --emit-resource-report
```

Schedule JSON and resource reports are available today (#112). Production
lowering into `quantum.na` MLIR is tracked in
[#167](https://github.com/arniber21/quon/issues/167).

See the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md)
for the architecture family, and ADR-0007 / ADR-0010 for IR and product framing.
