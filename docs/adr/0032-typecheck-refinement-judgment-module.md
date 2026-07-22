# ADR-0032: Typecheck refinement / Z3 obligation judgment module

- **Status:** Accepted
- **Date:** 2026-07-22
- **Issues:** #326, #207
- **Related ADRs:** [ADR-0002](0002-circuit-type-as-op-attributes.md) (DepthExpr), [ADR-0028](0028-typecheck-circuit-judgment-module.md) (Circuit carve), [ADR-0031](0031-typecheck-quantum-monad-judgment-module.md) (Monad carve)

## Context

`frontend/src/typecheck/mod.rs` was a ~3245-line monolith after the Circuit carve (#323, ADR-0028) and the Quantum Monad carve (#325, ADR-0031). Nat refinement / Z3 obligation generation — depth upper-bound discharge, width equality, well-founded termination checks, assumption recording for `match`-arm refinements, and deferred numeric obligations — was interspersed with Circuit typing and classical Γ rules.

The deletion test from issue #326: *removing Z3 should not require edits beside Circuit code.* Before this change, removing Z3 meant touching the Circuit judgment, the classical judgment, and the dispatch arms — complexity forked rather than concentrated.

## Decision

Carve all refinement / Z3 obligation logic into one judgment module: `frontend/src/typecheck/obligation.rs`.

### What moved

| Symbol | Role |
|---|---|
| `TypeChecker::verify_depth` | Depth upper-bound obligation (`inferred ≤ annotated`) discharged via Z3 under active assumptions |
| `TypeChecker::verify_width` | Width (qubit-count) equality obligation discharged under active assumptions |
| `TypeChecker::check_termination` / `RecCall` | Well-founded recursion measure (`arg + 1 ≤ p`, `0 ≤ arg`) |
| `TypeChecker::push_arm_refinement` | `match`-arm pattern equalities/disequalities pushed/popped as Z3 assumptions |
| `TypeChecker::numeric` | Deferred `Int`/`Float` numeric-type obligations |
| `RefinementState` | Per-scope assumption stack + deferred-obligation buffer |

### What stays

- **`frontend/src/refinement.rs`** (crate root) — the Z3 bridge (`RefinementCtx`, `Assumption`, `DepthError`). This module is the *typechecker's obligation-generation / discharge layer* that calls into it. The bridge itself is untouched.
- **`quon_core::DepthExpr`** — the symbolic depth algebra (ADR-0002). Stays in `quon_core`; this module consumes `DepthExpr` results from the Circuit judgment and discharges them.
- **Circuit judgment** (`circuit.rs`, ADR-0028) — *assembles* `DepthExpr`; this module *discharges* it. The seam is a `DepthExpr` value passed from Circuit to obligation.

### Pattern

Pure code-motion refactor, following ADR-0028 (Circuit carve) and ADR-0031 (Monad carve):
- `pub(super)` methods on `TypeChecker` — no new public exports.
- Private free functions for helpers.
- `mod obligation;` is a private module.
- Method bodies moved verbatim — no intentional behavior change.

## Consequences

- **Deletion test passes:** removing Z3 now means deleting `obligation.rs` and its call sites — Circuit and classical code are untouched.
- **Failure modes localized:** unsat / Z3 errors surface from one module, not scattered across the typechecker.
- **Future work:** issue #207 (typechecker decomposition) continues; remaining monolith chunks are the classical Γ judgment and the top-level dispatch/`check_decls` entry point.
- **No behavior change:** all existing frontend tests (253 lib + integration + fuzz) pass; refinement/Z3 fixtures unchanged.
