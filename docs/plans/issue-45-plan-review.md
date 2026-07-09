# Issue #45 plan review — adversarial review

**Plan reviewed**: `docs/plans/issue-45-plan.md` (2026-07-08)  
**Reviewer stance**: implementability, correctness, and drift risk against `frontend` as inspected on branch `issue-45-lsp-intelligence`.  
**Verdict**: **Grade C- — FAIL** (do not implement as written; amend plan first)

---

## Executive summary

The plan correctly identifies the four LSP surfaces, the one-pass `DocumentAnalysis` cache, and the existing `frontend` hooks (spanned AST, typechecker, linear `intro` spans). It fails on **span fidelity for definition sites**, **name-resolution duplication**, and **LSP failure semantics**. Those three gaps would produce wrong go-to-definition ranges, shadowing/linear-resolution bugs that diverge from the checker, and brittle handler code around `Result` propagation.

The milestone stack also starts too late: symbol indexing on bare `Name` strings cannot land before AST span fixes.

---

## Grading rubric

| Axis | Grade | Notes |
|------|-------|-------|
| Problem framing | B+ | Clear scope, good dependency on #43/#44, sensible handler split |
| Ground truth vs codebase | C | Misses bare `Name` on defs; overstates bind-target span availability |
| Architecture soundness | D+ | Parallel resolver re-implements `synth_var` lookup order |
| LSP operability | C- | `analyze_program` → `Result` breaks partial-program contract |
| Test plan | C | Missing explicit resolution-parity gate; AfterDot underspecified |
| Milestones / sequencing | D | M1 symbol index before span prerequisites is blocked |

**Overall: C- FAIL**

---

## Critical failures (must fix before implementation)

### F1 — Definition spans are missing on core AST nodes

**Evidence** (`frontend/src/ast.rs`):

- `Decl::Fn { name: Name, params: Vec<(Name, Sp<Type>)>, … }` — fn name and param names are bare `String`.
- `Decl::TypeAlias { name: Name, params: Vec<Name>, … }` — alias name and type params unspanned.
- `Expr::Borrow { bindings: Vec<(Name, Sp<Type>)>, … }` — borrow binder names unspanned.
- `Expr::Bind { param: Name, … }` — monadic bind target unspanned.

The plan's symbol table assigns `name_span` from "fn name ident" and "param name from tuple", but those sites have **no span today**. The risk note (§14) treats this as a follow-up lexer hack; for go-to-definition and semantic-token `definition` modifiers it is a **hard prerequisite**.

**Required fix**: Milestone **M0** — extend parser/AST to `Sp<Name>` (or equivalent) on fn/alias/param/borrow/bind definition identifiers; update typechecker/tests. No symbol-index PR before M0 merges.

---

### F2 — Parallel name resolver will drift from the typechecker

**Evidence** (`frontend/src/typecheck/mod.rs`, `synth_var`): resolution order is `env.get` → `delta.try_consume` → `builtins::lookup` → `circuit::gate_type` → lambda capture check → `is_quantum_builtin` → unbound.

Plan §3.4 proposes a **separate** scope-walk resolver with prelude fallback. That duplicates logic already subtle (linear consume vs classical shadowing, capture errors, quantum prelude edge cases). The plan mentions a property test in §14 but does not require it in the test strategy or wire resolution into the checker.

**Required fix**: Record resolutions inside `synth_var` (and equivalent check paths for type-position `Type::Named`) into a **`ResolutionMap`** keyed by use-site span. LSP `resolve_at` reads the map; no second algorithm. Add **resolution parity tests** (see F5).

---

### F3 — `analyze_program` must not return `Err`

Plan §3.3:

```rust
pub fn analyze_program(src: &str) -> Result<DocumentAnalysis, Vec<Diagnostic>>;
```

LSP handlers need a snapshot on every keystroke — parse failures, type errors, and partial trees included. Returning `Err` forces every caller to branch and invites empty-cache panics or skipped intelligence. §2 already states "failure tolerance"; the signature contradicts it.

**Required fix**: `pub fn analyze_program(src: &str) -> DocumentAnalysis` always. Diagnostics live **inside** the struct (`diagnostics: Vec<Diagnostic>`). Parse failure → `{ decls: empty, symbols: empty, resolutions: empty, diagnostics: parse_diags, src }`.

---

### F4 — `AfterDot` completion is in scope but unimplementable

Plan §6.1 lists `AfterDot` (UFCS `x.|`) and §6.2 implies method suggestions "defer method set to builtins for now". Quon has no UFCS/method surface in the AST or typechecker today. Shipping a context enum entry without behavior still advertises `.` as a trigger character (§12), producing empty or misleading completions.

**Required fix**: Remove `AfterDot` from v1 completion contexts; drop `.` from `triggerCharacters`; list UFCS/method completion in §13 out of scope.

---

### F5 — No acceptance test for resolution parity

§14 lists "Property test: resolve matches TypeChecker" as a risk mitigation only. Without a concrete test module and fixtures, shadowing/linear bugs will ship silently.

**Required fix**: Add `frontend/tests/analysis_resolution.rs` (or `analysis/resolution.rs` unit tests) with cases:

| Fixture theme | Assert |
|---------------|--------|
| Inner `let` shadows outer | use resolves to inner def |
| Linear qubit reuse / double consume | resolution still points at binding; checker error does not erase map entry |
| Gate vs builtin vs quantum prelude | synthetic symbol kind matches checker branch |
| Type alias `Type::Named` in annotation | resolves to alias def span (post-M0) |
| Unbound name | no map entry; hover shows unknown |

Run on every CI alongside existing typecheck fixtures.

---

## Major issues (fix in plan; not blocking if criticals addressed)

### M1 — Type annotation keying by `(start, end)` is fragile

Two adjacent idents can share boundaries in edge cases; prefer keyed by `SimpleSpan` or a stable node id. Low priority if resolution map carries types for idents.

### M2 — `Bind.param` lexer fallback is a dead end

Re-lexing inside bind stmt span is brittle after desugar. M0 `Sp<Name>` on bind target is the correct fix; remove lexer fallback from primary path.

### M3 — Milestone dependency graph

M1 (symbol index) before M0 is blocked. M2 (`analyze_program`) should depend on M0 + resolution map hook, not a freestanding parallel resolver in M3.

---

## Minor / documentation nits

- §3.2 table row for `Expr::Bind` should not claim "parser stores param: Name; locate via parent span walk" as acceptable v1 — mark blocked on M0.
- §8.3 `reanalyze` assigns `analyze_program` directly; with infallible return, drop `?` / error arm.
- Export `LinEntry.intro` visibility: plan assumes go-to-def for linear bindings; confirm `Delta` exposes intro spans or resolution map records them at consume sites.

---

## Required plan amendments (checklist)

- [ ] Add **M0**: `Sp<Name>` on fn, type alias, fn/param/borrow/bind definition names + parser/tests
- [ ] Replace §3.4 parallel resolver with **checker-backed `ResolutionMap`** populated in `synth_var` (and type-name resolution)
- [ ] Change `analyze_program` signature to **`-> DocumentAnalysis`** (diagnostics internal)
- [ ] **Defer `AfterDot`** completion; remove `.` trigger until UFCS exists
- [ ] Add **resolution parity tests** to §10 with named fixtures

---

## Re-review gate

Re-run this review after amending `issue-45-plan.md`. Target **B or higher** before an AFK agent starts M0/M1 implementation.
