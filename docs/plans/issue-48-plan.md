# Issue #48 — Rapid experiment loop: watch mode + metrics snapshots + regression checks

**Audience**: an AFK agent executing this plan in the `issue-48-watch-metrics` worktree (branch `issue-48-watch-metrics`, parent #1).
**Objective**: ship `quonc --watch`, structured compile metrics in JSON, baseline snapshot save/compare, and per-metric regression thresholds — so algorithm iteration is a tight edit→recompile→compare loop instead of ad-hoc shell scripts.
**Issue**: [Rapid experiment loop: watch mode + metrics snapshots + regression checks](https://github.com/quon-lang/quon/issues/48) (`ready-for-agent`, blocked by #27/#29/#30 — all landed on trunk as of 2026-07-08 @ `ea49b03`).

Read first: `CLAUDE.md`, `docs/agents/code-quality.md`, `docs/agents/graphite.md`, `CONTEXT.md`, `SPEC.md` §7 (pipeline), and this plan's parent issue body (`gh issue view 48`).

---

## 1. Verified current state (inspected 2026-07-08, `ea49b03`)

Do not re-derive; spot-check only what you touch.

**Already in place (building blocks for #48):**

| Area | Location | Relevance |
| ---- | -------- | --------- |
| Full pass pipeline wired in driver | `quonc/src/main.rs` `compile_to_qasm` | Metrics must be collected *after* the same pipeline watch mode runs |
| Pass order documented | `mlir_bridge/src/passes/mod.rs` | Defines the compile stages metrics should reflect |
| Scheduled physical depth | `mlir_bridge/src/passes/depth_scheduling.rs` | Writes `schedule_time` on each gate; segment depth = `max(time)+1` |
| Symbolic depth on funcs | `quon_core::DepthExpr`, `quantum.circ.func` `depth` attr | Optional secondary metric / debug field |
| Gate counting precedent (tests) | `mlir_bridge/tests/gate_cancellation.rs` `count_gate` | Walk IR, match `gate_name` — promote to production collector |
| QASM reify | `mlir_bridge/src/emit/openqasm3.rs` | Final gate list before emission; alternative T-count source |
| Aer simulation bridge | `python/quon_aer.py`, `test/verify/*.py` | Optional `simulation` summary via subprocess |
| JSON DTO pattern | `backend/src/descriptor.rs` | `#[serde(deny_unknown_fields)]` wire types |
| CLI integration tests | `quonc/tests/cli.rs`, `quonc/tests/smoke.rs` | Extend for new flags |

**Not present (the #48 gap):**

- No metrics emission from `quonc` (success path prints nothing unless `--emit-qasm`).
- No library surface on `quonc` — compile logic lives only in `main.rs`.
- No snapshot or regression tooling.
- No file watcher.
- No `serde` on `quon_core` (needed for portable metrics DTOs tested without MLIR).

---

## 2. Design goals and non-goals

### Goals

1. **Fast iteration**: edit a `.qn` file → automatic recompile → human-readable metric delta on stderr.
2. **Machine-readable output**: one JSON document per compile, stable schema version, suitable for CI and scripts.
3. **Regression guardrails**: save a baseline snapshot, compare later runs with per-metric absolute/relative tolerances.
4. **Minimal new concepts**: reuse the existing compile pipeline; do not add new optimization passes.

### Non-goals (defer)

- Multi-file `import` watching (Quon has no module system yet — watch the primary source file and optional `--target` JSON only).
- Neutral-atom-specific metrics (`t` rearrangement time from `docs/neutral_atom/`) — out of scope until `quon_na` lands; schema leaves room via `extensions`.
- In-process Qiskit/Rust simulation — keep optional simulation as a subprocess to `python/quon_aer.py`.
- Rewriting `test/verify/` to use snapshots (optional follow-up, not required for #48 acceptance).

---

## 3. Recommended architecture

Split responsibilities across three layers (matches repo conventions):

```
┌─────────────────────────────────────────────────────────────┐
│  quonc (bin + thin lib)                                     │
│  CLI flags, watch loop, subprocess simulation, user I/O     │
└───────────────────────────┬─────────────────────────────────┘
                            │ CompileRequest → CompileReport
┌───────────────────────────▼─────────────────────────────────┐
│  mlir_bridge::metrics                                       │
│  Walk final Module IR → raw counts (depth, gates, t_count)  │
└───────────────────────────┬─────────────────────────────────┘
                            │ populates Metrics fields
┌───────────────────────────▼─────────────────────────────────┐
│  quon_core::metrics                                         │
│  JSON schema, snapshot save/load, compare + tolerances        │
│  (MLIR-free — unit-testable)                                │
└─────────────────────────────────────────────────────────────┘
```

**Refactor `quonc` into lib + bin:**

- Add `quonc/src/lib.rs` exporting `compile::compile(CompileRequest) -> Result<CompileReport>`.
- Move `compile_to_qasm`, diagnostics printing, and pass orchestration from `main.rs` into the lib.
- `main.rs` becomes argument parsing + dispatch only.

This lets integration tests call the compiler without subprocess spawning and without duplicating pipeline logic for watch mode.

---

## 4. CLI flag design

Keep the existing flat `clap` struct (consistent with `--emit-qasm`, `--target`, `--dump-ir`). Add output-mode flags and snapshot subcommands as **flags**, not nested subcommands — simpler for scripts and matches current tests.

### New flags

| Flag | Type | Default | Description |
| ---- | ---- | ------- | ----------- |
| `--watch` | bool | false | Watch `source` (and `--target` if set) for changes; recompile on change |
| `--watch-debounce-ms` | u64 | 300 | Debounce window for filesystem events (editor save bursts) |
| `--metrics` | bool | false | Print a human-readable metrics summary to **stderr** after a successful compile |
| `--metrics-json` | optional path | — | Write metrics JSON to file; use `-` for stdout |
| `--metrics-snapshot` | enum | — | `save PATH` or `compare PATH` (see §7) |
| `--regression-config` | path | — | TOML/JSON tolerance file for `compare` (see §8) |

### Flag interactions (enforce in clap validators or early `main`)

| Combination | Behavior |
| ----------- | -------- |
| `--watch` without `--metrics` | Implicitly enable `--metrics` (watch mode is useless without output) |
| `--metrics-snapshot compare` without `--regression-config` | Use built-in strict defaults (zero tolerance on counts; `compile_ms` ±10% relative) |
| `--metrics-snapshot save` on failed compile | Do not overwrite snapshot; exit non-zero |
| `--metrics-json -` with `--emit-qasm` | Metrics JSON on **stderr** when `--emit-qasm` is set; stdout otherwise (decided — not open) |
| `--watch` + `--metrics-snapshot compare` | Compare after each successful recompile; print `FAIL` to stderr on regression but **do not exit** (watch loop continues). Exit code 1 reserved for one-shot `--metrics-snapshot compare` only. |

### Help text examples (for docs)

```bash
# Experiment loop: watch + live metrics
quonc algo.qn --watch --target device.json --metrics

# One-shot metrics JSON for scripting
quonc algo.qn --metrics-json metrics.json

# Save baseline, then compare in CI
quonc algo.qn --metrics-snapshot save baselines/algo.json
quonc algo.qn --metrics-snapshot compare baselines/algo.json --regression-config tolerances.toml

# Metrics + JSON (simulation deferred — see §2 non-goals)
quonc bell.qn --metrics --metrics-json -
```

### Dependencies to add

| Crate | Where | Why |
| ----- | ----- | --- |
| `notify` | `quonc` | Cross-platform file watching (`RecommendedWatcher`) |
| `serde`, `serde_json` | `quon_core` | Metrics + snapshot wire format |
| `toml` (optional) | `quon_core` | Ergonomic regression config; JSON also supported |

Add `notify` to workspace dependencies in root `Cargo.toml`.

---

## 5. Metrics schema

Define a versioned, self-describing JSON document. All Deserialize structs use `#[serde(deny_unknown_fields)]`.

### Top-level: `MetricsSnapshot` (one compile)

```json
{
  "schema_version": 1,
  "program": {
    "source": "algorithms/grover.qn",
    "source_sha256": "abc…",
    "entry": "main"
  },
  "target": {
    "id": "generic_openqasm",
    "descriptor_path": null
  },
  "toolchain": {
    "quonc_version": "0.1.0",
    "git_commit": "ea49b03",
    "git_dirty": false
  },
  "compile": {
    "status": "ok",
    "compile_ms": 142,
    "error": null
  },
  "metrics": {
    "depth": 87,
    "depth_bound": null,
    "gate_count": 312,
    "t_count": 48,
    "qubit_count": 5,
    "swap_count": 12
  },
  "simulation": null
}
```

On failure, `compile.status` is `"error"`, `compile.error` is a string, and `metrics` may be absent or partial — snapshot **save** should refuse error states.

**Note:** `simulation` field reserved for a follow-up issue; `--simulate` flags are **not** in v1 CLI.

### Field definitions

| Field | Type | Source | Notes |
| ----- | ---- | ------ | ----- |
| `depth` | `u64` | Post-`depth_scheduling` IR | **Critical-path scheduled depth**: `max(schedule_time) + 1` over all non-barrier gates in executed regions (same recurrence as `schedule_segment` return value in `depth_scheduling.rs`). This is the primary hardware-relevant depth. |
| `depth_bound` | `string` optional | `quantum.circ.func` `depth` attr (pre-lowering) | Symbolic `DepthExpr` S-expr. **Often `None` for real compiles:** after `monadic_lowering`, the executed program lives in the module top-level block, not inside a `quantum.circ.func` wrapper. Do not pin end-to-end test expectations on this field. |
| `gate_count` | `u64` | Final IR before emission | Count of executed gate ops (see collector rules below). Includes routing SWAPs. Excludes `barrier`, `measure`, `reset`. |
| `t_count` | `u64` | IR after routing, **before** second `native_gate_decomp` | Count gates where normalized `gate_name` ∈ `{T, t}` (case-insensitive). Count `tdg` as 1 T gate each. **Hook point (decided):** collect after `sabre_routing`, before the second `native_gate_decomp` pass — captures SWAPs + layout while preserving T gates on restrictive targets. Add target-matrix test on `generic_openqasm` and `device_5q.json`. |
| `qubit_count` | `u64` | Emitted/reified program | Number of qubits in the final circuit (from module wiring or `reify`). |
| `swap_count` | `u64` | Final IR | Count `gate_name` ∈ `{SWAP, swap}` — useful routing diagnostic; compare separately from `gate_count`. |
| `compile_ms` | `u64` | Wall clock | Whole `compile()` call, excluding simulation subprocess. |

### Optional `simulation` object (deferred)

**Deferred to follow-up issue.** Requires `quon_aer.py --json` and CLI flags (`--simulate`, `--simulate-shots`, `--simulate-seed`). Do not ship in v1 `--help`.

### Dynamic `if` over-counting (conservative semantics)

The metrics collector reuses `depth_scheduling::collect_gates`, which recurses **both** branches of `quantum.dynamic.if`. For programs with conditionals, `gate_count`, `t_count`, `depth`, and `swap_count` reflect **all branches** (conservative upper bound), not executed-path metrics. Document this in `--metrics` output and JSON schema comments. Add a test with `quantum.dynamic.if` in `mlir_bridge/tests/metrics.rs`.

### Collector implementation (`mlir_bridge/src/metrics.rs`)

Reuse the same recursive gate walk as `depth_scheduling::collect_gates`:

1. Seed `WireTracker` on module entry block (post-`monadic_lowering` shape).
2. Recurse `quantum.dynamic.unitary_region` and `quantum.dynamic.if` bodies.
3. For each gate op (`quantum.circ.gate` or dynamic native gate wrapper — match what `collect_gates` already treats as a gate):
   - Read `gate_name` string attribute.
   - Read `schedule_time` i64 if present (for depth).
4. Also walk dead `quantum.circ.func` bodies? **No** — only executed top-level path (matches scheduling).

Expose:

```rust
pub struct CircuitMetrics {
    pub depth: u64,
    pub gate_count: u64,
    pub t_count: u64,
    pub qubit_count: u64,
    pub swap_count: u64,
    pub depth_bound: Option<DepthExpr>,
}

pub fn collect_module_metrics(module: &Module<'_>, target: &BackendTarget) -> CircuitMetrics;
```

Unit-test against existing lit fixtures (`test/lit/physical/depth_schedule.*`) by constructing modules in `mlir_bridge/tests/metrics.rs`.

### Human-readable `--metrics` line (stderr)

Stable, grep-friendly single line plus optional multi-line detail:

```
[quonc] depth=87 gates=312 t=48 swaps=12 compile=142ms target=generic_openqasm
```

On watch recompile, prefix with ISO timestamp and delta vs previous run:

```
[quonc] 2026-07-08T16:00:01 depth=87 (+2) gates=310 (-2) t=48 (=) compile=138ms (-4ms)
```

---

## 6. Watch mode implementation

### Crate: `notify`

Use the `notify` crate with `RecommendedWatcher` (FSEvents on macOS, inotify on Linux). Do **not** shell out to `fswatch`.

### Watch set

| Path | Condition |
| ---- | --------- |
| CLI `source` | Always |
| `--target` descriptor | If provided |
| Parent directory of source | **No** (avoid noise from unrelated files) |

Register `RecursiveMode::NonRecursive` on the source file path. For editors that atomic-rename save (vim, some IDEs), also watch the **parent directory** with a filter: only react if the event path equals the canonical source path. Implementation sketch:

```rust
let (tx, rx) = std::sync::mpsc::channel();
let mut watcher = notify::recommended_watcher(move |res| { let _ = tx.send(res); })?;
watcher.watch(&source, RecursiveMode::NonRecursive)?;
watcher.watch(source.parent().unwrap_or(&source), RecursiveMode::NonRecursive)?;
```

Filter events in the receive loop: `EventKind::Modify(_) | EventKind::Create(_)` where `paths` contains the source.

### Debouncing

Accumulate events in a `Instant` deadline loop:

1. On event, set `deadline = now + debounce_ms`.
2. `recv_timeout` until deadline passes with no new events.
3. Run compile once.

Default 300 ms; expose `--watch-debounce-ms`.

### Loop behavior

```
initial compile → print metrics
loop:
  wait for debounced change
  re-read source (+ target json)
  compile
  on success: print metrics (+ delta), optional snapshot compare (print FAIL, keep watching)
  on compile error: print diagnostics, keep watching (do not exit watch loop)
  on regression (compare): print FAIL table to stderr, set sticky flag, **keep watching** (no exit)
```

Exit watch loop on `Ctrl+C` (graceful, status 130) or fatal watcher error.

### Performance notes

- Reuse one `melior::Context` per process if safe (verify MLIR context reuse — if not, recreate per compile; still fast enough for edit loops).
- Do not `--dump-ir` by default in watch mode.
- Skip simulation in watch unless `--simulate` (Aer is slow).

---

## 7. Snapshot file format

Snapshots are **pretty-printed JSON** files using the `MetricsSnapshot` schema from §5.

### Save (`--metrics-snapshot save PATH`)

1. Compile successfully.
2. Populate provenance: `source_sha256`, `git_commit` via `git rev-parse HEAD` (best-effort; null if not a git repo), `quonc_version` from `clap::crate_version!()`.
3. Write atomically: temp file + `rename`.
4. Print `saved snapshot → PATH`.

### Compare (`--metrics-snapshot compare PATH`)

1. Compile successfully.
2. Load baseline JSON from `PATH`; validate `schema_version`.
3. Apply regression rules (§8) field-by-field on the **numeric** metrics subset.
4. Print human-readable diff table to stderr; exit `0` if within tolerance, `1` if regression detected.

### Compare output example

```
metric          baseline  current   delta     tolerance   status
depth           87        89        +2        abs≤0       FAIL
gate_count      312       310       -2        abs≤5       ok
t_count         48        48        0         abs≤0       ok
compile_ms      140       138       -2        rel≤10%     ok
```

### Schema evolution

Bump `schema_version` on breaking changes. Compare rejects unknown versions with a clear error. Non-breaking additions (new optional fields) stay on the same version.

---

## 8. Regression config

Support **JSON and TOML** (same struct). File discovery: explicit `--regression-config PATH`; else embedded defaults.

### Default tolerances (when no config file)

```toml
# quonc default regression tolerances
[metrics.depth]
absolute = 0

[metrics.gate_count]
absolute = 0

[metrics.t_count]
absolute = 0

[metrics.swap_count]
absolute = 0

[metrics.compile_ms]
relative = 0.10   # 10% — compile time is noisy

# simulation fields optional — only checked if both snapshots include simulation
```

### Config schema (`RegressionConfig`)

```toml
[metrics.depth]
absolute = 2        # allow +2 depth layers
relative = 0.0      # optional fraction of baseline (use max of abs/rel)

[metrics.gate_count]
absolute = 5

[metrics.t_count]
absolute = 0

[metrics.compile_ms]
relative = 0.25
absolute = 50       # also allow +50ms flat

[ignore]
# optional: skip comparing these fields entirely
fields = ["compile_ms"]
```

Comparison rule for numeric metric `m` with baseline `b`, current `c`, delta `d = c - b`:

- Pass if `|d| ≤ absolute` OR (if `relative` set) `|d| ≤ relative * max(b, 1)`.
- When both set, pass if **either** bound satisfied (lenient) — document this; alternative strict mode can be a follow-up.

Implement in `quon_core::metrics::compare` returning `ComparisonReport { passed, violations: Vec<Violation> }`.

All config structs: `#[serde(deny_unknown_fields)]`.

---

## 9. Execution milestones (Graphite stack)

Each milestone is one PR; trunk stays green. Suggested branch names:

| # | Branch | Delivers | Acceptance |
|---|--------|----------|------------|
| M0 | `48-quonc-lib-refactor` | Extract `quonc/src/lib.rs` + `compile.rs` from `main.rs`; add `[lib]` to `quonc/Cargo.toml`; no behavior change | Existing `quonc/tests/smoke.rs` and `cli.rs` pass unchanged |
| M1 | `48-metrics-core` | `quon_core::metrics` DTOs + compare; `mlir_bridge::metrics` collector; `--metrics` + `--metrics-json` | Compile `bell_state.qn`, JSON matches golden (see below) |
| M2 | `48-snapshot-regression` | `--metrics-snapshot save/compare`, `--regression-config`, exit codes | Save baseline in test; perturb compile; compare fails/succeeds per config |
| M3 | `48-watch-mode` | `--watch`, debounce, delta printing | Debouncer unit test in CI; full watch subprocess test `#[ignore]` |
| M4 | `48-docs` | README + `docs/agents/experiment-loop.md` workflow | Acceptance criterion: documented iteration workflow |

### M0 detail — quonc lib refactor (no behavior change)

1. Add `[lib]` to `quonc/Cargo.toml`.
2. Create `quonc/src/lib.rs` + `compile.rs`; move `compile_to_qasm` and pass orchestration from `main.rs`.
3. `main.rs` becomes argument parsing + dispatch only.
4. Verify existing integration tests pass without modification.

### M1 detail — metrics foundation

1. Add `serde`/`serde_json` to `quon_core/Cargo.toml`.
2. Create `quon_core/src/metrics.rs` — wire types, `ComparisonReport`, defaults.
3. Create `mlir_bridge/src/metrics.rs` — IR collector (§5); hook T-count after routing, before second `native_gate_decomp`.
4. Extend `CompileReport { qasm: Option<String>, metrics: MetricsSnapshot }`.
5. Wire `--metrics` / `--metrics-json` flags.
6. Tests:
   - `quon_core`: compare logic proptest (random baselines, tolerances).
   - `mlir_bridge/tests/metrics.rs`: gate/depth counts on hand-built modules; dynamic-`if` over-counting test.
   - `quonc/tests/metrics.rs`: subprocess compile `frontend/tests/fixtures/bell_state.qn`, parse JSON against golden.

**Golden `bell_state.qn` metrics** (run once on trunk @ generic target, commit exact values in `quonc/tests/fixtures/metrics/bell_state_golden.json`):

| Metric | Golden value (pin at M1) |
|--------|--------------------------|
| `gate_count` | 2 (H + CX, pre-routing) |
| `t_count` | 0 (Clifford program) |
| `depth` | exact value from trunk run (not `≤ 3`) |
| `swap_count` | exact value from trunk run |
| `qubit_count` | exact value from trunk run |
| `depth_bound` | `null` (expected for lowered programs) |

Do not use loose bounds — lock exact values like `smoke.rs` does for QASM output.

### M2 detail — snapshots

1. Implement save/compare in `quonc` using `quon_core::metrics`.
2. Git provenance: `std::process::Command::new("git")` — best effort, no hard dependency on git in sandbox.
3. Add fixture `quonc/tests/fixtures/metrics/bell_baseline.json`.
4. Add `quonc/tests/regression.rs`.

### M3 detail — watch

1. Add `notify` dependency.
2. Implement debounced loop in `quonc/src/watch.rs`.
3. Test: temp file + programmatic write in `quonc/tests/watch.rs` using `std::thread` (set debounce 50ms, touch file, assert stderr contains metrics line within timeout). Mark full subprocess watch test `#[ignore]` (manual-only); **debouncer unit test is the CI gate**.

### M4 detail — documentation

Add `docs/agents/experiment-loop.md`:

1. **Goal**: algorithm iteration workflow.
2. **Basics**: `--metrics`, `--metrics-json`.
3. **Watch loop**: recommended terminal layout (split pane: editor + `quonc --watch`).
4. **Regression workflow**: save before refactor, compare after, tune `tolerances.toml`.
5. **CI example**: GitHub Actions step comparing against committed baseline.
6. Update `README.md` Usage section with a link.

---

## 10. Test strategy

| Layer | What | Where |
| ----- | ---- | ----- |
| Unit | Tolerance math, schema (de)serialize, deny-unknown-fields | `quon_core/tests/metrics.rs` |
| Unit | IR collector on synthetic modules | `mlir_bridge/tests/metrics.rs` |
| Integration | End-to-end JSON metrics for Clifford + universal fixtures | `quonc/tests/metrics.rs` |
| Integration | Snapshot save/compare exit codes | `quonc/tests/regression.rs` |
| Integration | CLI help lists new flags | extend `quonc/tests/cli.rs` |
| Integration | Watch debounce triggers recompile | `quonc/tests/watch.rs` |
| Manual | Two-terminal edit loop on `grover.qn` | PR test plan |

### Validation commands (pre-PR)

```bash
cargo fmt --check
cargo clippy --workspace --exclude flux_verify --all-targets -- -D warnings
cargo test --workspace --exclude flux_verify
npx @taskless/cli@latest check $(git diff --name-only main...HEAD)
```

### CI consideration

Optional non-blocking job: compile reference algorithms, `--metrics-snapshot compare` against committed baselines in `quonc/tests/fixtures/metrics/baselines/`. Start as follow-up issue if M2 is already large.

---

## 11. Error handling conventions

- **`quon_core` / `mlir_bridge`**: `thiserror` + `Result`; no `unwrap` in `src/`.
- **`quonc` lib**: return `anyhow::Result` with context.
- **Watch loop**: compilation errors print diagnostics but do not crash the watcher.
- **Snapshot compare failure**: exit code `1` (regression) vs `2` (tool error — bad baseline path, schema mismatch) — document in help.

---

## 12. Risks and mitigations

| Risk | Mitigation |
| ---- | ---------- |
| Metric drift when passes change | Commit baseline fixtures; review intentional changes in PR |
| `notify` duplicate events / atomic save | Debounce + parent-dir watch filter |
| T-count ambiguity after `native_gate_decomp` | Collect after routing, before second `native_gate_decomp` (decided); test on generic + device_5q targets |
| `depth_bound` still symbolic for parametric programs | Usually `None` after monadic lowering; omit from compare defaults |
| Dynamic `if` over-counting | Document conservative (all-branches) semantics; test in `mlir_bridge/tests/metrics.rs` |
| Simulation optional dep on Python | **Deferred** — `--simulate` not in v1 |

---

## 13. Resolved decisions (formerly open questions)

1. **Metrics JSON vs QASM stdout ordering** — **decided:** stderr for metrics JSON when `--emit-qasm` is active.
2. **Include per-pass timing breakdown?** — defer unless cheap.
3. **Baseline location convention** — `quonc/tests/fixtures/metrics/` for golden files; user projects use `metrics/baselines/<program>.json`.
4. **`--simulate`** — deferred to follow-up issue; not in v1 CLI.

---

## 14. Acceptance criteria mapping

| Criterion | Delivered by |
| --------- | ------------ |
| Watch mode recompiles on source change and prints updated metrics | M3 (`--watch`, M1 metrics printing) |
| Metrics available in machine-readable JSON output | M1 (`--metrics-json`, schema §5) |
| Baseline snapshot save/compare workflow implemented | M2 (`--metrics-snapshot`) |
| Regression threshold config supports per-metric tolerances | M2 (`--regression-config`, §8) |
| Documentation includes workflow for algorithm iteration experiments | M4 |

---

## 15. Reference commands (post-implementation)

```bash
# Daily algorithm iteration
quonc programs/qaoa.qn --watch --target devices/chain5.json --metrics

# Pin a baseline before a pass change
quonc programs/qaoa.qn --target devices/chain5.json \
  --metrics-snapshot save metrics/baselines/qaoa-chain5.json

# After optimization work
quonc programs/qaoa.qn --target devices/chain5.json \
  --metrics-snapshot compare metrics/baselines/qaoa-chain5.json \
  --regression-config metrics/tolerances.toml

# CI-friendly one-liner
quonc programs/qaoa.qn --metrics-json - | jq '.metrics.depth'
```
