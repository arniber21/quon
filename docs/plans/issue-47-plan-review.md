# Adversarial review: Issue #47 — quonlint implementation plan

**Plan:** `docs/plans/issue-47-plan.md`  
**Worktree inspected:** `/Users/arnabghosh/projects/quon-worktrees/47-quonlint` (2026-07-08)  
**Reviewer stance:** adversarial — assume the implementing agent will follow the plan literally.

---

## Overall grade: **C−**

## Pass / fail: **FAIL**

The plan is unusually thorough in structure (milestones, rule catalog, test matrix, repo cross-refs) and aligns with several real seams (`frontend::check_program`, `TypeChecker::fn_type_of`, Taskless orthogonality). It is **not safe to implement as written** because multiple rules contradict how this repo's reference programs are written, CI will fail on day one, and the plan contains internal contradictions an AFK agent will propagate into broken code.

---

## Critical blockers

### 1. Rule catalog conflicts with the 50-program corpus and reference fixtures

The plan proposes shipping root `quonlint.toml` over `frontend/tests/fixtures/**/*.qn` and enabling CI at `--fail-on warn`, but defers inline suppressions to **v1.1** (§14.3).

Spot-check against existing `.qn` files shows widespread false positives:

| Rule | Default | Corpus impact |
|------|---------|---------------|
| `depth/sequential-for-blowup` | **warn** | ~30/50 corpus files use `for … in range(…)` inside `circuit` blocks (Trotter, layers, QPE ladders). Examples: `ising.qn`, `tfim_trotter.qn`, `recursive_qft.qn`. |
| `depth/quadratic-recursion-gap` | **warn** | **Targets the wrong thing.** `recursive_qft.qn`, `shor.qn`, `qpe.qn`, etc. intentionally use `2*n*n` as a **proven upper bound** — the typechecker's refinement tests (`frontend/src/refinement.rs`, `qft_step_depth_bound_is_provable_under_n_ge_1`) treat this as correct, not suspicious. |
| `depth/repeat-non-literal-count` | info | `ising_evolve` uses `fold(range(n_steps), …)` with symbolic `n_steps` — intentional. |
| `gates/universal-in-clifford-block` | warn | Many corpus programs mix Clifford/universal subcircuits by design. |

§10.3 says corpus files should have **zero errors** but allows info-level hits — yet CI uses `--fail-on warn`. §352 acknowledges tuning is needed but provides no concrete strategy before M8.

**Verdict:** M8 (CI) cannot land without either (a) v1 inline suppressions, (b) a corpus allowlist file, or (c) demoting/noise-gating rules that fire on idiomatic Quon. As written, CI is guaranteed red.

### 2. Internal contradictions an agent will copy into broken CLI/CI

| Location | Says | Conflicts with |
|----------|------|----------------|
| §6 exit codes | CI default: `--fail-on error` | §9 workflow: `--fail-on warn` |
| §10.4 CLI tests | `--check` exit code 0/1 | §6 has no `--check` flag (uses `--fail-on`) |
| §1 design principle | "syntax-only degraded mode" on parse/desugar failure | §4 mermaid: typecheck failure → **empty lints** (no degraded mode) |
| §5 + §222 | "8+ enabled by default, 4 info optional" | 12 rules listed; math works but "enabled by default" vs severity filtering is undefined |

These are not cosmetic — integration tests and CI will be written against the wrong flags.

### 3. CI workflow snippet is incomplete vs existing `ci.yml`

Plan §9 LLVM install says "same as ci.yml" but **omits `libz3-dev`**. The frontend typechecker depends on Z3 (`frontend/Cargo.toml`). `quonlint` calling `check_program` / future `analyze_program` **will fail at link or runtime** without z3 on the runner.

Also: a standalone `quonlint.yml` duplicates a ~2-minute LLVM 22 install already in `ci.yml`. Acceptable, but the plan should specify either a reusable composite action or an extra job in `ci.yml` to avoid drift (ci.yml will evolve; quonlint.yml copy-paste will rot).

### 4. LSP acceptance criterion blocked on non-existent #43

Worktree has **no `quon_lsp` crate** (glob returns zero files). Plan header says "blocked by #43, #46" but §13 maps "Lint output consumable by LSP" to §8, and M7 depends on #43 landing.

M1–M6 "satisfy acceptance criteria without #43" is **partially false** — LSP consumption is an explicit criterion. Plan needs either:
- scope split (CLI-only issue vs LSP follow-up), or
- `to_lsp()` in `quonlint` + contract tests without `quon_lsp` (JSON round-trip / range mapping unit tests).

