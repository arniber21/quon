---
title: Parallel composition and parametric circuits
description: Compose circuits on disjoint qubit sets and write width-polymorphic circuits with Nat parameters.
---

So far we have composed circuits sequentially with `|>`, chaining them
end-to-end so the output of one feeds the input of the next. Quon also
supports **parallel composition** — placing two circuits on disjoint qubit
sets so they run in the same depth layer — and **parametric circuits** —
writing a circuit once and instantiating it at any width.

## Parallel composition with `par`

The `par` keyword tensor-products two circuits on disjoint qubits. When you
write `par { c } * n`, Quon creates an `n`-fold parallel composition of `c`.
The widths multiply by `n`, but the depth stays at the body circuit's depth:

```kotlin
fn had_one(): Circuit<1, 1, 1, Clifford> = circuit {
    H @0
}

fn hadamard_layer(n: Nat): Circuit<n, n, 1, Clifford> =
    par { had_one() } * n
```

This is a single-depth layer of Hadamard gates across `n` qubits. The type
`Circuit<n, n, 1, Clifford>` tells you three things at a glance: the circuit
consumes and produces `n` qubits, runs in depth 1, and contains only Clifford
gates. All of this is verified by the typechecker before lowering.

## Why parallel composition matters

In a sequential composition `a |> b`, the depth of the result is `depth(a) +
depth(b)`. In parallel composition, the depth is `max(depth(a), depth(b))`.
This distinction is not just about performance — it is a **type-level
guarantee**. When you write `Circuit<4, 4, 1, Clifford>`, the compiler has
proven that all four gates can run in a single layer. If you accidentally
sequenced them, the depth would be 4 and the type would be
`Circuit<4, 4, 4, Clifford>` — the type signature itself would reveal the
mistake.

## Parametric circuits with `Nat`

Quon circuits can take `Nat` (natural number) parameters. This lets you write
a circuit once and instantiate it at any width:

```kotlin
fn apply_hadamard(n: Nat): Circuit<n, n, 1, Clifford> = circuit {
    for q in qubits(n) { H q }
}
```

The `for q in qubits(n)` form iterates over all `n` qubits in the register,
applying `H` to each. The elaborator partially evaluates this loop at the
call site: `apply_hadamard(3)` becomes three concrete `H` gate placements.

Parameters can also appear in the depth bound:

```kotlin
fn step(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0
}

fn evolution(steps: Nat): Circuit<1, 1, steps * 2, Universal> =
    repeat(steps, step())
```

The depth `steps * 2` is a **symbolic expression**. The typechecker verifies
that the inferred depth of the body fits the declared bound — if you wrote
`steps * 3` as the bound, it would still be accepted (looser bounds are fine),
but `steps` alone would be rejected (too tight).

## Repeat and bounded repetition

The `repeat(n, c)` combinator runs circuit `c` sequentially `n` times. Its
depth is `n * depth(c)`:

```kotlin
fn trotter_step(n: Nat): Circuit<n, n, 2 * (n - 1), Universal> = circuit {
    for i in range(n - 1) { Rzz(0.5) @(i, i + 1) }
}

fn trotter_evolve(n: Nat, steps: Nat): Circuit<n, n, steps * 2 * (n - 1), Universal> =
    repeat(steps, trotter_step(n))
```

The type `Circuit<n, n, steps * 2 * (n - 1), Universal>` carries a
multiplicative depth bound that depends on both the register width and the
number of steps. The typechecker proves this bound holds for every
instantiation — `trotter_evolve(4, 3)` has depth `3 * 2 * 3 = 18`.

## How parameters specialize

When a parametric circuit is called with a concrete `Nat` argument, the
**elaborator** partially evaluates it: loop bounds become fixed, angles
become concrete, and the result is a first-order gate tree with no classical
parameters left. This happens through the `SpecializedCircuit` module
(ADR-0038), which is Melior-free — specialization can be tested without
linking LLVM.

The key point: parametric circuits are not templates or macros. They are
typed functions. The type `Circuit<n, n, 1, Clifford>` is a real type with a
symbolic depth, checked by the typechecker using Z3 when structural equality
is not enough. Specialization only runs when lowering needs a concrete gate
sequence.

## Next

The depth bounds in these types are symbolic expressions with their own
algebra. The next page explains how depth arithmetic works and why the
typechecker can prove bounds like `steps * 2 * (n - 1)`.

→ [Depth bounds](../depth/)
