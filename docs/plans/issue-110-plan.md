# Issue #110 — Resource estimator: JSON/Markdown reports + regression snapshots

**Status:** PLAN READY FOR RE-REVIEW (amended after CHANGES REQUIRED)  
**Role:** Planning agent (not implementer)  
**Issue:** [#110](https://github.com/arniber21/quon/issues/110)  
**Branch:** `issue-110-resource-report`  
**Worktree:** `/Users/arnabghosh/projects/quon/.worktrees/issue-110`  
**Parent / stack base:** `main` (includes #109 / PR #145; #107 RAP merged)  
**Soft-unblocked past:** #108 (schedule compaction) — do **not** wait  
**Hard prerequisite done:** #109 QEC logical-op layer — **merged on `main`**  
**Upstack consumer:** #112 (full `quonc` NA pipeline + unconditional `quon_na` dep; real emit wiring)

---

## Amendment log (vs prior plan)

Reviewer returned **CHANGES REQUIRED**. This amended plan **locks** the former open points:

| # | Amendment | Resolution |
| - | --------- | ---------- |
| 1 | `estimated_cycles` + `bottleneck` | **Required** v0 fields with defined semantics (below). Not optional. |
| 2 | CLI acceptance | **Option B required**: clap flag present, clear not-wired error, **no** `quon_na` dep yet. |
| 3 | Serde on new required numerics | **`#[serde(default)]`** on every new required numeric / enum field. |
| 4 | Non-QEC Markdown omit-vs-N/A | **Omit** QEC-only rows (do not print `N/A`). |

Soft-unblock #108, invent Markdown sample, insta snapshots, and QEC logical/physical fields are **kept**.

**Open points:** none. Implementer must not reopen these as “decide in PR.”

---

## 1. Goal

Complete the **library surface** for neutral-atom resource reporting, plus a **CLI stub** that satisfies the `--emit-resource-report` acceptance checkbox without fake metrics:

1. Emit `ResourceReport` as **stable JSON** (paper-aligned field names already present).
2. Emit the same report as **Markdown** matching a **new sample** added to `docs/neutral_atom/architecture_model.md` in this PR (living doc; sample does not exist today).
3. Add **logical vs physical** QEC fields from `CodeBlock` / `atoms_per_logical` (#109).
4. Add **required** `estimated_cycles` and `bottleneck` with the semantics locked in §4.2.
5. Lock metrics with **insta** snapshot regression tests.
6. Add **`--emit-resource-report`** to `quonc` as Option B stub (flag present; clear error; no `quon_na` dependency). Real emission lands in #112.

Fidelity (Enola Eq. 1) is **out of scope**.

---

## 2. Current state (repo evidence)

| Piece | Status on `main` |
| ----- | ---------------- |
| `quon_na::ResourceReport` | Exists in `quon_na/src/report.rs` with paper names: `rydberg_stages`, `rearrangement_steps`, `rearrangement_time_us`, transfers, entangle counts, meas/reset rounds, wait/total time |
| `ResourceReport::from_layers` | Aggregates from `&[ScheduleLayer]`; does **not** set sizing / cycles / bottleneck yet |
| JSON serde | `Serialize`/`Deserialize` + `deny_unknown_fields`; unit test pins field set |
| Markdown emitter | **Missing** |
| Logical/physical QEC fields on report | **Missing** |
| `estimated_cycles` / `bottleneck` | **Missing** (issue body requires them) |
| Sample Markdown in architecture_model | **Missing** (doc §9 mentions estimator; §10 has formulas; no report sample) |
| `CodeBlock` / `atoms_per_logical` / `expand_code_block` | Present (`quon_na/src/qec.rs`); demo `examples/repetition_code_toy.rs` |
| `quonc --emit-resource-report` | **Absent**; `quonc` has **no** `quon_na` dependency yet |
| Snapshot patterns | `insta` in `quonfmt`, `frontend`; workspace dep `insta = "1"` |
| #108 compaction | Open; soft-unblocked for this slice |
| #112 | Expects flag to exist and wires real emit + unconditional `quon_na` |

---

## 3. Locked decisions

### From issue triage (unchanged)

1. **Paper field names** — keep `rearrangement_steps`, `rearrangement_time_us`, `rydberg_stages`, etc. Do **not** revive “movement rounds” / “entangling layers” aliases.
2. **Logical vs physical** — from `CodeBlock` / §10 formulas; QEC circuits show both; non-QEC does not invent fake code blocks.
3. **Markdown sample** — invent and add to architecture_model in this PR; emitter must match structurally.
4. **Fidelity** — out of this slice.
5. **Soft-unblock #108** — stack on `main`; compaction only changes numbers later; snapshot refresh is intentional then.

### Locked by this amendment (formerly open)

6. **`estimated_cycles` + `bottleneck` are required** — see §4.2 semantics. No “future / #108” placeholder.
7. **CLI = Option B required** — `--emit-resource-report` clap flag present; clear stderr error; exit non-zero; **do not** add `quon_na` to `quonc` in this slice. #112 owns real emission + unconditional dep.
8. **`#[serde(default)]` on new required numerics** — `logical_qubits`, `physical_atoms`, `estimated_cycles`, and `bottleneck` (and any other new required fields). Keeps `deny_unknown_fields` while allowing old JSON without those keys to deserialize to defaults. Not a deliberate wire break.
9. **Non-QEC Markdown policy = omit** — when `atoms_per_logical` / `code_family` are unset, **omit** those table rows entirely. Do **not** print `N/A`. Sample + emitter + snapshots must agree.

---

## 4. Design

### 4.1 Module layout

Prefer extending `report.rs` rather than a new crate. Optional split only if file grows unwieldy:

```text
quon_na/src/
  report.rs          ← extend ResourceReport + from_layers + builders + emitters
  qec.rs             ← consume (no formula changes)
  lib.rs             ← re-export new types/fns
docs/neutral_atom/
  architecture_model.md  ← NEW §11 sample Markdown report
quon_na/tests/
  report_snapshots.rs    ← insta JSON + Markdown goldens
  snapshots/             ← *.snap
quonc/src/main.rs        ← REQUIRED Option B --emit-resource-report stub
quonc/tests/cli.rs       ← assert flag appears in --help; stub error path
```

Add `insta` as a **dev-dependency** of `quon_na` (workspace already has it).

### 4.2 Data model extensions (normative)

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottleneckKind {
    /// Default / empty schedule / all-zero time components.
    #[default]
    None,
    Rydberg,
    Rearrangement,
    Transfer,
    Measurement,
    /// Two or more categories tie for the maximum score.
    Mixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReport {
    // --- existing (unchanged names) ---
    pub rydberg_stages: u64,
    pub rearrangement_steps: u64,
    pub rearrangement_time_us: u64,
    pub trap_transfers: u64,
    pub transfer_time_us: u64,
    pub entangle2_count: u64,
    pub entangle_n_count: u64,
    pub measurement_rounds: u64,
    pub reset_rounds: u64,
    pub wait_time_us: u64,
    pub total_time_us: u64,

    // --- NEW: sizing / QEC (required numerics; Optionals stay skip-serializing) ---
    #[serde(default)]
    pub logical_qubits: u64,
    #[serde(default)]
    pub physical_atoms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atoms_per_logical: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_family: Option<String>,

    // --- NEW: issue-body required fields ---
    /// Number of schedule layers (= `layers.len()`). See semantics below.
    #[serde(default)]
    pub estimated_cycles: u64,
    #[serde(default)]
    pub bottleneck: BottleneckKind,
}
```

#### `estimated_cycles` semantics (locked)

- **Definition:** `estimated_cycles = layers.len() as u64`.
- **Rationale:** Each `ScheduleLayer` is one parallel cycle in the current IR. Using `layers.len()` is stable, independent of whether `cycle` fields are dense/sparse, and does not need #108 critical-path analysis.
- **Not:** `max(cycle) + 1` — rejected to avoid ambiguity when cycles are non-contiguous or empty layers exist with a high `cycle` id.
- **Empty input:** `from_layers(&[])` → `estimated_cycles = 0`.
- **Set by:** `from_layers` / `build_resource_report` always. Builders that only overlay QEC sizing must **not** clear this field.

#### `bottleneck` semantics (locked)

Classify from metrics **already on the report** after `from_layers` aggregation. No #108 dependency.

**Score each category** (u64):

| Kind | Score |
| ---- | ----- |
| `Rydberg` | `rydberg_stages` (stage count is the Enola/RAP headline; time is not separately tracked for entangle beyond layer max) |
| `Rearrangement` | `rearrangement_time_us` |
| `Transfer` | `transfer_time_us` |
| `Measurement` | `measurement_rounds` (rounds, not action count — matches existing round semantics) |

**Rules:**

1. If **all four scores are 0** → `BottleneckKind::None` (empty or wait-only schedules).
2. Else let `M = max(scores)`. If **exactly one** category equals `M` → that kind.
3. If **two or more** categories equal `M` → `BottleneckKind::Mixed`.
4. `wait_time_us` and `reset_rounds` are **not** bottleneck categories in v0 (wait is idle padding; reset is not a RAP Table I headline). They still appear in schedule metrics tables.
5. Classification uses the scores above **as-is**; do not invent a critical-path or compaction-aware bottleneck. When #108 changes times/counts, bottleneck may change; snapshots update intentionally.

**JSON wire form:** snake_case string enum (`"none"`, `"rydberg"`, `"rearrangement"`, `"transfer"`, `"measurement"`, `"mixed"`). Default deserialize → `None`.

#### QEC / sizing semantics (locked)

| Mode | How to populate |
| ---- | --------------- |
| **Bare `from_layers`** | Schedule metrics + `estimated_cycles` + `bottleneck` set; `logical_qubits = 0`, `physical_atoms = 0`, Options `None`. Documented: sizing is unset until a builder hint. |
| **Non-QEC via `with_physical_atoms(n)`** | `physical_atoms = n`, `logical_qubits = n` (1:1); `atoms_per_logical = None`; `code_family = None`. |
| **QEC via `with_code_blocks`** | logical = Σ `block.logical_qubits.len()`; physical = Σ `block.atoms.len()` (prefer `atoms.len()` over re-deriving); if all blocks share one family, set `atoms_per_logical` from `atoms_per_logical(&family)` and a stable `code_family` label; if mixed families, counts only, leave Options unset. |

**Do not** invent a `CodeBlock` when none is supplied.

**Stable `code_family` labels** (when set):

| `CodeFamily` | Label string |
| ------------ | ------------ |
| `SurfaceCodeLike { .. }` | `surface_code_like` |
| `RepetitionCodeToy { .. }` | `repetition_code_toy` |
| `HighRateQldpcLike { .. }` | `high_rate_qldpc_like` |
| `AbstractBlockCode { .. }` | `abstract_block_code` |

**Demo anchor:** `HighRateQldpcLike` with net rate `1/24` → `atoms_per_logical == 24`, 12 logical → 288 physical ([[144,12,12]]-style). Snapshot this explicitly.

### 4.3 Construction API

```rust
impl ResourceReport {
    pub fn from_layers(layers: &[ScheduleLayer]) -> Self;
    // sets schedule metrics, estimated_cycles = layers.len(), bottleneck from scores;
    // leaves logical_qubits/physical_atoms at 0 and Options at None

    /// Overlay sizing from an explicit physical atom count (non-QEC).
    pub fn with_physical_atoms(mut self, n: u64) -> Self;

    /// Overlay logical/physical from expanded code blocks.
    pub fn with_code_blocks(mut self, blocks: &[CodeBlock]) -> Result<Self, QecError>;
}

pub fn build_resource_report(
    layers: &[ScheduleLayer],
    qec: Option<&[CodeBlock]>,
    physical_atoms_hint: Option<u64>,
) -> Result<ResourceReport, ReportError>;
```

`build_resource_report` preference order:

1. Always start from `from_layers(layers)`.
2. If `qec` is `Some(blocks)` (even empty slice — treat empty as error or no-op: **empty → error** `ReportError::EmptyCodeBlocks` if `Some` was passed intending QEC; prefer: `Some(&[])` leaves sizing at 0; document: callers pass `None` for non-QEC). **Locked:** `qec: None` = non-QEC path; `qec: Some(blocks)` with `blocks.is_empty()` returns `Err(ReportError::EmptyCodeBlocks)`.
3. Else if `physical_atoms_hint` is `Some(n)` → `with_physical_atoms(n)`.
4. Else leave sizing zeros.

`ReportError` wraps `QecError` via `thiserror` (library crate rules).

### 4.4 Emitters

```rust
pub fn resource_report_to_json(report: &ResourceReport) -> Result<String, serde_json::Error>;
// pretty-printed; stable key order via struct field order

pub fn resource_report_to_markdown(report: &ResourceReport) -> String;
```

**JSON:** pretty `serde_json::to_string_pretty`. Keep `deny_unknown_fields`. Update existing `serializes_resource_report_metrics_to_json` for new fields (defaults / omitted Options). Missing keys on deserialize → defaults via `#[serde(default)]`.

**Markdown:** deterministic, no timestamps, no host paths. Structure must match architecture_model §11 **section-for-section**.

**Canonical sample shape** (invent in PR; change sample + emitter + snaps together):

```markdown
# Neutral-atom resource report

## Qubit resources
| Metric | Value |
| --- | ---: |
| Logical qubits | 12 |
| Physical atoms | 288 |
| Atoms per logical | 24 |
| Code family | high_rate_qldpc_like |

## Schedule metrics
| Metric | Value |
| --- | ---: |
| Estimated cycles | … |
| Bottleneck | rydberg |
| Rydberg stages | … |
| Rearrangement steps | … |
| Rearrangement time (µs) | … |
| Trap transfers | … |
| Transfer time (µs) | … |
| Entangle2 count | … |
| EntangleN count | … |
| Measurement rounds | … |
| Reset rounds | … |
| Wait time (µs) | … |
| Total time (µs) | … |

## Notes
- Field names align with TUM RAP Table I / Enola headline metrics.
- `estimated_cycles` is `layers.len()`; `bottleneck` is the max of rydberg stages / rearrangement time / transfer time / measurement rounds (ties → mixed; all-zero → none).
- Non-QEC reports omit atoms-per-logical and code-family rows.
```

#### Non-QEC Markdown omit policy (locked)

- Always emit **Logical qubits** and **Physical atoms** rows (may be `0` if sizing unset).
- Emit **Atoms per logical** and **Code family** rows **only if** the corresponding `Option` is `Some`.
- Never print `N/A` for omitted QEC detail.
- Bottleneck cell text: same snake_case strings as JSON (`rydberg`, `rearrangement`, …).
- Microsecond spelling in headers: `(µs)` (Unicode µ), locked for snapshot stability.

Cross-link from architecture_model §9 to new §11.

### 4.5 Snapshot regression tests

Follow `quonfmt` / `frontend` **insta** pattern (not `quonc --metrics-snapshot`).

**Fixtures (hand-built `ScheduleLayer`s + optional `CodeBlock`s)** — no full Quon→NA pipeline:

| Snapshot name | Intent |
| ------------- | ------ |
| `empty_schedule` | zeros / `bottleneck: none` / `estimated_cycles: 0` |
| `toy_move_entangle_measure` | adapt existing unit-test layers; assert cycles + bottleneck |
| `qec_repetition_d3` | 1 logical → 5 physical (`RepetitionCodeToy`); QEC rows present |
| `qec_qldpc_144_12_12_rate` | 12 logical, rate 1/24 → 24×, 288 physical |
| `non_qec_physical_only` | `with_physical_atoms`; logical == physical; **no** atoms-per-logical / code-family Markdown rows |

For each fixture:

1. `insta::assert_snapshot!(name_json, resource_report_to_json(&r)?)`
2. `insta::assert_snapshot!(name_md, resource_report_to_markdown(&r))`

Optional: one test that Markdown headline numbers match JSON for one fixture.

**Compaction (#108) policy:** snapshots pin **current** `from_layers` semantics. When #108 lands, updating `.snap` files is an **intentional** follow-up — call out in the PR.

### 4.6 Architecture-model doc update

Add **§11 Resource report formats** containing:

- The Markdown sample (canonical), including Estimated cycles + Bottleneck rows.
- Compact JSON example / field list with paper-name mapping to RAP Table I / Enola.
- Logical vs physical rules + pointer to §10.
- Explicit omit policy for non-QEC optional rows.
- Explicit: fidelity estimate deferred; cost weights in §9 remain placeholders.
- `estimated_cycles` / `bottleneck` definitions (same as §4.2).

### 4.7 `quonc` CLI — Option B required (locked)

**Required in this slice:**

```rust
/// Emit a neutral-atom resource report (JSON/Markdown).
/// Full wiring lands in #112; this flag is reserved and currently errors.
#[arg(long, value_name = "PATH", num_args = 0..=1, default_missing_value = "-")]
emit_resource_report: Option<String>,
```

(Exact clap shape may match sibling flags; PATH optional with `-` meaning stdout is fine. What matters: flag is **recognized** by clap.)

**Behavior when flag is present (any PATH):**

1. Do **not** attempt to build a `ResourceReport`.
2. Print to **stderr** a clear message, e.g.  
   `error: --emit-resource-report is not wired yet; resource reports require the neutral-atom schedule path (see #112)`  
3. Exit **non-zero** (`ExitCode::FAILURE` / `bail!`).
4. Do **not** add `quon_na` to `quonc/Cargo.toml` in this PR.
5. Do **not** break OpenQASM / Aer paths when the flag is absent.

**Tests (`quonc/tests/cli.rs`):**

- `--help` lists `--emit-resource-report`.
- Invoking `quonc <source> --emit-resource-report` (or with PATH) fails with stderr containing `not wired` / `#112` (stable substring locked in the test).

**#112 contract:** #112 adds unconditional `quon_na`, implements real emission for this existing flag, and removes the stub error. #110 must not claim end-to-end program→report.

---

## 5. What can ship before #108 vs must stack

| Work | Needs #108? | Notes |
| ---- | ----------- | ----- |
| Extend `ResourceReport` + QEC + cycles/bottleneck | No | Uses #109 on `main` |
| JSON / Markdown emitters | No | |
| architecture_model §11 sample | No | |
| Fixture-based insta snapshots | No | Hand-built layers |
| Option B CLI stub | No | No `quon_na` dep |
| Soft-unblock / PR against `main` | No | Stack on `main`, not on #108 |
| Numbers matching compacted critical path | Yes (later) | Snapshot refresh when #108 merges |
| Full `quonc` program → report | Needs #112 | Stub only here |
| RAP Table I external reproduction | #111 | Separate |

**Stacking:** `main ← issue-110-resource-report` (single PR). Do **not** parent on a future #108 branch.

---

## 6. Worktree + Graphite workflow

```bash
# Already done for planning:
git fetch origin main
git worktree add .worktrees/issue-110 -b issue-110-resource-report origin/main

cd .worktrees/issue-110
# implement → commit via gt create / gt modify
gt submit --no-interactive --no-edit
```

- Trunk: `main` only.
- PR title: `feat(quon_na): resource report JSON/Markdown + snapshots (#110)`
- Body: Fixes #110; cite soft-unblock past #108; note Option B CLI stub → #112 finishes emit; note snapshot refresh expected after compaction; note `estimated_cycles`/`bottleneck` semantics.

Worktree path: `/Users/arnabghosh/projects/quon/.worktrees/issue-110` (created from `origin/main`).

---

## 7. Implementation phases

### Phase 0 — Plan + worktree

- [x] Create worktree + branch from `origin/main`.
- [x] Land this amended plan under `docs/plans/issue-110-plan.md`.
- [ ] Confirm `ResourceReport` / `qec` APIs still match on tip of `main` at implement start.

### Phase 1 — Model + builders

- [ ] Add `BottleneckKind`, sizing fields, `estimated_cycles`, `bottleneck` with `#[serde(default)]` on new required fields.
- [ ] Extend `from_layers` to set `estimated_cycles` + `bottleneck`.
- [ ] Implement `with_physical_atoms` / `with_code_blocks` / `build_resource_report`.
- [ ] Unit tests: QEC aggregation (repetition d=3; QLDPC 24×); bottleneck ties → `Mixed`; empty → `None`; cycles = `layers.len()`.
- [ ] Update existing JSON field unit test (defaults for missing keys).

### Phase 2 — Emitters + living doc sample

- [ ] `resource_report_to_json` / `resource_report_to_markdown` (omit policy locked).
- [ ] Add §11 sample to `architecture_model.md`; link from §9.
- [ ] Unit test: Markdown structure matches sample headings/columns for one golden fixture.

### Phase 3 — Snapshot harness

- [ ] Add `insta` dev-dep; `quon_na/tests/report_snapshots.rs` + committed `.snap` files.
- [ ] Document update recipe: `INSTA_UPDATE=1 cargo test -p quon_na --test report_snapshots`.
- [ ] Ensure CI fails on metric drift without snapshot update.

### Phase 4 — CLI stub (required)

- [ ] Add `--emit-resource-report` clap flag (Option B).
- [ ] Stub: stderr clear error + non-zero exit; **no** `quon_na` dep.
- [ ] Extend `quonc/tests/cli.rs` help + error-path tests.
- [ ] Do not break existing OpenQASM / Aer paths.

### Phase 5 — Validation + submit

- [ ] Run validation (below).
- [ ] `gt submit`; Fixes #110.

---

## 8. Validation

Per `docs/agents/code-quality.md` / `validation.md`:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --exclude flux_verify -- -D warnings
cargo test -p quon_na
cargo test -p quonc
cargo test --workspace --exclude flux_verify
npx @taskless/cli@latest check $(git diff --name-only main...HEAD)
# if flux specs touched on report helpers:
cargo flux -p flux_verify   # only if applicable
```

Library rules: `thiserror`/`Result` in `quon_na` `src/`; no `unwrap`/`expect`/`anyhow` in library `src/`; JSON DTOs keep `deny_unknown_fields`.

---

## 9. Acceptance criteria mapping

| Criterion | How met |
| --------- | ------- |
| JSON resource report | `resource_report_to_json` + paper metrics + cycles/bottleneck + QEC fields |
| Markdown matching architecture-model sample | Sample added in-PR (§11); emitter matches; omit policy locked; snapshot locks it |
| Regression snapshots | insta goldens; drift fails CI |
| Logical vs physical for QEC | `with_code_blocks` + QLDPC/repetition fixtures |
| `--emit-resource-report` flag | Option B stub: flag present, clear error, no `quon_na` yet; #112 finishes |
| Library complete | Phases 1–3 required |
| Fidelity not required | Explicitly omitted |
| Soft-unblock #108 | Stack on `main`; PR cites triage |

---

## 10. Out of scope

- Schedule compaction / ASAP / critical-path from #108 (soft-unblocked; numbers may change later).
- Enola Eq. (1) fidelity product.
- Full `quonc` NA pipeline, `test/na/*.qn`, unconditional `quon_na` dep, real report emission (#112).
- AOD movement (#106), visualization (#113), TUM external number (#111).
- Source-language QEC annotations (forbidden by #109).
- Changing §10 formulas or `CodeFamily` variants.
- Using `max(cycle)+1` for `estimated_cycles` or inventing #108-aware bottleneck.

---

## 11. Risks and mitigations

| Risk | Mitigation |
| ---- | ---------- |
| Snapshot churn when #105/#106/#107/#108 change schedules | Fixture layers are hand-built; refresh only when `from_layers` semantics or fixtures change |
| Mixed-family code blocks ambiguous for `atoms_per_logical` | v0: counts only; single-family demos for snapshots |
| Original issue still lists #108 as blocker | Triage soft-unblock is authoritative; cite it in PR |
| CLI acceptance vs #112 | Option B locks flag introduction here; #112 wires emit |
| Doc/sample vs emitter drift | Single sample in architecture_model; Markdown snapshot must match; change both together |
| Serde field additions break old JSON | `#[serde(default)]` on new required fields; Options skip-serializing |
| `quonc` accidentally depends on incomplete NA path | Stub only; no `quon_na` dep until #112 |
| Bottleneck score units incomparable (stages vs µs vs rounds) | Accepted v0 heuristic; documented; not a fidelity/critical-path claim |

---

## 12. Suggested PR description skeleton

```markdown
## Summary
- Extend `ResourceReport` with logical/physical QEC fields, `estimated_cycles`, and `bottleneck`
- JSON + Markdown emitters; architecture_model §11 sample; insta regression snapshots
- Soft-unblocked past #108; `--emit-resource-report` Option B stub (real emit in #112)

## Test plan
- [ ] `cargo test -p quon_na` and `cargo test -p quonc`
- [ ] Snapshot update documented; intentional drift fails without INSTA_UPDATE
- [ ] QEC fixture shows 24× for [[144,12,12]]-style rate
- [ ] Non-QEC Markdown omits atoms-per-logical / code-family rows (no N/A)
- [ ] `estimated_cycles == layers.len()`; bottleneck ties → mixed; empty → none
- [ ] `quonc --help` lists `--emit-resource-report`; invoking it errors with #112 pointer
- [ ] OpenQASM/Aer path unchanged; no `quon_na` in quonc Cargo.toml yet
```

---

## 13. Locked checklist (no open points)

- [x] Soft-unblock past #108; stack on `main`.
- [x] Invent Markdown sample in architecture_model §11.
- [x] Insta snapshots on hand-built fixtures.
- [x] QEC logical/physical via `CodeBlock` / `atoms_per_logical`.
- [x] `estimated_cycles = layers.len() as u64` (required).
- [x] `bottleneck` via max of rydberg stages / rearrangement µs / transfer µs / measurement rounds; ties → `Mixed`; all-zero → `None` (required).
- [x] CLI Option B required (flag + clear error; no `quon_na` dep).
- [x] `#[serde(default)]` on new required numerics / `bottleneck`.
- [x] Non-QEC Markdown: **omit** optional QEC rows (never `N/A`).
- [x] Fidelity out of scope.
- [x] Multi-block heterogeneous families: counts-only in v0.

---

*End of amended plan. Planning agent only — no implementation was performed.*

**PLAN READY FOR RE-REVIEW**
