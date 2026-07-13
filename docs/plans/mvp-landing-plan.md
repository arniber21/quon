# MVP landing plan — full 6-phase pipeline, all 8 reference algorithms verified

**Audience**: a Claude Fable session executing this plan in `~/projects/quon`.
**Objective**: close out PRD #1's MVP — `quonc --emit-qasm` compiles all 8 reference
algorithms through the full pass pipeline and Qiskit Aer verification passes for
each (#29 done, #30 is the gate).

Read first: `CLAUDE.md`, `docs/agents/code-quality.md`, `docs/agents/graphite.md`,
`SPEC.md` §7 (pipeline), §9 (emission), §12 (reference algorithms), and the PRD
(`gh issue view 1` — it is an issue, not a file).

---

## 1. Verified current state (inspected 2026-07-05, main @ bbf038b)

Do not re-derive this; spot-check only what you touch.

**Done and landed** (issue checkboxes are stale — the code exists):
- Frontend: lexer/parser/desugar/typecheck complete, including linear contexts,
  Z3 symbolic depth, Clifford inference, value-dependent types (#57–#60). All 8
  reference fixtures in `frontend/tests/fixtures/` **type-check**.
- Dialects `quantum.circ` / `quantum.dynamic` + roundtrips; linearity verifiers
  (`mlir_bridge/src/passes/linearity_verifier.rs`, `dynamic_linearity_verifier.rs`).
- Nine optimization/physical passes exist with lit coverage: gate_cancellation,
  rotation_merging, compiler_uncomputation, zx_simplification,
  measurement_deferral (#22), classical_region_fusion (#23), native_gate_decomp
  (#24), sabre_routing (#25), depth_scheduling (#26). `clifford_t_opt` does **not**
  ship and is **not** in the circ fixpoint — name reserved for #96 (#214 removed
  a prior shallow alias that only re-ran gate cancellation).
- Emitter (#27): `mlir_bridge/src/emit/openqasm3.rs` reify (fallible) →
  `quon_core::qasm` render (total). Reads `phys_qubit` attrs if present.
- Clifford e2e (#29): Bell / teleport / Bernstein–Vazirani verified on Aer in CI
  (`.github/workflows/ci.yml` runs the three `test/verify/*.py`).

**The two real gaps** (each verified by direct experiment):
1. **`frontend/src/lower.rs` cannot lower parametric circuit functions.** All five
   universal fixtures fail with `ParametricCircuitFn` (`lower.rs:174`); also
   rejected: parametric run fns (`:277`), `adjoint` inside `@` (`:363`),
   non-static rotation angles (`:638`), and assorted `Unsupported` constructs.
   Grover/QFT/Shor/QAOA/Ising are blocked **before** MLIR exists.
2. **`quonc/src/main.rs` wires zero passes.** Pipeline is parse → lower →
   monadic_lowering → emit. No optimization passes, no physical lowering, and
   `--target` only affects the emitter's native-gate name check.

Plus two small ones: lit tests aren't run in CI (#28 residue, PRD story 38), and
issues #22–#27/#29 need their checkboxes ticked and closing.

---

## 2. Recommended execution order

Milestones are Graphite-stackable; each is one PR (or small stack) with trunk
green throughout. **M1 before M2**: wiring the pipeline on the already-working
Clifford programs is small, gives an e2e regression harness, and surfaces
pass-composition bugs before the elaborator multiplies IR volume.

| # | Milestone | Blocks |
|---|-----------|--------|
| M0 | Preflight + issue hygiene | — |
| M1 | Assemble pass pipeline in `quonc` | M3 |
| M2 | Parametric elaboration in lowering (the core) | M3 |
| M3 | Universal algorithm verification (#30) | MVP |
| M4 | lit in CI + `cargo test` integration (#28) | — (parallel) |
| M5 | MVP close-out audit | — |

---

## 3. M0 — Preflight

- Build env (macOS, this machine): `MLIR_SYS_220_PREFIX` and `LLVM_SYS_220_PREFIX`
  = `/opt/homebrew/opt/llvm@22`, that LLVM's `bin` on PATH,
  `BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/opt/z3/include"`,
  `LIBRARY_PATH=/opt/homebrew/opt/z3/lib`. If tests SIGABRT with
  `dyld: Library not loaded: …libz3…` after a brew upgrade, `cargo clean -p z3-sys`.
- Confirm `cargo test --workspace --exclude flux_verify` and
  `./test/verify/run_e2e.sh` are green before changing anything.
- Tick the completed checkboxes on #22–#27 and #29 (verify each claim against the
  code first) and close them, cross-referencing the landing commits. This makes
  the issue tracker truthful before new work stacks on it.

## 4. M1 — Assemble the pass pipeline in `quonc`

Wire `compile_to_qasm` (`quonc/src/main.rs:59`) to the SPEC §7.1 order already
documented in `mlir_bridge/src/passes/mod.rs`:

1. `quantum.circ` passes, iterated to fixpoint: gate_cancellation,
   rotation_merging, compiler_uncomputation, zx_simplification.
   (Do **not** wire `clifford_t_opt` here — #214; reserve that name for #96.)
   Fixpoint = repeat until a pass round reports no change (add a cheap
   changed-flag return or compare op counts; cap iterations, e.g. 10).
2. monadic_lowering (already wired).
3. `quantum.dynamic` passes: measurement_deferral, classical_region_fusion.
4. **Physical lowering**: a new small pass/step annotating ops with `phys_qubit`,
   `native_gate`, `fidelity` from the `BackendTarget`. Check what
   native_gate_decomp/sabre_routing/depth_scheduling already assume about these
   attrs (their lit tests in `test/lit/physical/` show the expected input shape)
   and make the driver produce exactly that.
5. Physical passes in strict order: native_gate_decomp → sabre_routing →
   depth_scheduling.

Driver details:
- `--dump-ir` prints after every pass (it already prints two stages; extend).
- `--verify-linear` runs `linearity_verifier` after each circ pass and
  `dynamic_linearity_verifier` after each dynamic pass. Run it in every lit/e2e
  test. Precedent: the `inline_func_as_unitary_body` SSA-threading bug was
  exactly the class of error these verifiers catch mechanically.
- `--target` now genuinely drives decomp (native set), routing (topology), and
  scheduling (T1/gate-time). `generic_openqasm` (all-to-all, all-native) must
  remain a near-no-op so existing emit tests stay byte-stable — if any emit
  FileCheck changes under the generic target, treat it as a pass bug, not a test
  to update.

Acceptance: the three Clifford e2e verifications still pass with the full
pipeline on; add one lit test compiling a `.qn` with a deliberately redundant
`H|>H` and FileCheck-ing it is gone from the emitted QASM (proves passes run in
the driver, not only in isolation); one e2e run with a non-trivial JSON target
(linear-chain topology) showing SWAPs inserted and only native gates emitted.

## 5. M2 — Parametric elaboration (the core of the MVP)

### Design decision (make deliberately; recommended: A)

**A. Elaborate-by-partial-evaluation (recommended).** The entry point is the
concrete `main : Q<τ>` (established convention). Lowering becomes a staged
interpreter: evaluate the *classical* fragment (Nat/Int/Float/List/Matrix values,
`let`, lambdas, `fold`, `match`, recursion, builtins) at compile time starting
from `main`'s concrete arguments, and unroll the *circuit* fragment into a flat,
first-order, fully-static gate sequence — which is exactly what today's
`lower.rs` already knows how to emit. Verification fixtures instantiate
concretely (n=3, fixed angles), so this is sufficient for the MVP and matches
how every quantum toolchain in practice stages parametric circuits.

**B. Parametric IR** (symbolic `scf.for`-style loops and angle SSA values in
`quantum.circ`). Strictly more general, much more invasive (every pass and the
emitter must learn symbolic structure), and unnecessary for #30. Do not do this
for the MVP; leave a note in SPEC that it is the post-MVP path for
hardware-parameterized circuits.

**Explicit scope boundary** (document in SPEC §9, do not leave as an accidental
gap): programs whose `main` has parameters, or whose classical inputs are not
compile-time constants, type-check but are rejected by `--emit-qasm` with a
dedicated diagnostic ("entry point must be fully instantiated for code
generation"). Everything else in the SPEC §2–§5 expression language must
elaborate — cover the full spec surface, not just the five fixtures
(standing project rule; prior minimal-scope plans were rejected).

### Language surface the elaborator must cover

From the fixtures (superset from SPEC §12 + corpus):
- Parametric circuit fns over `Nat`/`Int`/`Float`/`List`/`Matrix` args;
  call-site specialization including recursion (`qft(n-1)` under `match n`).
- Iteration forms: `for q in qubits(n)`, `for i in range(e)`,
  `for (i,j) in pairs(n)`, `diag(n)`; `repeat(k, c)` with computed `k`;
  `fold` over lists with a circuit accumulator; lambdas.
- Combinators: `adjoint(c)`, `controlled(c)`, `identity(n)`, `swap_reverse(n)`,
  `` c `on_high` n ``, `` a `tensored` b ``, `split(n, r)`.
- Classical builtins: `PI`, `round`, `sqrt`, `^`, `float`, arithmetic,
  list/matrix indexing, `take`; monadic `map_q` in run blocks.
- Angles: `Rz(gamma * q[i][j])` etc. — arbitrary classical expressions that are
  static *after* specialization.

### Elaboration semantics and soundness obligations

State these in the PR description and encode each as a test; this project
requires the argument, not just the steps:

1. **Termination.** The classical fragment is total for well-typed programs:
   recursion already passed the decreasing-`Nat`-measure check (#60), `fold`
   is over finite lists, loops over static ranges. Still add a fuel counter
   (e.g. 10⁶ steps) with a clean diagnostic naming the offending call chain —
   a defensive bound, not a semantic one.
2. **Linearity preservation.** Elaboration is a homomorphism over `|>`, `par`,
   `repeat`, `tensored`: each emitted gate consumes and produces its wires
   exactly once, so linearity of the composite follows by induction on circuit
   structure from linearity of each unrolled primitive. Mechanical check: run
   `--verify-linear` on the elaborated module in every test.
3. **Depth-bound soundness.** The typechecker proved (via Z3) `depth(c) ≤ D(args)`.
   After elaboration at concrete args, compute the realized critical-path depth
   and assert `realized ≤ eval(D, args)` (debug assertion + explicit test per
   reference algorithm). This converts the type-level promise into a checked
   invariant end-to-end. If it ever fails, the bug is in elaboration or in a
   depth rule — do not weaken the assertion.
4. **Adjoint correctness.** `elab(adjoint c) = reverse(map dagger (elab c))`,
   with gate-level daggers: H/X/Y/Z/CNOT/SWAP self-inverse, S↔Sdg, T↔Tdg,
   `R_(θ)† = R_(−θ)`, `controlled(c)† = controlled(c†)`. Property-test on
   random small circuits by comparing unitaries — `backend/src/unitary.rs`
   already has `mul2/mul4/tensor/unitary_distance`; reuse it.
5. **Controlled semantics.** `controlled(Rz(θ))` → a CRz (or its CX·Rz·CX
   decomposition); `controlled` of a multi-gate circuit → controlled version of
   each gate (correct because control distributes over composition of unitaries
   on the same register). Verify by unitary comparison for 1–2 qubit bodies.

### Implementation shape

- New module `frontend/src/elaborate.rs` (or grow `lower.rs`): an environment-
  passing evaluator producing a first-order "flat circuit" (gate list + qubit
  index map + measurement/feed-forward structure), which the existing lowering
  then emits as `quantum.circ`/staging ops. Keep the evaluator MLIR-free so it
  unit-tests without melior.
- `on_high`/`tensored`/`split` are pure index arithmetic on the flat form.
- Extend run-block lowering (`lower_run_fn`) for `map_q` and for applying
  elaborated (rather than named) circuits; the `if…then…else @` feed-forward
  path already exists.
- **Gate-set gap to close**: `quon_core::qasm` has no Rzz/CRz variant
  (`TwoQubitGate` = Cx/Cy/Cz/Swap; rotations are single-qubit only). QAOA/Ising
  need `Rzz`, Shor needs controlled-Rz. Options: extend the QASM tree with
  `crz`/`rzz` (note: `crz` is in stdgates, `rzz` is **not** — Qiskit's importer
  needs a `gate` def or a CX·Rz·CX decomposition). Recommended: elaborate
  `Rzz(θ)` → CX·Rz(θ)·CX and `controlled(Rz(θ))` → CRz with a new tree variant;
  add Flux specs for any new `quon_core` kernels (see `docs/agents/validation.md`
  and the established cfg_attr pattern; beware Flux's unbounded-int gotchas).
- Diagnostics: every remaining `Unsupported` must name the construct and span;
  no silent fallthrough.

Acceptance: all 8 fixtures + the `frontend/tests/fixtures/corpus/` programs that
have concrete entry points compile under `--emit-qasm` with the generic target;
depth assertions (obligation 3) pass for all 8; adjoint/controlled property
tests green; lit tests per construct in `test/lit/circ/` and `test/lit/emit/`.

## 6. M3 — Universal verification (#30)

Write `test/verify/{grover,qft,shor,qaoa,ising}.qn` (concrete instantiations of
the fixtures — n=3 Grover with a fixed marked item's oracle, n=4 QFT, small Shor
kernel, n=3 QAOA with a fixed cost matrix, Ising at t=0 and one t>0 point) plus
matching `.py` Aer checkers, following the #29 conventions: seeded sampler,
statistical tolerances, runnable standalone.

Acceptance criteria from #30, with one correction to make explicitly:
- Grover n=3: marked item probability > 0.9 over 4096 shots.
- **QFT**: "matches theoretical DFT" is not observable from measurement
  histograms (phases are invisible). Verify behaviorally instead:
  (a) round-trip `qft(n) |> adjoint(qft(n))` returns the input basis state with
  P≈1 (also exercises adjoint elaboration on the headline recursive circuit),
  and (b) a phase-estimation-style peak test. State this deviation in the issue.
- Shor kernel: period peaks at correct fractional positions.
- QAOA n=3: minimum-energy bitstring is the mode of the distribution.
- Ising t=0: all-zeros with P≈1.
Add all five to the verify step in `.github/workflows/ci.yml`.

Environment gotchas (already learned the hard way): use the venv with
`qiskit_qasm3_import` installed; single-bit conditions must render `c[0] == true`
(the importer rejects `== 1`) — already the emitter's behavior, don't regress it.

## 7. M4 — lit in CI (#28 residue; PRD story 38)

- CI job: install `lit` via pip, put `/opt/homebrew/opt/llvm@22/bin` (CI: the
  workflow's LLVM) on PATH for FileCheck, build the oracle binaries
  (`circ_roundtrip`, `monadic_lower`, per-pass oracles — see
  `test/lit/lit.cfg.py` substitutions) and `quonc`, run `lit test/lit/`.
- Story 38 wants `cargo test` to run lit too: add one ignored-by-default-in-CI?
  No — add a plain `#[test]` in a workspace test crate that shells out to `lit`
  and is `#[ignore]`d when `lit`/FileCheck are absent (skip with a message).
  If that proves brittle, document the deviation in README ("`cargo test` +
  `lit test/lit/`") and note it on #1.

## 8. M5 — Close-out audit

- Walk PRD #1's 38 user stories; for each, either point at the test that
  demonstrates it or file a follow-up issue. Known intentional deviations to
  record: Clifford+T pass absent (misleading `clifford_t_opt` alias removed in
  #214; real phase-polynomial / tableau work is #96), ZX extraction limits
  (#75 already tracks), parametric IR (post-MVP note from M2).
- Close #22–#30 (with #28 pointing at the CI job) and update `CONTEXT.md` /
  `README.md` for the now-real pipeline and `--target` semantics.
- Post-MVP queue, in rough value order: real Clifford+T optimization
  (phase polynomials + Aaronson–Gottesman, #96), #75 ZX multi-qubit extraction,
  #82 IBM hardware targeting, tooling track #43–#49.

## 9. Workflow rules

- Graphite (`gt`) stack per milestone, trunk `main`; small reviewable PRs
  (pipeline wiring / elaborator core / combinators / verification can be
  separate stacked PRs). Follow `docs/agents/code-quality.md` pre-PR checklist
  and taskless/Flux validation (`docs/agents/validation.md`).
- Never scope a component to "just the 8 algorithms" when the SPEC defines a
  larger surface — enumerate from SPEC + PRD first (standing project rule).
- If a genuine semantic question surfaces (e.g. depth exact-vs-bound corner, a
  SPEC contradiction), stop and ask rather than paper over it — but bring the
  worked alternatives and a recommendation.
