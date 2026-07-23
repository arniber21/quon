---
title: Quickstart
description: Compile a Bell pair from Quon to OpenQASM 3, sample it with Qiskit Aer, and emit a neutral-atom schedule — in four commands.
---

This walkthrough uses a checked-in Bell program, compiles it to OpenQASM 3, and
samples the emitted program through Quon's Qiskit Aer verification seam. Run the
commands from the repository root after completing the
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

Read the types before you read the gates. `bell_state` returns a
`Circuit<2, 2, 2, Clifford>` — that contract says the transformation takes two
qubits in, produces two out, finishes in exactly two steps, and stays within the
Clifford class. The typechecker verifies all four claims from the circuit body
before the program reaches the compiler backend. If you wrote `H @0 |> CNOT @​(0, 1) |> H @0` instead, the depth would be three and the program would fail to
typecheck against `Circuit<2, 2, 2, Clifford>`.

`main` wraps the circuit in a `run { }` block — the Quantum Monad. The `<-`
operator binds the two qubits from `bell_state()` to `q0` and `q1`, consuming
the circuit value in the process. `measure` then converts each qubit into a
classical `Bit`. Because qubits are linear, you cannot forget to measure them:
unconsumed quantum resources are a type error, not a silent leak.

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

The compiler pipeline runs seven stages on this file: it parses the source text
into an AST, desugars syntactic conveniences, typechecks every expression
against the linear resource context, elaborates circuit values into a gate list,
lowers through MLIR generic-form IR, applies optimization passes (gate
cancellation, rotation merging, Clifford+T simplification), and finally emits
OpenQASM 3. The `h q[0]` and `cx q[0], q[1]` lines are the lowered form of
`H @0 |> CNOT @(0, 1)` — the `|>` sequential composition maps to gate ordering,
and the `Circuit<2, 2, 2, Clifford>` type guarantees the emitted circuit
respects the depth and class bounds.

Notice that the emitted circuit is exactly two gates deep: one `h` and one
`cx`. The depth bound `2` from the source type is not a comment or a
suggestion — it is a verified invariant that survives the entire lowering
pipeline. If an optimization pass tried to reorder or expand the circuit in a
way that broke the depth contract, the compiler would catch it. This is what it
means for types to be load-bearing: they constrain the backend, not just the
frontend.

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

This is the verification seam. Quon emits standard OpenQASM 3, so any
simulator that consumes that format can run the program. The `quon_aer.py`
bridge is a thin wrapper around Qiskit Aer: it reads the QASM, instantiates the
circuit, samples it with a pinned random seed, and reports the bitstring
histogram. Because the seed is fixed, the output is reproducible — you can
regression-test against it. The two outcomes `00` and `11` confirm the Bell
state was prepared correctly: the qubits are entangled, so measuring one
determines the other. A bug in the compiler — a missing gate, a swapped index,
a broken optimization pass — would show up here as spurious `01` or `10`
outcomes.

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

Neutral-atom hardware reconfigures its qubit layout between entangling gates by
moving atoms with optical tweezers. The compiler must therefore emit not just a
gate list but a *movement schedule*: which atoms move where, when, and in what
order. The same `Circuit` type that constrained the gate-model backend now
constrains the scheduling — the depth bound tells the scheduler how many
entangling layers to expect, and the Clifford classification lets it skip
unnecessary decompositions. The resource report summarizes atom count, movement
distance, and expected execution time, giving you a cost estimate before you
commit to hardware.

## What just happened

```text
Quon source
  -> parse / typecheck / elaborate
  -> lower through MLIR generic-form IR
  -> optimize and adapt to the target
  -> emit OpenQASM 3 or neutral-atom schedule/resource artifacts
```

You wrote a typed quantum program in `.qn`, and the compiler verified its
resource contracts before lowering it to a target-native artifact. The pipeline
did not just translate syntax — it checked that the circuit's depth, qubit
count, and Clifford class matched the type signature, then optimized within
those bounds, then emitted code a real backend could execute. The same source
file produced OpenQASM 3 for a gate-model simulator and a movement schedule for
a neutral-atom target, with no source changes between the two.

Next: read [Your second program](/getting-started/second-program/) to see
measurement and classical control.
