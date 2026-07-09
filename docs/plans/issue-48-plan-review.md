# Issue #48 Plan Review — Adversarial Grade

**Plan:** `docs/plans/issue-48-plan.md`  
**Worktree inspected:** `48-watch-metrics` @ pipeline state matching `quonc/src/main.rs` and `mlir_bridge/`  
**Reviewer stance:** Adversarial — assume the AFK agent will implement exactly what is written.

---

## Overall Grade: **C+**

## Pass / Fail: **FAIL** (conditional — fix critical blockers before coding)

The plan is well-structured, correctly identifies the current gap (`compile_to_qasm` in `main.rs` with no metrics/watch/snapshot surface), and the three-layer split (`quonc` → `mlir_bridge::metrics` → `quon_core::metrics`) matches repo conventions. However, several metric semantics contradict the actual pass pipeline, and one CLI interaction spec directly breaks watch-mode UX. An agent following this plan as-is will ship subtly wrong metrics and inconsistent behavior.

---

## Critical Blockers

### 1. Watch + snapshot compare exit behavior is self-contradictory

§4 flag interactions say:

> `--watch` + `--metrics-snapshot compare` → exit 1 on regression

§6 watch loop says:

> on error: print diagnostics, **keep watching** (do not exit watch loop)

Regression failure is not a compile error, but exiting with status 1 terminates the watch loop — the opposite of "keep watching." This must be resolved before M3.

**Required fix:** In watch mode, regression violations print `FAIL` to stderr and optionally set a sticky "last compare failed" flag, but **do not exit**. Reserve exit code 1 for one-shot `--metrics-snapshot compare` only. Document both behaviors explicitly.

### 2. T-count collection point contradicts the stated goal and the actual pipeline

The plan says count T gates at **post-routing, pre-emission IR** so metrics reflect "algorithm+layout, not pulse decomposition."

But `quonc/src/main.rs` runs:

```
native_gate_decomp → sabre_routing → native_gate_decomp → depth_scheduling → emit
```

Metrics collected after `depth_scheduling` (the natural hook) are **after the second `native_gate_decomp`**. On restrictive targets (e.g. `device_5q.json` with `{rz, sx}`), T gates are decomposed before routing — so post-pipeline `t_count` will be **0** even for T-heavy algorithms. §12 acknowledges T-count ambiguity but does not reconcile it with §5's stated collection point.

**Required fix:** Pick one hook and document it with a target-matrix test:

| Option | Hook | Tradeoff |
|--------|------|----------|
| A (recommended) | After routing, **before** second `native_gate_decomp` | Captures SWAPs + layout; T preserved except first-pass decomp on non-native gates |
| B | After full pipeline | Matches emitted circuit; T lost on non-native targets |
| C | Dual metrics: `logical_*` + `physical_*` | Most accurate; more schema work |

Do not leave "before decomp **or** after pipeline" as an open implementation choice.

### 3. `depth_bound` source is mostly unavailable on the executed path

§5 assigns `depth_bound` from `quantum.circ.func` `depth` on `main`. After `monadic_lowering`, the **executed program lives in the module top-level block**, not inside a `quantum.circ.func` wrapper — as documented in `depth_scheduling.rs`:

```283:285:mlir_bridge/src/passes/depth_scheduling.rs
    // The module's own top-level block is the real, executed program after
    // `monadic_lowering` (see `native_gate_decomp::decompose_block`'s doc
    // comment) — a `quantum.circ.func` may no longer wrap it at all.
```

For real `quonc` compiles (e.g. `bell_state.qn` → `hello_bell` run block), `depth_bound` will usually be **`None`**, not the symbolic bound the plan's JSON example shows.

**Required fix:** Either (a) propagate `DepthExpr` through monadic lowering as a module-level attribute, (b) drop `depth_bound` from v1 schema, or (c) document it as "best-effort, often absent for lowered programs." Do not pin test expectations on it for end-to-end fixtures.

### 4. Dynamic `if` metrics over-count by design (undocumented)

`collect_gates` in `depth_scheduling.rs` recurses **both** branches of `quantum.dynamic.if`:

```97:100:mlir_bridge/src/passes/depth_scheduling.rs
        if name == quantum_dynamic::op::IF {
            recurse_region(current, 0, tracker, steps);
            recurse_region(current, 1, tracker, steps);
        }
```

The metrics collector reuses this walk. For programs with conditionals, `gate_count`, `t_count`, `depth`, and `swap_count` will reflect **both branches**, not executed-path metrics. The plan does not mention this.

