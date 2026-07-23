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

### A width mismatch is a type error

Because the interface is typed, passing the wrong number of qubits is a
compile-time error, not a runtime surprise. If you try to apply `encode` — which
expects 1 qubit in — to a two-qubit register, the typechecker rejects it before
anything reaches the elaborator:

```kotlin
fn bad_encode(): Q<QReg<3>> = run {
    out <- encode() @ qreg(2)
    return out
}
```

```
error: width mismatch in circuit application
  --> source.qn:2:14
   |
 2 |     out <- encode() @ qreg(2)
   |              ^^^^^^^    ^^^^^^^
   |              expects 1 input qubit, got 2
   |
   = Circuit<1, 3, ...> requires a QReg<1> or a single Qubit
```

The same check fires at every `|>` composition: the left circuit's output width
must equal the right circuit's input width, or the program does not compile.

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
caught long before emission:

```kotlin
fn out_of_range(): Circuit<2, 2, 1, Clifford> = circuit {
    H @5
}
```

```
error: gate position out of range
  --> source.qn:2:7
   |
 2 |     H @5
   |       ^ position 5 exceeds circuit width 2
   |
   = valid positions for Circuit<2, ...> are 0..1
```

Parameterized rotations wrap their angle in parentheses and then target a
position, so a Z-rotation reads `(Rz theta) @1`. The angle can be a literal or a
classical parameter threaded in from the function signature.

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

### Single-qubit gates: Paulis, Clifford, and T

The single-qubit primitives span both classification tiers. The Paulis `X`, `Y`,
`Z` and the Hadamard `H` and phase gate `S` are all Clifford, as is the identity
`I`. The `T` gate (π/8 phase) is non-Clifford — it is the resource that lifts
Clifford circuits to universality:

```kotlin
fn clifford_gates(): Circuit<1, 1, 1, Clifford> = circuit { S @0 }

fn universal_gate(): Circuit<1, 1, 1, Universal> = circuit { T @0 }
```

The classification is inferred from the gate, not annotated. If you declare
`Clifford` but use `T`, the typechecker catches the contradiction:

```kotlin
fn wrong_class(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }
```

```
error: classification mismatch
  --> source.qn:2:26
   |
 2 | fn wrong_class(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }
   |                                                ^^^^^^^^^^^^^^^^^^^
   |  inferred classification: Universal
   |  declared classification: Clifford
   |
   = T is a non-Clifford gate; Universal ⊉ Clifford
```

### Rotations: `Rz`, `Rx`, `Ry`

Rotations carry a continuous angle, so they are `Universal` by default. The
syntax wraps the angle before the position:

```kotlin
fn rz_gate(theta: Float): Circuit<1, 1, 1, Universal> = circuit {
    (Rz theta) @0
}
```

A special case: `Rz` at a multiple of `π/2` is Clifford (it equals `S`, `Z`, or
`S†`), so the compiler *may* re-classify a rotation as Clifford if the angle is
a compile-time constant at a Clifford-specializable value. In practice you write
`S` directly when you know the angle; the rotation form is for parametric work
where the angle is a runtime variable the optimizer cannot fold.

### Two-qubit gates: `CNOT` and `CZ`

Both two-qubit primitives are Clifford. They take an ordered pair of positions:
`CNOT @(control, target)` and `CZ @(0, 1)`:

```kotlin
fn entangler(): Circuit<2, 2, 1, Clifford> = circuit { CNOT @(0, 1) }

fn phase_entangler(): Circuit<2, 2, 1, Clifford> = circuit { CZ @(0, 1) }
```

`CZ` is symmetric — `CZ @(0, 1)` and `CZ @(1, 0)` are the same gate — but
`CNOT` is not: `CNOT @(0, 1)` and `CNOT @(1, 0)` produce different unitaries.
The typechecker accepts both orderings (both are valid `(Nat, Nat)` pairs within
the width), but the optimizer treats them as distinct circuits.

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

### Composition chains of mixed class

When you chain gates of mixed classification, the join propagates through the
whole chain. A single `T` anywhere in the sequence makes the entire composite
`Universal`:

```kotlin
fn mixed_chain(): Circuit<2, 2, 4, Universal> = circuit {
    H @0 |> CNOT @(0, 1) |> T @0 |> T_dag @1
}
```

Here `H` and `CNOT` are Clifford, but the `T` and `T_dag` gates force the
declared class to `Universal`. If you declared `Clifford`, the typechecker would
reject it — the inferred class is `Universal`, and `Universal ⊉ Clifford`. The
depth is `4` because all four operations are sequential on overlapping qubits.

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

### Passing and returning circuits

A circuit value can be stored in a variable, returned from a function, or passed
as an argument to another function — exactly like any other value. This is what
makes combinators like `repeat` and `controlled` possible: they are ordinary
functions that take a `Circuit` and return a `Circuit`:

```kotlin
fn with_control(c: Circuit<1, 1, d, Clifford>): Circuit<2, 2, d + 1, Clifford> =
    controlled(c)
```

`with_control` takes any single-qubit Clifford circuit and returns a two-qubit
controlled version. The type signature carries the transformation: the width
grows by 1 (the control qubit), the depth grows by 1, and the classification is
preserved. The caller never needs to inspect the body — the type is the
contract.

### Circuit combinators in the standard library

Quon's standard combinators are themselves typed circuit-returning functions:

| Combinator | Input | Output | Depth rule |
|---|---|---|---|
| `adjoint(c)` | `Circuit<n, m, d, C>` | `Circuit<m, n, d, C>` | same depth, class preserved |
| `controlled(c)` | `Circuit<n, m, d, C>` | `Circuit<n+1, m+1, d+1, C>` | depth + 1 |
| `repeat(k, c)` | `Circuit<n, n, d, C>` | `Circuit<n, n, k*d, C>` | depth × k |
| `par { c } * k` | `Circuit<n, n, d, C>` | `Circuit<k*n, k*n, d, C>` | depth unchanged |

Every row is a *type-level* identity: the output type is a function of the input
type, checked by the typechecker before the circuit is ever built. You will see
`repeat` and `par` in detail on the next pages; `adjoint` and `controlled`
appear throughout the cookbook.

## What the elaborator does with parametric circuits

When a parametric circuit like `hadamard_layer(n)` is called at a concrete
width, the **elaborator** partially evaluates it into a first-order gate DAG —
a `SpecializedCircuit` tree of `Compose` / `GateApp` / `Adjoint` nodes over
concrete qubit indices and literal angles. No `Nat` parameters survive this
step; the elaborator resolves loop bounds, instantiates parallel compositions,
and folds symbolic depth into a concrete number. This is the boundary between
the symbolic world (where the typechecker reasons) and the concrete world (where
the lowerer emits MLIR). The `SpecializedCircuit` module is deliberately
Melior-free so that specialization can be unit-tested without linking LLVM — a
decision recorded in ADR-0038.

The key insight is that the typechecker has *already* proven the depth and width
bounds against the symbolic form, so the elaborator does not re-check them. It
simply unfolds the structure the typechecker approved. If a parametric circuit's
depth bound is `steps * 2`, the elaborator substitutes the concrete `steps`
value and the lowerer emits that many gate layers — the proof that `steps * 2`
is a valid upper bound was discharged earlier, against the symbolic expression.

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
