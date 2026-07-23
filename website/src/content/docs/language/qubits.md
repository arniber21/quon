---
title: Qubits and registers
description: Qubit versus QReg<n>, destructuring and reshaping registers, and why Quon forbids register indexing.
sidebar:
  order: 3
---

A circuit type tells you *how many* qubits flow through, but it says nothing
about the qubits themselves as values. In Quon a qubit is a first-class value
with a linear type, and the way you group qubits into registers — and reshape
those registers — is deliberately constrained. This page introduces the two
qubit-bearing types and the operations that change a register's shape, and
explains why Quon refuses to let you index into a register the way a classical
array would.

## `Qubit` and `QReg<n>`

A `Qubit` is a single linear quantum value: one qubit, owned exactly once,
consumed exactly once. A `QReg<n>` is a single linear value that bundles a
*statically known* number `n` of qubits. The `n` is a compile-time `Nat`, so
`QReg<2>` and `QReg<3>` are distinct types — you cannot accidentally pass a
three-qubit register where a two-qubit register is expected. Both `Qubit` and
`QReg` live in the linear context, which means every qubit they contain must be
accounted for before it goes out of scope.

The distinction matters because of how composition works. A `Circuit<n, m, ...>`
applied to a `QReg<n>` consumes the whole register and produces a `QReg<m>`;
the register is the unit of application. But many algorithms need to reach
*individual* qubits — to apply a gate to one and not its neighbor, or to pair a
qubit from one register with a qubit from another. Quon does not let you reach
into a register by index. Instead it forces you to *destructure* the register
into named qubits, making ownership explicit at every step.

## Destructuring and reshaping

`destructure` takes a `QReg<n>` and splits it into `n` individual `Qubit`
values, each of which you bind to a name. Once destructured, the qubits are
independent linear values you can reorder, recombine, or feed to circuits one at
a time. To put qubits back together — say, to reverse a pair — you re-tensor
them into a new register:

```kotlin
fn reverse_pair(q: QReg<2>): QReg<2> =
    let (left, right) = destructure(q)
    in (right, left)
```

Here `destructure(q)` yields the two qubits as `left` and `right`; the
expression `(right, left)` tensors them back into a `QReg<2>` in reversed order.
Two further operations complete the reshaping toolkit. `split(k, reg)` divides a
register into its first `k` qubits and the remaining tail, returning both as
separate registers. `tensored` combines two registers (or qubits) into one
wider register. Together these let you change a register's shape — split it,
reorder pieces, join pieces from different sources — while preserving linear
ownership at every step.

## Why no indexing

A natural question: why not just write `reg[1]` to grab the second qubit? The
answer is aliasing. In a classical array, `reg[1]` is a *reference* into
storage that the array still owns; you can read it, copy it, and the array is
unchanged. A qubit cannot be copied (no-cloning), and a "reference" to a qubit
that someone else still holds would be a second name for the same linear
resource — exactly the ambiguity that breaks linearity. If two names could reach
the same qubit, the typechecker could no longer guarantee single use.

Quon closes that hole by making `destructure`, `split`, and `tensored` the
*only* way to change a register's shape, and by having each of them move
ownership rather than alias it. When you `destructure` a register, the original
register is consumed and ceases to exist; the qubits now live only under their
new names. There is no way to hold the register *and* an element of it at the
same time. This keeps the linear context simple — a `QReg<n>` is always one
linear value, never a bag of individually borrowable aliases — and it keeps the
no-cloning guarantee a local, checkable fact rather than a global hope.

## Splitting in practice

A common pattern is to encode one logical qubit into several physical ones,
operate, then decode back to one. Decoding leaves a multi-qubit register from
which you want only the first qubit; `split` extracts it while carrying the
remainder (here ignored with `_`) along for cleanup:

```kotlin
fn bit_flip_round(logical: Qubit): Q<QReg<1>> = run {
    encoded        <- (encode()) @ logical
    (data, s1, s2) <- syndrome_measure(encoded)
    corrected      <- correct(data, s1, s2)
    decoded        <- adjoint(encode()) @ corrected
    let (out, _rest) = split(1, decoded)
    return out
}
```

Don't worry about the `run` block, `measure`, or `adjoint` yet — those belong to
later pages. Notice the shape: `encode` widens one qubit into a register,
`syndrome_measure` and `correct` work on the register as a whole, `adjoint`
decodes it back, and `split(1, decoded)` pulls out the single logical qubit as
`out` while `_rest` names the auxiliary qubits for discard. Every qubit is
accounted for by name; nothing is indexed, nothing is aliased.

## Next

You have now seen that qubits are linear values consumed exactly once — but we
have only asserted that. The next page makes the linear type system precise: the
linear context, what "consume" really means, which values are unrestricted, and
the shape of the type errors you get when the rules are broken.

→ [The linear type system](/language/linearity/)