### 5. `TypedProgram` is soft-required but milestone treats it as optional

§4.1: "optional v1: compute on demand in rules" vs rules that **require** inferred types:

- `gates/universal-in-clifford-block` — needs inferred `CliffordClass`
- `monad/circuit-bind-without-apply` — needs bind RHS synthesis type
- `depth/quadratic-recursion-gap` — needs recursive call-site synthesized depth vs annotation
- `depth/sequential-for-blowup` — needs body depth in circuit context

`TypeChecker::fn_type_of` already exists (`frontend/src/typecheck/mod.rs:307`) but per-expression synthesis is **not exported**. Rules that re-walk AST and re-derive depth will **drift from inference** — exactly what §4.1 warns against.

M2 must be a **hard blocker** for M3–M5, with a concrete API, not "recommended."

### 6. IR analysis default-on vs performance budget is inconsistent

- `ir-analysis` feature "default on" (§3, §4.2) pulls `mlir_bridge` + `melior` into every lint.
- `lower_program` **re-runs** desugar + typecheck internally (`frontend/src/lower.rs:1165`) — triple work if lint already typechecked.
- §8 budget: AST < 50 ms; IR debounced/off in LSP — but default feature set includes IR rules unless config disables them.

`rayon` parallel project lint (§3) with `melior::Context` / `Module` is **likely not `Send`** — plan does not address per-thread Context or sequential IR lowering.

---

## Major concerns

### Rule semantics

