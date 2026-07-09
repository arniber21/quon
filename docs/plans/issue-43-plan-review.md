# Issue #43 Plan Review — Adversarial Grade

**Plan reviewed:** `docs/plans/issue-43-plan.md`  
**Worktree:** `43-lsp-foundation`  
**Issue:** [#43 — LSP foundation](https://github.com/arniber21/quon/issues/43)  
**Reviewer:** Adversarial plan review (pre-implementation)  
**Date:** 2026-07-08

---

## Overall Grade: **B-**

## Implementation Readiness: **FAIL**

The plan picks the right analysis seam (`frontend::check_program`), correctly scopes out lowering, and maps well to GitHub issue #43 acceptance criteria. Several concrete errors in fixtures/API sketches, a Taskless convention conflict, and underspecified concurrency/debounce design would cause an implementing agent to ship broken tests or fail CI on the first pass. Fix the critical blockers below before writing code.

---

## Critical Blockers (must fix before implement)

### 1. Test fixtures use invalid Quon syntax

The plan repeatedly uses:

```rust
let src = "fn f(x: Int) = x + y\n";
```

The parser **requires** an explicit return type after parameters (`:` + `ty` before `=`):

```790:806:frontend/src/parser.rs
    let fn_decl = just(Token::Fn)
        .ignore_then(ident)
        // ...
        .then_ignore(just(Token::Colon))
        .then(ty.clone())
        .then_ignore(nls.clone())
        .then_ignore(just(Token::Eq))
```

These fixtures would fail at **parse** time, not type-check time, producing the wrong diagnostic class and wrong span assertions. Use an existing pattern from `frontend/tests/typecheck.rs`, e.g.:

```rust
let src = "fn f(): Int = ghost";  // unbound variable, span on "ghost"
// or
let src = "fn f(x: Int): Int = x + y\n";  // unbound `y`
```

### 2. Wrong byte offset in worked example

Section 7.1 claims `y` starts at byte **18** in `"fn f(x: Int) = x + y\n"`. Even with corrected syntax `"fn f(x: Int): Int = x + y\n"`, `y` is at byte **24** (verified). An agent copying this assertion will get failing tests unrelated to the mapper.

### 3. Taskless `anyhow` rule will fail CI

Plan puts `anyhow::Result` in `main.rs` and cites the `no-anyhow-in-lib-src` rule with an exception for `main.rs`. **The rule does not grant that exception to `quon_lsp`:**

```8:9:.taskless/rules/no-anyhow-in-lib-src.yml
ignores:
  - "quonc/**"
```

Only `quonc/**` is ignored. `quon_lsp/src/main.rs` using `anyhow` **will fail Taskless** on PR. Either:

- Add `quon_lsp/**` to the rule ignores (mirror `quonc`), **or**
- Use `thiserror` + structured exit codes in `main.rs`, **or**
- Document a required Taskless rule update in step 6.10.

Acceptance criterion "No anyhow in library modules" is fine, but the plan must address the binary entry point vs Taskless gap explicitly.

### 4. `AnalysisScheduler` API incompatible with `tower-lsp`

`LanguageServer` methods take `&self`:

```360:361:docs/plans/issue-43-plan.md
#[tower_lsp::async_trait]
impl LanguageServer for QuonLanguageServer {
```

But `AnalysisScheduler::request_analysis(&mut self, ...)` and `pending: HashMap<Url, JoinHandle<()>>` require interior mutability. An agent following the sketch will hit compile errors or resort to `unsafe`/global statics.

**Required:** Wrap scheduler state in `Arc<Mutex<...>>` or use a dedicated analysis task + `mpsc` channel; document the pattern explicitly.

### 5. Debounce design is incomplete and contradicts test requirements

The plan declares `debounce: Duration` and `JoinHandle` coalescing but never specifies the mechanism (`tokio::time::sleep` + cancellation token? `Notify`? per-URI debounce tasks?). Section 7.3 simultaneously demands:

- "Assert diagnostics update **within one analysis cycle**"
- "**CI constraint**: no flaky timing"

These conflict unless tests use `debounce = Duration::ZERO` via `cfg(test)` or `QUON_LSP_DEBOUNCE_MS=0`. Without this, `incremental.rs` will be flaky in CI.

### 6. Missing `anyhow` in `quon_lsp/Cargo.toml`

Section 5.6 `main.rs` uses `anyhow::Result` but section 4.1 dependencies omit `anyhow`. Step 6.1 will not compile as written.

---

## Major Concerns

### Span mapping — API inconsistency and unresolved dependency choice

- Section 5.2 defines `line_col()`; section 5.3 calls `line_index.position()`. Pick one name and use it everywhere.
- `line-index` crate is "consider" but never decided. Given UTF-16 correctness is an acceptance criterion, **commit to `line-index`** (rust-analyzer proven) or specify a minimal tested implementation — don't leave this to the implementer mid-PR.
- Quon identifiers are **ASCII-only** (`text::ascii::ident()` in `frontend/src/lexer.rs`). UTF-16 edge-case tests for emoji *identifiers* are unrealistic today; test unicode in **comments** (`--` / `{- -}`) or string literals instead, which matches actual lexer behavior.

### Z3 context recreated on every analysis — understated risk

Each `check_program` call creates `TypeChecker::new()` → `RefinementCtx::new()` → `z3::Context::new()`:

```134:145:frontend/src/typecheck/mod.rs
    pub fn new() -> Self {
        Self {
            // ...
            refine: RefinementCtx::new(),
```

The plan says "Z3 init latency on **first** check" but cost is **per edit** (after debounce). For #43 this may be acceptable, but the plan should set expectations: first diagnostic after open may be 100–500ms+ depending on platform. Consider noting a follow-up to reuse `RefinementCtx` across analyses (not blocking #43, but don't mislabel the risk).

### Transitive LLVM/Melior link is real

`frontend/Cargo.toml` depends on `melior`, `mlir_bridge`. `check_program` does not call lowering, but `quon_lsp` will still link LLVM. Plan acknowledges this; CI claim "no workflow change needed" is **correct** — `.github/workflows/ci.yml` already installs LLVM 22 and runs `cargo build --release --workspace`.

### Integration test harness underspecified

Existing repo pattern uses `env!("CARGO_BIN_EXE_quonc")` (see `quonc/tests/cli.rs`). Plan says spawn `quon_lsp` but doesn't mention `CARGO_BIN_EXE_*`. Without it, tests may invoke wrong binary or require `cargo run` (slow, flaky).

Reading `textDocument/publishDiagnostics` **notifications** while also driving request/response requires an async stdout reader or dedicated thread. "~80 lines" is optimistic; budget 150–250 lines for a minimal framing client. Plan should reference a module layout (`tests/support/lsp_client.rs`).

### stdout vs stderr — unlisted risk

LSP uses stdout for JSON-RPC. `tracing_subscriber::fmt()` defaults to stderr (fine), but plan must **explicitly forbid** `println!` / logging to stdout in library code. One stray log breaks the protocol.

### `Document.line_index` field is redundant/confusing

Plan stores `line_index` on `Document` but also rebuilds in `run_analysis`. Clarify: `Document.line_index` is for **edit application** (LSP Position → byte offset); analysis rebuilds from snapshot text. Or drop the field from `Document` and rebuild only when needed.

### Issue blockers are closed — plan should note this

GitHub #43 lists blockers #7, #9–#12, #14, #15 — all **CLOSED**. Plan doesn't mention this; add a line so implementers don't re-verify.

### `tower-lsp` 0.20 is stale but acceptable

Last release August 2023. Fine for #43 foundation; note maintenance risk in follow-ups. No blocker.

---

## Minor Nits

- `diagnostic_to_lsp(..., source: &str, ...)` — `source` unused in sketch; use it for EOF clamping or drop the parameter.
- `NumberOrString` imported but unused in diagnostics snippet.
- `DiagnosticMapper` appears in architecture diagram but module only exports functions — pick struct vs free functions.
- Step 6.4 vs `tests/incremental.rs` — name collision between DocumentStore unit tests and integration tests; rename to `tests/document.rs` (unit) vs `tests/incremental_lsp.rs` (integration).
- Editor config uses `cargo run -p quon_lsp --quiet` — document that dev should point at `target/debug/quon_lsp` after first build to avoid rebuild latency.
- `initialize` response could include `server_info` for debugging (optional polish).
- `did_change` on unknown URI should no-op or log, not panic.
- Acceptance checklist item "No unwrap/expect/anyhow in library modules" — tests may use `expect` per Taskless ignores; clarify.

---

## Specific Required Plan Amendments

These are **mandatory** given grade B-:

| # | Section | Amendment |
|---|---------|-----------|
| A1 | §5.3, §7.1, §7.3 | Replace all `"fn f(x: Int) = x + y"` with valid Quon (`fn f(): Int = ghost` or `fn f(x: Int): Int = x + y`). Recompute byte offsets from `src.find(needle)`. |
| A2 | §4.1 | Add `anyhow = { workspace = true }` to `quon_lsp` **or** remove anyhow from main and use `thiserror`. |
| A3 | §4.1 / §6.10 | Add explicit Taskless rule update: extend `no-anyhow-in-lib-src` ignores to `quon_lsp/**` (or document why not). |
| A4 | §5.4, §5.5 | Redesign `AnalysisScheduler` for `&self` LanguageServer: `Arc<Mutex<SchedulerState>>` or channel-based worker; remove `&mut self` from public API. |
| A5 | §5.4, §7.3, §8 | Specify debounce implementation (per-URI `tokio::time::sleep` + cancel on new edit). Add `#[cfg(test)]` or env var to set debounce to 0 in tests. Remove contradictory "no flaky timing" vs "one debounce window" unless A5 is done. |
| A6 | §5.2, §5.3 | Unify API name (`position` or `line_col`, not both). **Decide:** add `line-index = "0.1"` to workspace deps. |
| A7 | §7.2, §7.3 | Use `env!("CARGO_BIN_EXE_quon_lsp")`; add `tests/support/lsp_client.rs` module; document notification-reading architecture. |
| A8 | §8 (risks) | Add stdout pollution risk; add per-edit Z3 Context allocation (not just first-check). |
| A9 | §7.1 | Revise UTF-16 test cases: ASCII identifiers + unicode in comments/strings, not emoji identifiers. |
| A10 | §1 or §9 | Note GitHub blockers #7/#9–#15 are closed; implementation unblocked. |

---

## Verification Summary (claims checked against codebase)

| Claim | Verdict |
|-------|---------|
| `check_program` is correct LSP entry point | ✅ Verified (`frontend/src/lib.rs:60-65`) |
| Spans are byte offsets | ✅ Verified (chumsky `SimpleSpan`, `lexer.rs` test at byte 2) |
| `quonc` uses byte ranges for ariadne | ✅ Verified (`quonc/src/main.rs:155`) |
| Do not call `lower_program_to_mlir` | ✅ Correct |
| `tower-lsp` re-exports `lsp_types` | ✅ Correct for 0.20 |
| Workspace has no tokio/tower-lsp yet | ✅ Plan correctly adds workspace deps |
| CI will pick up new member automatically | ✅ `ci.yml` uses `--workspace` |
| Edition 2024 | ✅ Matches other crates |
| Example `fn f(x: Int) = x + y` type-checks | ❌ Parse error — missing `: RetTy` |
| Byte 18 for `y` | ❌ Wrong |
| anyhow in main passes Taskless | ❌ Rule only ignores `quonc/**` |
| Issue #43 blockers open | ❌ All closed (implementation unblocked) |

---

## Conclusion

Proceed after amendments A1–A10. The architectural spine is sound; the gaps are in executable details that would cause immediate test/CI/concurrency failures. After fixes, this plan should reach **A-/A** and pass implementation readiness.
