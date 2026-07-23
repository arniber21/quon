---
title: Borrow blocks and ancilla management
description: Scoped ancilla allocation with no-escape guarantees enforced by the linear type system.
---

Real quantum algorithms often need temporary workspace — ancilla qubits that
are allocated, used for a sub-computation, and then returned to `|0⟩` or
measured away. Quon's `borrow` block makes this explicit, with lifetime
guarantees enforced by the type system.

## The borrow block

A `borrow` block allocates one or more ancilla qubits for a scoped
sub-computation:

```kotlin
fn use_ancilla(): Q<Unit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        discard(prepared)
    }
}
```

Inside the block, `anc` is a linear resource in Δ — it behaves exactly like
any other `Qubit`. The typechecker enforces three rules:

1. **Consumed before exit**: `anc` must be consumed (measured, discarded,
   or reset) before the block ends.
2. **No escape**: `anc` must not appear in the block's result. It cannot
   be returned, passed to an outer scope, or stored in a data structure
   that outlives the block.
3. **Valid cleanup**: the consumption must be through an approved operation
   — `measure`, `reset`, or `discard` (per issue #180, not only structural
   reset/discard as older wording implied).

## Why the no-escape rule matters

Ancilla qubits are temporary. If one could escape its borrow block, the
caller would receive a qubit whose state is unknown (it was used internally
and may not be in `|0⟩`). The no-escape rule prevents this at the type
level — it is the same mechanism that prevents dangling references in
Rust: the lifetime of the borrowed resource is bounded by the block scope.

### What the typechecker checks at block exit

When the typechecker enters a `borrow` block, it:

1. Introduces the ancilla name(s) into Δ.
2. Typechecks the body under the extended Δ.
3. At block exit, verifies that every borrowed name has been consumed.
4. Verifies that no borrowed name appears in the block's result type.

Step 4 is the no-escape check. If the result is `Q<Qubit>` and the
borrowed `anc` was not consumed, the checker rejects the program — the
qubit would escape its scope. If the result is `Q<Bit>` and `anc` was
consumed by `measure`, the program is accepted.

The check is structural: the typechecker walks the block's result type and
looks for any occurrence of the borrowed name's type. A `Q<Qubit>` result
that is *not* the borrowed ancilla (e.g. a qubit from outside the block) is
fine — the checker tracks *which* qubit is the borrowed one, not just the
type. This is why the borrow block is a scoped construct, not a function:
the typechecker can match the specific linear binding to the result.

## Multi-qubit borrow

A `borrow` block can allocate multiple ancillas at once, each named in the
binding:

```kotlin
fn two_ancilla(): Q<Bit> = run {
    borrow (a1: Qubit, a2: Qubit) in {
        p1 <- H @ a1
        p2 <- H @ a2
        entangled <- CNOT @(0, 1) @ (p1, p2)
        b1 <- measure(p1)
        b2 <- measure(p2)
        return b1
    }
}
```

Both `a1` and `a2` are introduced into Δ at block entry. Both must be
consumed before exit — if either survives, the typechecker rejects the
program. The no-escape check applies to both: neither may appear in the
result type.

### Register borrow

You can also borrow a register of ancillas, which is useful when a
sub-computation needs a workspace of known width:

```kotlin
fn workspace_borrow(n: Nat): Q<Bit> = run {
    borrow ws: QReg<n> in {
        result <- some_circuit(n) @ ws
        b <- measure_all(result)
        return b[0]
    }
}
```

Here `ws` is a `QReg<n>` — a register of `n` ancillas. The typechecker treats
it as a single linear value: it must be consumed (here by `some_circuit` and
`measure_all`) before the block exits. The no-escape check applies to the
register as a whole.

## Nested borrow

Borrow blocks can be nested — an inner borrow can allocate ancillas within
the scope of an outer borrow:

```kotlin
fn nested_borrow(): Q<Unit> = run {
    borrow outer: Qubit in {
        p_outer <- H @ outer
        borrow inner: Qubit in {
            p_inner <- H @ inner
            discard(p_inner)
        }
        discard(p_outer)
    }
}
```

