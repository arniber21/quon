---
title: What is Quon?
description: Quon is a functional quantum language with linear types, where a circuit is a typed value rather than a side-effect.
sidebar:
  order: 1
---

Quon is a statically typed, functional language for writing quantum programs.
Its compiler is written in Rust and lowers through MLIR to backend-specific
artifacts — OpenQASM 3 for fixed gate-model targets, or schedule and resource
outputs for reconfigurable neutral-atom targets. But the interesting part is not
the backend; it is the source language. Quon treats quantum operations as
*mathematical objects with types*, not as imperative instructions fired at a
device. This single design decision is what makes the rest of the language
coherent, and the rest of this guide explains why.

Quon's goal is not to be the most concise way to scribble a circuit. Plenty of
frameworks do that. Its goal is to make quantum programs *checkable*: to catch
width mismatches, depth overruns, duplicated qubits, and forgotten ancillae at
compile time, so that the program you type is the program that runs. Every
feature you will meet in this guide — linear types, symbolic depth, Clifford
classification, the strict split between unitary and dynamic code — exists in
service of that goal.

## A circuit is a value, not a side-effect

In most quantum frameworks you build a circuit by calling imperative functions
that mutate a global circuit object — `circuit.h(0)`, `circuit.cx(0, 1)` — and
the "circuit" is really a log of side-effects appended to some hidden state.
Quon rejects that. In Quon a circuit is a *first-class value* with a
*first-class type*. You write it down, it has a type, the typechecker reads it,
and only then does anything reach hardware. Consider the canonical example:

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}
```

`bell_state` is a function with no arguments that *returns a circuit*. That
circuit has the type `Circuit<2, 2, 2, Clifford>`: it takes 2 qubits in,
produces 2 qubits out, has gate depth bounded by 2, and contains only Clifford
gates. Nothing here runs yet. The value `bell_state()` can be returned from a
function, passed to a combinator, composed with another circuit, or stored in a
variable — exactly like an integer or a string in an ordinary functional
language. Because it is a value with a type, the compiler can check it for
correctness *before* a single qubit exists, and can rewrite it with algebraic
identities *without* worrying that it will change some hidden global state.

## Why linear types

Quantum physics imposes two constraints that ordinary programming languages
silently violate. You cannot *clone* an unknown quantum state (the no-cloning
theorem), and you cannot *silently drop* a qubit and pretend it never existed —
its entanglement with the rest of the system would be destroyed, and you would
have to account for its measurement outcome. A language that lets you write
`let (q1, q2) = (q, q)` or simply forget to use a qubit is a language that
permits physically impossible programs and pushes the consequences to run time.

Quon makes these physics into compile-time invariants using **linear types**.
Every quantum value — a `Qubit`, a register, an encoded logical block, a circuit
— lives in a *linear context* and must be **consumed exactly once**: passed to a
gate, measured, returned, or handed to a circuit that consumes it on your
behalf. Classical values like bits, booleans, and integers are *unrestricted*
and may be copied and discarded freely. The typechecker, not a runtime crash,
tells you when a qubit is used twice, used zero times, or escapes a scope it was
never permitted to leave. A program that tries to clone a qubit looks like this,
and it does not compile:

```kotlin
-- ERROR: q used twice — violates linearity
fn clone(q: Qubit): (Qubit, Qubit) =
    let (q1, q2) = (q, q)
    in (q1, q2)
```

Later pages show exactly how the linear context tracks each qubit and what the
resulting errors look like. For now, hold onto the principle: **linear types
turn the no-cloning and no-dropping theorems into type errors.** What is a
theorem in a physics textbook becomes a line in a compiler diagnostic.

## Two worlds: `circuit {}` and `run {}`

Quantum programming splits cleanly into two concerns. On one side there is the
*unitary* work — reversible gates that transform qubits without looking at them.
On the other there is the *dynamic* work — allocating fresh qubits, measuring
them mid-circuit, and feeding classical results forward into later gates. These
have radically different properties. Unitary circuits are pure mathematical
objects that compose algebraically and can be simplified with calculus.
Dynamic computations have side-effects, depend on random measurement outcomes,
and must thread state and control flow.

Quon gives each world its own syntax and its own type. Pure unitary work is
written in a `circuit { }` block and produces a `Circuit<...>` value — the thing
`bell_state` returns above. Dynamic work is written in a `run { }` block and
produces a value of type `Q<T>`, the **Quantum Monad**: a computation that may
allocate, measure, and feed forward, ultimately yielding a value of type `T`.
The full Bell experiment — prepare the state, then measure both qubits —
crosses that boundary:

```kotlin
fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
```

Here `qreg(2)` allocates two fresh qubits, `bell_state() @ qreg(2)` applies the
circuit value to them (consuming the register and producing two new qubits), and
`measure` consumes each qubit and produces a classical `Bit`. The `run` block is
the only place allocation and measurement may happen; inside a `circuit { }`
block they are forbidden. This boundary is not stylistic. It is what lets the
compiler apply algebraic simplifications — gate cancellation, ZX-calculus
rewrites, Clifford+T optimization — to the pure parts with mathematical
certainty, while routing the dynamic parts through the more careful control-flow
machinery they deserve. When you read a Quon program, the `circuit { }` /
`run { }` boundary tells you immediately which rules apply where.

## From source to artifact

A Quon program does not run directly on hardware. The compiler walks it through
a fixed pipeline: it parses the surface syntax, typechecks it (the linear
context, width, depth, and classification rules you are about to learn),
elaborates parametric loops into concrete circuits, lowers the result to MLIR,
runs optimization passes, and finally emits a backend artifact. The two worlds
above map onto that pipeline: `circuit { }` blocks become pure unitary IR that
the optimizer can rewrite freely, while `run { }` blocks become a dynamic IR
region that preserves measurement and feed-forward exactly. You do not need the
pipeline details to write Quon, but it helps to know that the type-level facts
the frontend establishes — depth bounds, Clifford classification, linear
consumption — are exactly what the later passes rely on. The full pipeline is
documented separately in the [compiler reference](/reference/compiler/).

## How to read this guide

This guide introduces Quon one concept at a time, in an order chosen so that
each page builds only on what came before. The arc is deliberate. You will first
meet circuits as values, then the qubits they act on, then the linear type
system that protects those qubits. With resources understood, you will see how
circuits compose — sequentially and in parallel, with parametric widths and
symbolic depth — and how the Clifford classification the optimizer depends on is
inferred. Only then does the guide turn to the dynamic side: the Quantum Monad,
measurement and classical feed-forward, ancilla borrowing, and error correction.
A final page ties every concept together in one complete program. Read the
pages in order the first time; afterward each page stands on its own as a
reference, and every page ends with a link to the next.

## Next

You have seen that a circuit is a typed value. The next page unpacks the
`Circuit<n, m, d, C>` type itself — what its four indices mean, how gates are
placed inside a `circuit { }` block, and why treating circuits as values makes
them composable and statically checkable.

→ [Circuits and gates](/language/circuits/)
