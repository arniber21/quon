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

## Subtyping: Clifford ⊑ Universal

`Clifford` is a subtype of `Universal`. This means a `Clifford` circuit can
be used wherever a `Universal` circuit is expected — but not vice versa:

```kotlin
fn expects_universal(c: Circuit<1, 1, 1, Universal>): Q<Qubit> = run {
    q <- c @ qinit()
    return q
}

fn ok(): Q<Qubit> = run {
    -- Clifford circuit passed where Universal is expected — fine
    q <- expects_universal(circuit { H @0 })
    return q
}
```

This subtyping is sound because every Clifford circuit is trivially a
Universal circuit (just with no non-Clifford gates). The reverse is not
sound: a circuit containing `T` cannot satisfy a `Clifford` bound.

## How the optimizer uses classification

The Clifford classification drives the compiler's optimization strategy
(ADR-0013, ADR-0039):

- **For `Clifford` circuits**, the compiler applies the **Aaronson-Gottesman
  stabilizer tableau** simulator. This represents the circuit's action on
  the Pauli group generators as a GF(2) matrix, detects when sequences
  compose to identity (or a single Pauli), and replaces them with
  shorter equivalents. This catches non-adjacent simplifications that
  peephole gate cancellation misses (e.g. `S⁴ = I` when the four `S` gates
  are separated by other Clifford gates).

- **For `Universal` circuits**, the compiler applies the **phase polynomial**
  pass. This extracts the non-Clifford (T-count) content as a sum of linear
  Boolean phase terms, merges and cancels terms algebraically (not just
  by adjacency), and re-synthesizes gates from the reduced polynomial.
  This catches non-adjacent T-count reductions like `T·CNOT·T → S·CNOT`
  (T-count 2→0).

Both algorithms handle **non-adjacent** gates — the key advantage over the
peephole gate-cancellation pass, which only catches immediately adjacent
redundant pairs.

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

## Next

Now we leave the pure unitary world. The Quantum Monad allows allocation,
measurement, and feed-forward — operations that are not unitary but are
essential for real quantum programs.

→ [The Quantum Monad](../monad/)