1. **`depth/quadratic-recursion-gap`** — In this codebase, loose annotations are **features** (#58, #60), not smells. The rule as specified punishes `recursive_qft.qn`-style programs the repo treats as gold standard. Needs redesign: flag only when synthesis **exceeds** annotation (unsound), or when Z3/refinement would fail, not when annotation is a valid upper bound.

2. **`depth/sequential-for-blowup`** — Sequential `for` is idiomatic for Trotterization (`ising.qn` line 5–7, `tfim_trotter.qn`). Should default to **info**, require `par`-commutativity analysis for warn, or exclude loops whose body depth is O(1) with documented scaling (still multiplies, but expected).

3. **`monad/circuit-bind-without-apply`** — `Circuit` is a linear resource (`Ty::is_linear_resource`). Many misuses may already be `LinearUnconsumed` type errors. Rule needs a precise delta vs typechecker: only fire when binding is **legal but footgun** (e.g., circuit bound and "used" only in dead paths). Otherwise duplicate diagnostics.

4. **`ancilla/unmeasured-ancilla-output`** — Overlaps `TypeError::BorrowEscape` (`find_borrow_escape`). Plan must specify exact cases typechecker misses; otherwise false positives on legitimate tuple returns.

5. **`gates/non-native-density`** — Requires `native_gate_decomp` + target JSON. Parametric/run functions may fail lowering (`LowerError::ParametricRunFn`, #27). Plan does not say whether IR rules skip, warn, or error on partial lowering.

### Pipeline architecture

- Plan should specify **single pass**: `desugar → typecheck (retain TypedProgram) → optional lower`, not separate `check_program` + `lower_program` each re-checking.
- Parse/desugar failure handling: §4 mermaid returns "parse diagnostics only" — but `LintDiagnostic` is rule-based, not frontend `Diagnostic`. Conversion layer unspecified.

### Configuration

- TOML schema needs `#[serde(deny_unknown_fields)]` on all config DTOs — repo Taskless rule `serde-deny-unknown-fields-on-dto` will flag violations.
- Nested table `[rules."gates/non-native-density"]` vs flat `"depth/..." = "warn"` — precedence and merge semantics for project vs file-level discovery need a test case in §10.

### Testing gaps

- No tests for: config precedence, parse-error path, type-error → empty lints, `--deep` without target file, parallel project lint determinism.
- §10.3 "zero errors on reference-quality files" will fail — needs explicit expected-warning baselines per fixture **before** CI.
- No Taskless rule or note that `quonlint` is a new library crate subject to `no-unwrap-expect-in-src`, `no-anyhow-in-lib-src`.

### Milestone ordering

- M6 (`quonlint.toml` + project mode) should follow M3–M5 so thresholds reflect real rule behavior.
- M8 before corpus baseline exists = broken main.
- Consider merging M6+M8 only after §10.3 baseline snapshot is green.

### Dependency / workspace

- Plan adds `glob`, `rayon`, `toml` — fine, but should add to `[workspace.dependencies]` per repo convention.
- `quonlint-cli` as separate crate matches `quonc` pattern — good.
- `#46 quonfmt` listed as blocker but non-goal; clarify **no runtime dependency** (currently implied, not stated).

---

## Minor nits

- §82 rationale "fast LSP linking" — splitting CLI doesn't avoid LSP linking to `quonlint` lib; only avoids linking clap.
- `--project` with no path: default cwd vs git root unspecified.
- CI `git diff … $FILES` breaks on paths with spaces; use `xargs -0` or `while read`.
- `diagnostics_to_lsp` return type `Vec<LspDiagnostic>` in public API couples `quonlint` to LSP types — prefer a neutral `LineColDiagnostic` DTO.
- §14 disable syntax `-- quonlint-disable-next-line` vs `# quonlint: disable=` — pick one in v1 if CI depends on it.
- Branch names use `47-quonlint-*` but worktree is `issue-47-quonlint` — minor inconsistency.
- `validation.md` update listed in M8 but pre-PR checklist (§12) already adds quonlint — should update `code-quality.md` §Pre-PR checklist in same milestone.

---

## Specific required plan amendments

1. **Add § "Corpus baseline strategy"** — Before M8: run all rules on 50 corpus + SPEC §12 fixtures; produce `quonlint-baseline.toml` or per-file suppressions; define which rules are off-by-default in repo config. Do not enable CI until baseline is green.

2. **Rewrite `depth/quadratic-recursion-gap`** — Invert or replace: detect **unsound** annotations (synthesis > annotation under assumptions), not loose upper bounds. Cite `refinement.rs` QFT proof as a **must-not-fire** regression test.

3. **Retune `depth/sequential-for-blowup`** — Default **info**; warn only when iteration count is non-literal **and** body depth > 1, or provide `commutes`/`par`-eligible heuristic. Add negative test using `ising.qn`.

4. **Resolve CLI flag contradictions** — Single source of truth: `--fail-on <severity>` (remove `--check` from §10.4 or add alias). Document CI uses `--fail-on warn` explicitly in §6.

5. **Fix §4 degraded-mode spec** — Choose one: (A) type errors → no lints, or (B) syntax-only rules on parse failure. Update mermaid + §1 to match.

6. **Harden M2** — Required deliverable:
   ```rust
   pub fn analyze_program(src: &str) -> Result<TypedProgram, AnalyzeError>;
   ```
   Minimum exports: `decls`, `fn_types: HashMap<Name, Ty>`, and `expr_types: HashMap<SimpleSpan, Ty>` (or typed AST wrapper). M3–M5 blocked until merged.

7. **Fix CI workflow** — Add `libz3-dev`; pin exact install block from `ci.yml`; consider `cargo test -p quonlint` in that job; use null-delimited file lists.

8. **IR feature default off** — `ir-analysis` default **false**; `--deep` and LSP `quonlint.deepAnalysis` opt-in. Document shared `LintContext` without `Module` in default build.

9. **Single-pass pipeline** — Add sequence diagram: one `TypeChecker` instance, optional `LoweringCtx` reuse; no double `check_program`.

10. **LSP scope split** — §13: mark LSP criterion as M7/#43; add M6.5 `quonlint` unit tests for byte-span → line/col mapping (no `quon_lsp`).

11. **Config DTO conventions** — Explicit: all config structs use `thiserror` errors + `#[serde(deny_unknown_fields)]`.

12. **Add Taskless to §12 validation** — Already present; add note that new `quonlint/src/**` is subject to existing rules.

13. **Ship inline suppressions in v1** — If CI is an acceptance criterion, `# quonlint-disable` (file or next-line) cannot be v1.1. Minimum viable suppression before M8.

---

## Grading rubric (summary)

| Axis | Grade | Notes |
|------|-------|-------|
| Correctness | D+ | Rule semantics fight reference programs; pipeline double-work; contradictions |
| Completeness | B− | Good surface area; missing corpus strategy, suppression, parametric lowering edge cases |
| Feasibility | C | M2/API underestimated; MLIR+rayon+Send issues; #43 absent |
| Test rigor | B− | Good per-rule pattern; missing baseline, config, degraded-mode, LSP-without-LSP tests |
| Missed risks | D | CI red on merge; QFT false positives; z3 omission; typechecker overlap |
| Repo conventions | B+ | thiserror/anyhow split, Graphite milestones, Taskless orthogonality, ariadne — mostly aligned |

---

## Recommendation

**Do not start M3–M8 until amendments 1–6 and 13 are integrated into the plan.** M1 scaffold is fine to prototype. Treat LSP (M7) as a separate track until #43 exists. Treat CI (M8) as last and contingent on a green corpus baseline.
