# M5 close-out audit — PRD #1 (38 user stories)

**Date**: 2026-07-13  
**Auditor**: agent session (in-depth M5, beyond the original one-pass checklist)  
**Main tip at audit**: `53770fd` (`fix(zed): load Quon language…`)  
**PRD**: [#1](https://github.com/arniber21/quon/issues/1)  
**Landing plan**: [`docs/plans/mvp-landing-plan.md`](./mvp-landing-plan.md) §8

## Executive verdict

| Question | Answer |
|----------|--------|
| Is the compiler MVP landed? | **Yes.** Phases 1–6 deliverables exist; `quonc` wires the full fixed-target pipeline; all **8** PRD Aer reference algorithms run in CI. |
| Can #1 close? | **Yes**, with documented intentional deviations and new follow-ups filed from this audit. |
| Blocking MVP gaps? | **None.** Remaining items are post-MVP depth, docs/ADR alignment, or partial cost-model wiring. |

**Story rollup**: 31 DONE · 1 PARTIAL · 5 INTENTIONAL_DEVIATION · 1 DOCUMENTED_EVOLUTION · 0 blocking GAP

---

## Method

For each of the 38 PRD stories we required:

1. **Implementation pointer** (crate/file/symbol)
2. **Primary evidence** (negative test preferred for “reject …” stories; lit/unit/verify/CI otherwise)
3. **Status**: `DONE` | `PARTIAL` | `GAP` | `INTENTIONAL_DEVIATION` | `DOCUMENTED_EVOLUTION`
4. **Tracker** if not DONE

Cross-checks: `quonc/src/compile.rs` pipeline, `.github/workflows/ci.yml`, README, SPEC §7/§9/§10.3/§12, ADR-0002/0003, open issues #75/#96/#97, and fresh follow-ups #180–#182.

Issues **#22–#30** were already closed (2026-07-05); this audit does not re-close them.

---

## Phase map (SPEC §10.3 ↔ reality)

| Phase | SPEC deliverable | Reality on main |
|-------|------------------|-----------------|
| 1 Dialect foundation | `quantum.circ` / `quantum.dynamic` + verifiers | Done; plus `quantum.na` (#102) beyond PRD |
| 2 Frontend | Linear TC, depth, Clifford, Z3 | Done (#5–#15, #57–#60) |
| 3 Lowering | AST→circ, monadic lowering | Done + `elaborate.rs` partial evaluation |
| 4 circ passes | Cancel, merge, ZX, Clifford+T, uncompute | Done with **thin** ZX / Clifford+T (see stories 24–25) |
| 5 Physical | BackendTarget, SABRE, decomp, schedule | Done; SABRE β/lookahead **unwired** (story 29 / #181) |
| 6 Emit + verify | OpenQASM 3, Aer on 8 algs | Done; lit+verify in CI |

---

## Stories 1–19 — source language & type system

| # | Story (abbrev) | Status | Evidence | Notes / tracker |
|---|----------------|--------|----------|-----------------|
| 1 | `circuit{}` `\|\|>` `par` | DONE | `typecheck/tests.rs` compose/par; `test/verify/bell.qn` | Binary `f par g` is not surface syntax; `par {c}*k` is. |
| 2 | Reject clone | DONE | Neg: `linearity.rs`, `using_a_qubit_twice_*` | |
| 3 | Reject drop | DONE | Neg: `dropping_a_qubit_*` | Via linear no-weakening, not a special “must measure” form. |
| 4 | `QReg<n>` bounds | DONE | `gate_index_out_of_bounds_*`; destructure arity | No `qreg[i]` API by design. |
| 5 | Depth infer `\|\|>`/`par` | DONE | `compose_chains_*`, `bell_gate_with_wrong_depth_is_rejected` | |
| 6 | Runtime `Int` in depth | DONE | `symbolic_fold_depth_*`; `test/verify/ising.qn` | |
| 7 | Z3 depth | DONE | `refinement.rs` + `symbolic_depth_exceeding_*` | |
| 8 | Clifford infer | DONE | `clifford_single_qubit_gates_*` | |
| 9 | Reject bad Clifford ann. | DONE | `a_t_gate_annotated_clifford_is_rejected` | Clifford⊑Universal (#58) intentional. |
| 10 | `run { <- }` | DONE | `bind_measure_then_return_*`; teleport verify | |
| 11 | Desugar before TC | DONE | `desugar.rs` tests; span preservation | |
| 12 | `borrow` | DONE | `borrow_*_terminator_*`; `error_correction.qn` | |
| 13 | Structural reset/discard terminal | **INTENTIONAL_DEVIATION** | Impl: consume+no-escape; `syndrome_measure_*` uses `measure` | ADR-0003 / CONTEXT / PRD text stale. **#180**. SPEC §4 `borrow_end` \|0⟩ claim also drifts. |
| 14 | `adjoint` | DONE | type + `test/verify/qft.qn` round-trip | |
| 15 | `controlled` | DONE (types) / see #182 | `synth_controlled`; elaborate X/Z/Rz only | Codegen generality → **#182** (post-MVP). |
| 16 | `repeat(k,c)` | DONE | `repeat_multiplies_*` + wrong-depth neg | |
| 17 | `destructure`/`split` only | DONE | `destructure_of_a_non_register_*` | Mild: no dedicated `split` neg unit test. |
| 18 | `for` / `par*k` | DONE | parallel vs sequential for depth tests | Mild: no wrong-depth-on-`for` neg. |
| 19 | Span-accurate errors | DONE | `linearity.rs` caret tests; LSP diagnostics | |

### Story 13 deep dive (new finding)

Closed [#15](https://github.com/arniber21/quon/issues/15) acceptance criteria were **internally inconsistent**: they demanded structural `reset`/`discard` *and* that SPEC §12 `syndrome_measure` type-check — but that fixture measures ancillas. Shipped code correctly prioritizes the QEC fixture:

```text
synth_borrow: introduce ancillas → forbid escape in Return → ensure_consumed
BorrowEscape message: "must be measured, reset, or discard'ed"
```

ADR-0003 and `CONTEXT.md` still describe the stricter structural rule. Operational SPEC (§4 Borrow block) still asserts `borrow_end` requires \|0⟩, which `measure` does not establish. **Resolution tracked in #180** (amend ADR vs restore structural check).

---

## Stories 20–38 — IR, passes, emit, workspace

| # | Story (abbrev) | Status | Evidence | Notes / tracker |
|---|----------------|--------|----------|-----------------|
| 20 | circ attrs in/out/depth/clifford | DONE | `quantum_circ.rs`; `frontend/tests/lower.rs`; lit roundtrip | ADR-0002 |
| 21 | Linearity verifier | DONE | `linearity_verifier.rs`; `mlir_bridge/tests/linearity.rs` | `--verify-linear` |
| 22 | Gate cancellation | DONE | lit `gate_cancellation_hh.mlir`; unit+prop | #22 closed |
| 23 | Rotation merging | DONE | lit `rotation_merging_rz.mlir`; unit+prop | |
| 24 | ZX rewrite | **INTENTIONAL_DEVIATION** | `zx_simplification.rs` single-wire only | **#75** |
| 25 | Clifford+T phase poly / tableaux | **INTENTIONAL_DEVIATION** | `clifford_t_opt.rs` → `gate_cancellation` only | **#96** |
| 26 | Compiler uncomputation | DONE | `compiler_uncomputation.rs`; unit+prop | |
| 27 | Measurement deferral | DONE | lit `measurement_deferral_single_if.mlir` | #22 closed |
| 28 | Classical region fusion | **INTENTIONAL_DEVIATION** | `same_condition` required | **#97** (PRD said independent conditions) |
| 29 | Depth-aware SABRE | **PARTIAL** | Routes + SWAPs; α/γ used; **β & lookahead never read** | **#181** (filed this audit) |
| 30 | Native decomp ZYZ/KAK | DONE | `native_gate_decomp` + `backend/decompose.rs`; lit | |
| 31 | ASAP/ALAP + T1 | DONE | `depth_scheduling.rs`; lit barrier; unit mode select | |
| 32 | `--emit-qasm` OpenQASM 3 | DONE | `openqasm3.rs`; smoke + lit emit | |
| 33 | `--target` JSON | DONE | `backend` JSON loader; smoke device fixtures | |
| 34 | `quon_aer.py` refs | DONE | CI runs all 8 + routing | SPEC §12 bit-flip is **typecheck fixture**, not Aer gate |
| 35 | `generic_openqasm` | DONE | `generic_openqasm.rs`; default compile path | |
| 36 | 5-crate workspace | **DOCUMENTED_EVOLUTION** | Now 12 members | README Workspace table; OK for MVP |
| 37 | Rust-only build | DONE | Melior links prebuilt LLVM 22; no in-tree TableGen | Host deps documented |
| 38 | `cargo test` + lit | DONE | `quonc/tests/lit.rs` skip-if-absent; CI installs lit | README accurate |

### Story 29 deep dive (new finding)

```rust
// sabre_routing.rs — fields exist with SPEC defaults…
pub beta: f64,       // 0.5
pub lookahead: usize // 20
// …but scoring uses alpha * distance + gamma * noise only.
```

CLI exposes `--sabre-gamma` only. Depth operationalization claimed in SPEC §7.4 / story 29 is incomplete. **#181**.

### Story 25 / 28 / 24 (known; confirmed)

| Pass | Claimed in PRD | Shipped | Tracker |
|------|----------------|---------|---------|
| ZX | Non-local rewrite | Sound, single-wire extract only | #75 |
| Clifford+T | Phase polynomials + tableaux | Peephole via gate cancellation | #96 |
| Classical fusion | Independent conditions | Same classical bit only | #97 |

---

## Pipeline wiring (M1 acceptance re-check)

From `quonc/src/compile.rs` + `--list-passes`:

1. Lower Quon → `quantum.circ` (via elaborate when needed)
2. Circ fixpoint: gate_cancellation, rotation_merging, compiler_uncomputation, zx_simplification, clifford_t_opt
3. Monadic lowering → `quantum.dynamic`
4. measurement_deferral, classical_region_fusion
5. Fixed path: native_gate_decomp → sabre_routing → native_gate_decomp → depth_scheduling → OpenQASM 3
6. Neutral-atom path: extract → entangle → zoned/flat move → compact → resource/schedule emit (#112)

`--target` drives native set, topology, and noise for the fixed path. `generic_openqasm` remains the default no-JSON path.

---

## CI / verification matrix

| Gate | Where | Status |
|------|-------|--------|
| Unit + integration (`cargo test`) | `ci.yml` | Green path |
| lit FileCheck | `ci.yml` installs `lit`; `quonc/tests/lit.rs` | Done (#28) |
| Aer 8 refs | `test/verify/{bell,teleport,bernstein_vazirani,grover,qft,ising,qaoa,shor}.py` | Done (#29/#30) |
| Routing e2e | `test/verify/routing.py` | Done |
| Tooling | `tooling` job + `scripts/tooling-check.sh` | Beyond PRD MVP |
| Flux | `flux.yml` path-filtered | Optional refinement |

**SPEC §12 vs PRD story 34**: Bit-flip ECC appears in SPEC reference algorithms and typechecks (`error_correction.qn` / unit `syndrome_measure_*`) but is **not** one of the 8 Aer programs in the PRD testing decision. Not an MVP miss; optional post-MVP Aer fixture if desired.

---

## Documentation accuracy (M5 checklist)

| Doc | Finding | Action |
|-----|---------|--------|
| README pipeline / `--target` / lit skip | Accurate | Minor: pass list already via `--list-passes`; no change required for correctness |
| README workspace | Accurate (expanded crates) | None |
| CONTEXT.md Borrow | **Stale** (claims structural reset/discard) | Updated in this change set to match shipped semantics; full ADR decision remains **#180** |
| ADR-0003 | Stale vs code | Do not silently rewrite — human decision on #180 |
| SPEC parametric IR note | Present (§9) | OK (post-MVP path documented) |
| SPEC borrow_end \|0⟩ | Conflicts with measure-cleanup | Covered by #180 |

---

## Post-MVP queue (value order, refreshed)

1. **#96** Real Clifford+T (phase polynomials + stabilizer tableaux)  
2. **#181** SABRE β + lookahead (complete story 29 cost model)  
3. **#75** Faithful multi-qubit ZX extraction  
4. **#97** Classical fusion for independent conditions  
5. **#182** Generalize `controlled` elaboration  
6. **#180** Resolve borrow ADR/SPEC/CONTEXT (docs correctness)  
7. **#82** / **#119** IBM → hardware-valid QASM / live checkpoint  
8. Neutral-atom completeness: **#167**, **#111**, **#114**  
9. IDE polish epic **#171–#178** (explicitly out of PRD scope; already largely shipped)  
10. Visualizers **#134–#136**

Tooling track #43–#49 from the original M5 note is **already closed** (Jul 9).

---

## Mild test gaps (non-blocking)

- No dedicated negative unit test that `split` of a non-`QReg` fails (impl rejects).  
- No wrong-depth-on-`for` negative (unlike `repeat` / `\|\|>`).  
- No dedicated ZX FileCheck lit file (unit+prop cover the pass).  
- No SABRE FileCheck under `test/lit/physical/` (unit + `routing.py` + smoke).  

Nice-to-haves; not MVP blockers.

---

## Close-out actions completed by this audit

- [x] Walked all 38 stories with evidence  
- [x] Confirmed #22–#30 closed  
- [x] Confirmed known deviations #75 / #96 / #97  
- [x] Filed **#180** (borrow ADR drift), **#181** (SABRE β/lookahead), **#182** (controlled elaboration)  
- [x] Updated `CONTEXT.md` Borrow entry to match shipped semantics (pending #180 ADR amend)  
- [x] Recorded audit in `docs/plans/m5-closeout-audit.md`  
- [x] Comment + close PRD #1  

---

## Recommendation for the written report

Treat this document’s story table as the authoritative “done / deferred” appendix. Core narrative can ship now; call out #96/#75/#97/#181 as optimization depth, and #180 as a specification hygiene item—not as missing MVP features.
