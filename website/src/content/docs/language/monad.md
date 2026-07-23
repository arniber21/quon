---
title: The Quantum Monad
description: How Q<T> and run { } blocks bring allocation, measurement, and feed-forward into typed quantum programs.
---

Circuits are pure values — unitary morphisms that transform qubits without
allocating, measuring, or branching on classical results. Real quantum
programs need all three. Quon expresses these effectful operations in the
**Quantum Monad**, written `Q<T>`.

## What `Q<T>` represents

`Q<T>` is a quantum computation that may:

- **Allocate** qubits (`qreg(n)`)
- **Apply circuits** to qubits (`circuit @ qubits`)
- **Measure** qubits (`measure(q)`)
- **Branch** on measured values (`if bit then ...`)
- **Return** a classical value of type `T`

A function returning `Q<T>` is not a pure function — it describes a
quantum computation with side effects. But the type system still tracks
linearity: every allocated qubit must be consumed before the computation
returns.

### Why a monad, not an imperative API

In an imperative quantum SDK (e.g. Qiskit's circuit builder), allocation and
measurement are just method calls on a circuit object — `circuit.measure(0, 0)`
mutates the shared circuit in place. There is no type-level distinction between
a pure unitary gate and a destructive measurement; both are just "operations"
appended to the same log. Quon rejects this because it makes the
unitary/dynamic boundary invisible: the optimizer cannot tell which operations
are safe to commute and which must preserve ordering.

By wrapping effectful operations in `Q<T>`, Quon makes the boundary a *type*.
A `Circuit<n, m, d, C>` value is pure — the optimizer can rewrite it freely.
A `Q<T>` value is effectful — the compiler must preserve its measurement and
feed-forward ordering. You cannot accidentally apply an algebraic
simplification to a `Q<T>` computation, because it is not a `Circuit`. This
is the same principle as Haskell's `IO` monad: effects are tracked in the
type, not hidden behind a global mutable state.

## The `run { }` block

Quantum monadic computations are written with `run { }`:

```kotlin
fn hello_bell(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

Inside a `run` block:

- `<-` sequences a quantum computation and binds its result. The right-hand
  side must produce a value in `Q<...>`.
- `return` lifts a classical value into `Q<T>`, ending the computation.
- Pure `let` bindings can be mixed in for classical intermediate values.

The `<-` is not assignment — it is monadic bind. The right-hand side is a
quantum computation, and the left-hand side receives the classical result
(if any). The qubits it consumed are tracked in the linear context Δ.

## Circuits vs. the monad

The split between `circuit { }` and `run { }` is fundamental:

| Property | `circuit { }` | `run { }` |
|---|---|---|
| Purity | Pure value | Effectful computation |
| Allocation | No | Yes (`qreg`) |
| Measurement | No | Yes (`measure`) |
| Classical control | No | Yes (`if bit then`) |
| Type | `Circuit<n, m, d, C>` | `Q<T>` |
| Composition | `\|>`, `par` | `<-`, `return` |
| IR dialect | `quantum.circ` | `quantum.dynamic` |
| Optimizable algebraically | Yes | No (ordering preserved) |

A circuit is a **value** — you can store it, pass it, return it, compose it.
A `run` block is a **computation** — it executes effects and produces a
result. Circuits are applied *inside* `run` blocks via the `@` operator,
which consumes input qubits and produces output qubits.

## Applying a circuit to qubits

The `@` operator applies a `Circuit<n, m, ...>` to a `QReg<n>` (or a tuple
of `Qubit`s), consuming the input and producing fresh output qubits:

```kotlin
fn apply_and_measure(): Q<Bit> = run {
    reg <- bell_state() @ qreg(2)
    (q0, q1) = reg
    b <- measure(q0)
    discard(q1)
    return b
}
```

The `@` operator is the bridge between the pure circuit world and the
effectful monadic world. It is the only way to execute a circuit.

## Mixed pure and monadic code

Pure `let` bindings work inside `run` blocks for classical values:

```kotlin
fn classical_logic(b: Bit): Q<Bit> = run {
    let label = if b { "got one" } else { "got zero" }
    q <- qinit()
    result <- measure(q)
    return result
}
```

The `let` binding is ordinary lexical scope. Only `<-` introduces linear
quantum resources into Δ. This lets you interleave classical computation —
string formatting, arithmetic, list building — with quantum effects, and
the typechecker keeps the two worlds separate: `label` is a `String` in Γ,
`q` is a `Qubit` in Δ.

### Bind chaining: threading qubits through multiple circuits

A `run` block often chains multiple circuit applications, threading qubits
forward through `<-`:

```kotlin
fn chain(): Q<Bit> = run {
    q1 <- prepare_one() @ qinit()
    q2 <- bell_state() @ qreg(2)     -- produces a pair, not one qubit
    (a, b) = q2
    q3 <- (H @0) @ a                  -- apply a single gate as a circuit
    r  <- measure(q3)
    discard(b)
    discard(q1)
    return r
}
```

Each `<-` consumes the qubits on the right and binds fresh ones on the left.
The linear context tracks every qubit: `qinit()` produces one, `prepare_one()`
consumes it and produces `q1`, and so on. If any qubit is left unconsumed at
the `return`, the typechecker rejects the program — the same "linear resource
not consumed" diagnostic from the linearity page.

### Mixing pure `let` and monadic `<-`

The distinction between `let` and `<-` is not stylistic — it is what the
typechecker uses to distinguish classical and quantum bindings. `let` puts a
name in Γ (unrestricted); `<-` puts a name in Δ (linear, must be consumed):

```kotlin
fn mixed(): Q<Bit> = run {
    let n = 3                          -- Γ: Int, unrestricted
    reg <- hadamard_layer(n) @ qreg(n) -- Δ: QReg<n>, linear
    bits <- measure_all(reg)           -- Δ: reg consumed, Γ: bits (List<Bit>)
    let first = bits[0]                -- Γ: Bit, unrestricted
    return first
}
```

`n` is classical — you can use it in `let`, in array indexing, in `if`
conditions. `reg` is linear — it is consumed by `measure_all`. `bits` is
classical (the output of measurement) — it goes into Γ and can be reused.
The typechecker enforces this split at every binding, which is what makes the
linear discipline tractable: the type of the binding operator (`let` vs `<-`)
determines which context the name enters.

## How `run` blocks lower to IR

When the compiler lowers a `run { }` block, it emits `quantum.dynamic` ops
directly — there is no intermediate staging dialect (ADR-0037 collapsed the
ephemeral `monadic_staging` dialect). Each construct maps to a specific IR op:

| Source construct | IR op |
|---|---|
| `qreg(n)` | `n` × `test.qubit` allocations |
| `circuit @ qubits` | `quantum.dynamic.unitary_region` (callee body inlined) |
| `measure(q)` | `quantum.dynamic.measure` |
| `reset(q)` | `quantum.dynamic.reset` |
| `if b then C₁ else C₂ @ qs` | `quantum.dynamic.if` (both branches inlined) |
| `return x` | the value `x` in SSA form |

The `unitary_region` op is how a pure circuit value enters the dynamic IR:
the callee's `quantum.circ` body is inlined into the region, and the circ
optimization passes can descend into it to reach the gates. But the `if` op
preserves both branches — the optimizer never collapses them into a single
path, because the branch depends on a runtime measurement outcome.

### What `quantum.dynamic` preserves

The dynamic IR is deliberately *not* aggressively optimized. The circ fixpoint
runs on `quantum.circ` functions and on `unitary_region` bodies, but the
`quantum.dynamic` ops themselves — `measure`, `reset`, `if` — are treated as
opaque by the optimizer. This is what makes the unitary/dynamic split safe: the
optimizer can rewrite gates inside a `unitary_region` (because they are pure),
but it cannot reorder a `measure` past a subsequent gate (because the
measurement outcome might feed forward into that gate's control flow).

The `measurement_deferral` pass and `classical_region_fusion` pass run *after*
the circ fixpoint, normalizing the dynamic side. Measurement deferral may
reorder measurements to reduce circuit depth — but only when the reordering
does not change the observable result (e.g. commuting a measurement past a
unitary that acts on a different qubit). Classical-region fusion merges
adjacent classical regions (sequences of `if` blocks on the same bit) into a
single region, reducing control-flow overhead in the emitted artifact.
Together, these passes make the dynamic IR as compact as possible without
ever violating the measurement and feed-forward semantics the source
language guarantees.

## Contrasting with imperative SDKs

In an imperative SDK like Qiskit, the equivalent of a `run` block is just a
sequence of method calls on a `QuantumCircuit` object:

```python
# Qiskit (imperative)
from qiskit import QuantumCircuit
qc = QuantumCircuit(2, 2)
qc.h(0)
qc.cx(0, 1)
qc.measure(0, 0)
qc.measure(1, 1)
```

There is no type-level distinction between the `h` (unitary) and `measure`
(dynamic) — both are just gates appended to the same circuit object. The
compiler cannot tell which operations are safe to commute, so optimization
passes must conservatively assume every operation is dynamic. In Quon, the
`Circuit` vs `Q<T>` split makes this distinction a *type*: the optimizer knows
exactly which regions are pure and which are effectful, and it optimizes
accordingly.

The monad also makes *linearity* a property of the computation's type, not a
runtime convention. In the Qiskit snippet above, nothing prevents you from
calling `qc.measure(0, 0)` twice on the same qubit — the circuit object happily
appends both measurements, and the error (if any) surfaces only at execution.
In Quon, the `<-` bind consumes the qubit, and a second `measure(q0)` is a
compile-time error: `q0` is no longer in Δ. The monad's sequencing is what
makes the linear discipline enforceable — each step of the computation has a
well-defined "before" and "after" state of the linear context, and the
typechecker checks the transition at every `<-`.

## Next

The most important effectful operation is measurement — and what you can do
with its result. The next page covers measurement, classical `if` control,
and feed-forward corrections.

→ [Measurement and control](../measurement/)
