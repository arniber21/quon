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

### Depth arithmetic in action

Consider a circuit that applies a Hadamard, then a CNOT, then a T gate:

```kotlin
fn three_gates(): Circuit<2, 2, 3, Universal> = circuit {
    H @0 |> CNOT @(0, 1) |> T @0
}
```

The typechecker walks the `|>` chain bottom-up: `H @0` has depth 1, `CNOT @(0, 1)`
has depth 1, `T @0` has depth 1. Sequential composition adds: `1 + 1 + 1 = 3`.
The declared bound is 3, so `3 ≤ 3` holds — accepted.

Now consider the same three gates, but with the CNOT and T on disjoint qubits,
composed in parallel:

```kotlin
fn parallel_depth(): Circuit<2, 2, 2, Universal> = circuit {
    H @0 |> par { CNOT @(0, 1), T @1 }
}
```

Here `H @0` (depth 1) is composed sequentially with `par { CNOT @(0,1), T @1 }`.
The `par` takes `max(1, 1) = 1`, so the total is `1 + 1 = 2`. The declared
bound is 2 — accepted. The type proves that the CNOT and T run in the same
layer, saving one depth unit.

### A depth arithmetic error

If you declare a bound that is too tight, the typechecker rejects it. Here is
a circuit whose actual depth is 3, but declared as 2:

```kotlin
fn too_tight(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0 |> S @0
}
```

```
error: depth bound too tight
  --> source.qn:2:33
   |
 2 | fn too_tight(): Circuit<1, 1, 2, Universal> = circuit { H @0 |> T @0 |> S @0 }
   |                                  ^
   |  inferred depth: 3 (1 + 1 + 1)
   |  declared bound: 2
   |
   = the inferred depth exceeds the declared bound
   = hint: declare Circuit<1, 1, 3, Universal> or simplify the circuit
```

The typechecker computed `1 + 1 + 1 = 3` from the three sequential gates and
found `3 > 2`. The error names both expressions and suggests a fix.

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

### Symbolic depth comparison examples

The depth algebra handles multiplication, addition, and max over symbolic
variables. Here are some expressions the typechecker routinely proves:

| Inferred depth | Declared bound | Result |
|---|---|---|
| `steps * 2` | `steps * 2` | accepted (structural equality) |
| `steps * 2` | `steps * 3` | accepted (looser: `2 ≤ 3`) |
| `steps * 2` | `steps` | rejected (tighter: `2 > 1`) |
| `2 * (n - 1) * (n - 1)` | `2 * n * n` | accepted (Z3 proves the inequality) |
| `n * 1` | `n` | accepted (algebraic simplification: `n * 1 = n`) |
| `max(n, n)` | `n` | accepted (algebraic simplification: `max(n, n) = n`) |

The first two rows use structural equality or a simple comparison. The fourth
row — the QFT recursion — requires Z3 because `(n-1)² ≤ n²` is a nonlinear
arithmetic fact that structural simplification cannot derive directly.

## How the typechecker proves depth bounds

The typechecker uses a three-step process, owned by the `obligation.rs`
judgment module (ADR-0032):

1. **Infer** — walk the composition tree bottom-up, computing the depth of
   each sub-expression using the algebra above. The result is a `DepthExpr`:
   a symbolic expression over `Nat` literals and `Int` variables.

2. **Check** — compare the inferred depth against the user-declared bound.
   If the inferred depth is structurally equal, accept. If not, try
   `DepthExpr::equiv` (algebraic simplification — e.g. `n * 1` reduces to
   `n`, `max(n, n)` reduces to `n`). If still not provably equal, dispatch to
   Z3 (the refinement solver) under the current assumptions.

3. **Report** — if Z3 cannot prove `inferred ≤ declared`, the program is
   rejected with a `DepthMismatch` error showing both expressions.

This means depth bounds are not annotations the user hopes are correct — they
are theorems the compiler proves. If you write `Circuit<4, 4, 3, Clifford>`
but the circuit actually has depth 4, the program does not compile.

