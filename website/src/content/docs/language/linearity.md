---
title: The linear type system
description: How Quon's linear types enforce no-cloning, no-dropping, and exact-once consumption at compile time.
---

The linear type system is Quon's most distinctive feature. It is the reason a
Quon program cannot accidentally clone a qubit, forget to measure one, or let
an ancilla escape its scope. These are not runtime checks or linter warnings —
they are **type errors**, caught before a single gate is lowered to IR.

## The linear context Δ

Every Quon expression is checked under two contexts:

- **Γ** (Gamma) — the unrestricted classical environment. Names bound to
  `Int`, `Float`, `Bool`, `Bit`, and function types live here. They can be
  used zero, one, or many times — standard Hindley-Milner scope.
- **Δ** (Delta) — the linear context. Names bound to `Qubit`, `QReg`,
  `QecBlock`, and `Circuit` values live here. Every binding in Δ **must be
  consumed exactly once** in the expression's scope.

When the typechecker processes `let (q0, q1) <- bell_state() @ qreg(2)`, it
introduces `q0` and `q1` into Δ. From that point until they are consumed,
they are "live" linear resources. The checker physically removes each name
from Δ when it is used — there is no "used" flag, just presence or absence.

## What counts as consumption

A linear resource is consumed when it is:

1. **Measured** — `measure(q)` consumes a `Qubit` and produces a `Bit`.
2. **Passed to a circuit** — `bell_state() @ qreg(2)` consumes the `QReg<2>`
   and produces two fresh `Qubit` values.
3. **Returned** — `return q` consumes `q` and lifts it into the monadic result.
4. **Discarded** — `discard(q)` explicitly consumes a `Qubit` without
   producing a classical value. This is only valid in positions where dropping
   is semantically acceptable (e.g. ancilla cleanup inside a `borrow` block).
5. **Reset** — `reset(q)` consumes a `Qubit` in an unknown state and produces
   a fresh `Qubit` in `|0⟩`.

After consumption, the name is gone from Δ. Any subsequent reference is a type
error — not a runtime null check, a compile-time error.

## What goes wrong

Here are the three canonical mistakes, and how the type system catches each.

### Using a qubit after measurement

```kotlin
fn bug(): Q<Bit> = run {
    q <- qinit()
    b <- measure(q)
    -- q is no longer in Δ here
    b2 <- measure(q)  -- TYPE ERROR: q already consumed
    return b2
}
```

The typechecker removed `q` from Δ when `measure(q)` was processed. The second
`measure(q)` is an unbound reference — the same error as using a variable
after `drop()` in Rust.

### Omitting a use

```kotlin
fn bug(): Q<Bit> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0 <- measure(q0)
    -- q1 is still in Δ — never consumed
    return b0  -- TYPE ERROR: linear resource q1 not consumed
}
```

The typechecker checks Δ at every return point. If any linear binding is
still live, the program is rejected. This is how Quon prevents "qubit leaks" —
a resource that was allocated but never measured or returned.

### Cloning

```kotlin
fn bug(): Q<(Qubit, Qubit)> = run {
    q <- qinit()
    return (q, q)  -- TYPE ERROR: q used twice
}
```

The first `q` in the tuple consumes it. The second reference is unbound.
No-cloning is not a documentation note — it is enforced by the same mechanism
that prevents double-free in Rust.

## Classical values are unrestricted

The linear discipline applies only to quantum resources. Classical values —
`Bit`, `Bool`, `Int`, `Float`, and functions over them — live in Γ and follow
ordinary lexical scoping:

```kotlin
fn ok(): Q<Bit> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0 <- measure(q0)
    b1 <- measure(q1)
    -- b0 and b1 are Bits: unrestricted, can be reused freely
    let same = b0  -- this is fine
    let also = b0  -- and this is fine too
    return b1
}
```

The `Bit` produced by `measure` is classical. It can be copied, stored in a
list, used in an `if` condition multiple times, or ignored. Only the `Qubit`
that was consumed to produce it was linear.

## Branching and residuals

When a quantum computation branches (e.g. `if bit then circuit_A else
circuit_B`), each branch sees a **clone** of the linear context. After the
branch, the typechecker requires that both branches left the **same set of
resources live**:

```kotlin
fn branch_ok(q: Qubit): Q<Qubit> = run {
    -- q is in Δ
    b <- measure_somehow()
    result <- (if b then circuit_a() else circuit_b()) @ q
    -- both branches consumed q, so Δ is empty in both
    return result
}
```

If one branch consumes `q` but the other doesn't, the typechecker rejects the
program with a `LinearBranchMismatch` error, pointing at the branch that
failed to consume the resource.

## Why this matters

The linear type system turns an entire class of quantum programming bugs —
qubit leaks, double-measurement, unauthorized cloning — into compile-time
errors. In most quantum SDKs, these are runtime failures or, worse, silent
incorrect behavior. In Quon, the program does not compile until every
quantum resource is accounted for.

This is not a linter. It is not an optional pass. It is the type system
itself, woven into every function signature, every `let` binding, and every
`return` statement.

## Next

Now that you understand how resources are tracked, the next page shows how
circuits compose in parallel and how `Nat` parameters make circuits reusable
at any width.

→ [Parallel composition](../parallel/)
