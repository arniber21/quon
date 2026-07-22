# Typecheck Quantum Monad / borrow judgment module

Status: Accepted · 2026-07-22 · Refs #325, #207, #180, ADR-0003, ADR-0006

## Context

The frontend type checker lived in a single monolith, `frontend/src/typecheck/mod.rs`,
mixing the bidirectional checking facade with every judgment fragment: the classical
unrestricted fragment, the linear context `Δ`, the Z3 refinement bridge, the **Circuit**
judgment (carved out in #323 / ADR-0028), and the **Quantum Monad / borrow** judgment —
Quon's `Q<τ>` monadic type, `<-` binds, `run { }` blocks, and `borrow` ancilla policy.

Architecture review (epic #207 — one deep module per judgment form) scheduled the
**Quantum Monad carve** (#325) as the second slice, directly after the Circuit carve. The
Q-monad logic was the most entangled fragment still in the facade: the monadic bind threads
the linear `Δ`, auto-lifts pure quantum resources (ADR-0006), and re-enters the facade's
generic `synth`; the borrow block coordinates a no-escape scan against `Δ` with a
consume-based cleanup check. A successful carve here proves the extraction strategy carries
to a judgment that is *not* a clean self-contained algebra (unlike Circuit composition).

A thin `frontend/src/typecheck/monad.rs` did not previously exist; all of the following
lived as private methods and free functions on/inside the monolith:

* `synth_return` — `return v ⇒ Q<τ>` (extracted from the inline `Return` arm).
* `synth_bind` — `x <- e₁; e₂`, with the ADR-0006 pure-resource auto-lift.
* `synth_borrow` — `borrow bᵢ: Tᵢ in { body } ⇒ Q<τ>`, the escape + cleanup policy.
* `synth_measure_all` / `synth_map_q` / `synth_sequence_q` — the SPEC §5 monadic combinators.
* `find_borrow_escape` / `first_borrowed_var` / `collect_var_hit` — the borrow escape scan.

## Decision

Carve the **Quantum Monad / borrow** module out of the monolith into one deep module,
`frontend/src/typecheck/monad.rs`, owning the judgment form:

```
  Γ ; Δ ⊢ return v ⇒ Q<τ>                    lift a value into the monad
  Γ ; Δ ⊢ x <- e₁; e₂ ⇒ Q<β>                 monadic bind (e₁ : Q<α>, auto-lifts a pure resource)
  Γ ; Δ ⊢ borrow bᵢ: Tᵢ in { body } ⇒ Q<τ>   scoped ancilla allocation
```

The module owns: `Q<τ>` synthesis from `return`; the monadic bind and its pure-resource
auto-lift (ADR-0006 — `e₁ : Q<α>` *or* a pure `Qubit`/`QReg`/`Circuit`, with a pure
classical value rejected as `ExpectedMonad`); the SPEC §5 monadic combinators
(`measure_all`, `map_q`, `sequence_q`); the borrow block's consume + no-escape policy; and
the escape-detection helpers. `TypeChecker` stays the **bidirectional facade** that
dispatches into it; `Δ` stays the bookkeeping module (`linear.rs`).

### Module shape

The moved logic stays as **methods on `TypeChecker`**, gathered into a single
`impl TypeChecker` block in `monad.rs` rather than re-encapsulated behind a new struct —
exactly the pattern ADR-0028 established for the Circuit carve. Rationale: every Q-monad
method reads and writes the checker's shared `table` and calls back into the facade's
generic `synth` / `resolve_type` / `bind_pat` / `ensure_consumed` helpers. Re-encapsulating
would re-plumb that state for no behavior gain. Keeping the methods on `TypeChecker` and
moving only their definition site is the smallest change that still places all Q-monad /
borrow logic behind one module interface.

Visibility is the carve's seam: the methods the facade dispatches into are `pub(super)`
(visible to the `typecheck` parent only — not a new public API export); the
escape-detection helpers (`find_borrow_escape` / `first_borrowed_var` / `collect_var_hit`)
stay private to `monad.rs`. The module itself is declared `mod monad;` (private). Rust
child-module privacy lets `monad.rs` reach the facade's private fields and helpers;
`pub(super)` lets the facade reach back in.

### Scope boundary

* **Moved:** only Quantum Monad / borrow logic — `synth_return`, `synth_bind`,
  `synth_borrow`, `synth_measure_all`, `synth_map_q`, `synth_sequence_q`, and the three
  escape-detection free functions. The `Return` / `Bind` / `Borrow` dispatch arms stay in the
  facade's `synth` match (they now call the `pub(super)` methods); the `measure_all` /
  `map_q` / `sequence_q` arms stay in `synth_app`.
* **Not moved:** the bare `measure` / `reset` / `discard` / `qubit` primitives are prelude
  builtins (`builtins.rs`); the `qreg(n)` allocation and `apply_circuit`'s monadic-source
  threading (ADR-0006) stay with the facade / Circuit module (`apply_circuit` is already
  Circuit-owned per ADR-0028); `ensure_consumed` is shared linear bookkeeping used by the
  classical `Let`, lambdas, and the monad, so it stays in the facade (the monad *coordinates*
  with it); the Z3 **refinement** discharge is slice #326.
* **No behavior change.** Pure code motion: method bodies are moved verbatim, `synth_return`
  is a behavior-preserving extraction of the inline `Return` arm, no new public exports are
  added beyond the `pub(super)` carve seam.

## Borrow policy: ADR-0003 superseded by #180

ADR-0003 specified that a `borrow` block's final use of an ancilla must *structurally* be
`reset(q)` or `discard(q)`. Issue #180 resolved that the shipped, intended rule is weaker
and consume-based: a borrow block is well-typed iff each ancilla is **consumed exactly once**
inside the block and **does not escape** the result. Valid cleanup includes `measure`,
`reset`, *and* `discard` (and any consuming op) — not only a structural `reset`/`discard`
terminal. This lets the 3-qubit bit-flip `syndrome_measure` reference algorithm type-check
with mid-circuit `measure` cleanup. **ADR-0003's structural-only wording is superseded by
#180 for borrow-block cleanup.** The `synth_borrow` implementation already encodes the #180
rule (consume via `ensure_consumed` + no-escape via `find_borrow_escape`); this carve moves
it verbatim and the module docs state the #180 resolution explicitly so ADR-0003 is not
reasserted without it. CONTEXT.md's "Borrow block" glossary already reflects #180.

## Consequences

* `Q<τ>` synthesis, monadic bind / autolift, the monadic combinators, and the borrow
  escape/cleanup policy are now behind **one** module interface
  (`frontend/src/typecheck/monad.rs`), queryable as a unit.
* The facade (`mod.rs`) drops from ~3245 to ~2973 lines and reads as an orchestrator over
  judgment fragments; the refinement slice (#326) is the remaining large fragment.
* A module-level test in `monad.rs` locks the extracted escape-detection interface directly
  as pure functions — escape detection (a borrowed name in a `return`, bare or buried in a
  returned register), the no-escape invariant (only *borrowed* names are flagged), and the
  cleanup-classification policy (`measure` / `reset` / `discard` in a consuming position do
  *not* escape) — plus an end-to-end check that `measure`-only borrow cleanup is accepted
  (the #180 resolution).
* No `check_program` / `check_decls` / `synth` signature change; all typecheck fixtures, the
  `reference_algorithms` corpus, and the fuzz/panic-freedom property targets stay green (they
  are the regression net for a no-behavior-change refactor).
