# Issue #44 — LSP diagnostics + quick fixes

**Branch:** `issue-44-lsp-diagnostics` (stacked on `issue-43-lsp-foundation`)  
**Goal:** Expose structured, stable-coded diagnostics from the frontend pipeline to the LSP, with related locations and safe quick fixes for the highest-value quantum/linearity errors.

**Depends on (#43):** `quon_lsp` crate, stdio JSON-RPC lifecycle, document cache, debounced analysis scheduler, `span → LSP Range` helpers, and `textDocument/publishDiagnostics` wiring. This issue enriches the *payload* the scheduler already publishes; it does not reimplement the server shell.

### #43 dependency gate (must be green before #44 work starts)

Do **not** open `#44` implementation PRs until all of the following are merged on `issue-43-lsp-foundation` and passing CI:

- [ ] `quon_lsp` crate exists with stdio JSON-RPC lifecycle and `initialize` / shutdown.
- [ ] Document cache + debounced analysis scheduler calling `frontend::check_program` (or stub `analyze`).
- [ ] `span → LSP Range` conversion helper (UTF-16 safe; covered by at least one unit test).
- [ ] `textDocument/publishDiagnostics` wired from scheduler output.
- [ ] LSP test harness (stdio JSON-RPC replay) usable by downstream integration tests.

**Explicitly out of `#43` scope (owned by `#44`):** `codeActionProvider` capability registration, `textDocument/codeAction` handler, and `QuickFix` → `CodeAction` mapping (see §5.3).

---

## 1. Current state

### Frontend diagnostic currency

Today `frontend/src/diagnostics.rs` defines a minimal struct:

```rust
pub struct Diagnostic {
    pub message: String,
    pub span: SimpleSpan,
}
```

Pipeline stages lower into this shape differently:

| Stage | Source | Structured? | Span? |
|-------|--------|-------------|-------|
| Lexer | `Vec<Sp<String>>` via `from_stage` | No (free-text message) | Yes |
| Parser | chumsky `Rich` → `(String, span)` | No | Yes |
| Desugar | 3 hard-coded `Diagnostic::new(...)` sites | No | Yes |
| Typecheck | `TypeError` enum (28 variants) → `to_diagnostic()` discards variant | **Lost at lowering** | Yes (+ `first` span on `LinearUsedTwice`, unused today) |
| Lower | `Diagnostic::new(err.to_string(), span)` | No | Yes |

`frontend::check_program` (the API `#43`'s scheduler calls) returns `Result<(), Vec<Diagnostic>>`, so type errors are already flattened to strings before the LSP sees them.

### Existing test coverage (baseline)

Integration tests already assert span accuracy and error *messages* for many paths:

- `frontend/tests/typecheck.rs` — classical type errors
- `frontend/tests/linearity.rs` — clone/drop/branch mismatch
- `frontend/tests/circuits.rs` — Clifford mismatch, qubit-count mismatch
- `frontend/tests/parser.rs` — parse error spans
- `frontend/tests/lexer.rs` — lex error spans (no panic on `#`)
- `frontend/src/typecheck/tests.rs` — exhaustive `TypeError` kind checks including depth, Clifford, borrow

**Gap:** no tests assert stable `code`, `severity`, `related_information`, or quick-fix edits. No LSP-layer tests yet.

---

## 2. Target architecture

```
┌─────────────────────────────────────────────────────────────┐
│ quon_lsp (issue #43 + #44)                                  │
│  DocumentCache → analyze(source) → publishDiagnostics       │
│                                  → codeAction (on demand)   │
└───────────────────────────┬─────────────────────────────────┘
                            │ calls
┌───────────────────────────▼─────────────────────────────────┐
│ frontend::analyze(source)  [NEW public API]                 │
│  lex → parse → desugar → typecheck                          │
│  returns AnalysisResult { diagnostics: Vec<RichDiagnostic> }  │
└───────────────────────────┬─────────────────────────────────┘
                            │
┌───────────────────────────▼─────────────────────────────────┐
│ RichDiagnostic                                              │
│  code, severity, message, span, related[], fixes[]          │
└─────────────────────────────────────────────────────────────┘
```

### Design principles

1. **Preserve structure at the source.** `TypeError` remains the typed error for the checker; add methods rather than scattering string matching in `quon_lsp`.
2. **Stable string codes.** LSP `Diagnostic.code` is a string; use dotted slugs (`quon.linearity.used-twice`) that never change meaning once shipped.
3. **Severity is deterministic.** All current frontend failures are hard errors (`DiagnosticSeverity::Error`). Warnings deferred to #47 (`quonlint`).
4. **Quick fixes are optional attachments.** Same `RichDiagnostic` carries zero or more `QuickFix` suggestions; `quon_lsp` maps them to `CodeAction` only when unambiguous.
5. **Fail soft on partial input.** Each stage returns `Vec<RichDiagnostic>` and stops the pipeline; no `unwrap`/`expect` on user source in library paths (`frontend/src/`).

---

## 3. Error code taxonomy

Codes use the prefix `quon.` and are grouped by pipeline stage / domain. Numeric aliases (for docs/telemetry) are stable IDs; the **string slug is canonical** for LSP.

### 3.1 Lexer (`quon.lex.*`)

| Code | Numeric | Severity | Trigger |
|------|---------|----------|---------|
| `quon.lex.invalid-char` | E001 | Error | Character not in Quon alphabet (e.g. `#`) |
| `quon.lex.unterminated-comment` | E002 | Error | Unclosed `{-` block comment |
| `quon.lex.unterminated-string` | E003 | Error | Reserved for future string literals |

*Implementation note:* chumsky lexer currently emits generic labels; map by inspecting error label / context at the `lex` boundary (see §6.2).

### 3.2 Parser (`quon.parse.*`)

| Code | Numeric | Severity | Trigger |
|------|---------|----------|---------|
| `quon.parse.unexpected-token` | E101 | Error | Token stream mismatch (default chumsky label) |
| `quon.parse.expected-expression` | E102 | Error | Missing RHS / empty application site |
| `quon.parse.expected-type` | E103 | Error | Type position parse failure |
| `quon.parse.invalid-nat-expr` | E104 | Error | Malformed `Nat` arithmetic in type position |
| `quon.parse.unclosed-delimiter` | E105 | Error | Unmatched `(`, `{`, `[`, `<` (classify from chumsky "expected X" messages where feasible) |

Parser codes start as **best-effort classification** of chumsky `Rich` messages; tighten with labeled parsers (`.label("expression")`) in a follow-up if needed.

**v1 acceptance:** only `quon.parse.unexpected-token` (E101) is required. Codes E102–E105 are **deferred** — the current grammar does not emit reliably classifiable chumsky labels for them; do not block `#44` on speculative message parsing. Track labeled-parser work as a follow-up.

### 3.3 Desugar (`quon.desugar.*`)

| Code | Numeric | Severity | Trigger | Current message substring |
|------|---------|----------|---------|---------------------------|
| `quon.desugar.run-trailing-bind` | E201 | Error | `run { … <- … }` last stmt is bind | `must end in an expression, not a \`<-\` bind` |
| `quon.desugar.run-trailing-let` | E202 | Error | `run { … let … }` last stmt is let | `must end in an expression, not a \`let\` binding` |
| `quon.desugar.refutable-bind-pattern` | E203 | Error | Literal pattern in `<-` bind | `cannot bind a literal pattern` |

### 3.4 Classical type (`quon.type.*`)

| Code | Numeric | `TypeError` variant |
|------|---------|---------------------|
| `quon.type.mismatch` | E301 | `Mismatch` |
| `quon.type.unbound-variable` | E302 | `UnboundVariable` |
| `quon.type.not-a-function` | E303 | `NotAFunction` |
| `quon.type.not-numeric` | E304 | `NotNumeric` |
| `quon.type.arity-mismatch` | E305 | `ArityMismatch` |
| `quon.type.non-exhaustive-match` | E306 | `NonExhaustive` |
| `quon.type.unreachable-arm` | E307 | `UnreachableArm` |
| `quon.type.ambiguous-lambda` | E308 | `AmbiguousLambda` |
| `quon.type.infinite-type` | E309 | `OccursCheck` |
| `quon.type.alias-arity` | E310 | `AliasArity` |

### 3.5 Linearity (`quon.linearity.*`)

| Code | Numeric | `TypeError` variant | Primary span | Related info |
|------|---------|---------------------|--------------|--------------|
| `quon.linearity.used-twice` | E401 | `LinearUsedTwice` | Second use (`span`) | First use (`first`) |
| `quon.linearity.unconsumed` | E402 | `LinearUnconsumed` | Binding introduction | — |
| `quon.linearity.branch-mismatch` | E403 | `LinearBranchMismatch` | Offending branch | Other branch binding site (best-effort) |
| `quon.linearity.discard` | E404 | `LinearDiscard` | Wildcard `_` pattern | Binding of `bound_name` |
| `quon.linearity.capture` | E405 | `LinearCapture` | Closure body reference | Binding of `name` |

### 3.6 Circuit surface (`quon.circuit.*`)

| Code | Numeric | `TypeError` variant |
|------|---------|---------------------|
| `quon.circuit.not-a-circuit` | E501 | `NotACircuit` |
| `quon.circuit.qubit-count-mismatch` | E502 | `QubitCountMismatch` |
| `quon.circuit.gate-target-arity` | E503 | `GateTargetArity` |
| `quon.circuit.index-out-of-bounds` | E504 | `IndexOutOfBounds` |

### 3.7 Refinement (`quon.refinement.*`)

| Code | Numeric | `TypeError` variant |
|------|---------|---------------------|
| `quon.refinement.clifford-mismatch` | E601 | `CliffordMismatch` |
| `quon.refinement.depth-mismatch` | E602 | `DepthMismatch` |
| `quon.refinement.depth-intractable` | E603 | `DepthIntractable` |

### 3.8 Quantum monad & borrow (`quon.monad.*`, `quon.borrow.*`)

| Code | Numeric | `TypeError` variant |
|------|---------|---------------------|
| `quon.monad.expected-monad` | E701 | `ExpectedMonad` |
| `quon.borrow.escape` | E702 | `BorrowEscape` |

### 3.9 Dependent types & recursion (`quon.dependent.*`, `quon.recursion.*`)

| Code | Numeric | `TypeError` variant |
|------|---------|----------|
| `quon.dependent.non-dependent-arg` | E801 | `NonDependentArg` |
| `quon.recursion.ill-founded` | E802 | `IllFoundedRecursion` |
| `quon.recursion.mutual` | E803 | `MutualRecursion` |

### 3.10 Other

| Code | Numeric | `TypeError` variant |
|------|---------|---------------------|
| `quon.unsupported.quantum-fragment` | E901 | `Unsupported` |
| `quon.lower.elaboration-failed` | E902 | Lowering `ElaborationError` (string today) |

**Acceptance mapping:** every row in §3.1 (lexer), §3.3 (desugar), §3.4–3.10 (typecheck), and §3.2 E101 (parser default) must have a unit test proving the code is emitted for at least one representative program. Parser codes E102–E105 are deferred (see §3.2 note). `TypeError` variants map 1:1 to codes in §3.4–3.10 (28 variants → 28 codes).

---

## 4. TypeError → code mapping table (complete)

| `TypeError` variant | Code | LSP severity | Notes |
|---------------------|------|--------------|-------|
| `Mismatch` | `quon.type.mismatch` | Error | Includes if-branch disagreement surfaced as mismatch |
| `UnboundVariable` | `quon.type.unbound-variable` | Error | |
| `NotAFunction` | `quon.type.not-a-function` | Error | |
| `NotNumeric` | `quon.type.not-numeric` | Error | |
| `ArityMismatch` | `quon.type.arity-mismatch` | Error | Tuple / pattern arity |
| `NonExhaustive` | `quon.type.non-exhaustive-match` | Error | Include `witness` in message |
| `UnreachableArm` | `quon.type.unreachable-arm` | Error | |
| `AmbiguousLambda` | `quon.type.ambiguous-lambda` | Error | |
| `OccursCheck` | `quon.type.infinite-type` | Error | |
| `AliasArity` | `quon.type.alias-arity` | Error | |
| `LinearUsedTwice` | `quon.linearity.used-twice` | Error | Related: `first` span |
| `LinearUnconsumed` | `quon.linearity.unconsumed` | Error | Quick fix candidate (§5) |
| `LinearBranchMismatch` | `quon.linearity.branch-mismatch` | Error | |
| `LinearDiscard` | `quon.linearity.discard` | Error | Quick fix candidate (§5) |
| `LinearCapture` | `quon.linearity.capture` | Error | No auto-fix (requires refactor) |
| `NotACircuit` | `quon.circuit.not-a-circuit` | Error | |
| `QubitCountMismatch` | `quon.circuit.qubit-count-mismatch` | Error | |
| `GateTargetArity` | `quon.circuit.gate-target-arity` | Error | |
| `IndexOutOfBounds` | `quon.circuit.index-out-of-bounds` | Error | |
| `CliffordMismatch` | `quon.refinement.clifford-mismatch` | Error | Quick fix candidate (§5) |
| `DepthMismatch` | `quon.refinement.depth-mismatch` | Error | Quick fix candidate (§5) |
| `DepthIntractable` | `quon.refinement.depth-intractable` | Error | Suggest static bound (text only) |
| `ExpectedMonad` | `quon.monad.expected-monad` | Error | |
| `BorrowEscape` | `quon.borrow.escape` | Error | Related info only (§5.2); **not** auto-fixable |
| `NonDependentArg` | `quon.dependent.non-dependent-arg` | Error | |
| `IllFoundedRecursion` | `quon.recursion.ill-founded` | Error | |
| `MutualRecursion` | `quon.recursion.mutual` | Error | |
| `Unsupported` | `quon.unsupported.quantum-fragment` | Error | |

Implement as inherent methods on `TypeError`:

```rust
impl TypeError {
    pub fn code(&self) -> DiagnosticCode { … }
    pub fn severity(&self) -> DiagnosticSeverity { DiagnosticSeverity::Error }
    pub fn related(&self) -> Vec<RelatedInfo> { … }
    /// Called once from `to_rich_diagnostic(source)` inside `analyze()` — not re-run per LSP request.
    pub fn quick_fixes(&self, source: &str) -> Vec<QuickFix> { … }
    pub fn to_rich_diagnostic(&self, source: &str) -> RichDiagnostic { … }
}
```

`analyze()` populates `RichDiagnostic.fixes` **exactly once** when lowering each error (via `to_rich_diagnostic(source)`). The LSP `codeAction` handler reads cached fixes; it does not re-invoke fix generation.

---

## 5. Code action design

### 5.1 Frontend `QuickFix` model

Add to `frontend/src/diagnostics.rs` (or `frontend/src/diagnostics/fixes.rs`):

```rust
pub struct QuickFix {
    pub title: String,
    pub kind: QuickFixKind,           // e.g. QuickAssist | RefactorRewrite
    pub edits: Vec<TextEdit>,         // byte offsets in UTF-8 source
}

pub struct TextEdit {
    pub span: SimpleSpan,
    pub replacement: String,
}
```

`RichDiagnostic` carries `fixes: Vec<QuickFix>`. The LSP layer converts byte spans to LSP `Range` using the `#43` UTF-16/line-column helper (must handle multi-byte UTF-8 safely; Quon sources are ASCII-heavy but tests should include a Unicode comment case).

### 5.2 Safe fixes to implement (minimum 3 + recommended extras)

Priority fixes — each must have a **frontend unit test** asserting exact `TextEdit` bytes and a **quon_lsp integration test** asserting `textDocument/codeAction` returns the action.

#### Fix A — `quon.linearity.unconsumed` (borrow block, auto-fixable)

**When:** `LinearUnconsumed { name, .. }` where `name` is introduced by an innermost `borrow … in { … }` block and the block body ends with `return <expr>` that does **not** mention `name` (no trailing cleanup).

**Action titles:**
- `Insert discard(<name>) before return`
- `Insert reset(<name>) before return` (second action, same diagnostic)

**Edit:** Find the `return` statement span inside the borrow block; insert a new line before it:

```quon
    discard(a)
```

Indentation: copy leading whitespace of the `return` line (preserve user style).

**Safety guards:**
- Only offer when `name` is a simple binding from the borrow header (not destructured tuple).
- Do not offer if `discard(<name>)` or `reset(<name>)` already appears in the block.
- Do not offer when the unconsumed binding is not from a `borrow` header (e.g. function-parameter linearity) — those require manual restructuring.

**Fixture:**

```quon
fn f(): Q<Int> = run {
  borrow a: Qubit in {
    return 0
  }
}
```

→ insert `discard(a)\n` before `return`.

#### Fix A′ — `quon.borrow.escape` (related info only, **not** auto-fixable)

**When:** `BorrowEscape { name, .. }` — the ancilla appears inside the returned expression (e.g. `return (q, a)`).

**v1 behavior:** emit `relatedInformation` pointing at the borrow binding site (`borrow_span`). Attach **zero** `QuickFix` entries — a prepend of `discard(a)` before `return (q, a)` does not resolve the escape and would mislead the user.

**Fixture:** `return a` or `return (q, a)` inside a borrow block → related info at `borrow a: Qubit`, no code action.

#### Fix B — `quon.refinement.clifford-mismatch` (annotation too strict)

**When:** `CliffordMismatch { expected: Clifford, found: Universal, … }` at a **return type annotation** or **parameter type annotation** site (span overlaps `Clifford` token in source).

**Action:** `Change annotation to Universal`

**Edit:** Replace the annotated `Clifford` token with `Universal`.

**Safety guards:**
- Only when `found == Universal` (never auto-weaken the other direction).
- Span must resolve to the literal `Clifford` class token in the type annotation (parse type AST from enclosing decl when needed, or match source slice).

**Fixture:** `frontend/tests/circuits.rs` `a_t_gate_annotated_clifford_is_rejected`.

#### Fix C — `quon.refinement.depth-mismatch` (constant inferred depth)

**When:** `DepthMismatch { expected, found, … }` and `found` parses as a concrete natural (`DepthExpr::Nat(n)` or decimal string from renderer).

**Action:** `Update depth annotation to <found>`

**Edit:** Replace the annotated depth subexpression in the `Circuit<…, D, …>` type with `found`'s textual form.

**Safety guards:**
- Only when inferred depth is a **constant** (fast-path / `Nat` literal). Do **not** auto-edit symbolic depths (`n + 1`, `2 * n`) — solver may accept looser bounds the user intended.
- Only replace the depth field position, not width/class fields.

**Fixture:** `constant_depth_below_the_synthesized_depth_is_a_depth_mismatch` in `typecheck/tests.rs` (bell depth 2 annotated 1).

#### Fix D (recommended 4th) — `quon.linearity.discard`

**When:** `LinearDiscard { bound_name, span, .. }` on `_` in `let _ = q`.

**Schema note:** extend `TypeError::LinearDiscard` with `bound_name: String` — the RHS identifier (e.g. `q`). Today the checker stores `name: resolved.to_string()` (the **type** name, e.g. `"Qubit"`), which is wrong for fixes and related info. Also add `binding_span: SimpleSpan` for the related-info site.

**Action:** `Replace _ with discard(<bound_name>)`

**Edit:** Rewrite `let _ = <bound_name>` → `discard(<bound_name>)` (or equivalent statement form after desugaring).

**Safety:** Only for simple `let _ = <var>` inside `run`/function body; not for tuple patterns.

### 5.3 LSP wiring (`quon_lsp`, **#44 scope**)

`#43` delivers diagnostics publishing only. **`#44` owns** `codeActionProvider` registration and the handler.

In `#44`'s `initialize` response, add:

```json
"codeActionProvider": { "codeActionKinds": ["quickfix", "refactor.rewrite"] }
```

On `textDocument/codeAction`:

1. Read cached `AnalysisResult` keyed by document version (do not re-run fix generation).
2. Filter diagnostics whose primary range intersects the requested range.
3. Map each pre-populated `QuickFix` → `CodeAction { title, kind, edit: WorkspaceEdit }`.
4. Set `is_preferred: true` only for the highest-confidence fix (Fix B/C when single-token replacement).

---

## 6. Frontend changes (detailed)

### 6.1 Enrich `diagnostics.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticCode(&'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity { Error, Warning, Info, Hint }

#[derive(Debug, Clone, PartialEq)]
pub struct RelatedInfo {
    pub message: String,
    pub span: SimpleSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RichDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub span: SimpleSpan,
    pub related: Vec<RelatedInfo>,
    pub fixes: Vec<QuickFix>,
}

impl RichDiagnostic {
    pub fn primary_label(&self) -> &str { &self.message }
}
```

Keep plain `Diagnostic` for backward compatibility (`quonc` ariadne path):

```rust
impl From<&RichDiagnostic> for Diagnostic {
    fn from(d: &RichDiagnostic) -> Self {
        Diagnostic::new(d.message.clone(), d.span)
    }
}
```

### 6.2 Stage adapters

| Stage | Function | Change |
|-------|----------|--------|
| Lexer | `lexer::lex` | Add `lex_rich(src) -> Result<_, Vec<RichDiagnostic>>` classifying chumsky labels |
| Parser | `parser::parse` | Add `parse_rich` mapping `Rich<Token>` → code via label/heuristic |
| Desugar | `desugar_decls` | Emit coded diagnostics at the 3 existing sites |
| Typecheck | `TypeError` | `to_rich_diagnostic(source)` — populates `fixes` once |
| Lower | out of scope for LSP analysis in #44 | `analyze` stops after typecheck |

### 6.3 New public API

```rust
// frontend/src/lib.rs

pub struct AnalysisResult {
    pub diagnostics: Vec<RichDiagnostic>,
}

/// Parse + desugar + type-check for IDE use. Does not lower to MLIR.
/// Accumates errors from all stages; returns after first failing stage
/// (same semantics as today) but preserves structure.
pub fn analyze(source: &str) -> AnalysisResult { … }

/// Back-compat wrapper — calls `analyze()` exactly once:
pub fn check_program(source: &str) -> Result<(), Vec<Diagnostic>> {
    let result = analyze(source);
    if result.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(result.diagnostics.iter().map(Diagnostic::from).collect())
    }
}
```

Each stage adapter passes `source` into `to_rich_diagnostic(source)` so `fixes` are populated once during the pipeline run.

**Partial / malformed source:** `analyze` must never panic. Explicitly test:

- Truncated mid-token (`fn f(): Cir`)
- Unclosed `run { borrow a: Qubit in {`
- Lone `#`, lone `(`, file ending mid-identifier
- Empty file

Wrap any future `assert!` sites in checker with error returns (audit `typecheck/mod.rs` `Unsupported` path — already total).

### 6.4 Related information rules

| Error | Related entry |
|-------|---------------|
| `LinearUsedTwice { name, first, span }` | `{ message: "first use of `{name}`", span: first }` |
| `LinearDiscard { bound_name, binding_span, .. }` | `{ message: "`{bound_name}` bound here", span: binding_span }` — extend variant with `bound_name` (RHS identifier, **not** type string) and `binding_span` |
| `BorrowEscape { name, borrow_span, .. }` | `{ message: "borrowed as `{name}` here", span: borrow_span }` — extend variant with `borrow_span: SimpleSpan` |
| Others | `related: []` initially |

**Schema extension (small):** add `bound_name` + `binding_span` to `LinearDiscard` (today `name` holds the resolved **type** string — wrong for fixes); add `borrow_span` to `BorrowEscape`; add `binding_span` to `LinearUnconsumed` where needed. Keep `Display` messages unchanged.

---

## 7. `quon_lsp` changes (this PR, atop #43)

| File | Responsibility |
|------|----------------|
| `quon_lsp/src/diagnostics.rs` | `RichDiagnostic` → `lsp_types::Diagnostic` (code, severity, relatedInformation) |
| `quon_lsp/src/code_action.rs` | `QuickFix` → `CodeAction`, range filtering |
| `quon_lsp/src/analysis.rs` | Call `frontend::analyze` instead of `check_program` |
| `quon_lsp/tests/diagnostics.rs` | JSON-RPC fixture harness |

LSP diagnostic fields:

```json
{
  "range": { … },
  "severity": 1,
  "code": "quon.linearity.unconsumed",
  "source": "quon",
  "message": "linear resource `a` is never consumed (no-dropping)",
  "relatedInformation": [
    {
      "location": { "uri": "…", "range": { … } },
      "message": "borrowed as `a` here"
    }
  ]
}
```

---

## 8. Test plan & fixtures

### 8.1 Frontend unit tests — `frontend/tests/lsp_diagnostics.rs`

Shared helpers:

```rust
fn analyze(src: &str) -> Vec<RichDiagnostic> {
    frontend::analyze(src).diagnostics
}

fn assert_code(src: &str, code: &str) { … }
fn assert_related_count(src: &str, n: usize) { … }
fn apply_fix(src: &str, fix: &QuickFix) -> String { … }
```

| Test | Fixture source | Assert |
|------|----------------|--------|
| `lex_invalid_char_has_code` | `a # b` | `quon.lex.invalid-char`, span `@ #` |
| `parse_error_has_code` | `fn f(): Int = )` | `quon.parse.*`, span on `)` |
| `linear_used_twice_related` | `fn f(q: Qubit): QReg<2> = (q, q)` | code + related at first `q` |
| `linear_unconsumed_borrow_fix` | borrow + `return 0` | `discard(a)` edit bytes |
| `clifford_mismatch_fix` | `T @0` annotated Clifford | `Universal` replacement |
| `depth_mismatch_constant_fix` | bell annotated depth 1 | depth → `2` |
| `borrow_escape_no_fix` | `return a` in borrow | related info at borrow site; **zero** fixes |
| `partial_source_no_panic` | corpus of truncated snippets | `analyze` returns, no panic |

Reuse/adapt programs from:

- `frontend/tests/linearity.rs`
- `frontend/tests/circuits.rs`
- `frontend/tests/typecheck.rs`
- `frontend/src/typecheck/tests.rs` (borrow / depth / Clifford cases)

### 8.2 Fixture files — `frontend/tests/fixtures/diagnostics/`

Add minimal `.qn` files (one error each) for legibility and optional insta snapshots:

```
frontend/tests/fixtures/diagnostics/
  linear_used_twice.qn
  linear_unconsumed_borrow.qn
  clifford_mismatch_t_gate.qn
  depth_mismatch_bell.qn
  parse_unclosed_paren.qn
  lex_invalid_char.qn
```

Each file paired with `.expect.json` (optional) listing `{ code, span_start, span_end, fixes: [titles] }` for regression.

### 8.3 `quon_lsp` integration tests — `quon_lsp/tests/diagnostics.rs`

Use `#43`'s LSP test harness (stdio JSON-RPC replay):

1. `didOpen` fixture → `publishDiagnostics` notification matches expected codes/ranges.
2. `codeAction` at diagnostic range returns ≥1 action for the three mandated fixes.
3. Apply `WorkspaceEdit` → re-analyze → diagnostic cleared (for Fix A/B/C).

Also add **fuzz-style** test: random suffix truncation of `frontend/tests/fixtures/bell_state.qn` (deterministic seed) — `analyze` never panics.

---

## 9. Implementation steps (ordered)

### Step 1 — Diagnostic model in `frontend` (no LSP yet)

- [ ] Add `DiagnosticCode`, `DiagnosticSeverity`, `RelatedInfo`, `QuickFix`, `RichDiagnostic`.
- [ ] Export from `frontend::diagnostics`.
- [ ] Unit test: construct + clone + `From` to legacy `Diagnostic`.

### Step 2 — `TypeError` enrichment

- [ ] `code()`, `related()`, `to_rich_diagnostic()`.
- [ ] Extend variants: `LinearDiscard { bound_name, binding_span, .. }`, `BorrowEscape { borrow_span, .. }`, `LinearUnconsumed { binding_span, .. }` as needed.
- [ ] Table-driven unit test: every `TypeError` variant maps to unique code (const array + `match` exhaustiveness).

### Step 3 — Stage classification

- [ ] `lex_rich`, `parse_rich`, coded desugar errors.
- [ ] Wire `analyze()` pipeline.
- [ ] Port existing integration tests to also assert `code` (non-breaking: add parallel assertions).

### Step 4 — Quick fix generation

- [ ] `QuickFix` helpers module with indentation detection.
- [ ] Implement Fixes A, B, C (+ D if time).
- [ ] Unit tests with `apply_fix` round-trip.

### Step 5 — `quon_lsp` adapter (#44 portion)

- [ ] Map to LSP diagnostics + relatedInformation.
- [ ] Implement `codeAction` handler reading cached analysis.
- [ ] Integration tests.

### Step 6 — `quonc` compatibility

- [ ] Keep `check_program` / ariadne output unchanged (uses legacy `Diagnostic`).
- [ ] Optional: print code in parentheses in verbose mode (not required for AC).

### Step 7 — Validation

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --exclude flux_verify`
- [ ] `cargo test --workspace --exclude flux_verify`
- [ ] `npx @taskless/cli@latest check` on touched files

---

## 10. Risks & mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| **Chumsky parser errors lack stable labels** | Parser codes devolve to `unexpected-token` bucket | Accept for v1; add `.label(...)` at key grammar nodes in a small follow-up; test at least one parse error per code where feasible |
| **`TypeError` → string already used in tests** | Changing messages breaks snapshots | Do not change `Display` text; codes are additive |
| **Quick fix span drift after edit** | Stale actions after apply | Bump document version; re-analyze on apply; LSP client usually refreshes |
| **Z3 latency on every keystroke** | UI jank | `#43` debounce (~150–300 ms); `#44` does not add new solver calls |
| **Borrow-block fix indentation** | Ugly or broken edits | Copy whitespace from `return` line; test with 2-space and 4-space fixtures |
| **Depth fix on symbolic bounds** | Incorrect auto-edit | Strict constant-only guard; no fix for symbolic `found` |
| **UTF-16 vs byte spans in LSP** | Misaligned squiggles | Centralize conversion in `#43`; add one multi-byte UTF-8 test in `#44` |
| **Related span missing after AST change** | Empty relatedInformation | Extend error variants with explicit spans rather than re-parsing |

---

## 11. Out of scope (explicit)

- Hover, completion, definition, semantic tokens → **#45**
- Lint warnings / stylistic diagnostics → **#47**
- Formatting quick fixes → **#46**
- MLIR/lowering diagnostics → not needed for `.qn` editing loop
- Auto-fix for `LinearUsedTwice` (requires program refactor, not safe single edit)
- CI LSP smoke job → **#49**

---

## 12. Definition of done (acceptance checklist)

- [ ] Every `TypeError` variant maps to a stable `quon.*` code (§4 table).
- [ ] Lexer, parser (E101 default), and desugar errors emit coded `RichDiagnostic`s.
- [ ] LSP `publishDiagnostics` includes `range`, `message`, `code`, `source`, `severity`, `relatedInformation`.
- [ ] ≥3 safe quick fixes implemented with frontend + `quon_lsp` tests (§5.2 A, B, C — `LinearUnconsumed` only for Fix A; `BorrowEscape` has no auto-fix).
- [ ] `codeActionProvider` registered and handled in `#44` (not `#43`).
- [ ] `frontend::analyze` never panics on malformed/partial fixtures (§8.1).
- [ ] Fixture-based tests cover parser, type, linearity, and depth diagnostic paths (§8).
- [ ] `check_program` / `quonc` remain backward compatible.

---

## 13. Suggested PR stack (Graphite)

1. **PR 1 (frontend):** Rich diagnostic model + `TypeError` codes + `analyze()` (Steps 1–3).
2. **PR 2 (frontend):** Quick fixes + frontend tests (Step 4).
3. **PR 3 (quon_lsp):** LSP mapping + code actions + integration tests (Step 5).

Each PR should pass the full workspace test suite independently.
