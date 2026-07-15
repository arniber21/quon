# Failure clinic: linearity / borrow

Two broken/fixed pairs, each showing a realistic slip caught by `quonc`'s
type checker: reusing a linear resource ("no-cloning"), and letting a
borrowed ancilla escape its scope. The goal isn't the specific bugs — it's
reading the diagnostic, understanding *why* it's correct, and knowing what
the fix actually changes.

## Status

Owned by pack [#188](https://github.com/arniber21/quon/issues/188). Catalog
ids: `workflows/failure-clinic-linearity-broken`,
`workflows/failure-clinic-linearity-fixed`,
`workflows/failure-clinic-borrow-broken`,
`workflows/failure-clinic-borrow-fixed`.

```bash
export QUONC=$PWD/target/debug/quonc   # cargo build -p quonc first
D=samples/workflows/failure_clinic
```

## Pair 1: linearity ("no-cloning")

`linearity_broken.qn` "double-checks" a measurement by reading the same
qubit twice — a realistic slip if you're used to classical values, where
reading twice is free:

```bash
$QUONC $D/linearity_broken.qn
```

```text
Error: type checking failed: linear resource `q0` is used more than once (no-cloning)
  ...
  b1 <- measure(q0)
       ─┬
        ╰── type checking failed: linear resource `q0` is used more than once (no-cloning)
```

The fix, `linearity_fixed.qn`, measures `q1` (the qubit that was actually
sitting unused) instead of re-reading `q0`:

```bash
$QUONC $D/linearity_fixed.qn --emit-qasm
```

```text
OPENQASM 3.0;
include "stdgates.inc";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c[0] = measure q[0];
c[1] = measure q[1];
```

Quantum measurement is destructive — there's no "read again" to fall back
to, which is exactly what the no-cloning check is protecting: a second
`measure(q0)` would either have to invent a value or (worse) silently
re-measure post-collapse state and call it new information. Rejecting it at
typecheck, before either program is elaborated further, is cheaper than
debugging a wrong answer downstream. This is
`frontend/tests/linearity.rs`'s `using_a_qubit_twice_points_at_the_second_use`
test, restated as a runnable program instead of an inline fixture string.

## Pair 2: borrow escape

`borrow_broken.qn` allocates a scratch qubit with `borrow anc: Qubit in {
... }`, operates on it, and then tries to return it:

```bash
$QUONC $D/borrow_broken.qn
```

```text
Error: type checking failed: borrowed ancilla `anc` escapes its borrow scope; it must be measured, `reset`, or `discard`ed inside the block, not returned
  ...
  return anc
         ─┬─
          ╰── type checking failed: borrowed ancilla `anc` escapes its borrow scope; ...
```

A `borrow` block is scoped scratch space (SPEC §3.4): the ancilla must be
cleaned up — measured, `reset`, or `discard`ed — before the block ends, so
callers never have to reason about whether a returned value secretly aliases
someone else's temporary. `borrow_fixed.qn` applies the same gate, then
`discard`s the ancilla instead of returning it:

```bash
$QUONC $D/borrow_fixed.qn
```

```text
Error: lowering is not implemented for `run-block expression`
```

**Learner note: `borrow_fixed.qn` still exits non-zero — that's expected.**
The fix corrected the *typecheck* bug; it does not (and can't yet) reach a
clean exit, because lowering `borrow` blocks to MLIR is a separate,
unimplemented stage — swapping the `run { ... }` wrapper for a
`circuit { ... }` one doesn't help either (verified: it just trades this
error for an earlier type mismatch, since a `circuit` block's type is
`Circuit`, not the `Q<Unit>` `main` needs here).

**This second error is not the bug the fix was for.** Compare the error
*kind*, not just its exit code: `borrow_broken.qn` fails type checking
(a `TypeError::BorrowEscape`, from `frontend/src/typecheck/`); `borrow_fixed.qn`
passes type checking cleanly and fails later, in an unrelated stage
(`LowerError::Unsupported`, from `frontend/src/lower.rs`) — standalone
`borrow` blocks don't lower to MLIR yet, regardless of whether the ancilla
inside them is used correctly. That's a real, pre-existing gap in the
compiler's lowering pass, not something #188 introduces or is scoped to fix
(see this pack's "no new compiler passes" boundary). `borrow_fixed.qn`'s
catalog row is `ci: none` for exactly this reason — it's not smoke-testable
until that lowering gap closes.

## See also

- [`frontend/tests/linearity.rs`](../../../frontend/tests/linearity.rs) —
  the linearity checker's own span-accuracy test suite (no-cloning,
  no-dropping, branch-residual agreement, closure capture).
- [`frontend/src/typecheck/tests.rs`](../../../frontend/src/typecheck/tests.rs) —
  the borrow-block acceptance/rejection tests this pair's diagnostics are
  drawn from verbatim.
- [`quonc` CLI reference — Debug options](../../../website/src/content/docs/reference/quonc.md#debug-options) —
  `--verify-linear` for a second linearity pass after lowering to dynamic IR.
