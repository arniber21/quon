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

### Parallel composition of different circuits

`par` is not limited to repeating a single circuit. You can place two *different*
circuits on disjoint qubit sets in the same layer. The depth of the result is
the maximum of the two depths, and the width is the sum:

```kotlin
fn mixed_layer(): Circuit<3, 3, 2, Universal> = circuit {
    par {
        H @0 |> T @0,
        H @1
    }
}
```

Here the first sub-circuit (`H @0 |> T @0`) has depth 2 and the second (`H @1`)
has depth 1. The `par` takes `max(2, 1) = 2`, so the composite has depth 2. The
classification is `Universal` because one sub-circuit contains `T`. The width is
`1 + 2 = 3` — three qubits, two touched by the first circuit and one by the
second.

## Why parallel composition matters

In a sequential composition `a |> b`, the depth of the result is `depth(a) +
depth(b)`. In parallel composition, the depth is `max(depth(a), depth(b))`.
This distinction is not just about performance — it is a **type-level
guarantee**. When you write `Circuit<4, 4, 1, Clifford>`, the compiler has
proven that all four gates can run in a single layer. If you accidentally
sequenced them, the depth would be 4 and the type would be
`Circuit<4, 4, 4, Clifford>` — the type signature itself would reveal the
mistake.

### `par` vs sequential: a depth comparison

Consider a four-qubit Hadamard layer. Written with `par`, the depth is 1:

```kotlin
fn fast_layer(n: Nat): Circuit<n, n, 1, Clifford> = par { had_one() } * n
```

Written naively with `|>` — which would be wrong, because `|>` chains on the
same qubits — the depth would be `n`. The type signature `Circuit<n, n, 1, ...>`
is the proof that the gates are truly parallel: the typechecker computed
`max(1, 1, ..., 1) = 1` from the `par` structure, not `1 + 1 + ... + 1 = n`
from `|>`. If you swapped `par` for `|>` by mistake, the inferred depth would
be `n`, and the declared bound `1` would no longer hold — the typechecker
would reject it with a `DepthMismatch`.

## Nested parallel composition

`par` can be nested: a parallel layer can itself contain parallel sub-layers.
This is how you build structured, hierarchical circuit layouts:

```kotlin
fn had_and_t(n: Nat): Circuit<n, n, 1, Universal> =
    par { H @0 } * n |> par { T @0 } * n
```

Here `par { H @0 } * n` is a parallel H layer (depth 1), and `par { T @0 } * n`
is a parallel T layer (depth 1). The `|>` between them sequences the two layers,
giving depth 2. But note the structure: *within* each layer, the gates are
parallel; *between* layers, they are sequential. The type `Circuit<n, n, 2, Universal>`
captures this exactly.

The depth difference between `par` and `|>` is not just a performance hint —
it is the single most common reason a circuit type fails to check. If you
intend a parallel layer but write `|>` by accident, the inferred depth doubles
or quadruples, and the declared bound no longer holds. The typechecker catches
this immediately: the inferred depth (`n` instead of `1`) exceeds the bound,
and the `DepthMismatch` diagnostic names the mismatched expressions. In
practice, this is the compiler telling you "you meant `par` here, not `|>`" —
a structural insight that would be invisible in an imperative circuit builder.

You can also nest `par` directly:

```kotlin
fn nested_par(): Circuit<4, 4, 1, Clifford> = circuit {
    par {
        par { H @0, H @1 },
        par { H @2, H @3 }
    }
}
```

The outer `par` composes two inner `par` blocks, each of depth 1. The total
depth is `max(max(1,1), max(1,1)) = 1` — all four gates run in one layer. The
typechecker evaluates the nested `max` expressions symbolically and verifies
the result fits the declared bound.

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

### `for` loops vs `par`

Both `par { c } * n` and `for q in qubits(n) { c q }` can express an
n-fold gate layer, but they differ in how the typechecker sees them:

- `par { c } * n` is a *single parallel composition* — the typechecker applies
  the `max` depth rule and proves depth 1.
- `for q in qubits(n) { c q }` is elaborated into `n` gate placements that the
  typechecker then recognizes as acting on disjoint qubits, also proving depth 1.

The distinction matters when the loop body is not a single gate — a `for` body
with internal sequencing produces a different depth than a `par` of the same
body. The `for` form is more flexible (you can compute per-qubit angles), while
`par` is more declarative (the parallelism is explicit in the syntax).

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

### A concrete repeat example

Consider repeating a simple two-gate sequence a fixed number of times:

```kotlin
fn echo(k: Nat): Circuit<1, 1, k * 3, Clifford> =
    repeat(k, circuit { H @0 |> S @0 |> H @0 })
```

The inner circuit has depth 3 (three sequential gates on one qubit). `repeat(k, ...)`
multiplies: depth `k * 3`. For `echo(4)`, the elaborator unrolls into 12 gate
placements on qubit 0, and the depth bound `4 * 3 = 12` is proven to hold. The
stabilizer tableau optimizer (since this is Clifford) will then check whether
the 12-gate sequence composes to something shorter — for instance, `H·S·H`
repeated four times might collapse to a single Pauli, reducing the actual
gate count without changing the proven depth bound.

## How parameters specialize

When a parametric circuit is called with a concrete `Nat` argument, the
**elaborator** partially evaluates it: loop bounds become fixed, angles
become concrete, and the result is a first-order gate tree with no classical
parameters left. This happens through the `SpecializedCircuit` module
(ADR-0038), which is Melior-free — specialization can be tested without
linking LLVM.

The specialization process works as follows:

1. **Substitute** — the concrete `Nat` argument replaces the parameter
   throughout the circuit's type and body. `hadamard_layer(4)` becomes a
   circuit over 4 qubits with depth 1.
2. **Unroll** — `for` loops and `par * n` are unrolled into concrete gate
   placements. `par { H @0 } * 4` becomes four `H` placements on positions
   0, 1, 2, 3.
3. **Flatten** — nested compositions (`|>`, `par`) are flattened into a
   first-order tree of `Compose` / `GateApp` / `Adjoint` nodes. No
   `repeat` or `par` constructs survive — they have been fully unrolled.
4. **Verify indices** — the concrete positions are checked against the
   concrete width. A gate at position 5 in a 4-qubit circuit is caught here,
   even if it was hidden inside a parametric loop.

The key point: parametric circuits are not templates or macros. They are
typed functions. The type `Circuit<n, n, 1, Clifford>` is a real type with a
symbolic depth, checked by the typechecker using Z3 when structural equality
is not enough. Specialization only runs when lowering needs a concrete gate
sequence — the symbolic proof happened earlier, so the elaborator trusts the
bound and simply unfolds the structure.

### What the elaborator produces

After specialization, the elaborator hands the lowerer a `SpecializedCircuit`:
a first-order tree over concrete qubit indices and literal angles, plus the
resolved `Circuit<n, m, d, C>` indices. No `Nat` parameters remain. The lowerer
then emits `quantum.circ` ops directly from this tree — one MLIR gate op per
`GateApp` node, one `quantum.circ.compose` per `Compose` node. The
`SpecializedCircuit` boundary is what makes this clean: the lowerer's only
input is the elaborator's output, and neither needs to know about the symbolic
world the typechecker reasoned over.

## Next

The depth bounds in these types are symbolic expressions with their own
algebra. The next page explains how depth arithmetic works and why the
typechecker can prove bounds like `steps * 2 * (n - 1)`.

→ [Depth bounds](../depth/)
