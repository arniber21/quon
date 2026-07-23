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

The linear context is implemented as a Rust data structure (a map from names
to linear types) in `typecheck/linear.rs`, the module that owns resource
bookkeeping: consumption, splitting, and residual joining at branch merges. It
is the same kind of borrow-check reasoning the Rust compiler performs on a
`&mut` reference — the parallel is not metaphorical, it is structural.

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
6. **Re-tensored** — `let (a, b) = destructure(q)` consumes `q` and produces
   `a` and `b`; `(a, b)` consumes both and produces a `QReg<2>`. The original
   names are gone; the new ones carry the linear ownership forward.

After consumption, the name is gone from Δ. Any subsequent reference is a type
error — not a runtime null check, a compile-time error.

## What goes wrong

Here are the canonical mistakes, and how the type system catches each. Each
produces a distinct diagnostic, so you can tell from the message exactly which
linear invariant was violated.

### Using a qubit after measurement

```kotlin
fn bug(): Q<Bit> = run {
    q <- qinit()
    b <- measure(q)
    b2 <- measure(q)
    return b2
}
```

The typechecker removed `q` from Δ when `measure(q)` was processed. The second
`measure(q)` is an unbound reference — the same error as using a variable
after `drop()` in Rust.

```
error: linear resource `q` already consumed
  --> source.qn:4:14
   |
 3 |     b <- measure(q)
   |                  - `q` consumed here
 4 |     b2 <- measure(q)
   |                   ^ `q` not in scope (already consumed)
   |
   = note: a Qubit is linear — it must be used exactly once
```

### Omitting a use

```kotlin
fn bug(): Q<Bit> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0 <- measure(q0)
    return b0
}
```

The typechecker checks Δ at every return point. If any linear binding is
still live, the program is rejected. This is how Quon prevents "qubit leaks" —
a resource that was allocated but never measured or returned.

```
error: linear resource `q1` not consumed
  --> source.qn:4:5
   |
 2 |     (q0, q1) <- bell_state() @ qreg(2)
   |           -- `q1` introduced here
 4 |     return b0
   |     ^^^^^^^^^ `q1` is still live at return
   |
   = note: every Qubit/QReg must be consumed before the run block returns
   = hint: add `discard(q1)` or `measure(q1)` before the return
```

### Cloning

```kotlin
fn bug(): Q<(Qubit, Qubit)> = run {
    q <- qinit()
    return (q, q)
}
```

The first `q` in the tuple consumes it. The second reference is unbound.
No-cloning is not a documentation note — it is enforced by the same mechanism
that prevents double-free in Rust.

```
error: linear resource `q` used twice
  --> source.qn:3:12
   |
 3 |     return (q, q)
   |               ^   ^ first use consumes `q`; second use is unbound
   |
   = note: no-cloning is a type error in Quon, not a runtime check
```

## Classical values are unrestricted

The linear discipline applies only to quantum resources. Classical values —
`Bit`, `Bool`, `Int`, `Float`, and functions over them — live in Γ and follow
ordinary lexical scoping:

```kotlin
fn ok(): Q<Bit> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0 <- measure(q0)
    b1 <- measure(q1)
    let same = b0
    let also = b0
    return b1
}
```

The `Bit` produced by `measure` is classical. It can be copied, stored in a
list, used in an `if` condition multiple times, or ignored. Only the `Qubit`
that was consumed to produce it was linear. This split is what the typechecker
module `classical.rs` (the classical Γ judgment, ADR-0035) owns: arithmetic,
lists, lambdas, and ordinary branching all live in the unrestricted world,
while `linear.rs` owns the Δ side.

## Branching and residuals

When a quantum computation branches (e.g. `if bit then circuit_A else
circuit_B`), each branch sees a **clone** of the linear context. After the
branch, the typechecker requires that both branches left the **same set of
resources live** — the *residual* contexts must match. This is the join operation
that `linear.rs` owns: at the merge point, the two branch contexts are
intersected, and any resource live in one branch but not the other is flagged.

### Both branches must consume the same resources

```kotlin
fn branch_ok(b: Bit, q: Qubit): Q<Qubit> = run {
    result <- (if b then circuit_a() else circuit_b()) @ q
    return result
}
```

Here both `circuit_a` and `circuit_b` consume `q`, so after the `if`, Δ is
empty in both branches — the residual contexts match. The typechecker is
satisfied.

### A residual mismatch error

If one branch consumes a resource but the other does not, the residual contexts
differ and the typechecker rejects the program with a `LinearBranchMismatch`:

```kotlin
fn branch_bug(b: Bit, q: Qubit): Q<Unit> = run {
    if b then {
        discard(q)
    } else {
        -- forgot to consume q!
        return ()
    }
}
```