**Required fix:** Document as "conservative upper bound (all branches)" or define executed-path metrics separately. Add a test with `quantum.dynamic.if` in `mlir_bridge/tests/metrics.rs`.

---

## Major Concerns

### Architecture & pipeline alignment

- **Verified correct:** Plan's "current state" table matches `quonc/src/main.rs` — full pipeline wired, no metrics, no lib surface, compile logic only in `main.rs`.
- **Verified correct:** Pass order in plan matches `mlir_bridge/src/passes/mod.rs`.
- **`collect_gates` reuse is sound** for depth (matches scheduling semantics including barrier segmentation via `schedule_time` offsets), but only if depth is computed as `max(schedule_time) + 1` over collected non-barrier gates — not by reimplementing scheduling.

### M1 scope is too large for one PR

M1 bundles: `serde` on `quon_core`, full DTO schema, compare logic + proptest, `mlir_bridge::metrics` collector, **`quonc` lib+bin split**, and two new CLI flags. That is 3–4 logical PRs compressed into one. Risk: a large M1 blocks M2/M3 and makes review harder.

**Amendment:** Split M1 into `48-quonc-lib-refactor` (extract `compile()`, no behavior change) and `48-metrics-core` (DTOs + collector + flags). Lib refactor first makes metrics wiring a smaller diff.

### `--simulate` is designed but not scheduled

§4 defines `--simulate`, `--simulate-shots`, `--simulate-seed`, and a `simulation` JSON object. No milestone delivers it; §14 acceptance criteria omit it. `python/quon_aer.py` today prints human-readable counts to stdout, not JSON.

**Amendment:** Explicitly defer `--simulate` to a follow-up issue, or add M2.5 with `quon_aer.py --json` and wire-up. Do not leave flags in `--help` without implementation.

### Regression compare lacks identity validation

Compare applies tolerances field-by-field but never requires matching `program.source_sha256`, `target.id`, or `entry`. Comparing baselines across different programs/targets could pass if numeric metrics coincidentally align.

**Amendment:** Compare must fail fast (exit 2) on mismatched `source_sha256` or `target.id` unless `--regression-config` has an explicit `allow_mismatch = true` escape hatch.

### `qubit_count` omitted from default regression tolerances

§8 default config covers `depth`, `gate_count`, `t_count`, `swap_count`, `compile_ms` — not `qubit_count`. A routing bug that changes qubit width could slip through default compare.

**Amendment:** Add `[metrics.qubit_count] absolute = 0` to defaults, or document intentional omission.

### CLI `--metrics-snapshot` shape unspecified

Plan shows `--metrics-snapshot save PATH` as an enum flag but does not specify the clap structure (`num_args = 2`, separate `--metrics-snapshot-save` / `--metrics-snapshot-compare`, etc.). Current `quonc` uses a flat struct (see `quonc/tests/cli.rs`); getting this wrong breaks scriptability and help tests.

**Amendment:** Specify exact clap definition and add to M2 acceptance: help lists the flag with example syntax.

### Bell state test expectations are underspecified

M1 pins `gate_count: 2` (reasonable for H + CX on generic target) but depth is "snapshot once or assert ≤ 3." That is not a regression lock — an agent will pick arbitrary values.

**Amendment:** Run `bell_state.qn` once on trunk, record exact `depth`, `gate_count`, `t_count`, `swap_count`, `qubit_count` in the plan or a committed golden JSON. `smoke.rs` already locks QASM output; metrics should get the same treatment.

### Watch mode test rigor is weak

M3 allows `#[ignore]` on flaky CI watch tests. Watch + filesystem events are notoriously flaky in sandboxed CI. "Prefer deterministic debouncer unit test" is mentioned but not required.

**Amendment:** Require a unit test of debounce logic (inject synthetic events via channel) as the CI gate; make the full watch subprocess test `#[ignore]` or manual-only explicitly.

### Wrong fixture reference

Plan cites `test/lit/physical/depth_schedule.*`; the repo has `test/lit/physical/depth_scheduling_barrier.mlir`. `mlir_bridge/tests/depth_scheduling_pass.rs` is the better reference for collector tests.

### `program.entry: "main"` is misleading

Quon programs use named run functions (`hello_bell` in `bell_state.qn`). Schema should record the actual entry symbol from lowering, or rename field to `run_symbol` / omit until defined.

### `quonc` lib extraction missing Cargo.toml step