The typechecker checks each borrow block independently at its own exit. The
inner block must consume `inner` before it closes; the outer block must
consume `outer` before it closes. The linear context is scoped: `inner` is
not visible outside the inner block, and `outer` is not visible after the
outer block. Nesting does not relax any rule — each level enforces
consumption and no-escape independently.

## Cleanup options

The borrow block accepts three forms of cleanup, all valid:

```kotlin
fn measured_ancilla(): Q<Bit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        b <- measure(prepared)
        return b
    }
}

fn reset_ancilla(): Q<Unit> = run {
    borrow anc: Qubit in {
        used <- some_computation() @ anc
        cleaned <- reset(used)
        discard(cleaned)
    }
}

fn discarded_ancilla(): Q<Unit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        discard(prepared)
    }
}
```

The choice of cleanup depends on the algorithm. Measurement is used when
the ancilla's classical outcome is needed (e.g. syndrome extraction in QEC).
Reset is used when the ancilla is pure workspace. Discard is used when the
qubit's state is known to be computational basis and the program does not
need the measurement result.

### When to choose each cleanup

| Cleanup | Produces | Use when |
|---|---|---|
| `measure` | a `Bit` (classical) | the ancilla's outcome feeds forward |
| `reset` | a fresh `\|0⟩` `Qubit` | the ancilla is reused as workspace |
| `discard` | nothing | the ancilla's state is known and irrelevant |

Measurement is the only cleanup that produces a *classical* value — it is
how syndrome extraction works in QEC: the ancilla is entangled with the data
qubits, measured, and the resulting `Bit` is the syndrome. Reset is used when
the ancilla will be re-used in a later sub-computation (you `reset` it back
to `|0⟩` and then apply another circuit). Discard is the lightest cleanup —
it simply removes the qubit from Δ without producing a value, suitable when
the ancilla's state is a known computational-basis state and the program
does not need the measurement result.

## What the typechecker rejects

### Forgetting to consume an ancilla

```kotlin
fn leak(): Q<Unit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        return ()
    }
}
```

```
error: borrowed ancilla `anc` not consumed at block exit
  --> source.qn:4:5
   |
 2 |     borrow anc: Qubit in {
 3 |         prepared <- H @ anc
 4 |         return ()
   |         ^^^^^^^^^ `anc` is still live
   |
   = a borrowed qubit must be consumed (measure, reset, or discard) before exit
```

Note that `prepared` (the output of `H @ anc`) *was* consumed — but `anc`
was the borrowed name, and the typechecker tracks *which* linear binding is
the borrowed one. After `H @ anc` consumes `anc` and produces `prepared`,
`prepared` is the live resource. If `prepared` is not consumed, the error
points at `prepared`, not `anc`. The borrow block's exit check applies to
*all* linear resources introduced inside the block, not just the borrowed
name — but the diagnostic names the specific unconsumed binding.

### Escaping an ancilla

```kotlin
fn escape(): Q<Qubit> = run {
    borrow anc: Qubit in {
        return anc
    }
}
```

```
error: borrowed ancilla `anc` escapes its scope
  --> source.qn:3:16
   |
 3 |         return anc
   |                ^^^ `anc` is a borrowed qubit and cannot escape
   |
   = a borrowed qubit must not appear in the block's result
```

The no-escape check fires when the borrowed name appears in the result. The
fix is to consume `anc` before returning — measure it, reset it, or discard
it, and return a different value.

## ADR history

The original borrow-block terminal policy (ADR-0003) required a structural
`reset`/`discard` as the only valid cleanup. Issue #180 relaxed this:
`measure`, `reset`, and `discard` are all valid cleanup, because each
fully consumes the qubit in a semantically sound way. ADR-0031 records
this resolution. The monad module docs state it explicitly: "Per issue
#180, valid cleanup includes measure, reset, and discard — not only a
structural reset/discard terminal."

The relaxation was motivated by QEC syndrome extraction, where the ancilla's
measurement outcome *is* the useful result — forcing a structural
`reset`/`discard` would throw away the syndrome. The type system still
enforces that the qubit is consumed (no leak, no escape), but the *form* of
consumption is now a programmer choice, not a language mandate.

## Next

For fault-tolerant computing, Quon introduces encoded logical qubits with
QEC code families. The next page covers `QecBlock` and the QEC builtins.

→ [QEC blocks](../qec/)
