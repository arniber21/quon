---
title: Depth bounds and symbolic arithmetic
description: How Quon tracks circuit depth as symbolic expressions and proves bounds at compile time.
---

Every `Circuit` type carries a depth bound — the third type parameter in
`Circuit<n, m, d, C>`. This bound is not a gate count or a performance
hint. It is a **proven upper bound on gate depth**, verified by the
typechecker before the circuit is ever lowered to IR.

## What depth means

Gate depth is the length of the longest chain of sequentially dependent
gates in a circuit. Two gates on disjoint qubits can run in the same layer
(depth 1); two gates on the same qubit must run sequentially (depth 2).

In Quon, depth is tracked in the type. The typechecker infers the depth of
every composition and checks it against the declared bound. A looser bound
is accepted; a tighter one is rejected.

## The depth algebra

Depth expressions support four operations, matching the four composition
forms:

| Composition | Operator | Depth rule |
|---|---|---|
| Sequential (`\|>`) | addition | `depth(a \|> b) = depth(a) + depth(b)` |
| Parallel (`par`) | maximum | `depth(par { a, b }) = max(depth(a), depth(b))` |
| Repeat (`repeat`) | multiplication | `depth(repeat(n, c)) = n * depth(c)` |
| Controlled | +1 | `depth(controlled(c)) = depth(c) + 1` |

These are the only operations. The algebra is simple on purpose — depth is
an upper bound, not an exact count, and the operations must compose
predictably.

## Symbolic depth with `Nat` parameters

When a circuit is parametric, its depth can contain `Nat` parameters and
arithmetic over them:

```kotlin
fn evolution(steps: Nat): Circuit<1, 1, steps * 2, Universal> =
    repeat(steps, step())
```

The type `Circuit<1, 1, steps * 2, Universal>` says: "for any `steps`, this
circuit has depth at most `steps * 2`." The typechecker proves this by
induction on the composition structure: `step()` has depth 2, `repeat` adds,
so the total is `steps * 2`.

More complex expressions are possible. The QFT uses recursive depth:

```kotlin
fn qft(n: Nat): Circuit<n, n, 2 * n * n, Universal> =
    match n {
        0 => identity(0),
        _ => apply_hadamard(n)
             |> controlled_rotations(n)
             |> (qft(n - 1) `on_high` n)
             |> swap_reverse(n)
    }
```

The depth `2 * n * n` is a bound on the recursive structure. The typechecker
verifies this using Z3 when structural equality is insufficient — for
example, proving that the recursive call's depth (`2 * (n-1) * (n-1)`) fits
under the declared bound (`2 * n * n`).

## How the typechecker proves depth bounds

The typechecker uses a three-step process:

1. **Infer** — walk the composition tree bottom-up, computing the depth of
   each sub-expression using the algebra above. The result is a `DepthExpr`:
   a symbolic expression over `Nat` literals and `Int` variables.

2. **Check** — compare the inferred depth against the user-declared bound.
   If the inferred depth is structurally equal, accept. If not, try
   `DepthExpr::equiv` (algebraic simplification). If still not provably
   equal, dispatch to Z3 (the refinement solver) under the current
   assumptions.

3. **Report** — if Z3 cannot prove `inferred ≤ declared`, the program is
   rejected with a `DepthMismatch` error showing both expressions.

This means depth bounds are not annotations the user hopes are correct — they
are theorems the compiler proves. If you write `Circuit<4, 4, 3, Clifford>`
but the circuit actually has depth 4, the program does not compile.

## Looser bounds are accepted

The typechecker checks `inferred ≤ declared`, not `inferred == declared`.
This means you can always write a looser bound:

```kotlin
-- actual depth is 1, but declared as 5 — accepted
fn loose(): Circuit<1, 1, 5, Clifford> = circuit { H @0 }
```

This is by design. The bound is an upper limit, not an exact count. The
compiler may optimize within it, and callers can rely on the bound without
knowing the exact depth. The optimizer can also reduce depth without
changing the type signature — if gate cancellation removes a redundant layer,
the bound still holds.

## Why depth bounds matter

Depth is the primary resource cost for quantum circuits on most hardware.
A depth-100 circuit on a noisy device decoheres more than a depth-10
circuit. By putting depth in the type, Quon makes this cost visible at
the API boundary: a function returning `Circuit<8, 8, 12, Universal>`
tells the caller exactly how deep the circuit is before they run it.

The optimizer also uses depth bounds. If the compiler can prove a
circuit's depth is within the bound, it can apply transformations that
might temporarily increase depth (e.g. unrolling) and then reduce it again
(e.g. gate cancellation), all within the proven bound.

## Next

The fourth type parameter — Clifford classification — tells the compiler
which optimization strategy to apply. The next page explains the
Clifford/Universal distinction and how it drives the Clifford+T optimizer.

→ [Clifford classification](../clifford/)
