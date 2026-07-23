---
title: Circuits and gates
description: The Circuit<n, m, d, C> type, gate placement with @, and sequential composition with |>.
sidebar:
  order: 2
---

The previous page introduced the central idea of Quon: a circuit is a typed
value. This page makes that concrete by unpacking the circuit type itself and
showing how gates are placed and composed inside a `circuit { }` block. By the
end you should be able to read a circuit definition and say, at a glance, how
many qubits it touches, how deep it is, and what class of gates it uses — all
from the type, before looking at the body.

## The four indices of `Circuit<n, m, d, C>`

Every circuit value carries four pieces of information in its type:
`Circuit<n, m, d, C>`. The first two are the *interface* — `n` is the number of
qubits the circuit consumes as input, and `m` is the number it produces as
output. The last two are *properties* — `d` is an upper bound on gate depth, and
`C` is the Clifford classification, either `Clifford` or `Universal`.

The interface is exact and unforgiving. A `Circuit<2, 2, ...>` consumes exactly
two qubits and produces exactly two: not one, not three. This is checked
statically, which is what lets you compose circuits knowing the widths will line
up. The properties are more permissive — depth and classification are *bounds*
that admit subtyping, which later pages explore. For now, read the type as a
contract: *give me `n` qubits, I will give back `m` qubits, using at most `d`
layers of gates, drawn from class `C`*. Note that a circuit need not be square:
`encode` below widens one qubit into three, so its input and output widths
differ. That is perfectly legal — `n` and `m` are independent.

```kotlin
fn encode(): Circuit<1, 3, 2, Clifford> = circuit {
    CNOT @(0, 1) |> CNOT @(0, 2)
}
```

## Placing gates with `@`

Inside a `circuit { }` block, gates are placed onto qubit *positions* with the
`@` operator. Positions are zero-based indices into the circuit's input
register, so within a circuit over `n` qubits the valid positions are `0` to
`n-1`, checked at compile time. A single-qubit gate takes one position; a
two-qubit gate takes a tuple:

```kotlin
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}
```

`H @0` places a Hadamard on qubit 0. `CNOT @(0, 1)` places a CNOT with control
on qubit 0 and target on qubit 1. The `@` is read as "at": H *at* zero, CNOT
*at* (zero, one). Because positions are compile-time integers checked against
the circuit's `n`, asking for `H @5` inside a two-qubit circuit is a type error
caught long before emission. Parameterized rotations wrap their angle in
parentheses and then target a position, so a Z-rotation reads `(Rz theta) @1`.

## The gate catalog

Quon ships a fixed set of gate primitives rather than letting you name arbitrary
unitaries. Single-qubit gates include the Paulis `X`, `Y`, `Z`, the Clifford
`H` and `S`, the identity `I`, and the non-Clifford `T`. Two-qubit gates include
`CNOT` and `CZ`. Each primitive has a known type and a known Clifford
classification: every single-qubit primitive is `Circuit<1, 1, 1, C>` and every
two-qubit primitive is `Circuit<2, 2, 1, C>`, with `C` inferred from the gate
itself. The rotation family — `Rz`, `Rx`, `Ry` — is `Universal` for arbitrary
angles (with the special-case exception of `Rz` at multiples of `π/2`, which
collapses to Clifford). This fixed catalog is what lets the compiler reason
about each gate's cost and class without inspecting a matrix.

## Sequential composition with `|>`

Gates and sub-circuits are strung together with the `|>` operator, which
performs **sequential composition** — "do this, then that." Composition is not a
free-for-all: the output width of the left circuit must equal the input width
of the right, because the qubits the left side produces are exactly the qubits
the right side consumes. When the widths match, the composite is well-formed and
its properties combine predictably: depths add (each stage happens after the
previous), and the classification becomes the *join* of the two — a composition
involving any non-Clifford gate is itself non-Clifford.

```kotlin
fn prepare_one(): Circuit<1, 1, 2, Universal> = circuit {
    H @0 |> T @0
}
```

`prepare_one` applies a Hadamard then a T gate to a single qubit. Its depth is
`2` because the two gates are sequential — the T cannot start until the H
finishes, since both act on qubit 0. Its classification is `Universal` because
T is a non-Clifford gate; even though H is Clifford, the composition's class is
the join, and `Clifford ⊔ Universal = Universal`. The width rule is what
catches a common mistake: composing a circuit that outputs 3 qubits with one
that expects 2 is a static error, because `3 ≠ 2`. You will see the depth and
classification rules formalized on later pages; the point here is that they fall
out naturally from treating `|>` as composition of typed values.

## Circuits are combinable values

Because a circuit is a value, you do not only compose gates with `|>`. You can
also hand a circuit to a *combinator* that produces a new circuit. Two of the
most important are `adjoint` and `controlled`. `adjoint(c)` returns the unitary
inverse of `c` — invaluable for "undo" patterns like decoding an encoded logical
qubit. `controlled(c)` adds a control qubit, turning a `Circuit<n, m, d, C>`
into a `Circuit<n+1, m+1, d+1, C>`. Both preserve the circuit's classification and
both are themselves circuits, so they compose with everything else:

```kotlin
fn decode(): Circuit<3, 1, 2, Clifford> = adjoint(encode())
```

Here `adjoint(encode())` is the inverse of the `encode` circuit from earlier,
and it is itself a typed `Circuit` value that can be applied, composed, or passed
further. This is the payoff of values-over-side-effects: every transformation on
a circuit is just another function returning another circuit, and the type tells
you the new interface without running anything.

## Why circuits-as-values matters

It is worth pausing on why this is worth the type annotation. When a circuit is
an imperative side-effect log, composition is ad-hoc: you write helper functions
that mutate a shared object, and the only way to know the result is correct is
to trace the execution. When a circuit is a *value*, composition is *algebra*.
`bell_state()` is a value of a known type; `prepare_one()` is a value of a known
type; combining them with `|>` or passing them to combinators like `repeat` or
`controlled` is just building a larger value from smaller ones, with the types
guaranteeing the interfaces line up.

This pays off in three ways. First, **composability**: any circuit can be
returned, stored, or handed to a combinator, because it is an ordinary value.
Second, **typechecking before hardware**: width mismatches, out-of-range
positions, and depth overruns are caught by the typechecker, not discovered at
run time. Third, **optimization**: because a `circuit { }` block is a pure,
side-effect-free value, the compiler can rewrite it with algebraic identities —
cancelling adjacent inverse gates, merging rotations, simplifying Clifford
regions — with mathematical certainty that the observable behavior is unchanged.
The dynamic, effectful side (which you will meet soon) gets none of those
optimizations, which is exactly why Quon keeps the two worlds separate.

## Next

Circuits act on qubits, but so far we have only referred to qubits by position.
The next page introduces the qubit values themselves — the difference between a
single `Qubit` and a `QReg<n>` register, how to destructure and reshape
registers, and why Quon forbids indexing into them the way a classical array
would.

→ [Qubits and registers](/language/qubits/)
