---
title: Quickstart
description: Compile a Bell pair from Quon to OpenQASM 3 and sample it with Qiskit Aer.
---

This walkthrough uses a checked-in Bell program, compiles it to OpenQASM 3, and
samples the emitted program through Quon's Qiskit Aer verification seam.

Run the commands from the repository root after completing the
[installation guide](/getting-started/install/).

## 1. Inspect the program

`test/verify/bell.qn` prepares a Bell state, measures both qubits, and returns
the correlated classical bits:

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}

fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

`H @0` puts the first qubit into superposition. `CNOT @(0, 1)` entangles the
second qubit with the first. The linear typechecker tracks the two output
qubits until measurement consumes them.

## 2. Compile to OpenQASM 3

```bash
cargo run -p quonc -- test/verify/bell.qn --emit-qasm
```

The output has the OpenQASM 3 header, typed qubit/classical declarations,
target-native operations, and explicit measurements:

```text
OPENQASM 3.0;
include "stdgates.inc";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c[0] = measure q[0];
c[1] = measure q[1];
```

With no `--target`, `quonc` uses the built-in all-to-all fixed gate-model
target. Pass `--target backend/tests/fixtures/device_5q.json` to compile
through a constrained fixed topology.

## 3. Sample with Aer

Set up the optional Python environment if you have not already:

```bash
just setup-python
source .venv/bin/activate
```

Then pipe the emitted OpenQASM into the Aer bridge:

```bash
cargo run -p quonc -- test/verify/bell.qn --emit-qasm \
  | python python/quon_aer.py --shots 1024 --seed 7
```

The output resembles:

```text
00: 520
11: 504
```

Shot counts are samples, so exact values can differ. The important result is
that only `00` and `11` appear: the measurements are correlated even though
each individual bit is random.

## 4. Emit a neutral-atom artifact

The same compiler driver also targets reconfigurable neutral-atom descriptors.
This command emits a schedule JSON document and a resource report:

```bash
cargo run -p quonc -- test/na/bell.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule \
  --emit-resource-report
```

That path extracts the interaction graph, schedules entangling layers, applies
the selected movement backend, compacts the schedule, and reports resource and
timing estimates.

## What this exercised

```text
Quon source
  -> parse / typecheck / elaborate
  -> lower through MLIR generic-form IR
  -> optimize and adapt to the target
  -> emit OpenQASM 3 or neutral-atom schedule/resource artifacts
```

For deeper examples, browse the [cookbook](/cookbook/) and the
[backend guide](/guides/backends/).
