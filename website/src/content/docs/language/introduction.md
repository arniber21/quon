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

### Parametric circuits: a value for any width

Because a circuit is a value, it can also be *parametric* — written once and
instantiated at any width. Quon's `Nat` parameters make a circuit a family of
values indexed by a natural number:

```kotlin
fn had_one(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }

fn hadamard_layer(n: Nat): Circuit<n, n, 1, Clifford> =
    par { had_one() } * n
```

`hadamard_layer` is a function that, given any `n`, returns a circuit over `n`
qubits with depth 1 — a single parallel layer of Hadamard gates. The type
`Circuit<n, n, 1, Clifford>` carries a *symbolic* width and depth: the
typechecker proves the bound holds for every instantiation of `n`, not just a
specific one. When `hadamard_layer(4)` is called at a concrete site, the
elaborator partially evaluates the parallel composition into four concrete `H`
gate placements — but the type-level proof happened earlier, against the
symbolic form. This is the key difference between a Quon parametric circuit and
a C++ template or a macro: the symbolic type is checked *before* specialization,
so a width or depth error is caught at the type level, not after unrolling.

A practical illustration: when you write `hadamard_layer(4)` inside a larger
program, the elaborator unrolls the `par` into four `H` placements and the
lowerer emits four `quantum.circ` gate ops. But the typechecker has *already*
proven, from the symbolic `Circuit<n, n, 1, Clifford>`, that the depth is 1
regardless of `n`. If you later change the call to `hadamard_layer(8)`, the
proof still holds — you never need to re-check depth for each width. This is
what it means for a circuit to be a *typed* value, not a mutable log.

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
fn clone(q: Qubit): (Qubit, Qubit) =
    let (q1, q2) = (q, q)
    in (q1, q2)
```

```
error: linear resource `q` used twice
  --> source.qn:2:18
   |
 2 |     let (q1, q2) = (q, q)
   |                      ^   ^ first use consumes `q`
   |                          second use is unbound — `q` already consumed
   |
   = note: quantum values are linear; each must be consumed exactly once
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

The boundary also maps directly onto the compiler's two IR dialects. A
`circuit { }` block lowers to `quantum.circ` ops — pure unitary gates over `!qubit`
SSA values, where linearity is enforced at the IR level too. A `run { }` block
lowers to `quantum.dynamic` ops — `measure`, `reset`, `unitary_region`, and
`if` conditioned on classical bits. The circ optimization passes can descend
into `unitary_region` and `if` branch bodies to reach the gates that live there,
but they never reorder measurements or merge control-flow regions — that is the
dynamic side's job.

## From source to artifact

A Quon program does not run directly on hardware. The compiler walks it through
a fixed pipeline, and every feature in this guide maps onto a stage of it:

1. **Parse and desugar** — Tree-sitter plus a Rust parser turn `.qn` source into
   the surface AST, then desugar infix combinators and sugar into the core AST.
   Syntax errors stop here.

2. **Typecheck and elaborate** — the bidirectional linear typechecker validates
   declarations, quantum ownership (the linear context `Δ`), symbolic depth
   bounds, register widths, and Clifford classifications. Refinement obligations
   — is this inferred depth really ≤ the declared bound? are these two widths
   equal? — are discharged through Z3 when structural equality is not enough.
   Parametric circuit calls are specialized at concrete call sites via the
   Melior-free **SpecializedCircuit** module, producing a first-order gate DAG
   with no classical parameters left.

3. **Lower to MLIR** — the elaborated DAG lowers through verified Rust builders
   into `quantum.circ` (unitary) and `quantum.dynamic` (measurement,
   feed-forward) ops. The `Circuit<n, m, d, C>` indices ride along as op
   attributes, not as MLIR parameterized types.

4. **Optimize** — the circ fixpoint runs gate cancellation, rotation merging,
   Clifford+T optimization (stabilizer tableau for Clifford circuits, phase
   polynomial for Universal circuits), compiler uncomputation, and bounded ZX
   simplification, iterating to a fixpoint. Dynamic passes (measurement
   deferral, classical-region fusion) normalize the `quantum.dynamic` side.

5. **Emit** — fixed targets run native-gate decomposition, SABRE-style routing,
   depth scheduling, and OpenQASM 3 emission. Neutral-atom targets extract an
   atom-indexed interaction graph, schedule entangling layers, plan movement,
   and build a resource report.

### What the typechecker proves vs. what the optimizer does

It helps to be precise about the division of labor between the frontend and the
backend, because the type-level facts the frontend establishes are exactly what
the backend relies on:

- The **typechecker** *proves* static invariants: every qubit is consumed
  exactly once (linearity), the inferred depth fits the declared bound (symbolic
  arithmetic, Z3), the width of every composition lines up, and the Clifford
  classification is correctly inferred bottom-up. These are *theorems* — if the
  typechecker accepts the program, the invariants hold for every instantiation.

- The **optimizer** *uses* those invariants as licenses to transform. It does
  not re-derive them; it trusts them. The Clifford classification tells it
  which kernel to dispatch (stabilizer tableau vs. phase polynomial). The depth
  bound gives it room to temporarily increase depth (e.g. by unrolling) and
  then reduce it again, all within the proven bound. The circuit/monad split
  tells it which regions are safe to rewrite algebraically and which must
  preserve measurement ordering.

The consequence is that optimization in Quon is *never* a guess. Every rewrite
the optimizer applies is enabled by a fact the typechecker already proved. If
the typechecker cannot prove a fact, the program is rejected — it does not reach
the optimizer in an ambiguous state. The full pipeline is documented in the
[compiler reference](/reference/compiler/) and explored stage-by-stage in the
[compiler internals](/architecture/compiler-internals/) page.

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
