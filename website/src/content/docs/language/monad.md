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

A circuit is a **value** — you can store it, pass it, return it, compose it.
A `run` block is a **computation** — it executes effects and produces a
result. Circuits are applied *inside* `run` blocks via the `@` operator,
which consumes input qubits and produces output qubits.

## Applying a circuit to qubits

The `@` operator applies a `Circuit<n, m, ...>` to a `QReg<n>` (or a tuple
of `Qubit`s), consuming the input and producing fresh output qubits:

```kotlin
fn apply_and_measure(): Q<Bit> = run {
    reg <- bell_state() @ qreg(2)    -- consumes QReg<2>, produces (Qubit, Qubit)
    (q0, q1) = reg                    -- destructure the pair
    b <- measure(q0)                  -- consume q0, produce Bit
    discard(q1)                        -- consume q1 without measuring
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
    -- label is a classical String, not in Δ
    q <- qinit()
    result <- measure(q)
    return result
}
```

The `let` binding is ordinary lexical scope. Only `<-` introduces linear
quantum resources into Δ.

## Next

The most important effectful operation is measurement — and what you can do
with its result. The next page covers measurement, classical `if` control,
and feed-forward corrections.

→ [Measurement and control](../measurement/)
