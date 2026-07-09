# Issue #108 — Schedule compaction: ASAP baseline + greedy compaction (amended)

**Role**: PLANNING AGENT (not implementer, not reviewer)  
**Issue**: [#108](https://github.com/arniber21/quon/issues/108)  
**Depends on**: [#107](https://github.com/arniber21/quon/issues/107) / [PR #159](https://github.com/arniber21/quon/pull/159) — **merged to `main`** (`9791173`)  
**Downstack layers**: #105 Misra–Gries (`entangling_schedule.rs`) → #107 RAP (`zoned.rs`) → **#108 compaction**  
**Worktree**: `/Users/arnabghosh/projects/quon/.worktrees/issue-108`  
**Branch**: `issue-108-compaction` stacked on **`main`** (B6)  
**Crate focus**: `quon_na` (MLIR-free default path; dialect verify optional *extra*, never the only legality gate)  
**Status**: Amended after adversarial plan review (**CHANGES REQUIRED** on B1–B6), then re-amended for Enola-optimality mis-claim on exclusive-cycle ASAP. This document is the implementation contract.

---

## Amendment log (B1–B6 + re-review)

| ID | Reviewer finding | Locked resolution |
| -- | ---------------- | ----------------- |
| **B1** | ASAP-with-merge packing made AC2 unsatisfiable | **Exclusive-cycle ASAP** (engineering): earliest ≥ preds+1, one layer per cycle (bump on collision), **no content merge**. Greedy is the **only** merge pass. AC2: two disjoint E0 layers → baseline makespan 2 → greedy makespan 1. **Not** Enola-optimal ASAP. |
| **B2** | False “ASAP from RAP Sec. III-A” | Cite **Enola Sec. 3** for the **critical-path lower bound** and for **true ASAP** only. Attribute **RAP Sec. III-A** only to **reuse analysis**. ZAC is not an ASAP citation. Exclusive-cycle ASAP is **not** cited as Enola-optimal. |
| **R1** | Exclusive-cycle ASAP claimed Enola-optimal | **Do not** claim stage-optimality for exclusive-cycle ASAP. Name it an engineering serialization for a merge-free baseline. Enola optimality applies only to **true ASAP** (independent work may share a cycle). |
| **B3** | Physical re-verify was optional / position-blind | When `layout` (+ limits) is present, **mandatory** MLIR-free position-aware R2/R3. AOD M1–M3 only when AOD metadata is non-placeholder; otherwise apply B5 forbid policy. Zone re-validate is best-effort on static bindings — reject merges that change zone-relevant occupancy without a motion simulator. |
| **B4** | Feed-forward fixture collapsed to AtomHazard | AC3: measure on `q_m`, correction on **disjoint** atom(s), **only** explicit `FeedForward` edge (no shared-atom hazard). Document: without caller-supplied FeedForward, compaction cannot claim classical-control safety. |
| **B5** | Unrestricted merge breaks AOD/zone/#107 emission | v0 **allowed merge classes** only (below). AC2 built from an allowed class. |
| **B6** | Stack on stale `issue-107-zoned` | Stack on **`main`** post-#159. Treat #107 AOD placeholders + static zone validate as **hard constraints**, not soft risks. |

Non-blocking (must fix before merge of the implementation PR): N1–N4 in §15.

---

## 1. Problem statement

After #105/#107 (on `main`), schedules are multi-cycle sequences of transfers, moves, and entangles (plus optional measures). Layers are emitted conservatively: segment order is preserved (#105 deferred cross-segment merging to #108), and #107 emits `load → move → store → entangle` as separate cycles per entangling layer. There is no post-pass that:

1. Establishes a clear **exclusive-cycle ASAP baseline** over the physical schedule (engineering merge-free serialization; see §3 — **not** Enola-optimal true ASAP).
2. **Greedily compacts** within **allowed merge classes** (commutation freedom; limited move∥entangle overlap) to cut cycle count.
3. **Never reorders across measurement / feed-forward dependencies** (explicit edges mandatory for classical control).
4. **Re-verifies physical legality** after merges (position-aware R2/R3 when layout present; AOD per B5) — not dependency order alone (#99 / issue comments).
5. **Reports the critical path** in schedule output.

Without this, makespan and idle-atom Rydberg exposure (architecture_model §6 R4) stay inflated even when the dependency DAG allows overlap.

---

## 2. Goal / success definition

Ship a pure `quon_na` post-pass that takes a filled `GraphScheduleRequest` (typically after `schedule_entangling_layers` ± `schedule_zoned`) and returns a compacted schedule plus critical-path metadata:

| Acceptance criterion (issue #108) | How this amended plan satisfies it |
| --------------------------------- | ---------------------------------- |
| ASAP baseline schedule implemented | `asap_schedule_layers`: **exclusive-cycle ASAP** (earliest ≥ preds+1, one layer/cycle, no merge) (B1) |
| Greedy compaction reduces total cycles vs ASAP on ≥1 benchmark | AC2: exclusive-cycle baseline leaves serial independent layers; greedy E0 merges → `compacted_makespan < asap_makespan` (B1, B5). Win is vs the **engineering baseline**, not vs Enola-optimal true ASAP. |
| Measurement/feed-forward ordering never violated | Explicit `FeedForward` on disjoint measure/correction atoms; same-cycle merge forbidden (B4) |
| Critical-path report in schedule output | `CriticalPathReport` recomputed on post-compaction layer DAG (N3) |

Plus brief-mandated: **mandatory** post-merge legality per §6.3 / B3 — not optional.

---

## 3. Literature / design constraints (must state in rustdoc)

### 3.1 Two distinct “ASAP” notions (R1 — locked)

| Notion | What it does | Stage-optimality? | Role in #108 |
| ------ | ------------ | ----------------- | ------------ |
| **True ASAP** | Each layer at `max(preds)+1`; **independent layers may share a cycle** (parallel) | **Yes** for dependency-ordered circuits: makespan = critical-path lower bound (**Enola Sec. 3**) | Literature reference only. Cite for the **lower bound** and for what “optimal ASAP” means. **Not** the v0 baseline API (would make AC2 vacuous without merging inside ASAP). |
| **Exclusive-cycle ASAP** | Same earliest-pred rule, then **bump** so at most one layer per cycle; **never merge** | **No** — serializes independent work (e.g. two disjoint CZs → makespan **2** vs critical path **1**) | **v0 baseline** implemented by `asap_schedule_layers`. **Engineering serialization** for a merge-free baseline so greedy is the only merge pass (B1). |

**Hard rule for rustdoc / §14:** never claim that exclusive-cycle ASAP is Enola-optimal, or that its stage count equals the Enola Sec. 3 critical-path lower bound, except as a **numerical coincidence on total-order (dependency-chain) inputs**. In the general case (any independent layers), exclusive-cycle makespan **strictly exceeds** that lower bound.

**Where Enola Sec. 3 may be cited:**

- Critical-path **lower bound** reporting (`CriticalPathReport.critical_path_length` / longest-path length) — this is a DAG property, independent of which ASAP variant assigned cycles.
- Explaining **true ASAP** (parallel independent work) as the literature-optimal schedule Enola refers to.
- On a **dependency chain** only: exclusive-cycle and true ASAP produce the **same** cycle assignment (no independent work to serialize), so makespan = chain length = critical path. Phrase this as “coincides with the lower bound on chains,” **not** as “exclusive-cycle ASAP is Enola-optimal.”

**Do not cite Enola Sec. 3 as justifying exclusive-cycle ASAP itself.**

### 3.2 Other constraints

1. **Compaction gains** (vs exclusive-cycle baseline) come mainly from recovering legal parallelism that exclusive-cycle deferred (E0 merge), plus limited move∥entangle overlap within allowed classes — **not** from beating true ASAP on a pure dependency chain (there, greedy cannot improve).
2. **Objective**: primary AC2 metric = **cycle makespan** vs the exclusive-cycle baseline (issue text). Secondary: `rydberg_stages` via `ResourceReport::from_layers` (N2). Fidelity motivation: Enola Sec. 2; architecture_model §6 R4 / §9 `w_stage · n_rydberg_stages`.
3. **Attribution (B2 / R1 — locked)**:
   - Critical-path **lower bound** and **true ASAP** optimality → **Enola Sec. 3** (+ issue/#99 notes).
   - Exclusive-cycle ASAP → **engineering glue only** (this issue). **Never** present it as Enola-optimal ASAP.
   - On dependency chains, note numerical coincidence with the lower bound if useful in tests — do **not** upgrade that into a stage-optimality claim for the exclusive-cycle algorithm.
   - RAP Sec. III-A → **reuse analysis only** (architecture_model §4 row “Reuse…”).
   - ZAC → routing-agnostic placement baseline in #107, **not** an ASAP scheduler citation.
   - Critical-path marking reuses Enola-style longest-path logic already in `graph::mark_critical_path` (adapt to schedule **layers**).
4. **Do not claim** Enola Thm. 1 (Misra–Gries) or RAP Eq. (1) optimality for the compaction heuristic.
5. **architecture_model §4**: add a row for schedule compaction (#108) as **engineering glue**, not a paper reproduction (N4).

---

## 4. Upstream contracts and #107 known holes (B6 hard constraints)

From `main` @ `9791173` (merged #159):

| Type / API | Role for #108 |
| ---------- | ------------- |
| `ScheduleLayer { cycle, actions }` | Unit of scheduling; ASAP renumbers `cycle`; greedy may merge actions |
| `NeutralAtomAction` | Move / Transfer / Entangle2\|N / Measure / Reset / Wait — **no conditional opcode** |
| `GraphScheduleRequest { graph, layers, layout }` | Input/output; overwrite `layers`; preserve `graph` + `layout` |
| `ZonedScheduleResult` / `schedule_zoned` | Typical producer; compaction is a **post-pass**, not inside `schedule_zoned` |
| `schedule_entangling_layers` (#105) | Graph-only layers; primary AC2 producer (entangle-only merges) |
| `ScheduleLayer::validate_conflicts` / `validate_occupancy` | Always re-run after merge |
| `validate_zone_constraints` | Static `initial_bindings` only — **not** a motion simulator (see hole below) |
| `ResourceReport::from_layers` | Secondary `rydberg_stages` metric |
| Dialect `verify_entangling_geometry` / AOD checks (`dialect.rs`) | Geometry logic to **extract/share** into MLIR-free helpers; do not hide R2/R3 behind `feature = "mlir"` |

### #107 holes compaction must assume (hard, not “risks”)

1. **Placeholder AOD refs**: `schedule_zoned` emits `AodTrapRef { aod_id: 0, row: 0, col: 0 }` on every transfer. Running M1–M3 on merged transfers is meaningless or always-fail. → **B5 forbids** merging Transfer/Move layers from #107 until real AOD metadata exists (or fail-closed if attempted).
2. **Static zone validate**: `validate_zone_constraints` uses `layout.initial_bindings` sites; it does **not** replay atom motion across layers. Re-running it after merge does not prove zone legality of a compacted moving schedule. → Reject merges that change zone-relevant occupancy without a position-replay simulator; document best-effort for entangle-only merges that do not move atoms.
3. **`Entangle2` has no coordinates**: R2/R3 need positions from `layout` (+ site coordinates). Without layout, position-aware checks are skipped and merges that require them are rejected when `legality` is configured (see §6.3).

**Important gap**: `NeutralAtomAction` has `Measure` but **no classical conditional**. Feed-forward deps are **caller-supplied** `ScheduleDependencyKind::FeedForward` (B4).

---

## 5. API / module layout

### 5.1 New module

```text
quon_na/src/
  compaction.rs          ← NEW: exclusive-cycle ASAP + greedy compaction + critical path + legality helpers
  lib.rs                 ← mod + re-exports
  schedule_entry.rs      ← rustdoc: point to compaction post-pass (#108)
  schedule.rs            ← optional thin helpers for atom sets / action kind
  report.rs              ← NOT required to absorb CriticalPathReport in #108 (defer #110)
```

Follow `#104 place()` / `#105 schedule_entangling_layers()`: **new entry points**; do not change `schedule_from_graph` stub or auto-run compaction inside `schedule_zoned`.

### 5.2 Public API (proposed)

```rust
/// Hard ordering constraints that compaction must never violate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDependency {
    /// Predecessor layer index in the input schedule (pre-compaction).
    pub before: u32,
    /// Successor layer index that may not move earlier than `before` completes.
    pub after: u32,
    pub kind: ScheduleDependencyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleDependencyKind {
    /// Same atom appears in both layers (data dependence).
    AtomHazard,
    /// Explicit barrier / segment boundary.
    Barrier,
    /// Mid-circuit measurement must complete before dependent correction.
    Measurement,
    /// Classical feed-forward: measure → conditional correction (caller-supplied).
    FeedForward,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriticalPathReport {
    /// Number of cycles = max(cycle)+1 after renumber / compaction.
    pub makespan_cycles: u32,
    /// Longest dependency-chain length in **post-merge cycle vertices** (N3).
    pub critical_path_length: u32,
    /// Stable **pre-merge input layer indices** that lie on some longest path
    /// (layers that were merged share the successor's cycle but keep ids listed).
    pub critical_layer_indices: Vec<u32>,
    /// Best-effort interaction ids when layers map cleanly to entangles.
    pub critical_interaction_ids: Vec<InteractionId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionResult {
    pub request: GraphScheduleRequest, // layers overwritten + renumbered
    pub asap_makespan_cycles: u32,
    pub compacted_makespan_cycles: u32,
    pub critical_path: CriticalPathReport,
    /// True if greedy pass merged at least one pair of layers.
    pub compacted: bool,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CompactionError {
    #[error("empty schedule")]
    EmptySchedule,
    #[error("schedule layer conflict after compaction: {0}")]
    Conflict(String),
    #[error("occupancy conflict after compaction: {0}")]
    Occupancy(String),
    #[error("zone constraint violated after compaction: {0}")]
    Zone(String),
    #[error("physical legality violated after compaction: {0}")]
    PhysicalLegality(String),
    #[error("merge class forbidden in v0: {0}")]
    ForbiddenMergeClass(String),
    #[error("feed-forward / measurement dependency would be violated")]
    DependencyViolation,
    #[error("invalid dependency edge {0:?} → {1:?}")]
    InvalidDependency(u32, u32),
    #[error("layout required for position-aware legality")]
    LayoutRequired,
}

pub struct LegalityLimits {
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub aod_min_separation_um: f64,
}

pub struct CompactionOptions {
    /// When set with layout, run zone checks after compaction (best-effort; see B3).
    pub arch: Option<ZonedArchitecture>,
    /// When set **with** `request.layout`, enable mandatory position-aware R2/R3.
    pub legality: Option<LegalityLimits>,
    /// If true, run greedy compaction after ASAP; if false, ASAP-only baseline.
    pub greedy: bool,
}

/// Exclusive-cycle ASAP: earliest ≥ preds+1, one layer/cycle; **never merges** (B1).
/// Engineering baseline — **not** Enola-optimal true ASAP (§3.1).
pub fn asap_schedule_layers(
    req: GraphScheduleRequest,
    deps: &[ScheduleDependency],
) -> Result<CompactionResult, CompactionError>;

/// Exclusive-cycle ASAP then greedy merge within allowed classes; re-verify legality.
pub fn compact_schedule(
    req: GraphScheduleRequest,
    deps: &[ScheduleDependency],
    opts: &CompactionOptions,
) -> Result<CompactionResult, CompactionError>;

/// Infer AtomHazard deps only. Does **not** invent FeedForward (B4).
pub fn infer_atom_dependencies(layers: &[ScheduleLayer]) -> Vec<ScheduleDependency>;

/// Build measure→correction FeedForward edges from an explicit pairing list.
pub fn feed_forward_dependencies(
    measure_layer: u32,
    correction_layers: &[u32],
) -> Vec<ScheduleDependency>;
```

**Design decisions (locked for implementer):**

1. Compaction operates on **layers as vertices**. Merging = union of `actions` into one layer at a shared cycle, then re-validate.
2. **Exclusive-cycle ASAP (B1)**: earliest time ≥ all preds+1; **at most one layer per cycle**; on collision, bump the later layer (by input index). **Never union actions.** Greedy is the only pass that merges. **Not** Enola-optimal true ASAP (§3.1).
3. `schedule_zoned` / `schedule_entangling_layers` unchanged; compose:  
   `schedule_entangling_layers → schedule_zoned? → compact_schedule`.
4. Serde DTOs: `#[serde(deny_unknown_fields)]`.
5. Errors: `thiserror`; no `unwrap`/`expect`/`anyhow` in `src/`.
6. **N1**: rustdoc must state `#108` exclusive-cycle ASAP is a **physical-layer engineering baseline**; it does **not** replace `#105` `asap_buckets` / Enola interaction ASAP, and it is **not** Enola-optimal true ASAP (§3.1).

---

## 6. Algorithms

### 6.1 Dependency DAG construction

**Input**: ordered `layers[0..n)`, plus `deps`.

**Default**: `infer_atom_dependencies(layers) ∪ caller deps` (dedupe by `(before, after, kind)`).

- **AtomHazard**: if layer `i` and layer `j` (`i < j`) share an `AtomId` in any action, add edge `i → j` (consecutive uses per atom is enough).
- **Caller-supplied** `Barrier`, `Measurement`, `FeedForward` edges.
- **Never invent FeedForward** from Measure alone (B4). Without caller FeedForward, classical control on other qubits is **not** protected — document in rustdoc and AC3.

**Barrier semantics**: forbid merge across the barrier cut (edges from every pre-barrier layer to barrier and barrier to every post-barrier layer, or equivalent).

### 6.2 Exclusive-cycle ASAP baseline (B1 — locked)

Process layer indices in increasing order (stable). For each `i`:

```
t = max( asap[p] + 1 for p in preds[i] )  // or 0 if no preds
while some already-scheduled j has asap[j] == t:
  t += 1
asap[i] = t
```

Emit `ScheduleLayer { cycle: asap[i], actions: layers[i].actions.clone() }` — **never union actions**.

`asap_makespan_cycles = max(asap[i]) + 1`.

**Semantics vs Enola (§3.1):**

- Pred-respecting: never starts a layer before its predecessors finish.
- **Not stage-optimal:** serializes independent layers → makespan can **exceed** the Enola Sec. 3 critical-path lower bound (AC2 fixture: 2 vs 1).
- On a **pure dependency chain** of length L, exclusive-cycle and true ASAP agree numerically (makespan = L); that is coincidence from lack of independent work — **not** an Enola-optimality claim for exclusive-cycle ASAP.
- True ASAP would place the AC2 independent layers on the same cycle; exclusive-cycle deliberately does not until greedy merges.

### 6.3 Allowed merge classes (B5 — locked) + legality (B3 — locked)

#### v0 allowed classes

| Class | Allowed? | Conditions |
| ----- | -------- | ---------- |
| **E0** — merge two **entangle-only** layers (no Move/Transfer/Measure/Reset in either) | **Yes** | Disjoint entangling atoms; `validate_conflicts` + `validate_occupancy`; if `layout`+`legality`: position-aware R2/R3 on union; no Barrier/Measurement/FeedForward forbidding collapse |
| **M0** — overlap **move-only** layer with **entangle-only** layer on **disjoint atoms** | **Yes, only if** | Geometry R2/R3 pass on the entangle side **and** AOD M1–M3 pass on the move side with **non-placeholder** AOD metadata; else reject |
| **T\*** — merge any layers containing `Transfer` | **No in v0** | #107 placeholder AOD (`aod_id/row/col` all 0) → fail-closed via `ForbiddenMergeClass` |
| **X\*** — merge load/store/transfer across different RAP steps; merge Measure with its correction; merge layers sharing placeholder AOD row/col | **No in v0** | `ForbiddenMergeClass` / `DependencyViolation` |
| Merge two entangle stages whose atoms were placed for incompatible pair geometry | **No** | Rejected by position-aware R2/R3 when layout present |

AC2 **must** use class **E0** (or M0 with real AOD fixtures — prefer E0 for v0).

#### Greedy loop

After exclusive-cycle ASAP:

```
repeat until no improvement:
  for candidate pairs (i, j) in deterministic order
      (cycle[j] >= cycle[i], prefer adjacent cycles first):
    if dependency kinds forbid collapsing i..j: skip
    if merge_class(i, j) ∉ allowed: skip  // ForbiddenMergeClass if forced
    if can_merge_software(i, j) AND legal_after_merge(i, j):
      union actions into one layer; remove the other
      renumber cycles densely (or re-run exclusive-cycle ASAP on remaining layers)
      accept if makespan_cycles decreases
```

**Greedy order (deterministic)**: candidates sorted by `(cycle[j], cycle[i], i, j)` ascending; first legal improving merge wins; restart scan.

**`can_merge_software`**:

1. No Barrier / Measurement / FeedForward forbidding collapse.
2. Merge class allowed (B5).
3. `validate_conflicts` on union.
4. `validate_occupancy` on union.

**`legal_after_merge` (mandatory when applicable — B3)**:

| Check | When | How |
| ----- | ---- | --- |
| R2 compulsion + R3 isolation | `opts.legality` **and** `layout` present; merge involves entangle | MLIR-free geometry helper: resolve atom → site → `(x,y)` from layout; port distance loops from `dialect.rs` `verify_entangling_geometry` into `compaction` (or shared `legality` private module). **Default-features / `--no-default-features` path — not behind `mlir` only.** |
| AOD M1–M3 | Merge involves Move with AOD-bearing transfers/moves | Run coupled-motion / axis checks **only** if AOD refs are non-placeholder; else `ForbiddenMergeClass` |
| Zone constraints | `opts.arch` + layout; entangle-only merges that do not relocate atoms | `validate_zone_constraints` best-effort; **reject** merges that include Move/Transfer changing occupancy (no motion simulator in v0) |
| Layout missing but legality requested | Entangle merge needing R2/R3 | `LayoutRequired` or reject merge |

**Remove** all “optional physical verify” / “pragmatic skip” language from the acceptance path. Optional `#[cfg(feature = "mlir")]` ScheduleSpec round-trip remains an **extra** test, not the gate.

**Placeholder detection (locked)**: treat `AodTrapRef { aod_id: 0, row: 0, col: 0 }` as placeholder when every transfer in the candidate layers uses that triple (matches #107 emission). Fail closed for T\*/M0 involving those transfers.

### 6.4 Never reorder across measurement / feed-forward (B4 — locked)

**Rules:**

1. Explicit `FeedForward` / `Measurement` edges: measure layer ≺ correction layer(s). Compaction may not assign `cycle(correction) ≤ cycle(measure)` in a way that merges them into the same cycle or swaps order.
2. Same-cycle merge of Measure with its correction is **forbidden**.
3. Inferred AtomHazard alone is **insufficient** to claim feed-forward safety when correction atoms ≠ measured atom.

**AC3 fixture (mandatory shape):**

```text
Layer 0: Entangle2(q0, q1)           // setup (optional)
Layer 1: Measure(q0)                 // mid-circuit on q_m = q0
Layer 2: Entangle2(q2, q3)           // independent work (disjoint atoms)
Layer 3: Entangle2(q2, q4)           // "correction" — atoms DISJOINT from q0
```

Deps: **only** `FeedForward { before: 1, after: 3 }` (plus AtomHazard edges that do **not** connect measure→correction — correction atoms `{q2,q4}` share no atom with Measure(`q0`)).

Assert:

- After compaction, `cycle(L1) < cycle(L3)`; never same cycle; never swapped.
- Attempted merge of L1 with L3 → `DependencyViolation`.
- Without the FeedForward edge, L3 could legally merge with L2 (document trust boundary).

No new IR opcode.

### 6.5 Critical-path reporting (N3)

1. Build layer DAG from deps (+ inferred).
2. After compaction, vertices are **post-merge layers** (one per remaining layer); map back to pre-merge input indices for `critical_layer_indices`.
3. `asap[i]`, `longest_suffix[i]`, mark critical iff `asap[i] + longest_suffix[i] == global_max` on the **post-compaction** DAG.
4. Fill `CriticalPathReport` on both ASAP-only and compacted results (recompute after greedy).

---

## 7. Acceptance criteria mapping

| # | Criterion | Deliverable | Test |
| - | --------- | ----------- | ---- |
| AC1 | Exclusive-cycle ASAP baseline | `asap_schedule_layers` / `compact_schedule` with `greedy: false` | `asap_chain_equals_critical_path` — 3 dependent entangles → 3 cycles; **no merge**; chain case matches critical path |
| AC2 | Greedy reduces cycles vs exclusive-cycle ASAP | `compact_schedule` greedy | `greedy_reduces_vs_asap_e0` — two disjoint E0 layers → exclusive-cycle makespan 2 → greedy 1; `compacted < asap` |
| AC3 | Measure/feed-forward never violated | deps + merge guards | `mid_circuit_feed_forward_disjoint` — fixture §6.4; illegal merge → `DependencyViolation` |
| AC4 | Critical-path report | `CriticalPathReport` | `critical_path_marks_longest_chain`; serde round-trip |
| Brief | Re-verify physical legality | post-merge validators | `merge_rejected_when_r2_r3_violated` (layout+positions); `forbidden_merge_transfer_layers`; `zone_reject_move_merge_without_simulator` |

### AC2 fixture (satisfiable by design — B1)

Under **true ASAP**, two independent layers both get time 0 → makespan 1, so greedy cannot beat that baseline on makespan without merging inside ASAP (forbidden by B1). To keep AC2 meaningful, lock **exclusive-cycle ASAP** as the engineering baseline:

```
asap[i] = max(asap[p] + 1 for p in preds[i], else 0)
# at most one layer per cycle: if cycle asap[i] already taken by some j < i, bump asap[i] += 1 until free
# NEVER union actions
```

**Fixture `greedy_reduces_vs_asap_e0`:**

```text
L0: Entangle2(0,1)    # entangle-only
L1: Entangle2(2,3)    # entangle-only, disjoint atoms
deps: none (no AtomHazard between L0 and L1)
critical-path length = 1
true ASAP makespan would be 1
```

| Pass | Result |
| ---- | ------ |
| Exclusive-cycle ASAP | L0 @ cycle 0, L1 @ cycle 1 → `asap_makespan = 2` (serialized; **not** Enola-optimal) |
| Greedy E0 | merge → one layer @ cycle 0 → `compacted_makespan = 1` |
| Assert | `compacted_makespan < asap_makespan` (win vs **engineering** baseline) |

Rustdoc: “exclusive-cycle ASAP = engineering merge-free baseline (serializes independent work); greedy recovers legal E0 parallelism. Enola Sec. 3 optimality refers to **true ASAP**, not this baseline. AC2 measures improvement over exclusive-cycle ASAP, not over the critical-path lower bound.”

---

## 8. Tests (concrete)

### 8.1 Unit (`quon_na/src/compaction.rs` / `quon_na/tests/compaction.rs`)

| Test | Asserts |
| ---- | ------- |
| `asap_dependency_chain_matches_critical_path` | Chain of 3 AtomHazard-linked entangle layers → makespan 3 (= critical path). Phrase as chain coincidence with lower bound; **not** “exclusive-cycle is Enola-optimal.” |
| `asap_exclusive_cycle_serializes_independent` | Two disjoint entangles, no deps → exclusive-cycle makespan **2** (> critical path 1); documents **non**-optimality |
| `greedy_reduces_vs_asap_e0` | **AC2**: same fixture → greedy merge → makespan **1**; `compacted_makespan < asap_makespan` |
| `asap_does_not_union_actions` | After ASAP-only, `layers.len()` unchanged |
| `measure_feed_forward_disjoint` | **AC3**: §6.4 fixture; order preserved |
| `cannot_merge_measure_with_correction` | Same-cycle / swap rejected (`DependencyViolation`) |
| `feed_forward_not_inferred` | Measure + disjoint correction **without** FeedForward edge → no FeedForward dep invented |
| `barrier_blocks_cross_merge` | Explicit Barrier |
| `forbidden_merge_transfer_layers` | Two Transfer layers from zoned-like placeholder AOD → `ForbiddenMergeClass` |
| `merge_rejected_when_r2_r3_violated` | Layout with too-close non-partners → reject |
| `r2_r3_runs_without_mlir_feature` | `--no-default-features` test with layout+legality |
| `critical_path_report_populated` | **AC4**; recomputed after merge |
| `zoned_entangle_only_passthrough` | Entangle-only compact on toy layout; zone best-effort ok |
| `infer_atom_dependencies_shared_atom` | Shared atom induces edge |

### 8.2 Property tests (`quon_na/tests/compaction_props.rs`)

- Random commutation graphs: after entangling schedule + compaction with inferred deps only, every layer `validate_conflicts` Ok; makespan ≤ exclusive-cycle ASAP makespan; for pure `DependencyDag` chains, compacted makespan == exclusive-cycle ASAP makespan (== chain length / critical path).
- Determinism: same input → same cycles.

### 8.3 Optional MLIR (`feature = "mlir"`)

- Extra: convert compacted layers to `ScheduleSpec` and `verify` — not the acceptance gate.

---

## 9. Implementation order (tiny commits)

Each commit leaves `cargo test -p quon_na --no-default-features` green.

1. **Scaffold** — `compaction.rs`, error/result types, re-exports; stub identity / `EmptySchedule`.
2. **Dependency inference** — `infer_atom_dependencies` + FeedForward helper; tests that FeedForward is never inferred.
3. **Exclusive-cycle ASAP** — `asap_schedule_layers` + critical-path report + AC1 + `asap_exclusive_cycle_serializes_independent` (documents makespan > critical path on independent layers).
4. **Merge class gate + software can_merge** — E0 only first; forbid Transfer merges.
5. **Position-aware R2/R3** — shared geometry helper; `merge_rejected_when_r2_r3_violated`.
6. **Greedy loop** — AC2 `greedy_reduces_vs_asap_e0`.
7. **Measure / feed-forward** — AC3 disjoint fixture.
8. **Zoned integration** — entangle-only path; document Transfer forbid; zone best-effort.
9. **Docs** — rustdoc per §14: exclusive-cycle ASAP = engineering baseline (**not** Enola-optimal); Enola Sec. 3 only for lower bound + true ASAP (+ chain coincidence note); RAP III-A reuse only; `architecture_model.md` §4 engineering-glue row; N1 dual-ASAP note.
10. **Props + validation** — proptest; fmt/clippy/taskless.

---

## 10. Stack / worktree / Graphite (B6)

```bash
# Already done by planner:
git fetch origin main
git worktree add .worktrees/issue-108 -b issue-108-compaction origin/main
cd .worktrees/issue-108
gt track --parent main --no-interactive
```

**Do not** stack on `issue-107-zoned` — PR #159 is merged; local zoned tip is not the integration parent.

**PR title**: `feat(quon_na): ASAP baseline + greedy schedule compaction (#108)`  
**PR body**: Fixes #108; based on `main` (post-#159).  
**Do not** commit on `main`. Prefer `gt submit --no-interactive --no-edit`.

Plan artifact: `docs/plans/issue-108-plan.md` (this file).

---

## 11. Validation commands

```bash
cargo fmt --all -- --check
cargo clippy -p quon_na --no-default-features --all-targets -- -D warnings
cargo test -p quon_na --no-default-features
npx @taskless/cli@latest check $(git diff --name-only main...HEAD)
```

Optional MLIR extra:

```bash
cargo test -p quon_na --features mlir --test quantum_na_dialect
```

Broader pre-PR if needed:

```bash
cargo test --workspace --exclude flux_verify
```

---

## 12. Out of scope

- Replacing RAP (#107) or Misra–Gries (#105); inventing new movement models (#106).
- Full ALAP scheduling.
- Pulse-level / heating / atom-loss models.
- Resource Markdown reports (#110).
- Benchmark reproduction lock to RAP Table I (#111).
- New `quantum.na` conditional opcode.
- Auto-compact inside `schedule_from_graph` / `schedule_zoned`.
- Claiming asymptotic optimality for greedy compaction.
- Implementing real AOD row/col assignment for #107 placeholders (follow-up).
- Full atom-motion zone simulator (follow-up); v0 rejects move merges that need it.
- M0 move∥entangle in v0 **unless** a fixture supplies non-placeholder AOD metadata (default: ship E0 only; M0 code path may exist but tests can defer).

---

## 13. Risks and mitigations (post-amendment)

| Risk | Mitigation |
| ---- | ---------- |
| AC2 unsatisfiable under merge-ASAP | **Fixed (B1)**: exclusive-cycle ASAP + greedy-only merge |
| Misreading AC2 as beating Enola-optimal ASAP | **Fixed (R1)**: rustdoc + §3.1 distinguish exclusive-cycle vs true ASAP; AC2 is vs engineering baseline |
| Physical verify only behind `mlir` | **Fixed (B3)**: MLIR-free R2/R3 when layout+limits present |
| #107 placeholder AOD | **Fixed (B5)**: forbid Transfer merges in v0 |
| Static zone validate | **Fixed (B3)**: reject occupancy-changing move merges; best-effort entangle-only |
| Feed-forward = AtomHazard | **Fixed (B4)**: disjoint-atom explicit FeedForward fixture |
| Wrong stack parent | **Fixed (B6)**: `main` @ post-#159 |
| Dual ASAP confusion (#105 vs #108) | N1 rustdoc |
| Critical path vertex identity | N3: post-compaction layers + pre-merge index list |

---

## 14. Docs / attribution checklist

- [ ] `compaction.rs` module docs (**R1** — copy this wording closely):
  - Baseline name: **exclusive-cycle ASAP** — engineering serialization for a merge-free baseline.
  - **Not stage-optimal.** Independent layers are serialized (makespan can exceed the critical-path lower bound).
  - **True ASAP** (independent work may share a cycle) is what **Enola Sec. 3** stage-optimality refers to.
  - Cite Enola Sec. 3 only for: (1) critical-path **lower bound** reporting, (2) describing true ASAP, (3) optional note that on **dependency chains** exclusive-cycle and true ASAP coincide numerically.
  - **Forbidden phrase** about `asap_schedule_layers`: “stage count equals the Enola / critical-path lower bound” (except the chain-coincidence note above).
  - RAP III-A = reuse only; not an ASAP citation.
  - Compaction gains = recovering legal E0 parallelism deferred by exclusive-cycle (+ allowed overlap); R4 stage objective secondary.
- [ ] Dual-ASAP note (N1): `#108` exclusive-cycle physical-layer **engineering** baseline vs `#105` `asap_buckets` / Enola interaction ASAP.
- [ ] `schedule_entry.rs`: mention #108 post-pass.
- [ ] `lib.rs` crate docs: one line in the #103–#107 pipeline list for compaction.
- [ ] `architecture_model.md` §4: add row — schedule compaction (#108) = **engineering glue** (exclusive-cycle ASAP + greedy E0 merge); not a paper reproduction; **not** Enola-optimal ASAP.
- [ ] Do **not** attribute compaction to RAP Eq. (1) or Enola Thm. 1.
- [ ] Document FeedForward trust boundary (B4).

---

## 15. Non-blocking (fix before implementation PR merge)

| ID | Issue | Fix |
| -- | ----- | --- |
| N1 | Dual ASAP: #105 `asap_buckets` vs #108 exclusive-cycle | Rustdoc as §14 |
| N2 | Cycles vs Rydberg stages | AC2 primary = cycle makespan vs exclusive-cycle baseline; `rydberg_stages` secondary via `ResourceReport` |
| N3 | Critical path after merges | Recompute on post-compaction DAG; `critical_layer_indices` = pre-merge ids; report critical-path **length** separately from exclusive-cycle makespan |
| N4 | `architecture_model` §4 row | Engineering glue row for #108 |

---

## 16. Decision summary (for re-review)

| Decision | Choice |
| -------- | ------ |
| Module | `quon_na/src/compaction.rs` |
| Entry points | `asap_schedule_layers`, `compact_schedule` |
| ASAP (B1) | **Exclusive-cycle ASAP**; **no content merge**; **not** Enola-optimal |
| True ASAP (Enola Sec. 3) | Literature / lower-bound reporting only — not the v0 baseline API |
| Greedy | Only merge pass; deterministic; **E0** primary; Transfer forbid |
| Citations (B2/R1) | Enola Sec. 3 → critical-path lower bound + **true ASAP** only; exclusive-cycle → engineering (not optimal); RAP III-A → reuse only |
| Legality (B3) | Mandatory position-aware R2/R3 when layout+limits; no mlir-only gate |
| Feed-forward (B4) | Disjoint atoms + explicit FeedForward only |
| Merge classes (B5) | E0 yes; Transfer/placeholder AOD no; M0 deferred unless real AOD |
| Stack (B6) | `.worktrees/issue-108` / `issue-108-compaction` on **`main`** |
| Critical path | Post-compaction DAG + pre-merge layer indices |
| HITL | Flag if legality shortcuts, Transfer merges, or Enola-optimality claims for exclusive-cycle sneak in |

---

## 17. Verdict path

1. **This amended plan** → adversarial **re-review** (expect APPROVED or further deltas).  
2. Implementer: worktree already on `main`; implement per §9.  
3. Validate §11.  
4. `gt submit` PR, `Fixes #108`.  
5. Code review against AC table §7.

---

**PLAN READY FOR RE-REVIEW**

*End of amended plan. Planner only — no implementation was performed.*