### What Z3 sees

When structural equality and algebraic simplification both fail, the
obligation module encodes the depth inequality as a Z3 constraint. For the
QFT example, Z3 receives:

```
forall n: Nat. n >= 0 => 2 * (n - 1) * (n - 1) <= 2 * n * n
```

Z3 proves this in milliseconds — it is a simple nonlinear arithmetic fact.
The typechecker records the assumption stack (e.g. `n > 0` from the `match`
guard) so Z3 has the context it needs. If Z3 returns `unsat` (the inequality
does not hold), the `DepthMismatch` diagnostic fires. If Z3 times out or
returns `unknown`, the typechecker conservatively rejects the bound — it
never accepts a depth it cannot prove.

## Looser bounds are accepted

The typechecker checks `inferred ≤ declared`, not `inferred == declared`.
This means you can always write a looser bound:

```kotlin
fn loose(): Circuit<1, 1, 5, Clifford> = circuit { H @0 }
```

The actual depth is 1, but the declared bound is 5 — `1 ≤ 5`, so it is
accepted. This is by design. The bound is an upper limit, not an exact count.
The compiler may optimize within it, and callers can rely on the bound without
knowing the exact depth. The optimizer can also reduce depth without
changing the type signature — if gate cancellation removes a redundant layer,
the bound still holds.

### When a looser bound is the right choice

Sometimes a looser bound is intentional. If you are writing a library circuit
whose internal implementation may change (e.g. a decomposition that could
be replaced by a shorter one later), declaring a looser bound means callers
that depend on the depth budget won't break when you tighten the
implementation. The type signature `Circuit<4, 4, 10, Clifford>` is a
*public contract*: it promises the circuit will never exceed depth 10, even
if the current implementation is depth 6. Callers can budget against 10 with
confidence.

## The optimizer reducing depth within the bound

The optimizer uses depth bounds as a license to transform. If the compiler
can prove a circuit's depth is within the bound, it can apply transformations
that might temporarily increase depth (e.g. unrolling a rotation into a
sequence of T gates) and then reduce it again (e.g. T-count synthesis), all
within the proven bound.

For example, a Clifford circuit declared with depth 4 might be optimized by
the stabilizer tableau pass to depth 2 (if the full sequence composes to a
shorter equivalent). The type signature still says `Circuit<..., 4, Clifford>`
— the bound is an upper limit, not the post-optimization depth. The optimizer
reduces the *actual* depth; the *proven* bound remains the guarantee callers
rely on. This is why the bound is checked at the type level (before
optimization) but the optimizer is free to improve within it.

### Gate cancellation as depth reduction

The simplest optimizer pass — `gate_cancellation` — removes adjacent inverse
pairs. A circuit `H @0 |> H @0` has declared depth 2, but the optimizer
reduces it to the identity (depth 0). The bound `2` still holds (`0 ≤ 2`),
and the emitted QASM contains no gates. The caller never sees the
cancellation — they see the type `Circuit<1, 1, 2, Clifford>`, which is
a valid upper bound on the optimized depth-0 circuit.

## Why depth bounds matter

Depth is the primary resource cost for quantum circuits on most hardware.
A depth-100 circuit on a noisy device decoheres more than a depth-10
circuit. By putting depth in the type, Quon makes this cost visible at
the API boundary: a function returning `Circuit<8, 8, 12, Universal>`
tells the caller exactly how deep the circuit is before they run it.

The optimizer also uses depth bounds. If the compiler can prove a
circuit's depth is within the bound, it can apply transformations that
might temporarily increase depth (e.g. unrolling) and then reduce it again
(e.g. gate cancellation), all within the proven bound. The depth bound is
the typechecker's gift to the optimizer: a proven ceiling that makes
aggressive rewriting safe.

## Next

The fourth type parameter — Clifford classification — tells the compiler
which optimization strategy to apply. The next page explains the
Clifford/Universal distinction and how it drives the Clifford+T optimizer.

→ [Clifford classification](../clifford/)
