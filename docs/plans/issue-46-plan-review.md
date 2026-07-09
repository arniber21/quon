# Issue #46 Plan Review — Adversarial Grade

**Plan:** `docs/plans/issue-46-plan.md`  
**Reviewed against:** `frontend/src/pretty.rs`, `frontend/src/parser.rs`, `frontend/src/ast.rs`, `frontend/tests/support/mod.rs`, `frontend/Cargo.toml`, `quonc/`, fixtures  
**Reviewer stance:** Adversarial — assume the plan will fail unless gaps are closed before coding.

---

## Overall Grade: **C+**

## Pass / Fail: **FAIL**

The plan is well-structured, correctly separates `frontend::pretty` from canonical formatting, and proposes a sound three-layer test strategy (golden + idempotency + AST stability). It is not implementation-ready: several correctness contracts are underspecified or wrong, a central printing problem (`App`/juxtaposition) is missing, and a stated build constraint (MLIR-free `quonfmt`) contradicts the actual dependency graph.

---

## Critical Blockers

### 1. False claim: `quonfmt` is MLIR-free / fast to build

§4 and §5 Phase D state that `quonfmt` depends only on `frontend::parse_program` and needs no MLIR env. **This is incorrect.**

`frontend/Cargo.toml` pulls `mlir_bridge`, `melior`, and `z3` unconditionally. `frontend/src/lower.rs` and `frontend/src/lib.rs` reference `melior` at compile time. Building `quonfmt` with `frontend` as a path dependency will compile and link the full frontend crate graph, including MLIR/LLVM and Z3 — same toolchain burden as `quonc`, minus actually *running* lowering.

**Required amendment:** Either (a) add a `frontend` feature such as `parser-only` that excludes `lower`, `mlir_bridge`, `melior`, and optionally `refinement`/Z3; (b) extract lexer/parser/ast into a thin crate (e.g. extend `quon_core` or add `quon_syntax`); or (c) drop the MLIR-free claim and document LLVM/Z3 prerequisites for `cargo test -p quonfmt`. Option (a) or (b) is strongly preferred if README/CI messaging is to remain truthful.

### 2. No specification for `Expr::App` / juxtaposition / postfix printing

The parser desugars surface syntax into nested `Expr::App` nodes:

- Juxtaposition application (same-line only) — `parser.rs` ~541–545  
- `f(a, b, …)` curried via `Post::Call` — `apply_post` ~37–63  
- `q[i]` → `index(q, i)` — `Post::Index`  
- `` x `f` y `` → `f(x, y)` — backtick tier ~673–687  
- `x.method(a)` → `method(x, a)` — `Post::Method`

The AST has **no** nodes for juxtaposition, index, backtick, or dot syntax. A formatter must define a **canonical re-surface policy** for `App` trees. The plan never mentions this — the largest semantic gap in the whole design.

`pretty.rs` sidesteps the problem by always emitting `f(x)` with heavy parenthesization (`atom()` → `({})` for most forms). A canonical formatter cannot copy that approach and also claim minimal, idiomatic output.

**Required amendment:** Add §3.9 “Application and postfix forms” specifying:

- When to emit `f(x)` vs juxtaposition `f x` (same-line constraint from parser)  
- How to uncurry nested `App` into multi-arg call syntax  
- Canonical form for desugared `index`/`method`/backtick (always `index(q, i)` and `f(x, y)`, or attempt sugar recovery?)  
- Precedence/paren rules when an `App` operand sits under `@`, `|>`, `+`, etc.  
- Golden corpus file: `application.qn` covering juxtaposition, multi-arg calls, `q[i]`, dot calls, and backtick (parse → format → AST-stable even if surface sugar is lost)

Without this, PR 3 (“expr printer with precedence”) has no complete spec and will stall on ad-hoc decisions.

### 3. `check_str` byte semantics vs CRLF normalization

§3.1 says normalize CRLF → LF on read. §4 API says `check_str` compares **raw bytes**. §6 matrix says `--check` exits 1 if not formatted.

If input is CRLF-terminated, `format_str` produces LF output; `check_str` on the original file always fails even when semantically formatted. Same issue for trailing whitespace stripping.

**Required amendment:** Define `check_str` as: parse → format → compare **normalized** representations (LF, no trailing WS, final newline), not raw input bytes. Document that `--check` is intentionally normalization-aware. Add CLI test with CRLF input.

### 4. Precedence enum incomplete / misleading for prefix forms

Plan’s `Prec` enum lists `IfLet` as one tier, but the parser binds prefix forms via `choice((lambda, let_in, if_expr, return_expr, ascribed))` — **flat choice, not precedence climbing** (`parser.rs` ~783). `return` is a prefix form at expression level; it also appears inside `run` blocks as `Stmt::Expr(Return(…))`.

**Required amendment:** Document parenthesization rules for each prefix form (`fn(…) ->`, `let … in`, `if … then … else`, `return`, `: τ`) when nested as operands. Clarify `Return` printing in run-block final statements vs inner expressions. Add golden cases for nested `if`/`let`/`fn` inside operands.