Plan adds `quonc/src/lib.rs` but does not mention adding `[lib]` to `quonc/Cargo.toml` or updating integration tests to use `quonc::compile()` directly. Taskless allows `anyhow` in `quonc/**` — good — but the lib/bin split needs explicit manifest changes.

---

## Minor Nits

- §5 table lists `compile_ms` under metrics definitions but JSON puts it under `compile.compile_ms` — internal doc inconsistency.
- Open question §13.1 (JSON vs QASM stdout ordering) should be **resolved in the plan**, not deferred to M1 PR description; recommend stderr for metrics JSON when `--emit-qasm` is set (plan already recommends this — just mark it decided).
- `git_dirty` in schema with no collection method specified (`git diff --quiet` or `git status --porcelain`).
- Plan says promote `gate_cancellation.rs` `count_gate` to production — that helper uses **IR text matching** (`text.matches("gate_name = \"H\"")`), which is test-only quality. Production collector must walk IR like `collect_gates`, not string-match.
- M1 milestone table lists `quon_core::metrics` compare in M1 but M2 owns snapshot compare — fine, but table should clarify compare **logic** vs compare **CLI**.
- `notify` not yet in workspace `Cargo.toml` (only `serde`/`serde_json` are) — plan mentions adding it; correct.
- MLIR context reuse (§6) left as "verify if safe" — acceptable for M3 spike, but default should be **recreate per compile** until proven safe.

---

## Repo Conventions Assessment

| Convention | Plan compliance |
|------------|-----------------|
| `quon_core` MLIR-free | ✅ Correct layer for JSON DTOs + compare |
| `#[serde(deny_unknown_fields)]` on wire DTOs | ✅ Explicit; matches `.taskless/rules/serde-deny-unknown-fields-on-dto.yml` |
| `thiserror` in libs, `anyhow` in quonc | ✅ Matches `code-quality.md`; taskless ignores `quonc/**` |
| Graphite stacked PRs | ✅ M1–M4 structure is reasonable (after M1 split) |
| Pre-PR validation commands | ✅ Matches `validation.md` |
| Property tests for tolerance math | ✅ Good |
| Reuse `collect_gates` walk | ✅ Good intent; needs if-branch documentation |
| JSON DTO pattern from `backend/src/descriptor.rs` | ✅ Good reference |

---

## Specific Required Plan Amendments

1. **Resolve watch + compare exit behavior** — no exit on regression in watch mode; document stderr output and exit codes (0=ok, 1=regression one-shot, 2=tool error).

2. **Fix T-count hook point** — single explicit pipeline stage; add `qft.qn` or T-heavy fixture test on both `generic_openqasm` and `device_5q.json`.

3. **Fix or defer `depth_bound`** — do not promise it for lowered programs without propagation work.

4. **Document dynamic-`if` over-counting** — conservative semantics + test.

5. **Split M1** or justify single PR size; add `quonc/Cargo.toml` `[lib]` step.

6. **Defer or milestone `--simulate`** — remove from CLI design until scheduled; add `quon_aer.py --json` if kept.

7. **Add snapshot identity checks** on compare (`source_sha256`, `target.id`).

8. **Add `qubit_count` to default regression tolerances.**

9. **Specify clap structure** for `--metrics-snapshot`.

10. **Commit golden metrics** for `bell_state.qn` (exact values, not `≤ 3`).

11. **Require debouncer unit test** as CI gate; demote full watch subprocess test.

12. **Fix lit fixture path** → `depth_scheduling_barrier.mlir` / `depth_scheduling_pass.rs`.

13. **Resolve open question §13.1** in plan (stderr for JSON when `--emit-qasm`).

14. **Clarify `program.entry`** field semantics or rename.

---

## Summary Verdict

| Axis | Grade | Notes |
|------|-------|-------|
| Correctness | D+ | T-count hook, depth_bound, if-branch counting, watch/compare contradiction |
| Completeness | B- | Core features covered; simulate orphaned; Cargo.toml/lib steps missing |
| Feasibility | B+ | Pipeline verified; dependencies reasonable; collector approach sound |
| Test rigor | C | Golden values vague; watch tests allow ignore; good proptest intent |
| Risk coverage | B | Good notify/debounce risks; misses compare identity and if-branch |
| Repo conventions | A- | Strong alignment with quon_core/mlir_bridge/quonc split |

**Do not start implementation until critical blockers 1–4 are amended in the plan.** After amendments, this is a solid B+/A- plan suitable for an AFK agent.
