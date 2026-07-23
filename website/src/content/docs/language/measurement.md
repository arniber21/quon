---
title: Measurement and classical control
description: Consume qubits with measurement, branch on classical bits, and apply feed-forward corrections.
---

Measurement is where quantum computation becomes classical. In Quon,
`measure(q)` consumes a `Qubit` and produces a `Bit` — a classical value
that can branch control flow. This is the mechanism behind teleportation,
error correction, and any circuit with mid-circuit measurement and
feed-forward.

## Measurement as consumption

`measure(q)` has type `Qubit -> Q<Bit>`. It removes `q` from the linear
context Δ (consuming it) and introduces a `Bit` into the unrestricted context
Γ. The `Bit` is classical: it can be copied, stored, and reused freely.

```kotlin
fn read(q: Qubit): Q<Bit> = run {
    measure(q)
}
```

After `measure(q)`, `q` is gone from Δ. Any subsequent use is a type error.
The `Bit` result can be used in `if` conditions, returned from the function,
or passed to classical functions.

## `Bit` vs `Bool`

Quon distinguishes two classical bit types:

- **`Bit`** — a measured quantum bit. Produced by `measure(q)`. Represents
  a classical snapshot of a quantum measurement outcome.
- **`Bool`** — a pure classical boolean. A literal `true` or `false`, or the
  result of a comparison.

Both are unrestricted (can be copied and reused). The distinction matters for
semantics: a `Bit` comes from a quantum measurement (irreversible, random),
while a `Bool` is deterministic. The `if` construct works on both.

## Classical control: `if bit then ... else ...`

The `if` expression branches on a `Bit` or `Bool` and applies different
circuits depending on the outcome. This is **feed-forward** — the classical
result of a measurement determines which quantum operation runs next:

```kotlin
fn conditional_gate(b: Bit, q: Qubit): Q<Qubit> = run {
    result <- (if b then pauli_x() else id_one()) @ q
    return result
}
```

Here, `pauli_x()` and `id_one()` are both `Circuit<1, 1, 1, Clifford>`. The
`if` selects which circuit to apply based on `b`. Both branches consume the
same qubit `q`, so the linear context is consistent.

## Teleportation: measurement and feed-forward together

Quantum teleportation is the canonical example of measurement-based
feed-forward. The protocol:

1. Prepare a Bell pair shared between Alice and Bob.
2. Alice measures her qubit and the message qubit in the Bell basis.
3. Alice sends the two classical bits to Bob.
4. Bob applies a correction circuit based on Alice's bits.

In Quon:

```kotlin
fn prep(): Circuit<3, 3, 3, Clifford> = circuit {
    X @0 |> H @1 |> CNOT @(1, 2)
}

fn bell_basis(): Circuit<2, 2, 2, Clifford> = circuit {
    CNOT @(0, 1) |> H @0
}

fn pauli_x(): Circuit<1, 1, 1, Clifford> = circuit { X @0 }
fn pauli_z(): Circuit<1, 1, 1, Clifford> = circuit { Z @0 }
fn id_one(): Circuit<1, 1, 1, Clifford> = circuit { I @0 }

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

The linear type system verifies several things at compile time:

- All three qubits (`msg`, `alice`, `bob`) are consumed exactly once.
- The correction circuits are all `Clifford` (the type system proves this).
- `bob` is threaded through both corrections: `b2` is the output of the
  first correction, `b3` is the output of the second.
- The measured bits (`x_bit`, `z_bit`) are classical and can be reused in
  both `if` conditions.

## How `if` lowers to IR

When the compiler lowers an `if bit then circuit_A else circuit_B`, it
emits `quantum.dynamic` IR with a conditional application: the gate
operations from both branches are present, but gated on the classical bit
value. The `measurement_deferral` pass may reorder measurements to reduce
circuit depth, and `classical_region_fusion` may merge adjacent classical
regions.

On hardware without mid-circuit measurement, the `if` lowers to a deferred
correction: all branches are applied with controlled operations rather than
classical branching. The emitted OpenQASM uses `cx` and `cz` gates rather
than `if` blocks.

## Next

Not all qubits need to be allocated up front. Borrow blocks let you
request temporary ancilla qubits with scoped lifetimes and no-escape
guarantees.

→ [Borrow blocks](../borrow/)
