---
title: Language guide
description: Write unitary circuits and dynamic quantum computations in Quon.
---

Quon is a functional quantum language with linear types. Its central value is a
`Circuit`; dynamic operations such as allocation and measurement live in the
Quantum Monad. This guide introduces the language concepts you need to read and
write small Quon programs.

For the complete type rules and built-in catalog, see the
[language and compiler specification](https://github.com/arniber21/quon/blob/main/SPEC.md).

## Circuits

`Circuit<n, m, d, C>` is a unitary quantum morphism. It consumes exactly `n`
qubits, produces exactly `m` qubits, has gate depth bounded by `d`, and has
Clifford classification `C`.

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}
```

Inside `circuit { }`, `@` targets a gate at one or more zero-based qubit
positions. A circuit is a value: it can be returned from a function, passed to a
combinator, or applied to qubits.

## Qubit and QReg

A `Qubit` is one linear quantum register element. A `QReg<n>` is a single
linear value containing a statically known number of qubits. Destructure a
`QReg` before working with its individual `Qubit` values:

```kotlin
fn reverse_pair(q: QReg<2>): QReg<2> =
    let (left, right) = destructure(q)
    in (right, left)
```

There is no register indexing that aliases an element. Use `destructure`,
`split`, and `tensored` to change a register's shape while preserving ownership.

## The linear context and no-cloning

Quon tracks quantum values in a **linear context**. Every `Qubit`, `QReg`, and
circuit value in that context must be consumed exactly once. Measurement is one
way to consume a qubit:

```kotlin
fn read(q: Qubit): Q<Bit> = run {
    measure(q)
}
```

Using `q` again after `measure(q)`, omitting its use, or trying to construct
`(q, q)` is a type error. Classical values such as `Bit`, `Bool`, and `Int` are
unrestricted and may be reused.

## Sequential and parallel composition

The `|>` operator performs **sequential composition**. The output width on the
left must equal the input width on the right, and their depth bounds add:

```kotlin
fn prepare_one(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0
}
```

The `par` form performs **parallel composition** over disjoint qubits. In the
surface language, `par { c } * n` makes an `n`-fold parallel composition of
`c`: widths multiply by `n`, while the depth remains the body's depth.

```kotlin
fn had_one(): Circuit<1, 1, 1, Clifford> = circuit {
    H @0
}

fn hadamard_layer(n: Nat): Circuit<n, n, 1, Clifford> =
    par { had_one() } * n
```

Use `|>` for layers that must happen one after another and `par` for independent
work that can occupy the same depth layer.

## Borrow blocks

A **borrow block** allocates scoped ancilla qubits. Borrowed qubits enter the
linear context, cannot escape the block, and must be consumed before the block
ends. Measurement, reset, discard, or another consuming operation can discharge
the borrowed value depending on the computation.

```kotlin
fn use_ancilla(): Q<Unit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        discard(prepared)
    }
}
```

Returning `anc` from the block is rejected. This makes the lifetime of
temporary quantum resources explicit and keeps scoped ancilla use visible to
the typechecker.

## The Quantum Monad

`Q<T>` is the **Quantum Monad**: a quantum computation that may allocate,
apply circuits, perform mid-circuit measurement, and return a value of type
`T`. Write these computations with `run { }`.

```kotlin
fn hello_bell(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0 <- measure(q0)
    b1 <- measure(q1)
    return (b0, b1)
}
```

`<-` sequences a quantum computation and binds its result. `return` lifts a
value into the Quantum Monad. Pure `let` bindings may be mixed into the same
block.

## Depth bounds

The third `Circuit` index is an upper bound on gate depth, not a gate count.
Sequential depth adds; parallel depth takes the maximum. For example, every
Hadamard in `hadamard_layer` above runs in parallel, so its depth is `1`, while
this repeated circuit has depth `steps * 2`:

```kotlin
fn step(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0
}

fn evolution(steps: Nat): Circuit<1, 1, steps * 2, Universal> =
    repeat(steps, step())
```

Depth bounds may contain symbolic arithmetic over parameters. Quon infers a
tight compositional depth for an expression and checks that it fits the
declared bound; a looser valid upper bound is accepted.

## Clifford classification

Every circuit has an inferred **Clifford classification**: `Clifford` or
`Universal`. A circuit containing only Clifford primitives remains `Clifford`;
including a non-Clifford primitive makes the composition `Universal`.

```kotlin
fn basis_change(): Circuit<1, 1, 1, Clifford> = circuit {
    H @0
}

fn phase_resource(): Circuit<1, 1, 1, Universal> = circuit {
    T @0
}
```

The compiler infers classification bottom-up from primitives and checks the
classification written in a function's return type. `Clifford` is below
`Universal`, so a Clifford circuit can satisfy a `Universal` bound, but a
Universal circuit cannot satisfy a `Clifford` bound.

## Putting it together

The complete Bell program above demonstrates the usual split:

1. Build a unitary `Circuit` in `circuit { }`.
2. Enter `run { }` to allocate a `QReg`.
3. Apply the circuit, consuming the input register and producing fresh linear
   outputs.
4. Measure each output exactly once and return unrestricted classical bits.

Browse the repository's
[tracked programs](https://github.com/arniber21/quon/tree/main/frontend/tests/fixtures)
for larger examples that exercise the same concepts.
