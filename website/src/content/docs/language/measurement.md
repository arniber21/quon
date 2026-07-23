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

### `measure_all`: consuming a register

`measure_all(reg)` consumes an entire `QReg<n>` and produces a `List<Bit>` of
length `n` — one classical bit per qubit. This is the standard way to read out
a register at the end of a computation:

```kotlin
fn read_all(reg: QReg<4>): Q<List<Bit>> = run {
    bits <- measure_all(reg)
    return bits
}
```

The register is consumed atomically: all `n` qubits are measured in the same
step, and the linear context Δ is cleared of the register. You cannot measure
"half" of a register — `measure_all` is all-or-nothing. If you need to measure
only some qubits, `split` the register first and `measure_all` only the piece
you want.

## `Bit` vs `Bool`

Quon distinguishes two classical bit types:

- **`Bit`** — a measured quantum bit. Produced by `measure(q)`. Represents
  a classical snapshot of a quantum measurement outcome.
- **`Bool`** — a pure classical boolean. A literal `true` or `false`, or the
  result of a comparison.

Both are unrestricted (can be copied and reused). The distinction matters for
semantics: a `Bit` comes from a quantum measurement (irreversible, random),
while a `Bool` is deterministic. The `if` construct works on both.

### Why the distinction matters

The `Bit`/`Bool` split reflects a fundamental physics boundary: a `Bit` is the
result of an irreversible operation (measurement collapses the quantum state),
while a `Bool` is a pure classical value with no quantum side effects. The
typechecker tracks this because it affects what the optimizer can do: a
circuit that branches on a `Bit` has a genuine runtime dependency on a
measurement outcome, and the optimizer must preserve that dependency. A
circuit that branches on a `Bool` is branching on a compile-time-known value,
and the optimizer can potentially constant-fold the branch away.

In practice, you produce a `Bit` from `measure` and a `Bool` from comparisons
or literals. The `if` construct accepts both, but the emitted IR differs: a
`Bit`-conditioned `if` lowers to `quantum.dynamic.if` (both branches present),
while a `Bool`-conditioned `if` may be resolved at compile time if the `Bool`
is a known constant.

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
same qubit `q`, so the linear context is consistent — the branching residual
rule from the linearity page applies: both branches must leave the same
resources live.

### Nested `if` for multi-bit corrections

When a protocol produces multiple measurement outcomes, you chain `if`
expressions to apply corrections based on each bit. Teleportation uses two
bits; more complex protocols (e.g. certain QEC decoders) use more:

```kotlin
fn two_bit_correction(b1: Bit, b2: Bit, q: Qubit): Q<Qubit> = run {
    q1 <- (if b1 then pauli_x() else id_one()) @ q
    q2 <- (if b2 then pauli_z() else id_one()) @ q1
    return q2
}
```

The `Bit`-in-`if` pattern also appears in `measure_all`-driven post-processing,
where the list of bits is indexed and each bit drives a correction:

```kotlin
fn post_correct(bits: List<Bit>, q: Qubit): Q<Qubit> = run {
    let b0 = bits[0]
    let b1 = bits[1]
    q1 <- (if b0 then pauli_x() else id_one()) @ q
    q2 <- (if b1 then pauli_z() else id_one()) @ q1
    return q2
}
```

Here `bits` is a classical `List<Bit>` in Γ — unrestricted, indexable, reusable.
The `if` conditions read individual bits from the list, and each branch
consumes the same qubit. The typechecker verifies the branching residual at
each `if`: both branches leave `q1` (or `q2`) live and Δ empty otherwise.

The first `if` applies an X correction (or identity) based on `b1`; the second
applies a Z correction (or identity) based on `b2`. The qubit threads through
both: `q` is consumed by the first `if`, producing `q1`, which is consumed
by the second, producing `q2`. The typechecker verifies at each step that
both branches of each `if` consume the same qubit — the branching residual
rule.

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

### `Bit` used in multiple `if` conditions

Note that `x_bit` and `z_bit` are each used in exactly one `if` condition, but
they *could* be used in many — they are classical `Bit` values in Γ, not linear
resources in Δ. If a protocol needed the same measurement bit to control two
different corrections, that would be perfectly legal:

```kotlin
fn reuse_bit(b: Bit, q1: Qubit, q2: Qubit): Q<(Qubit, Qubit)> = run {
    r1 <- (if b then pauli_x() else id_one()) @ q1
    r2 <- (if b then pauli_z() else id_one()) @ q2
    return (r1, r2)
}
```

`b` is used in both `if` conditions — this is fine because `Bit` is
unrestricted. The linear resources (`q1`, `q2`) are each consumed exactly once.
This is the practical payoff of the `Bit`/`Qubit` split: measurement produces
a copyable classical value, and you can branch on it as many times as needed.

## How `if` and measurement lower to IR

When the compiler lowers an `if bit then circuit_A else circuit_B`, it
emits `quantum.dynamic.if` IR with a conditional application: the gate
operations from both branches are present, but gated on the classical bit
value. The `measurement_deferral` pass may reorder measurements to reduce
circuit depth, and `classical_region_fusion` may merge adjacent classical
regions.

### How measurement lowers to IR

Each `measure(q)` in the source lowers to a `quantum.dynamic.measure` op
that consumes the qubit's SSA value and produces a classical `i1` (a single
bit). The op records which qubit was measured and produces the classical
result as an SSA value that subsequent `if` conditions can reference:

```text
%bit_0 = quantum.dynamic.measure %q0 : !qubit -> i1
```

The `measure` op is opaque to the circ optimization passes — it is never
rewritten, commuted, or removed by the algebraic simplifiers. This is what
makes measurement safe: the optimizer can simplify the unitary gates *around*
a measurement, but it cannot touch the measurement itself or change which qubit
it reads.

### Measurement deferral optimization

On hardware without mid-circuit measurement, the `if` lowers to a deferred
correction: all branches are applied with controlled operations rather than
classical branching. The emitted OpenQASM uses `cx` and `cz` gates rather
than `if` blocks. The `measurement_deferral` pass transforms a sequence like:

```text
%bit = measure %q0
%result = if %bit then (X @ %q1) else (id @ %q1)
```

into a controlled-X (CNOT) that applies the correction conditionally:

```text
%result = cx %q0, %q1
%bit = measure %q0
```

The measurement is deferred to *after* the controlled correction, so the
hardware sees only unitary gates followed by a final measurement — no
mid-circuit branching. This is essential for backends that do not support
dynamic circuits. The pass preserves the observable result (the deferred
CNOT is equivalent to the feed-forward correction) while changing the
execution model.

## Next

Not all qubits need to be allocated up front. Borrow blocks let you
request temporary ancilla qubits with scoped lifetimes and no-escape
guarantees.

→ [Borrow blocks](../borrow/)
