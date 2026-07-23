---
title: Clifford classification
description: How Quon infers Clifford vs Universal circuit types and uses them to select optimization strategies.
---

Every `Circuit` type carries a classification: `Clifford` or `Universal`.
This is the fourth type parameter in `Circuit<n, m, d, C>`. It is not an
annotation — it is **inferred bottom-up** from the gates in the circuit, and
the typechecker verifies it against the declared type.

## What Clifford means

A **Clifford circuit** is one built entirely from Clifford gates: `H`, `S`,
`S†`, `CNOT`, `X`, `Y`, `Z`, and `CZ`. Clifford circuits can be efficiently
simulated on a classical computer (the Gottesman-Knill theorem), and they
can be optimized using stabilizer tableau methods.

A **Universal circuit** contains at least one non-Clifford gate — typically
`T`, `T†`, or a rotation like `Rz(θ)`. These circuits are computationally
universal for quantum computing, but they cannot be classically simulated
efficiently and require different optimization strategies.

The distinction is not merely academic — it determines which optimization
kernel the compiler dispatches, which simulator can validate the circuit, and
whether the Gottesman-Knill theorem applies. By lifting this distinction into
the type system, Quon makes the dispatch decision a *compile-time* fact rather
than a runtime heuristic.

## Inference

The compiler infers classification from the gate primitives:

```kotlin
fn basis_change(): Circuit<1, 1, 1, Clifford> = circuit {
    H @0
}

fn phase_resource(): Circuit<1, 1, 1, Universal> = circuit {
    T @0
}
```

`H` is Clifford, so `basis_change` is `Clifford`. `T` is not Clifford, so
`phase_resource` is `Universal`. The inference is purely bottom-up: the
classification of a composition is the join of the classifications of its
parts.

### The S gate and Clifford composition

The `S` gate (π/2 phase) is Clifford, as is its dagger `S†`. A circuit using
only `S` gates is Clifford:

```kotlin
fn s_circuit(): Circuit<1, 1, 2, Clifford> = circuit {
    S @0 |> S @0
}
```

Two `S` gates compose to `Z` (a Pauli, still Clifford). The typechecker
infers `Clifford` from the primitives and checks it against the declared
`Clifford`. The stabilizer tableau optimizer will later prove that `S · S = Z`
and potentially reduce the gate count — but the classification was already
settled at the type level before the optimizer ran.

### CZ: a Clifford two-qubit gate

`CZ` (controlled-Z) is Clifford, like `CNOT`. A circuit using both is
Clifford:

```kotlin
fn two_qubit_clifford(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CZ @(0, 1) |> H @1
}
```

All three gates are Clifford, so the composite is Clifford. The join
`Clifford ⊔ Clifford ⊔ Clifford = Clifford`. This is the kind of circuit
that appears in Grover's diffusion operator and in stabilizer-state
preparation — both are efficiently simulable, and the type says so.

### Composition rules: the join lattice

The classification forms a two-element lattice:

```
        Universal
           |
        Clifford
```

The join (`⊔`) of two classifications is `Universal` if either operand is
`Universal`, and `Clifford` only if both are `Clifford`. This means:

| Left | Right | `a |> b` |
|---|---|---|
| Clifford | Clifford | Clifford |
| Clifford | Universal | Universal |
| Universal | Clifford | Universal |
| Universal | Universal | Universal |

A single non-Clifford gate anywhere in the composition chain forces the
entire composite to `Universal`. This is why the classification is inferred
bottom-up: the join propagates from the leaves (individual gates) to the root
(the full circuit).

## Subtyping: Clifford ⊑ Universal

`Clifford` is a subtype of `Universal`. This means a `Clifford` circuit can
be used wherever a `Universal` circuit is expected — but not vice versa:

```kotlin
fn expects_universal(c: Circuit<1, 1, 1, Universal>): Q<Qubit> = run {
    q <- c @ qinit()
    return q
}

fn ok(): Q<Qubit> = run {
    q <- expects_universal(circuit { H @0 })
    return q
}
```

This subtyping is sound because every Clifford circuit is trivially a
Universal circuit (just with no non-Clifford gates). The reverse is not
sound: a circuit containing `T` cannot satisfy a `Clifford` bound.

### A classification subtyping error

If you try to pass a `Universal` circuit where a `Clifford` is expected, the
typechecker rejects it:

```kotlin
fn expects_clifford(c: Circuit<1, 1, 1, Clifford>): Q<Qubit> = run {
    q <- c @ qinit()
    return q
}

fn bad(): Q<Qubit> = run {
    q <- expects_clifford(circuit { T @0 })
    return q
}
```

```
error: classification mismatch
  --> source.qn:7:22
   |
 7 |     q <- expects_clifford(circuit { T @0 })
   |                      ^^^^^^^^^^^^
   |  expected: Circuit<1, 1, 1, Clifford>
   |  got:      Circuit<1, 1, 1, Universal>
   |
   = T is a non-Clifford gate; Universal is not a subtype of Clifford
```

