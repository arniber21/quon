# Re-review: Idiomatic Rust code quality (2026-08-04)

**Prior review:** session `019fce3b-…` (2026-08-04, "Prepare audit issue cleanup"),
using the `rust-skills` skill. That audit scored the repo **3/10 for 5× scale
readiness**, filed **27 issues (#389–#415)**, and changed no files.

**This review:** re-assesses the same axes against the current `main`
(`13c60e9`), after all 27 issues were closed. Every prior finding was
re-derived from source this session; gate commands were re-run locally.

---

## Verdict

**Substantial, real improvement on every substantive axis — but the quality
baseline is red again.** The stop-ship correctness and safety findings that
made the prior audit say "do not scale 5×" are fixed at the source level and
verified. However, the most recent batch of fix-PRs (#455–#460) landed without
running the canonical `just test-ci` gate, so fmt, clippy, rustdoc, and the
backend tests all fail on a clean checkout. This is the *same failure mode*
the prior audit flagged in #389: fix-PRs landing while the baseline is
untrustworthy, so regressions are invisible.

### Current scores

| Area | Prior | Current | Assessment |
|---|---:|---:|---|
| Correctness | 4/10 | **8/10** | All five reproduced panics/fail-open paths are closed and verified |
| Rust safety | 4/10 | **8/10** | Workspace `unsafe_code = "deny"`; FFI centralized in `ffi.rs` with `SAFETY:` everywhere |
| Tests | 7/10 (red) | **7/10 (red)** | 3,767 pass in the unblocked crates; backend test crate does not compile |
| Architecture | 5/10 | **7/10** | Invariant-bearing fields sealed; canonical AST visitor; feature seams aligned |
| Flux | 2/10 | **6/10** | #404–#414 closed; load-bearing contracts added (not independently re-verified this session — needs nightly) |
| 5× scale readiness | 3/10 | **6/10** | Trust gaps closed; blocked mainly by the red baseline, not design |

---

## Gate status (observed on `main` @ `13c60e9`)

| Gate | Prior | Current | Evidence |
|---|---|---|---|
| `cargo fmt --all -- --check` | RED (14 files) | **RED (~30 files)** | Drift across backend, frontend, mlir_bridge, quonlint, quon_na, flux_verify from #455–#460 |
| `cargo clippy --workspace --exclude flux_verify --all-targets -- -D warnings` | RED (2 errors) | **RED (1 error)** | `quon_core/src/depth.rs:261` `needless_borrow` (`&b"_"[..]`) from #398 |
| `just test-fast` / nextest | RED (5 snapshots, 331 unrun) | **PARTIAL** | frontend/quon_core/quon_qec/zx: 3,767 pass, 0 fail. **backend tests do not compile** |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --exclude flux_verify --no-deps` | RED (37 warnings) | **RED** | `mlir_bridge` fails: public docs link private `PassContext`/`with_context`/`emit_error` (#459) |
| `npx @taskless/cli@latest check` | GREEN despite 556 warnings | **GREEN, 0 findings** | #390 converted warnings to a real failing gate |
| `cargo build -p mlir_bridge --tests` | RED (removed import) | **GREEN** | Fuzz targets import `DepthExpr` from `quon_core` (#410) |
| `just test-ci` (canonical) | RED | **RED** | Fails at fmt; would also fail clippy, rustdoc, backend tests |

**The backend test regression is a cross-PR integration defect:** `backend/tests/target.rs:522`
(the `all_to_all_1000_qubits_constructs_quickly` test from #408) accesses
`graph.dist` as a *field*, but #394 sealed `ConnectivityGraph` fields private.
Three `E0616` errors block the entire `backend` test crate. #408 and #394
both closed green in isolation but never agreed with each other.

---

## Stop-ship findings — re-verification

Every prior stop-ship finding was re-checked against current source.

### 1. Classical arithmetic panics (#409) — FIXED
`frontend/src/elaborate.rs`: `eval_classical` and `eval_binop` now use
`checked_add/sub/mul/div`, with dedicated `ElabError::{Overflow, DivByZero,
NegativeExponent}` variants. `eval_classical` is documented "Total for the
supported fragment". A `mod arithmetic_totality_tests` test module locks the
behavior. `eval_classical(1/0)` returns `Err(DivByZero)`, not a panic.

### 2. MLIR passes fail open (#391) — FIXED
`native_gate_decomp.rs` now defines `DecompError` and threads a
`&mut Diagnostics<'c>` accumulator through `decompose_block`/`decompose_module`;
`run_on_module` returns `Diagnostics`. The `eprintln!`-and-continue pattern is
gone. The pass functions no longer return `()` — failure is communicated.

### 3. Phase-polynomial 128-qubit panic (#392) — FIXED
`phase_polynomial.rs` represents parities as a dynamic `Parity` bitset
(`Vec<u64>`), explicitly "well beyond the 128-qubit limit of a fixed `u128`".
`extract(129, &[])` no longer shifts overflow.

### 4. QEC workload deserialization bypass (#396) — FIXED
`quon_qec/src/workload.rs`: `QecWorkload` now holds private `blocks`/`ops`
fields and deserializes via `#[serde(try_from = "QecWorkloadRaw")]`, which
replays the `WorkloadBuilder` ordering validation. `WorkloadBlock` has a
custom `Deserialize` that derives `code_family`. The post-measure memory round
the prior audit constructed is now rejected at the type boundary.

### 5. OpenQASM silent semantic loss (#405) — FIXED
`quonc/src/qasm.rs`: `measure`, `reset`, `gate`/`opaque` definitions, and
classical control flow are all rejected with line-tagged `QasmError::Unsupported`.
Multi-parameter gates (`u2`/`u3`, parameterized entanglers) are rejected with
`QasmError::TooManyParams` *before* graph construction. No operation is
silently dropped. Tests cover each rejection path.

### 6. "Exact" state prep heuristic substitution (#397) — FIXED
`quon_na/src/qec_schedule.rs`: `StatePrepMode::Exact` now invokes a real z3
`schedule_exact` solver (behind the `solver` feature); the report is labelled
`Exact` only when `SolverOutcome::Proven`, else `Heuristic`. **Fail-closed
without the feature:** a non-solver build returns the typed
`NaPipelineError::ExactStatePrepRequiresSolver` — never a silent fallback.

### 7. Unsafe boundary does not exist (#415) — FIXED
`Cargo.toml` now sets `[workspace.lints.rust] unsafe_code = "deny"` and
`undocumented_unsafe_blocks = "warn"`. `mlir_bridge/src/ffi.rs` is the single
`#![allow(unsafe_code)]` module, with a `// SAFETY:` comment on every block.
The raw `*mut` sinks were removed from `TypeChecker` (#393, closed). The pass
modules no longer call `mlir-sys` directly.

### 8. `test.qubit` unverified allocation (#401) — FIXED
`frontend/src/lower.rs::alloc_qubit` now builds a verified
`quantum.dynamic.alloc` op (dialect verifier runs inside the builder), with a
doc note that it replaces the unregistered `test.qubit`. A `BuildError` is
returned on failure.

---

## Architecture & maintainability — what improved

- **Invariant-bearing fields sealed (#394, #402).** `ConnectivityGraph`,
  `FixedTarget`, interaction graphs, and reports no longer expose mutable
  invariant-bearing fields publicly. (The backend test regression is the
  flip side: a consumer test wasn't migrated — see gates.)
- **Canonical AST visitor (#399).** `frontend/src/visitor.rs` provides one
  traversal interface, reducing the duplicated exhaustive walkers across
  analysis/formatter/linter/LSP.
- **Feature seams aligned (#407).** `quon_na`'s MLIR dependency is no longer
  default; `frontend` default no longer pulls the heavy MLIR/Z3 stack for
  tooling consumers.
- **Dead scaffolds removed (#395).** The orphaned `compaction/types.rs`,
  no-op ZX stubs, and crate-wide `#![allow(dead_code)]` are gone (one residual
  `is_captured` dead-code rustdoc warning remains in `mlir_bridge`).
- **All-to-all is O(N²) (#408).** Analytic distance matrix; no Floyd-Warshall
  for the fully-connected case. (Test compiles against a private field — see
  gates.)
- **DepthExpr canonical ordering (#398), elaboration borrowing (#400).**
  Both closed; the former introduced the one current clippy error.

---

## What is still red / regressions introduced by the fixes

These are the actionable items for the next baseline pass:

1. **fmt drift (~30 files).** `cargo fmt --all` fixes it; the fix-PRs simply
   weren't formatted.
2. **clippy: `quon_core/src/depth.rs:261`** — `(&b"_"[..]).cmp(b.as_bytes())`
   should be `b"_"[..].cmp(...)`. One-line fix.
3. **backend tests don't compile** — `backend/tests/target.rs:522-523`
   accesses private `graph.dist`; switch to the `graph.dist(i, j)` accessor
   (or a public len method). Cross-PR regression from #408 vs #394.
4. **rustdoc `-D warnings` fails on `mlir_bridge`** — the `ffi.rs` module
   doc links to private `PassContext`, `with_context`, `emit_error`. Either
   make those `pub(crate)`-documented or reword the links. This contradicts
   #406's "CI-enforced warning-free" claim; the gate exists in `ci-rust`
   (Justfile:152) but #459 broke it.

**Root cause is process, not code:** the 27 fix-PRs were correct individually
but the last several merged without a green `just test-ci`. The prior audit's
#389 existed for exactly this reason. Until a PR cannot merge without a green
`ci-rust` (fmt + clippy + rustdoc + build + tests), this will recur.

---

## Not independently verified this session

- **Flux (#404, #411–#414).** The prior audit found `quon_qec` Flux failing
  with 7 proof errors and `backend` Flux broken via the dependency. All four
  issues are closed and the commits are on `main` (#456–#458), but running
  `cargo flux` requires the nightly Flux toolchain and was not re-run here.
  The contracts are present in source (`flux_verify`, `quon_qec/family.rs`
  `#[flux]` attrs, `qldpc.rs`). Treat the Flux score as "closed, contracts
  present, not re-proven" rather than verified green.
- **Snapshot tests (quon_na).** The prior audit had 5 stale snapshots. They
  were not re-run this session because the backend test crate blocks first;
  the quon_na snapshot crate was not exercised in isolation.

---

## Recommended order of work

1. `cargo fmt --all` — restore formatting.
2. Fix the one clippy error in `quon_core/src/depth.rs:261`.
3. Fix `backend/tests/target.rs:522` to use the `dist` accessor.
4. Fix `mlir_bridge/src/ffi.rs` rustdoc private-item links.
5. Run `just test-ci` to green. Re-run `cargo flux` on the nightly toolchain
   to confirm #404–#414.
6. Add branch protection so `ci-rust` must be green to merge — this is the
   single highest-leverage change to prevent recurrence.
