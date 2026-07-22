# Typecheck Circuit judgment module

Status: Accepted · 2026-07-21 · Refs #323, #207, ADR-0005, ADR-0006

## Context

The frontend type checker lived in a single 3906-line monolith,
`frontend/src/typecheck/mod.rs`, mixing the bidirectional checking facade with every
judgment fragment: the classical unrestricted fragment, the linear context `Δ`, the
quantum monad `Q<τ>`, the Z3 refinement bridge, and the **Circuit** judgment (Quon's
central type). Architecture review (epic #207 — splitting the monolith into one deep
module per judgment form) ranked the **Circuit carve** as the highest-leverage first slice:
Circuit composition has a clean, self-contained algebra, and a successful carve proves the
extraction strategy before the more entangled Q-monad (#325) and refinement (#326) slices.

A thin `frontend/src/typecheck/circuit.rs` helper already existed (~67 lines:
`gate_type` / `rotation_arity` / `is_specialisable_rotation`) holding only the static gate
signatures. The composition *rules* — `|>`, `par`, `adjoint`, `controlled`, gate
placement, the `identity`/`repeat`/`on_high`/`on_low`/`swap_reverse`/`for`/`fold`
depth-building families, the branch depth join, and `circuit { }` block checking — all
lived as private methods on `TypeChecker` inside the monolith.

## Decision

Carve the **Circuit typing** module out of the monolith into one deep module,
`frontend/src/typecheck/circuit.rs`, owning the Circuit judgment form:

```
  Γ ; Δ ⊢ e ⇒ Circuit<n, m, d, C>     synthesis of a circuit morphism
  Γ ; Δ ⊢ e ⇐ Circuit<n, m, d, C>     checking a circuit block against an annotation
```

The module owns: gate placement into the innermost circuit register; the composition
algebra (`|>` sequential — depths add, `par` parallel — depth = max, `adjoint` — width
swap, `controlled` — `+1` widths and `+1` depth, `repeat` — `k*d`); the Clifford
classification join (`Clifford ∘ Clifford = Clifford`, any `Universal` absorbs) and the
synthesis-mode branch depth join by `max` (ADR-0005); DepthExpr assembly over the circuit
indices; and the gate-registry static signatures. `TypeChecker` stays the **bidirectional
facade** that dispatches into it.

### Module shape

The moved logic stays as **methods on `TypeChecker`**, gathered into a single `impl
TypeChecker` block in `circuit.rs` rather than re-encapsulated behind a new `CircuitCtx`
struct. Rationale: every Circuit method reads and writes the checker's shared state — the
metavariable `table`, the `refine`ment bridge, the active `assumptions`, and the
`circuit_width` / `circuit_width_cap` register stacks — and calls back into the facade's
generic `synth` / `check` / `expr_to_depth` / `depth_of` helpers. A `CircuitCtx` borrowing
that state would either re-plumb every field through a new struct (churn for no behavior
gain) or hold `&mut TypeChecker` (an alias of the facade, not a deep module). Keeping the
methods on `TypeChecker` and moving only their definition site is the smallest change that
still places all Circuit logic behind one module interface — the carve the issue asks for.

Visibility is the carve's seam: the methods the facade dispatches into are `pub(super)`
(visible to the `typecheck` parent only — not a new public API export); intra-circuit
helpers (`as_circuit`, `ambient_width`, `placement_width`, `expect_width`, `iter_info`,
`synth_fold_step`, `list_length`) stay private to `circuit.rs`. Rust child-module privacy
lets `circuit.rs` reach the facade's private fields and helpers; `pub(super)` lets the
facade reach back in.

### Scope boundary

* **Moved:** only Circuit logic. The classical fragment, the **Quantum Monad** (`Q<τ>`,
  `<-` binds, `run { }`), the **borrow** block, and the Z3 **refinement** discharge
  (`verify_depth` / `verify_width`, the `expect_type` Circuit subtyping arm) stay in the
  facade — they are slices #325 and #326 (and the borrow judgment, #15). Pure circuit
  application (`@` outside a block) and the bind auto-lift (ADR-0006) live with the Q-monad
  slice and are *not* moved, even though `apply_circuit` reads Circuit indices; its
  monadic-source handling is Q-monad logic, so it stays callable from the facade but the
  method itself is Circuit-owned here.
* **No behavior change.** This is pure code motion: method bodies are moved verbatim, no
  logic is rewritten, no new public exports are added beyond the `pub(super)` carve seam.
* **Gate registry unchanged.** `circuit.rs` continues to consume `quon_core::gates`
  (`surface_gate`) — no new string tables.

## Consequences

* Circuit composition / placement / Clifford join / DepthExpr-on-Circuit logic is now behind
  **one** module interface (`frontend/src/typecheck/circuit.rs`), queryable as a unit.
* The facade (`mod.rs`) drops from ~3906 to ~3245 lines and reads as an orchestrator over
  judgment fragments; the next slices (#325, #326) follow the same carve pattern.
* A module-level test in `circuit.rs` locks the extracted interface's pure algebra (seq add,
  par max, controlled `+1`, repeat `k*d`, Clifford join, gate-registry signatures) with
  proptest coverage over the `DepthExpr` `normalize`/`equiv` oracle, so a future change to
  the carve's seam is caught directly.
* No `check_program` / `check_decls` / `synth_last_body` signature change; all typecheck
  fixtures, the `reference_algorithms` corpus, and the fuzz/panic-freedom property targets
  stay green (they are the regression net for a no-behavior-change refactor).
