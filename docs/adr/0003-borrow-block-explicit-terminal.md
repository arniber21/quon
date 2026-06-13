# Borrow block ancilla safety enforced via explicit terminal, not theorem proving

A `borrow q: Qubit in { body }` block is well-typed only if the final use of `q` in the body is `reset(q)` (which returns a fresh |0⟩ qubit) or `discard(q)` (measure and discard). The type checker enforces this as a structural constraint — it inspects the last statement of the borrow body that mentions `q` and rejects the program if it is not one of these two forms.

## Considered Options

**Track a borrow post-condition in the type checker** — introduce a third context `Ψ` for borrowed qubits and verify that `q` is provably in state |0⟩ at the block exit via symbolic reasoning over the body. Rejected as disproportionately complex: it requires reasoning about quantum state through arbitrary circuit expressions, which is undecidable in general.

**Rely on the compiler-assisted uncomputation pass** — let the type checker only verify that `q` doesn't escape the borrow scope, and have the `compiler-uncomputation` optimization pass append `adjoint(body)` to restore |0⟩ automatically. Rejected because correctness should not depend on an optimization pass running; the static guarantee must be unconditional.

## Consequences

Users must always explicitly terminate a borrow block with `reset` or `discard`. This is more verbose than auto-uncomputation but the intent is always visible at the call site. The `compiler-uncomputation` pass (Phase 4) still runs as an optimization — it can eliminate redundant manual uncomputation — but it is not load-bearing for correctness.
