# Issue #44 plan — adversarial review

**Document reviewed:** `docs/plans/issue-44-plan.md` (initial draft)  
**Reviewer stance:** adversarial (spec fidelity, implementability, dependency boundaries)  
**Grade:** **C+ — FAIL** (do not implement from the draft as written)

---

## Executive summary

The plan correctly identifies the core problem — structured diagnostic codes and quick fixes are lost when `TypeError` is flattened to strings — and proposes a sensible `RichDiagnostic` → LSP pipeline. However, several sections conflate distinct error semantics, misread existing `TypeError` payloads, double-call `analyze()`, assign `#43` work that belongs in `#44`, and set v1 acceptance criteria for parser codes the current grammar cannot emit. These are blocking defects, not polish items.

**Verdict:** amend the plan (see `issue-44-plan.md` post-review) before opening implementation PRs.

---

## Blocking findings

### F1 — Fix A conflates `BorrowEscape` and `LinearUnconsumed` (CRITICAL)

The draft treats both variants as a single auto-fixable family ("Fix A — `quon.borrow.escape` / borrow-scoped `quon.linearity.unconsumed`"). That is incorrect:

| Variant | Situation | Safe auto-fix? |
|---------|-----------|----------------|
| `LinearUnconsumed` | Ancilla never consumed; block ends with `return <expr>` not mentioning the name | **Yes** — prepend `discard(name)` / `reset(name)` before `return` |
| `BorrowEscape` | Ancilla appears **inside** the returned expression (e.g. `return (q, a)`) | **No** — no single-line prepend resolves the escape; user must restructure the return |

Offering an auto-fix on `BorrowEscape` that inserts `discard(a)` before `return (q, a)` is misleading: the ancilla still escapes in the tuple. The draft's own safety guard ("still offer discard before return as *assist*") contradicts the acceptance criterion "≥3 **safe** quick fixes."

**Required fix:** split into Fix A (`LinearUnconsumed` only, auto-fixable) and document `BorrowEscape` as related-information-only in v1 (no `QuickFix` attachment).

---

### F2 — `LinearDiscard.name` is the type string, not the binding name (CRITICAL)

The draft assumes `LinearDiscard { name }` carries the discarded variable (e.g. `q` in `let _ = q`). The checker today sets:

```rust
TypeError::LinearDiscard {
    name: resolved.to_string(),  // e.g. "Qubit", not "q"
    span,                        // wildcard `_` site
}
```

Related information ("`q` bound here") and Fix D ("`discard(q)`") both require the **RHS binding name**, not the resolved type. Without a schema extension the plan's quick-fix and related-info specs are unimplementable as written.

**Required fix:** extend `LinearDiscard` with `bound_name: String` (the identifier on the RHS, e.g. `q`); keep `name`/type rendering for the message if needed, or derive type from context. Thread `binding_span: SimpleSpan` for related info.

---

### F3 — No `#43` dependency gate (CRITICAL)

The plan lists `#43` deliverables inline but never defines a **hard gate**: what must be merged and green before `#44` work starts. Without this, frontend quick-fix work can land before span→Range helpers or the analysis cache exist, producing untestable LSP integration.

**Required fix:** add an explicit dependency gate checklist (§0) with merge + test preconditions from `#43`.

---

### F4 — `check_program` / `analyze()` double-call and unfixed quick-fix population (CRITICAL)

The draft's back-compat wrapper:

```rust
pub fn check_program(source: &str) -> Result<(), Vec<Diagnostic>> {
    match analyze(source).diagnostics.is_empty() {
        true => Ok(()),
        false => Err(analyze(source).diagnostics.iter().map(Diagnostic::from).collect()),
    }
}
```

calls `analyze()` twice on every failure — wasteful (re-runs lexer/parser/typecheck, potentially Z3) and risks inconsistent `quick_fixes` if source-dependent fix generation is added later.

Additionally, placing `quick_fixes()` on `TypeError` methods without specifying **when** the pipeline populates `RichDiagnostic.fixes` leaves ambiguity: fixes must be computed **once** inside `analyze()` (or `to_rich_diagnostic(source)`) at diagnostic creation time, not lazily on every LSP `codeAction` re-query with divergent logic.

**Required fix:** single `analyze()` call in `check_program`; document that `analyze()` populates `fixes` exactly once per diagnostic during pipeline lowering.

---

### F5 — Parser code acceptance includes unreachable codes (MAJOR)

§3.2 lists five parser codes; §3 acceptance says "every row above must have a unit test." Codes `quon.parse.expected-expression` (E102), `expected-type` (E103), `invalid-nat-expr` (E104), and `unclosed-delimiter` (E105) are **not reliably classifiable** from current chumsky `Rich` output — the grammar emits generic "unexpected token" labels. Mandating tests for unreachable codes blocks the PR on speculative classifier work unrelated to LSP value.

**Required fix:** v1 acceptance requires `quon.parse.unexpected-token` (E101) plus desugar/lexer/type codes; defer E102–E105 to a labeled-parser follow-up (track in issue or `#44` stretch goal, not DoD).

---

### F6 — `codeActionProvider` capability assigned to `#43` (MAJOR)

§5.3 says "Register server capabilities in `#43`'s `initialize` response." Code actions are entirely `#44` scope; `#43` should publish diagnostics only. Mixing capability registration creates cross-PR coupling and makes `#43`'s DoD depend on unimplemented handlers.

**Required fix:** `#43` registers `textDocument/publishDiagnostics` support; `#44` adds `codeActionProvider` to `initialize` and implements `textDocument/codeAction`.

---

## Non-blocking observations

| ID | Severity | Note |
|----|----------|------|
| N1 | Minor | `TypeError::quick_fixes(&self, source: &str)` on the enum is fine, but call site must pass source once in `analyze()` — consider `to_rich_diagnostic(&self, source: &str)` instead of a separate lazy method. |
| N2 | Minor | Fix D (`LinearDiscard`) depends on F2; defer to PR 2 if binding-name threading slips schedule. |
| N3 | Info | 28 variant → 28 code mapping is sound; exhaustive const-array test is the right enforcement mechanism. |
| N4 | Info | Partial-source no-panic corpus is well specified; keep it in PR 1 (frontend) not gated on LSP harness. |

---

## Grade breakdown

| Criterion | Score | Notes |
|-----------|-------|-------|
| Problem diagnosis | A | Correctly identifies structure loss at `to_diagnostic()`. |
| Architecture | B+ | `RichDiagnostic` + stage adapters is the right shape. |
| Spec fidelity | D | Fix A semantics wrong; `LinearDiscard` field misread. |
| Dependency boundaries | C | Missing `#43` gate; capability ownership wrong. |
| Implementability / DoD | C− | Unreachable parser codes; double `analyze()` call. |
| Test plan | B | Good fixture strategy; undermined by wrong fix assumptions. |

**Overall: C+ — FAIL.** Core direction is salvageable; blocking findings F1–F6 must be resolved in the plan before implementation.

---

## Required amendments (applied in `issue-44-plan.md`)

1. Split Fix A: `LinearUnconsumed` auto-fixable; `BorrowEscape` related-info only.
2. Extend `LinearDiscard` with `bound_name` (+ `binding_span` for related info).
3. Add §0 `#43` dependency gate.
4. Fix `check_program` to call `analyze()` once; document single-pass `fixes` population.
5. Defer parser codes E102–E105 from v1 acceptance.
6. Own `codeActionProvider` registration and handler in `#44`.