### 5. Block-body kind confusion (`circuit`/`run` vs `par`/`for`)

Parser structure:

| Form | Body |
|------|------|
| `circuit { }`, `run { }`, `borrow … in { }` | `Vec<Sp<Stmt>>` (statement block) |
| `par { }`, `for … in … { }` | single `expr` in braces (`brace_expr`) |

Plan §3.4 treats blocks uniformly; §3.6 says “for … body obeys block rules” — **wrong** for `for`. `par { had_one() } * n` in `stdlib_forms.qn` is an expression body, not one-statement-per-line.

**Required amendment:** Split style spec into “statement blocks” vs “expression blocks”. Specify `par { expr }` and `for p in iter { expr }` layout separately (likely single-line if fits, else break inside braces around expr).

---

## Major Concerns

### 6. Custom `Doc` algebra scope underestimated

Building Wadler-style `Doc` with `Group`/`Nest`/`Break`, width-aware flatten, **and** bind-column alignment (two-pass max-LHS-width padding) is substantial — easily 300–500+ lines before any AST printing. Plan allocates it to PR 1 as scaffold but doesn’t budget alignment integration with `Doc`.

**Amendment:** Either (a) spike `Doc` + alignment on `run_binds.qn` in PR 1 before expr printing; or (b) use an existing pretty-print crate if policy allows; or (c) split alignment into a post-pass on flattened lines (simpler but must preserve idempotency). Add explicit milestone acceptance: “teleport.qn bind alignment matches fixture.”

### 7. Float literal policy underspecified vs lexer

Plan §3.6: “shortest round-trippable literal.” Lexer **requires** a `.` in float tokens (`lexer.rs` ~161–170). `pretty.rs` uses `Debug` and appends `.0` when needed. Plan proposes extracting `render_float` but doesn’t define the algorithm.

**Amendment:** Specify: must re-parse via `lexer`; must contain `.`; tie-break order (decimal vs scientific); handling of `-0.0`, `inf`, `NaN`. Add unit tests mirroring `float_str` edge cases plus property: `lex(format_float(f))` preserves float value.

### 8. `Cargo.toml` dependency list incomplete

Plan lists only `frontend` + `thiserror` (+ `insta` dev). Missing:

- **Bin:** `clap`, `anyhow` (per `quonc` pattern), `ariadne` (§6 diagnostics)  
- **Dev:** `proptest`, `arbitrary` (§8 Layer 2 proptest sketch)  
- **Edition:** workspace uses `edition = "2024"` — not stated for `quonfmt`

Also: library crate must use `thiserror` for `FormatError`; CLI may use `anyhow` per code-quality table — plan’s main sketch uses bare `Result<()>` without saying which.

### 9. Test harness: `frontend/tests/support` not reusable as-is

`assert_ast_stable` depends on `strip_decls` in `frontend/tests/support/mod.rs`, which is **not** exported from the `frontend` library. Plan hand-waves `#[path]` or “move helpers.”

**Amendment:** Pick one: (a) `#[path = "../frontend/tests/support/mod.rs"]` in `quonfmt/tests` (fragile); (b) move strip helpers to `frontend/src/test_util.rs` behind `#[cfg(test)]` — **not** viable for integration tests; (c) duplicate minimal strip in `quonfmt/tests/support` (maintenance cost); (d) publish `frontend` test support as optional feature. Recommend (a) short-term with comment, or extract `strip_*` into `frontend` as `pub(crate)` test helpers in a `testing` module. Be explicit in PR 5.

### 10. Proptest sketch is syntactically invalid

§8 Layer 2:

```rust
let f1 = quonfmt::format_str(&src)?;  // `?` invalid in proptest closure
```

Also doesn’t assert AST stability in the proptest (only byte idempotency). Layer 3 says both — merge into one prop or add second.

**Amendment:** Use `prop_assert` / `prop_map` pattern from `roundtrip_props.rs`. Add `prop_assert_eq!(strip(parse(src)), strip(parse(f1)))`. Consider filtering seeds that `parse_program` rejects after pretty (shouldn’t happen but guard anyway).

### 11. Golden fixture coverage gaps

Corpus omits several high-risk forms present in real fixtures:

- Bare final expr in run block (`measure(q)` in `stdlib_forms.qn`)  
- `reset`/`discard` terminal statements (`borrow.qn` mentioned but not exemplified)  
- Comment stripping (`bell_state.qn` line 10)  
- Token-adjacency identifiers (`o`, `o1` in generator — unary minus vs `-o`)  
- Nat division in types (`n/2` in `stdlib_forms.qn`)  
- Spacing normalization (`CNOT @(0,1)` → `@(0, 1)`?)  
- Empty `circuit {}` / `run {}`  
- Multi-line `|>` bridging (not just long single-line chains)

**Amendment:** Extend corpus table; add `stdlib_forms.qn` or subset as golden input; add explicit comment-stripping test.

### 12. `|>` break style left as “pick one and test”

§3.5 and §12 leave leading vs trailing `|>` unresolved despite a recommendation. For a normative formatter, ambiguity here causes snapshot churn across PRs.

