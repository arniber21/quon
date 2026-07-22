# Typecheck classical Γ judgment module

Status: Accepted · 2026-07-22 · Refs #324, #207, ADR-0028, ADR-0031, ADR-0032

## Context

The frontend type checker lived in a single monolith, `frontend/src/typecheck/mod.rs`,
mixing the bidirectional checking facade with every judgment fragment: the classical
unrestricted fragment (`Γ`), the linear context `Δ`, the quantum monad `Q<τ>`, the Z3
refinement bridge, and the **Circuit** judgment. Architecture review (epic #207 —
splitting the monolith into one deep module per judgment form) established the carve
strategy: move one judgment form at a time into its own module, keeping the methods on
`TypeChecker` and using `pub(super)` as the visibility seam.

The **Circuit carve** (#323, ADR-0028) was the first slice, proving the extraction
strategy. The **classical Γ carve** (#324, this ADR) is the next: it owns the
unrestricted, non-quantum typing that every Quon program starts from — arithmetic,
lists, lambdas, branches, `match` with exhaustiveness, pattern binding, and the
subsumption / function-shape / instantiation helpers that the bidirectional algorithm
bottoms out in.

A thin `unify.rs` (first-order unification via `Table`) and `exhaust.rs` (Maranget's
usefulness algorithm) already existed as standalone helper modules. The classical carve
does **not** absorb them — it *coordinates* them from the classical judgment. The
quantum monad (`Q<τ>`, `<-` binds, `run { }`), the borrow block, and the Z3 refinement
discharge (`verify_depth` / `verify_width`, the `expect_type` Circuit subtyping arm)
are *other slices* (#325, #326) and stay in the facade.

## Decision

Carve the **classical Γ** typing module out of the monolith into one deep module,
`frontend/src/typecheck/classical.rs`, owning the classical judgment form:

```
  Γ ; Δ ⊢ e ⇒ τ      synthesis: read the type off a classical term bottom-up
  Γ ; Δ ⊢ e ⇐ τ      checking:  push an expected type top-down
```

The module owns: arithmetic over `Int`/`Float` with deferred numeric metavariable
resolution (`synth_arith` / `synth_pow` / `numeric` / `finalize_numeric`); list
synthesis (`synth_list`); lambda synthesis and checking with a fresh linear context
per body and capture-error framing (`synth_lambda` / `check_lambda` /
`in_lambda_scope`); the subsumption rule (`subsume`); `if`/`then`/`else` in synthesis
and checking with residual `Δ` join (`branch_if` / `merge_branches`); `match` with
exhaustiveness + reachability via `exhaust::analyze` and dependent `Nat` refinement
assumption push (`check_match` / `push_arm_refinement`); pattern binding and checking,
routing linear resources into `Δ` and unrestricted names into `Γ` (`bind_pat` /
`bind_pat_with_rhs` / `check_pat` / `check_pat_into`); and the function-shape and
scheme-instantiation helpers (`as_function` / `instantiate`). `TypeChecker` stays the
**bidirectional facade** that dispatches into them.

### Module shape

The moved logic stays as **methods on `TypeChecker`**, gathered into a single `impl
TypeChecker` block in `classical.rs` rather than re-encapsulated behind a new
`ClassicalCtx` struct. Rationale (identical to ADR-0028): every classical method reads
and writes the checker's shared state — the metavariable `table`, the deferred
`numeric` obligations, the `lambda_linears` capture stack, and the refinement
`assumptions` — and calls back into the facade's generic `synth` / `check` /
`expect_type` / `ensure_consumed` / `depth_of` helpers. A `ClassicalCtx` borrowing that
state would either re-plumb every field through a new struct (churn for no behavior
gain) or hold `&mut TypeChecker` (an alias of the facade, not a deep module). Keeping
the methods on `TypeChecker` and moving only their definition site is the smallest
change that still places all classical logic behind one module interface.

Visibility is the carve's seam: the methods the facade (or a sibling slice like
`circuit.rs`) dispatches into are `pub(super)` (visible to the `typecheck` parent and
its descendants — not a new public API export); intra-classical helpers (`synth_pow`,
`merge_branches`, `push_arm_refinement`, `check_pat`) stay private to `classical.rs`.
Rust child-module privacy lets `classical.rs` reach the facade's private fields and
helpers; `pub(super)` lets the facade reach back in.

### Scope boundary

* **Moved:** only classical Γ logic. The **Circuit** judgment (#323/ADR-0028), the
  **Quantum Monad** (`Q<τ>`, `<-` binds, `run { }`), the **borrow** block, and the Z3
  **refinement** discharge (`verify_depth` / `verify_width`, the `expect_type` Circuit
  subtyping arm) stay in the facade — they are slices #325 and #326 (and the borrow
  judgment, #15). `synth_var` (which dispatches into Γ, `Δ`, prelude, gates, and
  quantum builtins), `synth_app` (which dispatches into circuit combinators, QEC
  constructors, kinded/dependent application, and ordinary application), and
  `synth_tuple` (which doubles as the quantum tensor-introduction form) are mixed
  dispatchers that stay in the facade; only their classical-only callees move.
  `check` (the checking-mode dispatcher) and `synth` (the synthesis-mode dispatcher)
  stay as the facade entry points.
* **Unification and exhaustiveness stay separate.** `Table::unify` lives in
  `unify.rs`; `exhaust::analyze` lives in `exhaust.rs`. The classical module
  *coordinates* them (every subsumption and branch join bottoms out in `Table::unify`;
  every `match` calls `exhaust::analyze`) but does not re-implement them. This keeps
  each algorithm testable in isolation and avoids duplicating the Maranget usefulness
  core or the occurs-check unifier.
* **`push_arm_refinement`** moves with `check_match` (its only caller) even though it
  pushes refinement `Assumption`s. It collects assumptions; the Z3 *discharge*
  (`expect_type` / `verify_depth` / `verify_width`) stays in the facade for the #326
  carve. This keeps the match arm's refinement context with the match judgment while
  leaving the solver bridge unmoved.
* **No behavior change.** This is pure code motion: method bodies are moved verbatim,
  no logic is rewritten, no new public exports are added beyond the `pub(super)` carve
  seam.

## Consequences

* Classical synth / check / unify-coordination / exhaustiveness / pattern-binding
  logic is now behind **one** module interface
  (`frontend/src/typecheck/classical.rs`), queryable as a unit.
* The facade (`mod.rs`) drops from ~3245 to ~2672 lines and reads as an orchestrator
  over judgment fragments; the next slices (#325 Q-monad, #326 refinement) follow the
  same carve pattern.
* A module-level test in `classical.rs` locks the extracted interface's pure seams —
  `as_function` (function-shape view + unification coordination), `numeric` /
  `finalize_numeric` (deferred obligation defaulting), `instantiate` (scheme
  instantiation), and `check_pat_into` (pattern binding / Γ-Δ routing) — so a future
  change to the carve's seam is caught directly.
* No `check_program` / `check_decls` / `synth_last_body` / `fn_type_of` signature
  change; all typecheck fixtures, the `reference_algorithms` corpus, and the fuzz /
  panic-freedom property targets stay green (they are the regression net for a
  no-behavior-change refactor).
