---
title: Your second program
description: A teleportation circuit that exercises QReg destructuring, the Quantum Monad, measurement, and classical feed-forward control.
---

The quickstart showed a Bell pair: prepare, measure, done. Real quantum
algorithms do more — they branch on measurement results and apply corrections
conditionally. Quantum teleportation is the smallest program that exercises
every one of those features, and it fits in a single file. This walkthrough
builds it line by line.

## The circuit primitives

Teleportation needs five small circuits. Each is a `Circuit<n, m, d, Clifford>`
value — a pure, unitary transformation with a type-level contract.

```kotlin
fn prep(): Circuit<3, 3, 3, Clifford> = circuit { X @0 |> H @1 |> CNOT @(1, 2) }
fn bell_basis(): Circuit<2, 2, 2, Clifford> = circuit { CNOT @(0, 1) |> H @0 }
fn pauli_x(): Circuit<1, 1, 1, Clifford> = circuit { X @0 }
fn pauli_z(): Circuit<1, 1, 1, Clifford> = circuit { Z @0 }
fn id_one(): Circuit<1, 1, 1, Clifford> = circuit { I @0 }
```

`prep` prepares the initial state on all three qubits: an `X` gate flips the
message qubit to $|1\rangle$, while an `H` followed by a `CNOT` creates a Bell
pair between Alice and Bob. Its type says three qubits in, three out, depth
three, Clifford class — and the typechecker verifies that `X`, `H`, and `CNOT`
are all Clifford gates and that the `|>` chain is exactly three steps deep.
`bell_basis` is the Bell measurement circuit: it is the self-inverse adjoint of
Bell preparation, so the lowering needs no adjoint synthesis. `pauli_x` and
`pauli_z` are the correction gates; `id_one` is the identity circuit used when
no correction is needed. All five share the same structure: a pure circuit value
whose type is a verified contract, not a runtime description.

## The monadic main

```kotlin
fn main(): Q<Bit> = run {
    (msg, alice, bob) <- prep() @ qreg(3)
    (m2, a2)          <- bell_basis() @ (msg, alice)
    x_bit             <- measure(m2)
    z_bit             <- measure(a2)
    b2                <- (if z_bit then pauli_x() else id_one()) @ bob
    b3                <- (if x_bit then pauli_z() else id_one()) @ b2
    result            <- measure(b3)
    return result
}
```

`main` lives inside a `run { }` block — the Quantum Monad. Everything quantum
happens here: allocation, circuit application, measurement, and feed-forward
control. The return type `Q<Bit>` says the monad produces a single classical
bit when it terminates. Let's walk through each line.

### Allocating and destructuring

```kotlin
    (msg, alice, bob) <- prep() @ qreg(3)
```

`qreg(3)` allocates a three-qubit register. The `<-` operator applies `prep()`
to that register and destructures the three output qubits into `msg`, `alice`,
and `bob` in a single binding. This is `QReg<n>` destructuring: the linear
typechecker tracks each qubit individually, and each name must be consumed
exactly once downstream. After this line, the three qubits are in the prepared
state — the message holds $|1\rangle$, and Alice and Bob share a Bell pair.

### Applying a circuit to a subset

```kotlin
    (m2, a2) <- bell_basis() @ (msg, alice)
```

`bell_basis()` is a `Circuit<2, 2, 2, Clifford>` — it takes two qubits and
produces two. The `@ (msg, alice)` syntax applies it to the `msg` and `alice`
qubits specifically, leaving `bob` untouched. The `<-` binding destructures the
two outputs into `m2` and `a2`. The typechecker verifies that the circuit's
input arity (2) matches the number of qubits supplied, and that `msg` and
`alice` are consumed by this application — they are no longer in scope. This
is the linear resource discipline at work: you cannot accidentally reuse `msg`
after feeding it into `bell_basis`.

### Measurement produces classical values

```kotlin
    x_bit <- measure(m2)
    z_bit <- measure(a2)
```

`measure` collapses a qubit into a classical `Bit`. Unlike qubits, classical
values are unrestricted — you can copy them, discard them, and branch on them.
`x_bit` and `z_bit` are the two classical bits that encode which correction Bob
needs. After this point, `m2` and `a2` are gone from the quantum context; only
their classical shadows remain. The type system enforces this transition: a
`Qubit` is linear and consumed once, a `Bit` is affine and free to reuse.

### Classical control of quantum gates

```kotlin
    b2 <- (if z_bit then pauli_x() else id_one()) @ bob
    b3 <- (if x_bit then pauli_z() else id_one()) @ b2
```

This is feed-forward: the correction applied to Bob depends on the classical
measurement outcomes. The `if ... then ... else ...` expression selects
between two circuit values — `pauli_x` or `id_one`, then `pauli_z` or
`id_one` — and the `@ bob` / `@ b2` syntax applies the selected circuit to the
qubit. The type of the `if` expression is `Circuit<1, 1, 1, Clifford>` in both
branches, so the typechecker confirms the branches agree before the monadic
binding proceeds. The `b2` output of the first correction feeds into the
second as `b2` becomes the input qubit — linear threading, gate by gate.

### Final measurement

```kotlin
    result <- measure(b3)
    return result
```

After the corrections, Bob's qubit holds the teleported state. A final
`measure` collapses it to a classical `Bit`, which the monad returns. Since
the source program prepared the message as $|1\rangle$, the expected result is
`1` on every shot — teleportation should preserve the state perfectly.

## Compile and run

```bash
cargo run -p quonc -- test/verify/teleport.qn --emit-qasm
```

The emitted OpenQASM 3 is:

```text
OPENQASM 3.0;
include "stdgates.inc";
qubit[3] q;
bit[3] c;
x q[0];
h q[1];
cx q[1], q[2];
cx q[0], q[1];
h q[0];
cx q[1], q[2];
c[0] = measure q[1];
cz q[0], q[2];
c[1] = measure q[0];
c[2] = measure q[2];
```

The first four gates are `prep` and `bell_basis` lowered directly — `x`, `h`,
`cx` on the prepared qubits, then the Bell-basis `cx` and `h` on the message
and Alice. The measurements produce classical bits `c[0]` and `c[1]`, which
drive the corrections. Notice what is *not* here: there is no `if` block in
the QASM output. The compiler applied measurement deferral by default,
converting the classically-controlled `if` corrections into coherent `cx` and
`cz` gates applied before the final measurement. The `cz q[0], q[2]` line is
the deferred Z correction; the `cx q[1], q[2]` before the first measurement is
the deferred X correction. This is an optimization the type system makes
safe: because the circuits are all Clifford class, the corrections commute
with the measurements and can be pulled through.

You can verify the result on Qiskit Aer:

```bash
cargo run -p quonc -- test/verify/teleport.qn --emit-qasm \
  | python python/quon_aer.py --shots 1024 --seed 7
```

The output should be `1` on every shot — teleportation recovered the original
$|1\rangle$ state with feed-forward corrections.

## What you have seen

You started with circuit primitives — pure `Circuit` values whose types
guarantee depth, arity, and Clifford class. You composed them inside a `run`
block that allocated qubits, applied circuits to subsets, measured qubits into
classical bits, and branched on those bits to apply conditional corrections.
The typechecker tracked every qubit's lifecycle: allocation, consumption,
measurement, and linear threading through corrections. The compiler then
lowered the feed-forward `if` into coherent deferred gates, emitting a flat
OpenQASM 3 program with no runtime branching.

Now you've seen circuits, the Quantum Monad, and classical control. The
[Language Guide](/language/introduction/) explains each in depth.