**Amendment:** Lock **leading `|>` on continuation** in §3.5 (remove “pick one”). Add before/after example to style doc and one golden snapshot.

### 13. Width counting inconsistency

§3.1: max width = Unicode scalar counts. §3.4 bind alignment: “display width” / longest LHS. §12 risk: “char/byte counts consistently.” These diverge for non-ASCII identifiers (future).

**Amendment:** State one rule: column width = Unicode scalar count everywhere (matches ASCII today). Bind padding = max scalar length of LHS strings + fixed spaces before `<-`.

---

## Minor Nits

1. **§1 table** — Correct that `pretty.rs` is not formatter-ready; good diagnosis of `atom()` over-parenthesization.

2. **§5 Phase A optional extract** — `binop_str`/`class_str` are trivial `match`es; extracting them adds churn for little drift risk. Prioritize `render_float`/`render_int` only.

3. **§6 CLI sketch** — `emit(&cli, &src, path, &src)` passes `&src` twice; likely `emit(&cli, path, &src, &formatted)`.

4. **§6 behavior matrix** — No rule for `-w` + `--check` together, multiple files with mixed parse success, or exit code aggregation. Specify: clap conflict; exit 2 on parse error; highest exit code wins.

5. **`--check` diff hint** — API promises “unified diff hint” but no implementation approach (similarity crate? manual line diff? message only?). Specify minimal v1: print path + “would reformat” or unified diff via `similar` crate.

6. **§11 validation** — Uses `cargo clippy -p quonfmt` but code-quality defaults to workspace clippy; fine for PR scope but note both.

7. **§10 PR stack** — Six PRs is reasonable; PR 3+4 may be large. Consider splitting `ty`/`nat`/`pat` from expr if goldens go red.

8. **`Expr::Bind` in pretty.rs** — Post-desugaring node; plan correctly avoids desugar. Note formatter must not emit `bind(...)` if given desugared AST (guard: only `parse_program` entry).

9. **Plan self-reference** — §14 lists creating `issue-46-plan.md` as work item; file already exists in worktree.

10. **Taskless** — No mention of running Taskless on new `quonfmt/src/**`; add to §11 checklist explicitly.

---

## Specific Required Plan Amendments

| # | Section | Amendment |
|---|---------|-----------|
| A | §4, §5D, §8 CI, README | Resolve MLIR/Z3 dependency lie: feature gate or crate split; update “No MLIR env required” |
| B | New §3.9 | Application/postfix canonical printing rules + `application.qn` corpus |
| C | §3.1, §4 `check_str`, §6 | Normalization-aware check (LF, trailing WS, final newline); CRLF test |
| D | §3.4–3.6 | Split statement blocks vs expression blocks (`par`, `for`) |
| E | §5B `Prec` | Prefix-form parenthesization table; `Return` in blocks |
| F | §3.5, §12 | Lock leading `\|>`; remove “pick one” |
| G | §3.6 floats | Lexer-constrained shortest round-trip algorithm + tests |
| H | §4 `Cargo.toml` | Add `clap`, `anyhow`, `ariadne`, `proptest`, `arbitrary`; `edition = "2024"` |
| I | §7, §8 | Fix proptest sketch; specify strip helper sharing strategy |
| J | §7 corpus | Add stdlib forms, comments, bare expr, adjacency, empty blocks |
| K | §5B / §3.4 | Doc + bind-alignment integration strategy (spike acceptance) |
| L | §6 | `-w`/`--check` conflict, multi-file exit codes, diff implementation |
| M | §11 | Explicit Taskless on `quonfmt/` |

---

## What the Plan Gets Right

- Correctly identifies `pretty.rs` as roundtrip-faithful debug printer, not canonical formatter (lines 1–7, 173–196 of `pretty.rs`).  
- Parser precedence table in §1 aligns with `parser.rs` tiers (gateapp → neg → pow → mul/par → add → backtick → compose → ascribe).  
- Semantic contract `parse(format(d)) = d` (mod spans) is the right oracle; AST stability via `strip_decls` matches existing test infrastructure.  
- Keeping `frontend::pretty` untouched preserves fuzz/proptest invariants — correct call.  
- Three-layer testing (golden, idempotency, AST stability) is appropriate rigor for a formatter.  
- Graphite PR decomposition is sensible; style spec before goldens is the right order.  
- Comment stripping as v1 non-goal is honest given lexer discards comments.  
- `run` bind alignment example matches `teleport.qn` / `bell_state.qn` fixture intent.

---

## Summary Verdict

**Do not start implementation until blockers A–E are written into the plan.** The architecture (separate crate, Doc-based printer, golden corpus) is sound, but the plan reads like “precedence + layout” while the hard problems are **App resugaring**, **block kind distinctions**, **normalization semantics for `--check`**, and **dependency isolation**. Without those, expect rework across PRs 3–5 and false CI/docs promises about build requirements.

---

*Review generated adversarially against worktree `46-quonfmt` @ plan in `docs/plans/issue-46-plan.md`.*