```
error: linear branch mismatch
  --> source.qn:4:5
   |
 3 |     if b then {
 4 |         discard(q)    -- Δ is empty after this branch
   |     } else {
 6 |         return ()     -- `q` is still live in this branch
   |     }
   |
   = the `then` branch consumed `q`, but the `else` branch left it live
   = both branches of an `if` must leave the same linear resources live
```

The error names the resource, the branch that consumed it, and the branch that
did not. The fix is to consume `q` in both branches — either by measuring,
discarding, or returning it. This rule is what makes feed-forward corrections
safe: every `if bit then correction else identity` must consume the qubit
exactly once on each side.

## Lambda capture rejection

Quantum resources cannot be captured by closures. A lambda (function value)
lives in the unrestricted context Γ, and Γ cannot contain linear bindings —
so a function that closes over a `Qubit` is a type error:

```kotlin
fn capture_bug(q: Qubit): Bit -> Qubit =
    fn (b: Bit) { measure(q); X @0 }
```

```
error: linear resource `q` captured by closure
  --> source.qn:2:5
   |
 2 |     fn (b: Bit) { measure(q); X @0 }
   |                        ^ `q` is linear but the closure is unrestricted
   |
   = note: closures live in Γ (unrestricted); linear values live in Δ
   = a linear value captured by a closure could be used twice (once per call)
```

The reason is soundness: a closure can be called many times, and each call would
consume the captured qubit — a first-order violation of single use. Even a
closure called once is rejected, because the type system cannot prove
"called exactly once" for arbitrary function values. This is the same restriction
Rust applies to `Fn`/`FnMut`/`FnOnce` closures: linear captures are only
permitted in `FnOnce`, and Quon's closures are the `Fn` (unrestricted) kind. If
you need to thread a qubit through a higher-order structure, use a `circuit`
value or a `run` block — both are designed to carry linear resources safely.

## `QecBlock` linearity

Encoded logical qubits — `QecBlock<F, d>` values — are linear for exactly the
same reasons as bare `Qubit`s. You cannot clone an encoded block, drop it without
running a syndrome round, or let it escape a scope. The linear context tracks
`QecBlock` alongside `Qubit` and `QReg`:

```kotlin
fn qec_ok(block: QecBlock<Repetition, 3>): Q<QecBlock<Repetition, 3>> = run {
    after <- memory_round(block)
    return after
}
```

`memory_round` consumes `block` and produces a fresh `QecBlock` — the linear
ownership is threaded forward, just as with circuit application on bare qubits.
If you forgot to consume the block, the typechecker would reject it with the same
"linear resource not consumed" diagnostic:

```kotlin
fn qec_bug(block: QecBlock<Repetition, 3>): Q<Unit> = run {
    return ()
}
```

```
error: linear resource `block` not consumed
  --> source.qn:2:5
   |
 1 | fn qec_bug(block: QecBlock<Repetition, 3>): Q<Unit> = run {
 2 |     return ()
   |     ^^^^^^^^^ `block` is still live at return
   |
   = a QecBlock is linear — run memory_round or a logical gate before returning
```

The linearity of `QecBlock` means you can never accidentally forget a syndrome
extraction round: the encoded block must pass through `memory_round` (or a
logical gate, or explicit discard) before it goes out of scope.

## The borrow-block lifecycle

A `borrow` block introduces an ancilla into Δ for a scoped sub-computation, then
verifies at block exit that the ancilla was consumed. This is a *scoped* linear
check — the linear context is checked at the block boundary, not just at the
function return:

```kotlin
fn borrow_ok(): Q<Unit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        discard(prepared)
    }
}
```

If the borrow body does not consume `anc`, the typechecker rejects the program
at the block's closing brace, with the same "not consumed" diagnostic — but
pointing at the borrow block rather than the function. The borrow block's
linearity is owned by the `monad.rs` judgment module (ADR-0031), which also
enforces the no-escape rule: the borrowed name must not appear in the block's
result type. You will see the full borrow-block mechanics on the borrow page.

## Why this matters

The linear type system turns an entire class of quantum programming bugs —
qubit leaks, double-measurement, unauthorized cloning — into compile-time
errors. In most quantum SDKs, these are runtime failures or, worse, silent
incorrect behavior. In Quon, the program does not compile until every
quantum resource is accounted for.

This is not a linter. It is not an optional pass. It is the type system
itself, woven into every function signature, every `let` binding, and every
`return` statement. The `linear.rs` module is small — its job is bookkeeping,
not inference — but it is the keystone that makes the rest of the type system
trustworthy. Without it, the depth bounds, width checks, and classification
rules would all be checking properties of a program that might not even be
physically realizable.

## Next

Now that you understand how resources are tracked, the next page shows how
circuits compose in parallel and how `Nat` parameters make circuits reusable
at any width.

→ [Parallel composition](../parallel/)
