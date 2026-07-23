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

## Cleanup options

The borrow block accepts three forms of cleanup, all valid:

```kotlin
-- Measurement: produces a classical bit, consumes the qubit
fn measured_ancilla(): Q<Bit> = run {
    borrow anc: Qubit in {
        prepared <- H @ anc
        b <- measure(prepared)
        return b
    }
}

-- Reset: returns the qubit to |0⟩, produces a fresh Qubit (discard it)
fn reset_ancilla(): Q<Unit> = run {
    borrow anc: Qubit in {
        used <- some_computation() @ anc
        cleaned <- reset(used)
        discard(cleaned)
    }
}

-- Discard: explicitly drops the qubit without measuring
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

## How the typechecker enforces it

When the typechecker enters a `borrow` block, it:

1. Introduces the ancilla name(s) into Δ.
2. Typechecks the body under the extended Δ.
3. At block exit, verifies that every borrowed name has been consumed.
4. Verifies that no borrowed name appears in the block's result type.

Step 4 is the no-escape check. If the result is `Q<Qubit>` and the
borrowed `anc` was not consumed, the checker rejects the program — the
qubit would escape its scope. If the result is `Q<Bit>` and `anc` was
consumed by `measure`, the program is accepted.

## ADR history

The original borrow-block terminal policy (ADR-0003) required a structural
`reset`/`discard` as the only valid cleanup. Issue #180 relaxed this:
`measure`, `reset`, and `discard` are all valid cleanup, because each
fully consumes the qubit in a semantically sound way. ADR-0031 records
this resolution. The monad module docs state it explicitly: "Per issue
#180, valid cleanup includes measure, reset, and discard — not only a
structural reset/discard terminal."

## Next

For fault-tolerant computing, Quon introduces encoded logical qubits with
QEC code families. The next page covers `QecBlock` and the QEC builtins.

→ [QEC blocks](../qec/)