The error names the expected and actual classifications and explains why the
subtyping direction does not hold.

## How the optimizer uses classification

The Clifford classification drives the compiler's optimization strategy
(ADR-0013, ADR-0039). The optimizer does not *guess* which strategy to use —
it reads the classification from the circuit's type attributes and dispatches
deterministically:

- **For `Clifford` circuits** (`clifford = true`), the compiler applies the
  **Aaronson-Gottesman stabilizer tableau** simulator. This represents the
  circuit's action on the Pauli group generators as a GF(2) matrix (the CHP
  tableau), detects when sequences compose to identity (or a single Pauli),
  and replaces them with shorter equivalents. This catches non-adjacent
  simplifications that peephole gate cancellation misses (e.g. `S⁴ = I` when
  the four `S` gates are separated by other Clifford gates).

- **For `Universal` circuits** (`clifford = false`), the compiler applies the
  **phase polynomial** pass. This extracts the non-Clifford (T-count) content
  as a sum of linear Boolean phase terms, merges and cancels terms algebraically
  (not just by adjacency), and re-synthesizes gates from the reduced polynomial.
  This catches non-adjacent T-count reductions like `T·CNOT·T → S·CNOT`
  (T-count 2→0).

Both algorithms handle **non-adjacent** gates — the key advantage over the
peephole gate-cancellation pass, which only catches immediately adjacent
redundant pairs.

### What the optimizer sees at the classification boundary

When the optimizer runs, it reads the `clifford` boolean attribute on each
`quantum.circ.func` — the attribute that the lowerer wrote from the
typechecker's inferred classification. For a `Clifford` circuit, the stabilizer
tableau pass simulates the circuit's action on the Pauli generators. If the
full sequence conjugates to the identity, every gate is removed:

```text
before:  S @0 |> S @0 |> S @0 |> S @0      after:  identity(1)
```

`S` is not self-inverse, so `gate_cancellation` will not collapse `S·S·S·S`;
the tableau proves it equals the identity and removes every gate. The same
machinery catches non-adjacent identities like `H·S⁴·H = I`, where the
Hadamards are separated by four `S` gates, and reduces a sequence to a single
Pauli when it conjugates to one (e.g. `S·S → Z`).

For a `Universal` circuit, the phase polynomial pass works on the `{CNOT, T,
T†}` block. It extracts each `T`/`T†` as a `±1` contribution (in π/4 units) to
the parity of its qubit — a parity that may be a non-trivial XOR of input bits
after CNOTs have acted. Terms with the same parity merge, and their coefficients
sum mod 8. A coefficient of 2 is an `S` gate (Clifford, T-count zero), so:

```text
before:  T @0 |> CNOT @(0,1) |> T @0      T-count 2
after:   S @0 |> CNOT @(0,1)             T-count 0
```

The CNOT network is preserved; only the T-count content is re-synthesized.

### Re-classification after optimization

On Universal circuits, the phase polynomial pass also canonizes maximal initial
and final Clifford layers. If the pass removes all `T`/`T†` from a circuit, it
re-infers the classification and may flip the `clifford` attribute from `false`
to `true`. This means a circuit that was typed `Universal` (because it contained
`T`) can become `Clifford` after optimization — the optimizer discovered the
T gates were redundant. The caller's type signature still says `Universal`
(the type-level bound is an upper bound, not the post-optimization truth),
but the emitted IR reflects the tighter classification.

## Practical implications

When you write a circuit, the classification tells the optimizer what to do:

```kotlin
fn all_clifford(): Circuit<2, 2, 3, Clifford> = circuit {
    H @0 |> CNOT @(0, 1) |> S @0
}

fn with_t(): Circuit<2, 2, 4, Universal> = circuit {
    H @0 |> CNOT @(0, 1) |> T @0 |> T_dag @1
}
```

The `Clifford` circuit will go through stabilizer tableau optimization.
The `Universal` circuit will go through phase polynomial optimization.
You don't choose the optimizer — the type does.

### When to expect optimization gains

The stabilizer tableau pass finds reductions when a Clifford sequence composes
to a shorter Clifford — identities, single Paulis, or shorter CNOT networks.
The phase polynomial pass finds reductions when T gates on the same parity
(through CNOTs) can be merged or cancelled. Both passes run after
`gate_cancellation` and `rotation_merging` in the fixpoint loop
(ADR-0013), so the simple peephole reductions are applied first, then the
classification-specific kernel finds the deeper non-adjacent ones. The
fixpoint iterates until no pass makes a change, so reductions that *enable*
further peephole cancellations are caught in later rounds.

## Next

Now we leave the pure unitary world. The Quantum Monad allows allocation,
measurement, and feed-forward — operations that are not unitary but are
essential for real quantum programs.

→ [The Quantum Monad](../monad/)
