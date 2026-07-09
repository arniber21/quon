---
title: Backends and verification
description: Compile Quon programs for fixed gate-model targets and verify the emitted OpenQASM 3 with Qiskit Aer.
---

Quon's fixed backend path compiles a program for a gate-model `BackendTarget`,
emits OpenQASM 3, and can run that output on Qiskit Aer. This path works locally:
it does not require an account or access to live quantum hardware.

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

## Compile and emit OpenQASM 3

From the repository root, compile a checked-in Bell-state program for the
five-qubit fixture:

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

## Different architecture: neutral atoms

Fixed targets model gate connectivity and OpenQASM emission. A
`neutral_atom_reconfigurable` target instead describes zones, array geometry,
AOD movement, Rydberg interactions, timing, fidelity, and a cost model. Those
fields and that lowering path are intentionally separate; the OpenQASM pipeline
currently accepts fixed targets only.

See the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md)
for that architecture family.
