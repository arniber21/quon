---
title: Language guide
description: The Quon language from first principles — circuits as typed values, linear qubits, the Quantum Monad, and how the typechecker proves depth, width, and Clifford class before a single gate is lowered.
---

The language guide is the reference-depth tour of Quon's source language. It is
the canonical starting point for anyone who wants to *write* Quon rather than
just run it: what a `Circuit<n, m, d, C>` value is, why a qubit is a linear
resource consumed exactly once, how `|>` and `par` compose, and how the
typechecker turns depth bounds and Clifford classifications into compile-time
facts instead of runtime hopes.

## Who this is for

Programmers who want to understand the language on its own terms — the type
system, the constructs, and the rules the compiler enforces — before reading the
compiler internals or the cookbook. No prior quantum-framework experience is
assumed, but the pages move at reference depth: each construct is defined
precisely, with the invariant the typechecker proves around it.

If you are new to quantum computing entirely, the [learning track](/learn/) is
the gentler on-ramp; it covers the same two ideas — a circuit is a typed value,
a qubit is a linear value — in six short lessons before handing off here.

## What this covers

The twelve pages below proceed from the smallest complete program to the
features that compose in real algorithms:

- **Introduction** — what Quon is and how a circuit becomes a typed value.
- **Circuits and gates** — `circuit` blocks, `|>`, and the `Circuit<n, m, d, C>`
  contract.
- **Qubits and registers** — linear qubit values, `QReg`, and `tensored`/`split`.
- **The linear type system** — no-cloning as a compile-time fact.
- **Parallel composition** — `par` and the `max` (not `+`) depth rule.
- **Depth bounds** — depth as a *verified* type-level bound.
- **Clifford classification** — the `Clifford`/`Universal` class in the type.
- **The Quantum Monad** — `run` blocks and allocation.
- **Measurement and control** — `measure`, `Bit`, and `if` feed-forward.
- **Borrow blocks** — temporary ancilla workspace with a linear debt.
- **QEC blocks** — error-correction blocks as typed circuit combinators.
- **Putting it together** — every feature composed in one program.

For the formal language definition, see the
[Quon specification](https://github.com/arniber21/quon/blob/main/SPEC.md).

## Where to go next

Once the language is familiar, the [cookbook](/cookbook/) walks complete,
CI-verified programs that exercise these features end to end, and the
[architecture](/architecture/compiler-internals/) pages show what the compiler
does with them.

→ Start with [Introduction — What is Quon?](./introduction/).
