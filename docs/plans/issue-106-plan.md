# Issue #106 — AOD-constrained movement planner baseline (no zones)

**Role**: Planning agent (amended after adversarial review)  
**Issue**: [#106](https://github.com/arniber21/quon/issues/106) — *AOD-constrained movement planner baseline (no zones)*  
**Depends on**: [#105](https://github.com/arniber21/quon/issues/105) (merged as PR [#158](https://github.com/arniber21/quon/pull/158)); [#102](https://github.com/arniber21/quon/issues/102) dialect verifiers  
**Branch / worktree**: `issue-106` at `.worktrees/issue-106`, **stacked on `origin/main`**  
**Stack base (locked)**: `#105` and `#107` are already on `origin/main` (PRs #158 / #159 merged). **Do not** stack on `issue-105`. Reuse `movement_duration_us` / `euclidean_um` from `zoned.rs` (or extract a shared module) — **no duplicate √-law helpers**.  
**Literature anchors**: [Enola] Sec. 5 (**three conflict types** + greedy longest-first IS *idea* only — **not** Enola one-atom duals); `docs/neutral_atom/architecture_model.md` §5 (M1–M5, √-law) and §6 (R1–R3); geometry-gated adjacency skip inspired by [RAP] Sec. III-A (already-adjacent only when R1–R3 would pass — **not** zoned RAP routing). **Quon** pipeline: interaction-pair bank + both-atom duals + B7 packing (**B9**).

**Review status**: B1–B8 retained; **B9–B13** lock the five adversarial blockers from review `5d03ba68`; **B14** locks the remaining re-review blocker (bank↔placement isolation vs dialect `≤`). **B9** Enola attribution honesty (Quon both-atom/pair-bank/B7 ≠ Enola one-atom duals). **B10** geometry-gated skip (no illegal in-place entangle on dense `#104` grid). **B11** dialect-identical R1–R3 predicates + **all occupied** atoms. **B12** conflict oracle includes M3 `min_row_col_separation_um`. **B13** B8 half-pair / partial-overlap reclaim (`partial_overlap_pair_reclaimed`). **B14** bank origin `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`; isolation test asserts placement↔bank **`>`** `min_rydberg_spacing_um` (default 18.75 satisfiable).

---

## 1. Goal

Implement the **flat** (non-zoned) AOD movement planner that, for each entangling layer produced by #105 and a placement layout from #104 (after interaction-pair bank enlargement — §5.0), turns adjacency requirements into **legal parallel rearrangement steps**:

1. Per 2Q gate: **Quon** dual **orientations** onto a free **interaction pair** (two pre-existing sites with gap ≤ \(r_b\)); typically **both** atoms move onto that pair — two mutually exclusive candidates per gate (**not** Enola’s one-atom “move either endpoint” duals; see **B9**).
2. Move-conflict graph: Enola Sec. 5’s **three conflict types per axis**, plus dialect M3 separation (**B12**), capacity, and site conflicts.
3. Greedy **maximal** independent sets in the **sortIS spirit** (distance-sorted, longest first) — Enola *idea*; applied first to Quon duals, then again in the B7 leg-packing pass.
4. Cost: \(t = \sqrt{d_{\max}/a}\) + trap transfers; report rounds/time.
5. Skip already-adjacent pairs **only when** a full R1–R3 check would pass without moves (**B10** / **B11**); otherwise force bank duals (or fail closed).
6. Collision detection (M5) with **cross-cycle** occupancy + empty-trap transfer rule.
7. Emit **explicit** `Transfer` layers around moves, then grouped `NeutralAtomAction::Move(MovementGroup)`, so `quantum.na` AOD + entangling-geometry verifiers can check legality.
8. Before each entangle: enforce **R1–R3** with dialect predicates over **all occupied** atoms (**B11**).

This is **not** [RAP] zoned joint placement-routing (#107) and must not be documented as such. `#107` already ships zoned RAP on `main`; `#106` is the separate **flat** line (Enola-inspired conflicts + Quon pair-bank pipeline).

---

## 2. Acceptance criteria mapping

| Acceptance criterion (issue + brief) | How the plan satisfies it |
| --- | --- |
| Movement respects AOD row/column coupling (verifier-aligned) | Emit `MovementGroup`s that are ISs under Enola three-types **+ M3 separation (B12)** + capacity/sites; Quon dual selection + **B7** packing; every group passes `verify_aod_legality` (incl. unequal-y). |
| Collision detection rejects two atoms on same site same cycle | Maintain **cross-cycle** occupancy map; reject destination collisions and transfer-into-occupied; also call `ScheduleLayer::validate_occupancy` per layer (necessary but not sufficient alone). |
| Movement cost model implemented and reported | Reuse `movement_duration_us` / `euclidean_um`; `duration_us = ceil(√(d_max_m / a) · 1e6)` + `n_transfers · trap_transfer_us`; expose in result + `ResourceReport::from_layers`. |
| Movement rounds counted in schedule output | Each packed parallel AOD step → one move layer; result field `rearrangement_steps`. |
| Redundant moves skipped | Skip only if partners ≤ \(r_b\) **and** full R1–R3 over all occupied atoms would pass without moves (**B10**); else relocate to bank. |
| Docs cite Enola Sec. 5 / architecture_model §5; not RAP zoned | Cite Enola **only** for three conflict types + greedy longest-first IS *idea*; label pair-bank / both-atom duals / B7 as **Quon** (**B9**). Do **not** claim #107 / RAP A* or “Enola duals = this algorithm.” |
| Verifier-aligned entangles (architecture §6) | `check_entangling_geometry` uses dialect `≤` predicates over **all occupied** atoms before every entangle (**B11**). |

---

## 3. Upstream / downstream contracts

### 3.1 Inputs (on current `main`)

| API | Role |
| --- | --- |
| `GraphScheduleRequest` | `graph` + `layers` (entangling from `schedule_entangling_layers`) + `layout: Some(NeutralAtomLayout)` from `place`, then enlarged by interaction-pair bank (§5.0) |
| `ScheduleLayer` with `Entangle2` / `EntangleN` | Per-layer gate set to realize |
| `NeutralAtomLayout` | Sites with `Position { x_um, y_um }`; bindings (`TrapBinding::Slm` from #104) |
| `MovementParams` (plain DTO — **no hard `backend` dep**) | See §4.2; includes `min_rydberg_spacing_um` |
| Shared geometry (already on main) | `zoned::movement_duration_us`, `zoned::euclidean_um` — **reuse**; optionally extract to `geometry.rs` and re-export from both modules |

### 3.2 Outputs (existing schedule types)

| Type | Role |
| --- | --- |
| `NeutralAtomAction::Transfer(TrapTransfer)` | **Required** SLM↔AOD pick-up/drop-off around every move (M4) |
| `NeutralAtomAction::Move(MovementGroup { moves, duration_us })` | Grouped rearrangement step |
| `AtomMove { atom, from, to }` | Per-atom displacement inside the group |
| `ScheduleLayer::validate_occupancy` / `validate_conflicts` | Per-layer invariants after emission |
| Cross-cycle occupancy map (planner-internal) | Global empty-trap / collision truth across cycles |
| `ResourceReport::from_layers` | Counts rearrangement steps/time and transfers |

### 3.3 Dialect verifier bridge (`quantum.na`, #102)

Schedule `AtomMove` is **site-id only**. Dialect `MoveSpec` needs `aod_id`, `row`, `col`, `from_*_um`, `to_*_um` for M1–M3 (`verify_coupled_motion` / `verify_axis_order_and_separation`).

Entangling geometry uses `verify_entangling_geometry` (R1–R3). Planner must satisfy those constraints in Rust **before** emit; tests may also lower to `ScheduleSpec` / `ActionSpec` and call the dialect path when the `mlir` feature is available.

**Plan decision (v0):**

1. Planner works in **coordinate + AOD-index space** internally (`CandidateMove` with src/dst positions and assigned AOD row/col).
2. Emit schedule `Transfer` + `AtomMove` / `MovementGroup` as the primary product.
3. Provide pure helper `atom_moves_to_move_specs(...) -> Vec<MoveSpec>` used by unit tests that call `verify_aod_legality`.
4. Full MLIR emission is **not** required in the #106 PR; “verifier-aligned” means emitted groups pass the **same rules** the verifier encodes, proven by tests.

### 3.4 Pipeline order (caller convention)

```text
schedule_from_graph(graph)
  → place(req, strategy)                         // #104: compact SLM grid
  → ensure_interaction_pairs(&mut layout, …)     // #106: pair bank (B1)
  → schedule_entangling_layers(req, cap)         // #105: entangle-only layers
  → plan_aod_movement(req, params)               // #106: expand with transfers/moves
```

`plan_aod_movement` **requires** non-empty layout (with an interaction-pair bank) and existing entangling layers; it **rewrites** `layers` into a finer cycle sequence (load → move rounds → store → entangle → optional return), renumbering `cycle` monotonically. It may call `ensure_interaction_pairs` itself if the layout has no pair bank yet (idempotent).

---

## 4. Module / API design (repo patterns)

### 4.1 Files

```text
quon_na/src/
  movement.rs              ← NEW: interaction-pair bank, duals, conflict graph, sortIS, R1–R3, emission
  geometry.rs              ← OPTIONAL extract: euclidean_um + movement_duration_us (from zoned.rs)
  zoned.rs                 ← re-export / call shared helpers; do NOT duplicate formulas
  schedule_entry.rs        ← docs: #106 fills movement into layers
  lib.rs                   ← mod + re-exports
  report.rs                ← reuse as-is
tests/
  movement.rs              ← unit tests (incl. B1–B8 locks)
  movement_props.rs        ← proptest
```

**B3 lock — shared helpers:**

- Prefer `use crate::zoned::{euclidean_um, movement_duration_us}` from `movement.rs`, **or**
- Extract both into `quon_na/src/geometry.rs`, have `zoned` and `movement` import from there, re-export from `lib.rs` for stability.
- **Forbidden**: a second copy of the √-law formula in `movement.rs`.

### 4.2 Public API (sketch)

```rust
/// Parameters for flat AOD movement (#106). Sourced from NeutralAtomTarget JSON
/// by callers; kept crate-local to avoid a backend dependency (same pattern as #105).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementParams {
    pub acceleration_m_s2: f64,       // default 2750
    pub trap_transfer_us: u64,        // default 15
    pub rydberg_range_um: f64,        // R1 / adjacency / skip threshold
    /// R3 isolation: non-partners must be farther than this (generic_rna: 18.75 = 2.5×r_b).
    pub min_rydberg_spacing_um: f64,
    pub min_row_col_separation_um: f64,
    pub aod_rows: u32,
    pub aod_cols: u32,
    pub num_aods: u32,                // v0: typically 1; M2/M3 bind per aod_id
    /// Intra-pair gap (µm) between the two sites of one interaction pair.
    /// Must satisfy 0 < pair_gap_um ≤ rydberg_range_um (R1 for partners).
    /// Default: 2.0 (matches generic_rna entanglement-zone pair_gap_um).
    pub pair_gap_um: f64,
    /// Center-to-center pitch (µm) between distinct interaction pairs.
    /// Must be ≥ min_rydberg_spacing_um. Inter-pair / placement↔bank isolation
    /// still needs dialect-strict `>` min (B11/B14): bank origin adds
    /// `BANK_ISOLATION_EPS_UM` so default pitch == min remains satisfiable.
    /// Default: 18.75.
    pub pair_pitch_um: f64,
    /// Quon reuse default: 2 transfers per *moved atom* (SLM→AOD + AOD→SLM) with
    /// return_home=false. Enola-comparable: return_home=true (4 per moved atom).
    /// Do NOT document the default as Enola Sec. 2/6.1.
    pub transfers_per_moved_atom: u32,
    /// When true, after entangle emit return moves to home SLM sites (Enola-comparable 4-xfer).
    pub return_home: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementPlanResult {
    pub request: GraphScheduleRequest, // layers rewritten; layout updated to final occupancy
    pub rearrangement_steps: u64,
    pub rearrangement_time_us: u64,
    pub trap_transfers: u64,
    pub transfer_time_us: u64,
    pub skipped_already_adjacent: u64,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum MovementPlanError {
    #[error("layout is required for AOD movement planning")]
    MissingLayout,
    #[error("no entangling layers to plan movement for")]
    EmptySchedule,
    #[error("acceleration_m_s2 must be positive, got {0}")]
    InvalidAcceleration(f64),
    #[error("rydberg_range_um must be positive, got {0}")]
    InvalidRydbergRange(f64),
    #[error("min_rydberg_spacing_um must be positive, got {0}")]
    InvalidMinRydbergSpacing(f64),
    #[error("pair_gap_um ({gap}) must be in (0, rydberg_range_um={rb}]")]
    InvalidPairGap { gap: f64, rb: f64 },
    #[error("pair_pitch_um ({pitch}) must be >= min_rydberg_spacing_um ({min})")]
    InvalidPairPitch { pitch: f64, min: f64 },
    #[error("missing site or binding for atom {0:?}")]
    MissingAtom(AtomId),
    #[error("collision: site {site:?} claimed by multiple atoms in cycle {cycle}")]
    Collision { cycle: u32, site: SiteId },
    #[error("transfer into occupied site {site:?} at cycle {cycle}")]
    TransferIntoOccupied { cycle: u32, site: SiteId },
    #[error("AOD capacity exceeded: need {needed} {axis} but target allows {limit}")]
    AodCapacity { axis: &'static str, needed: u32, limit: u32 },
    #[error("unsatisfiable move set under AOD conflicts for layer cycle {0}")]
    Unsatisfiable(u32),
    #[error("no free interaction pair for gate ({lhs:?}, {rhs:?}) at cycle {cycle}")]
    NoInteractionPair { cycle: u32, lhs: AtomId, rhs: AtomId },
    #[error("entangling geometry violation (R1–R3) at cycle {cycle}: {detail}")]
    EntanglingGeometry { cycle: u32, detail: String },
    #[error("multi-qubit entangle (k={k}) unsupported by flat movement planner at cycle {cycle}")]
    UnsupportedEntangleArity { cycle: u32, k: usize },
    #[error("schedule layer conflict: {0}")]
    Conflict(String),
}

/// One reserved interaction pair: two empty sites with gap ≤ r_b.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionPair {
    pub left: SiteId,
    pub right: SiteId,
}

/// Ensure layout has a bank of interaction pairs (B1).
/// Idempotent: if a pair bank already exists (tagged via returned metadata or
/// detectable as appended pair sites), no-op.
/// Returns the list of pairs (site-id couples) for the planner to assign.
pub fn ensure_interaction_pairs(
    layout: &mut NeutralAtomLayout,
    params: &MovementParams,
    min_pairs: usize,
) -> Result<Vec<InteractionPair>, MovementPlanError>;

/// Expand entangling layers with AOD-legal movement (Quon pair-bank duals +
/// Enola-inspired conflict types / greedy longest-first IS; see B9).
pub fn plan_aod_movement(
    req: GraphScheduleRequest,
    params: &MovementParams,
) -> Result<MovementPlanResult, MovementPlanError>;
```

**Conventions** (code-quality / Taskless):

- `thiserror` for `MovementPlanError`; no `anyhow` in `quon_na`.
- No `unwrap`/`expect` in `src/` (tests may use them).
- `#[serde(deny_unknown_fields)]` on all public DTOs.
- Pure helpers for conflict checks; reuse shared √-law duration (optional Flux already on `zoned` helpers — do not fork specs).

### 4.3 Defaults

`MovementParams::generic_rna_v0()` matching `targets/neutral_atom/generic_rna_v0.json` / architecture_model §8.6:

| Field | Default | Notes |
| --- | --- | --- |
| `acceleration_m_s2` | 2750 | √-law |
| `trap_transfer_us` | 15 | |
| `rydberg_range_um` | 7.5 | R1 |
| `min_rydberg_spacing_um` | 18.75 | R3 (= 2.5 × r_b) |
| `pair_gap_um` | **2.0** | ≤ \(r_b\); partners interact (R1) |
| `pair_pitch_um` | **18.75** | ≥ `min_rydberg_spacing_um`; bank uses `+ BANK_ISOLATION_EPS_UM` so placement↔bank / inter-pair distances are dialect-strict `>` min (**B14**) |
| `min_row_col_separation_um` | 2.0 | M3 / dialect |
| `aod_rows` / `aod_cols` / `num_aods` | 100 / 100 / 1 | |
| `transfers_per_moved_atom` | **2** | **Quon reuse policy** — not Enola |
| `return_home` | **false** | atoms stay on interaction-pair sites for later reuse |

Enola-comparable mode for tests / fidelity accounting: `return_home: true` and `transfers_per_moved_atom: 4`.

**Entry validation (fail closed):**

- `0 < pair_gap_um ≤ rydberg_range_um`
- `pair_pitch_um ≥ min_rydberg_spacing_um` (pitch may equal min; **B14** ε on bank origin supplies the dialect-strict gap)
- `pair_pitch_um > pair_gap_um` (pairs do not overlap)

---

## 5. Algorithm (precise, cited)

### 5.0 Destination geometry — interaction-pair bank (**B1 — locked**)

**Rejected model (do not implement):** sparse interaction lattice at pitch = `min_rydberg_spacing_um` (18.75 µm) outside the placement bbox + partner-stationary duals requiring a destination within \(r_b = 7.5\) µm of a partner still on the placement grid. Impossible under defaults (bank gap ≥ 18.75 > 7.5).

**Locked model:** interleaved **interaction pairs**; duals move **both** atoms onto a free pair (two orientations). Partners become adjacent via `pair_gap_um ≤ r_b`, not by parking next to a stationary partner. Inspired by EZ pair layout in `generic_rna_v0.json` / architecture §6–7 — **flat engineering glue**, not RAP zoned routing.

#### Geometry (single formula — no alternatives; **B14** locked)

**B14 — bank isolation vs dialect `≤` (locked fix):** Dialect R3 rejects non-partner distance **`≤`** `min_rydberg_spacing_um` (exact 18.75 fails). Validating only `pair_pitch_um ≥ min` and asserting placement↔bank **`≥ pair_pitch_um`** leaves default geometry unsatisfiable when pitch == min. **Do not** require `pair_pitch_um > min_rydberg_spacing_um` (defaults keep pitch = 18.75).

**Locked constants / formula:**

```text
/// Extra µm beyond pair_pitch so placement↔bank edge is dialect-strict.
/// Module const in movement.rs; rustdoc + tests name this symbol.
const BANK_ISOLATION_EPS_UM: f64 = 0.01;

x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM
y0 = mid_y of placement bbox   // or 0.0; fixed in rustdoc + tests

for i in 0..min_pairs:
  L_i = (x0 + i * pair_pitch_um,  y0 + (i % 2) * pair_pitch_um)
  R_i = (L_i.x + pair_gap_um,     L_i.y)
```

Defaults: `pair_gap_um = 2.0`, `pair_pitch_um = 18.75`, `rydberg_range_um = 7.5`, `min_rydberg_spacing_um = 18.75`, `BANK_ISOLATION_EPS_UM = 0.01`.

| Quantity | Value under defaults | Constraint |
| --- | --- | --- |
| Intra-pair `‖R−L‖` | 2.0 | ≤ \(r_b\) (R1) |
| Adjacent-pair `‖L_i − L_{i+1}‖` | \(18.75\sqrt{2} ≈ 26.5\) | Must satisfy dialect R3: distance **`>`** `min_rydberg_spacing_um` (reject `≤`; **B11**) |
| Bank↔placement gap | `pair_pitch_um + BANK_ISOLATION_EPS_UM` (= 18.76) | Must be **`>`** `min_rydberg_spacing_um` under dialect `≤` reject (**B11**/**B14**). Test `bank_outside_placement_isolated` asserts min placement↔bank distance **`>`** `min_rydberg_spacing_um` (not `≥ pair_pitch_um`). |

`min_pairs = max(1, W_max)` where `W_max` = max number of `Entangle2` actions in any **single** input layer.
Bank is **not** sized to total gates across layers — multi-layer reuse is handled by **B8 eviction** (below).

`ensure_interaction_pairs` appends these empty sites (ids after `max(existing)+1`), leaves placement bindings untouched, returns `Vec<InteractionPair>`. Idempotent if bank already present. Destinations **must** be `layout.sites` ids — no virtual coordinates.

#### Dual generation (Quon both-atom orientations — **B9**)

For unsatisfied gate \((a,b)\) and each free pair \(P=(L,R)\) (both sites empty in the cross-cycle map):

| Dual | Moves |
| --- | --- |
| \(d_0\) | \(a → L\), \(b → R\) |
| \(d_1\) | \(a → R\), \(b → L\) |

- Omit a leg if the atom is already at that site (ε).
- Mutual exclusion: at most one dual / one pair per gate.
- sortIS-style distance of a dual = max leg length in its move-set.
- Conflict oracle for **selecting** duals treats each dual as a unit (mutual exclusion + pair-site claims). Intra-dual leg legality is handled at **emission** (B7 + **B12**).
- No free pair → `NoInteractionPair` / `Unsatisfiable` (after B8/B13 eviction attempt between layers).
- **Forbidden:** “move \(a\) to a site within \(r_b\) of stationary \(b\) on the placement grid.”
- **Forbidden in docs:** calling these “Enola duals.” Enola Sec. 5 duals are one-atom “move either endpoint” 4-tuples; this table is Quon engineering on the pair bank.

After a chosen dual, partners sit `pair_gap_um` apart (R1 by construction). Non-partners on other pairs / placement grid stay isolated only if distances satisfy dialect `≤` rejects (**B11**). Still run the explicit R1–R3 check before entangle (B2/B11).

#### B7 — Emit dual legs without breaking order preservation (**locked**)

**Problem.** Horizontal pairs share one `y`. If partners start at different `y` (typical after `#104` place), co-scheduling both legs in one `MovementGroup` yields `src_y(a) ≠ src_y(b)` but `dst_y(a) = dst_y(b)`, which violates Enola / dialect **order preservation** (M2) and fails `verify_aod_legality`.

**Locked emission rule (serialize dual legs):** Because v0 pairs are **horizontal** (shared dst \(y\)), **never** place both non-zero legs of one both-atom dual into the same `MovementGroup`. Always emit them as separate packed groups (order: longer Euclidean leg first; tie → lower `AtomId`). This is the chosen fix (not vertical pairs).

After sortIS selects a set \(S\) of duals:

1. Expand each dual into ordered legs (skip zero-length / already-at-dest).
2. **Force-split:** legs from the same dual must start in distinct pending slots (cannot enter the same pack round together).
3. Pack pending legs into successive `MovementGroup`s with a **second greedy pass** under the full Enola conflict oracle (order preservation, coupling, site conflicts). Legs from *different* duals may share a group when legal.
4. Each packed group uses the §5.5 load → move → store pattern.
5. Every emitted group **must** pass `verify_aod_legality` after `atom_moves_to_move_specs` (B4).

**Test (B7):** `unequal_y_partners_pass_aod_verifier` — `#104` place two atoms on different rows (unequal `src_y`); one far `Entangle2`; `return_home` either; after `plan_aod_movement`: (a) plan `Ok`, (b) the two relocation legs are in **different** `MovementGroup`s, (c) every group lowers to `MoveSpec`s that pass `verify_aod_legality`.

#### B8 — Multi-layer pair occupancy under `return_home=false` (**locked**; extended by **B13**)

**Problem.** Default leaves atoms on pair sites after entangle. A bank sized to `W_max` fills after the first layer; later layers with new gates see no free pairs → spurious `NoInteractionPair`.

**Locked rule — bank size \(W_{\max}\) + inter-layer eviction + half-pair reclaim** (not “size bank to total gates”):

Record each atom’s **home** site at plan start (`#104` placement `TrapBinding::Slm` site).

Under `return_home=false`:

1. **Same-atom reuse:** If a next-layer gate’s two atoms already occupy both sites of one interaction pair (any orientation) with distance ≤ \(r_b\), **and** the gate would pass the geometry-gated skip predicate (**B10**), treat as already-adjacent (no moves; pair stays theirs).
2. **Ownership:** A pair occupied by \(\{a,b\}\) is assignable only to gates on exactly those atoms until vacated.
3. **Between input entangling layers** (after entangle, before next layer’s dual generation), run **B13 half-pair reclaim** first (below), then let
   `need =` count of next-layer gates that are **not** already sitting on a reusable pair,
   `free =` count of pairs with both sites empty.
4. While `need > free`: pick an occupied pair whose **both** atoms are non-participants in the next layer (atom ∉ next layer’s gate atom set); **evict** both atoms to their home SLM sites (transfer+move under B7); mark the pair free; recount `free`. Eviction order: ascending pair index.
5. If still `need > free` → `NoInteractionPair { .. }` (fail closed). With B13 reclaim + non-participant eviction, any schedule whose every layer width ≤ \(W_{\max}\) must succeed without silent exhaustion — locked by tests.
6. Atoms that **do** participate in the next layer stay on their pairs (reuse) unless a dual reassigns them or B13 vacates their pair.

Under `return_home=true`: after each entangle, return all bank occupants home; all pairs free; no B8/B13 eviction pass; 4-xfer Enola-comparable mode.

#### B13 — Half-pair / partial-overlap reclaim (**locked**)

**Problem.** B8 step 4 only evicts when **both** occupants are non-participants; same-atom reuse needs **both** on one pair. Partial overlap (e.g. layer-1 left \(\{a,b\}\) on pair \(P\); layer-2 gate \((a,c)\)) leaves \(P\) non-evictable under step 4 and non-reusable under step 1 → `free` can stay too low despite `width ≤ W_max`, contradicting “eviction is sufficient.”

**Locked vacate/reassign rule** (run between layers, before dual generation, after computing next-layer gate atom set \(A\) and gate set \(G\)):

For each occupied interaction pair \(P\) with occupants \(\{u,v\}\):

| Case | Condition | Action |
| --- | --- | --- |
| Full reuse | Some \(g \in G\) is exactly \(\{u,v\}\) and B10 skip would pass | Keep \(P\); no moves |
| Full non-participant | \(u \notin A\) and \(v \notin A\) | Eligible for B8 step-4 eviction (both home) |
| **Partial overlap** | Exactly one of \(\{u,v\}\) is in \(A\), **or** some \(g \in G\) shares exactly one atom with \(\{u,v\}\) | **Reclaim:** (1) move the **orphan** (the occupant not needed as partner for a full-reuse gate on \(P\)) home via transfer+move (B7); (2) if the remaining occupant must leave \(P\) to join a new dual (always when the next gate is not \(\{u,v\}\)), also move that atom off \(P\) as part of the subsequent dual — but **immediately** mark both sites of \(P\) free for assignment once the orphan is home and the shared atom has been scheduled to leave (implement as: vacate **whole pair** to home first, then generate duals onto free pairs). **v0 lock:** vacate **both** atoms of a partial-overlap pair to home (same emission as full eviction), then free \(P\). Do **not** leave a half-occupied pair. |
| Orphan-only / stuck | After reclaim + B8 eviction, still `need > free` | `NoInteractionPair` (documented fail-closed — never claim “sufficient” without B13) |

**Forbidden:** leaving one atom on \(P\) while the other is gone; treating partial overlap as “owned, non-evictable” without reclaim; returning silent `NoInteractionPair` when reclaim would have freed a pair.

**Test (B13):** `partial_overlap_pair_reclaimed` — layer 1: `Entangle2(a,b)` → both on some pair \(P\); layer 2: `Entangle2(a,c)` with \(c\) disjoint, `return_home=false`, width ≤ \(W_{\max}\). Assert plan `Ok`; before layer-2 dual assign, \(P\) is fully vacated (both sites empty or reassigned); \(a\) and \(c\) end on some free pair; no spurious `NoInteractionPair`.

**Tests (B8 + B13):**

| Test | Fixture / asserts |
| --- | --- |
| `multi_layer_reuse_default_no_exhaustion` | ≥2 layers, each width ≤ \(W_{\max}\), `return_home=false`, layer-2 gates on a **disjoint** atom set (forces eviction). Plan `Ok`; pairs vacated before layer-2 assign; no bank-exhaustion `NoInteractionPair`. |
| `multi_layer_same_atom_reuse_no_eviction` | Layer 2 repeats a gate on the **same** atom pair already on a bank pair **and** B10 skip passes. Plan `Ok`; `skipped_already_adjacent ≥ 1`; zero eviction moves for that gate. |
| `partial_overlap_pair_reclaimed` | Layer-2 shares exactly one atom with an occupied pair → whole pair vacated; plan `Ok` (**B13**) |

**Tests that lock B1:**

| Test | Asserts |
| --- | --- |
| `pair_gap_leq_rb` | Every pair: `euclidean(left,right) ≤ rydberg_range_um` |
| `pair_pitch_isolates_non_partners` | Min cross-pair site distance > `min_rydberg_spacing_um` |
| `bank_outside_placement_isolated` | Min placement-site → pair-site distance **`>`** `min_rydberg_spacing_um` (**B14**; not `≥ pair_pitch_um`) |
| `dual_moves_both_atoms_onto_pair` | Far gate → both atoms relocate onto some pair (unless already there) |
| `partner_reachable_under_defaults` | `generic_rna_v0` params + `place` + bank + one far edge → plan **Ok** (not geometrically stuck) |
| `dual_dest_is_existing_site_id` | Every `AtomMove.to` ∈ `layout.sites` |
| `no_virtual_coords` | No destination outside `layout.sites[*].position` |
| `invalid_pair_gap_rejected` | `pair_gap_um > r_b` → `InvalidPairGap` |
| `no_partner_stationary_parking` | Dual candidates never target a site whose only R1 neighbor is a stationary partner still on the placement grid |
| `unequal_y_partners_pass_aod_verifier` | Different src y → plan Ok + all groups pass `verify_aod_legality` (B7) |
| `multi_layer_reuse_default_no_exhaustion` | Disjoint layer-2 atoms → eviction (B8) |
| `multi_layer_same_atom_reuse_no_eviction` | Same atoms stay on pair when B10 skip passes (B8) |
| `partial_overlap_pair_reclaimed` | Half-pair / one-atom overlap → whole pair vacated; plan Ok (B13) |
| `skip_requires_geometry_ok` | Dense `#104` neighbors within \(r_b\) but R2/R3 fail → **no** skip; bank duals or `EntanglingGeometry` (B10) |
| `dense_placement_skip_not_ok` | Regression: place + adjacent gate + idle neighbors → must not emit in-place entangle that fails dialect R1–R3 (B10/B11) |
| `check_geometry_matches_dialect_leq` | Distance exactly `min_rydberg_spacing_um` → reject (dialect `≤`); all occupied atoms in scope (B11) |
| `m3_separation_conflict_in_oracle` | Two legs with dest axis delta `< min_row_col_separation_um` conflict in planner oracle **and** fail `verify_aod_legality` (B12) |

### 5.1 What we claim (and do not)

| Claim | Source | Status |
| --- | --- | --- |
| **Quon** dual = both-atom orientations onto a free interaction pair (two mutually exclusive move-sets) | Quon engineering (pair bank §5.0); **not** Enola Sec. 5 | Implement (**B9**) |
| Enola Sec. 5 duals = one atom move / gate (“move either endpoint”) | [Enola] Sec. 5; issue #106 comment | **Do not implement / do not claim as this algorithm** (**B9**) |
| Three conflict types per axis on moves-as-(src,dst) | [Enola] Sec. 5; architecture_model §5 M1–M3 | Implement |
| Dialect M3 dest separation (`\|dst_i − dst_j\| < min_row_col_separation_um`) in packing oracle | `dialect.rs` `verify_axis_order_and_separation` | Implement (**B12**) |
| Parallel round = independent set; **sortIS spirit** = greedy maximal IS on distance-sorted list (longest first) | [Enola] Sec. 5 *idea*; applied to Quon duals + B7 legs | Implement; docs must not say “implements Enola sortIS dual selection” (**B9**) |
| windowIS (window of K longest) | [Enola] Sec. 5 | **Out of scope for v0** |
| \(t = \sqrt{d/a}\), \(a = 2750\,\mathrm{m/s^2}\); +15 µs/transfer | [Enola] Sec. 2; [RAP] Sec. VI-B; architecture_model §5 | Reuse shared helper; **not** Atomique’s flat 300 µs |
| Geometry-gated already-adjacent skip | [RAP] Sec. III-A reuse *idea*, flattened; **B10** gates on full R1–R3 | Implement; **not** bare “distance ≤ \(r_b\) ⇒ skip” |
| Occupancy / empty-trap transfer | M5; [OLSQ-DPQA] App. D Eq. 16 | Cross-cycle map + explicit transfers |
| R1–R3 before entangle (dialect `≤`, all occupied) | architecture_model §6; dialect `verify_entangling_geometry` | Implement (**B11**) |
| Interaction-pair bank (`pair_gap` ≤ \(r_b\), inter-pair isolation) | architecture §6–7; EZ pair layout analogy (not RAP routing) | Implement (§5.0) |
| Partner-stationary parking on sparse lattice outside bbox | — | **Rejected** (geometrically incompatible with \(r_b\)) |
| Both dual legs in one MovementGroup | — | **Rejected** when they break M2 (B7: serialize) |
| Bank sized to total gates / no eviction | — | **Rejected**; size \(W_{max}\) + inter-layer eviction (B8) + half-pair reclaim (B13) |
| Zoned RAP A* / joint placement-routing | [RAP] | **#107 only** (already on main) |
| Default 2 transfers = Enola | — | **Do not claim** (B6) |
| Issue comment “move either endpoint” = shipped algorithm | — | **Do not claim** (**B9**) |

sortIS-style selection is a **greedy maximal** independent set, **not** KaMIS MIS. Do not imply Enola optimality. Do not equate Quon both-atom duals with Enola one-atom duals.

### 5.2 Per entangling layer loop

For each input layer \(L\) containing entangling actions:

1. **Snapshot occupancy**  
   Maintain `BTreeMap<SiteId, AtomId>` and `BTreeMap<AtomId, SiteId>` across the whole plan (not per-layer only). Sites not in the map are empty.

2. **Collect 2Q pairs**  
   From `Entangle2`. For `EntangleN` with `atoms.len() != 2`: return `UnsupportedEntangleArity` (**N1** — dedicated error, not `Unsatisfiable`). `#105` already rejects multi-qubit commutation graphs; this covers DAG-emitted `EntangleN`.

3. **Geometry-gated already-adjacent skip (**B10**)**  
   For pair \((a,b)\): if `euclidean_um(pos(a), pos(b)) ≤ rydberg_range_um` **and** a provisional `check_entangling_geometry` over **current occupancy of all atoms** (with this gate’s partners marked as the only allowed ≤\(r_b\) pair among the layer’s entangle set — see **B11**) would **pass without any moves**, mark gate **satisfied**; increment `skipped_already_adjacent`; generate **no** candidate moves.  
   Otherwise (distance ≤ \(r_b\) but geometry would fail — typical on `#104` `SITE_PITCH_UM = 5.0` with idle neighbors within \(r_b\) / within `min_rydberg_spacing_um`): **do not skip**; treat as unsatisfied and generate bank duals.  
   **N3 (rewritten):** If **all** pairs in the schedule are geometry-gated-skippable → success with `rearrangement_steps = 0`, entangle layers preserved. If some/all are distance-adjacent but geometry-illegal in place → they **must** relocate (or fail `EntanglingGeometry` / `NoInteractionPair`); **never** `Ok` with zero moves while leaving dense-grid R2/R3 violations. `EmptySchedule` remains only when input `layers` is empty / has no entangling actions.

4. **Dual candidates (Quon orientations onto interaction pairs — §5.0; **B9**)**  
   For each unsatisfied gate \((a,b)\) and each free interaction pair \(P = (L, R)\):
   - Dual \(d_0\): atom moves \(\{a→L,\ b→R\}\) (omit a leg if already at dest).
   - Dual \(d_1\): atom moves \(\{a→R,\ b→L\}\).  
   Each dual is a **candidate move-set** (1–2 `AtomMove`s) with AOD indices from §5.4 on every leg.  
   Distance of a dual for sortIS-style selection = max Euclidean leg length in that set.  
   Do **not** generate partner-stationary “park next to \(b\) on the placement grid” candidates.  
   Do **not** document these as Enola Sec. 5 duals.

5. **Select duals via conflict graph + sortIS-style greedy**  
   - Build conflict graph over **all** dual candidates (across gates and pairs).
   - Mutual exclusion: the two orientations of the same gate; also any two duals of the same gate that use different pairs (at most one pair per gate).
   - Site conflicts: duals whose move-sets share a destination site.
   - Greedy longest-first maximal set \(S\); gates not covered remain for the next round (retry with remaining free pairs).

6. **Move-conflict graph (Enola three types + dialect M3 separation — **B12**)**  
   Represent each candidate leg by source/dest coordinates and AOD indices.

   For the **row (Y) axis** (column/X axis analogous):

   | Conflict type | Condition (conflict edge if violated) | Hardware meaning |
   | --- | --- | --- |
   | Same source row ⇒ shared destination row | Two moves share source row index but different destination rows | M1/M3 coupling / no split |
   | Same destination row ⇒ shared source row | Share dest row but different source rows | M3 no-merge |
   | Order preservation | \(\mathrm{src}_y(m) > \mathrm{src}_y(m')\) but \(\mathrm{dst}_y(m) \le \mathrm{dst}_y(m')\) (match verifier `total_cmp`) | M2 no crossing |
   | **Dest separation (M3)** | Same `aod_id`; \(\lvert \mathrm{dst}_y(m) - \mathrm{dst}_y(m') \rvert <\) `min_row_col_separation_um` (and X analog). Match dialect: reject when abs-delta **`<`** separation (not `≤`). | Dialect `verify_axis_order_and_separation` |

   Also add conflicts for:
   - **Same atom** in two candidates.
   - **Same destination site** (M5).
   - **AOD capacity**: more distinct active rows/cols than `aod_rows`/`aod_cols`.

   Constraints bind **per `aod_id`**. The packing pass (**B7**) **must** use this full oracle (including M3 separation), not only the three Enola types.

7. **sortIS-style dual selection**  
   - Pending = all dual candidates for yet-unsatisfied gates.  
   - Sort by dual distance (max leg) **descending**.  
   - Greedy: add dual if it conflicts with none already chosen (mutual exclusion / pair sites); produce maximal set \(S\).  
   - Gates not covered remain for the next selection round after emission frees capacity.

8. **Emit selected duals with B7 leg serialization**  
   - Expand \(S\) into individual legs.  
   - **Force-split (B7):** the two non-zero legs of any single dual are **never** eligible for the same pack round (horizontal pairs share dst \(y\); unequal `src_y` would break M2).  
   - Pack remaining cross-dual legs with a second greedy pass under the **full** conflict oracle (**including B12 M3 separation**).  
   - For each packed group: §5.5 load → move → store; cost with shared `movement_duration_us`; must pass `verify_aod_legality(..., params.min_row_col_separation_um)`.  
   - Repeat selection+emit until all gates in the layer are satisfied or stuck → `Unsatisfiable` / `NoInteractionPair`.

9. **Collision / empty-trap checks (B5)**  
   Before committing each packed group:
   - No two legs share `to` site.
   - Destination empty at start of round (**v0: no site swaps**).
   - Claimed interaction pairs marked occupied after both legs of a dual have stored (pair fully occupied only then).
   - Every `Transfer` target site empty under the **cross-cycle** map.
   - Update occupancy after load/move/store.

10. **R1–R3 check before entangle (**B2** / **B11**)**  
    After all moves+stores for this layer’s gates, before emitting entangle actions, run `check_entangling_geometry(...)` with dialect-identical predicates over **all occupied atoms**. On failure → `EntanglingGeometry`.

11. **Entangle layer**  
    Emit the original entangle actions at the new cycle. Atoms remain on pair sites when `return_home=false`.

12. **Between layers / return-home (B8 + B13 + B6)**  
    - If more input entangling layers remain and `return_home=false`: run **B13 half-pair reclaim**, then **B8 eviction**, so the next layer has enough free pairs.  
    - If `return_home=true`: after entangle, move all bank occupants back to home SLM sites (Enola-comparable); skip B8/B13 eviction.

### 5.3 R1–R3 params, dialect predicates, and tests (**B2** + **B11** — locked)

- `MovementParams` **must** include `min_rydberg_spacing_um`, `pair_gap_um`, `pair_pitch_um`, and `min_row_col_separation_um` (required fields).
- Validate at plan entry: `min_rydberg_spacing_um > 0`, `0 < pair_gap_um ≤ rydberg_range_um`, `pair_pitch_um ≥ min_rydberg_spacing_um`, `pair_pitch_um > pair_gap_um`, `min_row_col_separation_um > 0`.
- Geometry check is **mandatory** before every entangle emission.

**Locked predicate parity with `dialect.rs` `verify_entangling_geometry` (**B11**):**

| Rule | Planner / dialect predicate (fail closed) |
| --- | --- |
| R1 | Partner distance **`>`** `rydberg_range_um` → error (partners must be ≤ \(r_b\)) |
| R2 | Non-partner distance **`≤`** `rydberg_range_um` → error |
| R3 | Non-partner distance **`≤`** `min_rydberg_spacing_um` → error |

Architecture §6 prose says isolation **`>`** \(2.5×r_b\); the **shipped verifier uses `≤` reject**. The planner **must match the verifier**, not the prose. Consequence: distance **exactly** `min_rydberg_spacing_um` (e.g. 18.75) is **illegal**. Bank construction (**B14**): `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM` so placement↔bank is dialect-strict `>` min under defaults (pitch may equal min); staggered inter-pair ≈26.5 is fine. **Forbidden:** treating `pair_pitch_um ≥ min` alone as sufficient isolation, or locking `bank_outside_placement_isolated` as `≥ pair_pitch_um`.

**Locked scope (**B11**):** At the Rydberg stage, consider **every occupied atom** in the cross-cycle occupancy map (global laser / flat model), not only atoms named in the current layer’s `Entangle2` actions. Idle atoms on the `#104` placement grid participate in R2/R3. Partner-pair set = the gates being entangled in this emitted layer.

| Test | Asserts |
| --- | --- |
| `r1_partners_out_of_range_rejected` | Force partners > r_b → `EntanglingGeometry` |
| `r2_non_partner_too_close_fails` | Park a non-partner within r_b of a gate atom → plan errors (does not emit entangle) |
| `r3_isolation_spacing_fails` | Non-partner at distance `≤ min_rydberg_spacing_um` (incl. **exactly** 18.75) → `EntanglingGeometry` |
| `params_require_min_rydberg_spacing` | Missing/non-positive spacing → error at entry |
| `check_geometry_matches_dialect_leq` | Same fixture: planner check and dialect `verify_entangling_geometry` agree on Ok/Err (**B11**) |
| `dense_placement_skip_not_ok` | `#104` place (≥3 atoms, pitch 5 µm); one adjacent gate; idle neighbor within \(r_b\) → skip forbidden; relocate or `EntanglingGeometry` — never `Ok` in-place (**B10**/**B11**) |
| `geometry_scope_includes_idle_atoms` | Idle occupied atom within spacing of a partner → fail even if idle ∉ layer gate list (**B11**) |

### 5.3a Geometry-gated skip (**B10** — locked)

`#104` `SITE_PITCH_UM = 5.0` with defaults \(r_b=7.5\), `min_rydberg_spacing_um=18.75` ⇒ grid neighbors are within \(r_b\) and far inside R3. Bank isolation only protects atoms that **moved** onto pairs.

**Rule:** `skipped_already_adjacent` increments only when:

1. Partner distance ≤ `rydberg_range_um`, **and**
2. `check_entangling_geometry` (B11 predicates + all-occupied scope) would return `Ok` for the prospective entangle layer **with zero moves**.

Otherwise force Quon bank duals (or fail closed). Rewrite tests:

| Test | Asserts |
| --- | --- |
| `skip_already_adjacent_no_move` | Fixture where partners ≤ \(r_b\) **and** full geometry passes (e.g. only those two atoms occupied, or already isolated on a bank pair) → 0 rearrange for that gate; entangle kept |
| `skip_requires_geometry_ok` / `dense_placement_skip_not_ok` | Dense placement + idle neighbors → **must not** skip; must relocate to bank (plan Ok with moves) or `EntanglingGeometry` — never greenlight illegal in-place entangle |
| `all_adjacent_zero_moves_success` | **N3:** only when **all** gates are geometry-gated-skippable; not merely all distance-adjacent |

Property tests must check isolation among **all occupied atoms**, not “among layer atoms” only.

### 5.4 AOD index assignment (**B4 — locked**)

Dialect coupling is on shared `(aod_id, row)` / `(aod_id, col)` indices with identical Δy/Δx — **not** raw Enola src/dst coordinates alone. Enola-style IS ⇒ verifier pass only under a stable assignment **plus** M3 separation (**B12**).

**Locked assignment rule (v0, single AOD):**

1. `aod_id = 0` for all moves (`num_aods >= 1`; if `num_aods == 0` → `AodCapacity`).
2. Build a global **AOD grid overlay** on all layout sites (placement + interaction):
   - Sort unique site `y_um` ascending → assign dense `row` indices `0..`.
   - Sort unique site `x_um` ascending → assign dense `col` indices `0..`.
   - Each site maps to `(row, col)` by its coordinates (sites sharing y share row; sharing x share col).
3. When an atom is loaded SLM→AOD for a move from `from` to `to`, the `MoveSpec` uses:
   - `row = row_index(from)` and `col = col_index(from)` at load time (AOD trap indices follow the **source** site’s grid indices for the duration of that move — matching “atom rides its AOD intersection”),
   - `from_*_um` / `to_*_um` from site positions,
   - After store, binding returns to `TrapBinding::Slm { site: to }` (AOD indices cleared from resting layout).
4. Capacity: number of distinct `row` (resp. `col`) indices among simultaneously moving atoms in one `MovementGroup` must be ≤ `aod_rows` (resp. `aod_cols`).
5. **Proof obligation:** every emitted `MovementGroup` lowered with this rule must pass `verify_aod_legality(cycle, min_row_col_separation_um)`.

| Test | Asserts |
| --- | --- |
| `aod_indices_stable_from_site_coords` | Same site → same (row,col) across rounds |
| `verifier_accepts_emitted_group` | Lower group → `MoveSpec`s → `verify_aod_legality` Ok |
| `coupled_row_delta_consistent` | Two atoms sharing source row in one IS have identical Δy in MoveSpecs |
| `order_preservation_passes_verifier` | Non-crossing IS → Ok; intentionally crossed MoveSpecs → Err (oracle sanity) |
| `m3_separation_conflict_in_oracle` | Co-scheduling two legs with \(\lvert dst_y_1 - dst_y_2 \rvert < min_row_col_separation_um\) (same aod) is a planner conflict **and** `verify_aod_legality` Err (**B12**) |

**Claim discipline:** Docs must say “Enola three conflict types + dialect M3 separation + AOD index assignment ⇒ `verify_aod_legality`”, not “Enola IS = verifier” without assignment/separation.

### 5.4a M3 separation in the conflict oracle (**B12** — locked)

`verify_axis_order_and_separation` rejects when \((lhs\_to - rhs\_to).abs() < min_separation_um` (strict `<`). §5.2’s three Enola types alone are insufficient under §5.4’s dense index overlay: distinct rows can still have dest \(y\) closer than `min_row_col_separation_um` (default 2.0 µm).

**Lock:** Both dual-selection site conflicts and **B7 packing** must add a conflict edge whenever two co-scheduled legs on the same `aod_id` violate dest-axis separation on X or Y, using the **same** abs-delta / `total_cmp` semantics as the dialect. Pass `params.min_row_col_separation_um` into `verify_aod_legality` in tests.

**Test:** `m3_separation_conflict_in_oracle` — construct two otherwise Enola-legal legs whose dest \(y\) (or \(x\)) differ by `< min_row_col_separation_um`; assert (a) planner oracle reports conflict / refuses same `MovementGroup`, (b) if forcibly lowered together, `verify_aod_legality` returns Err.

### 5.5 Explicit transfers + cross-cycle occupancy (**B5 — locked**)

M4: SLM atoms do not move without transfer. `ScheduleLayer::validate_occupancy` only checks claims **inside one layer**.

**Locked emission pattern per packed `MovementGroup` \(G\)** (after B7 leg packing; not “all dual legs in \(S\)”):

```text
cycle c:   Transfer(SLM→AOD) × |G|     // load at each leg.from
cycle c+1: Move(MovementGroup { moves: G, duration_us })
cycle c+2: Transfer(AOD→SLM) × |G|     // store at each leg.to
```

One sortIS dual-set \(S\) may expand into **multiple** such triples when B7 serializes conflicting legs.

- Each `TrapTransfer` carries `AodTrapRef { aod_id, row, col }` from §5.4.
- `duration_us` on transfers = `trap_transfer_us`.
- **Cross-cycle occupancy map** updated after each cycle:
  - Load: atom leaves SLM site (site becomes empty); atom is AOD-resident (v0 may track as “occupying” source until move commits — document: after load, source SLM site is empty; dest still empty until store).
  - Move: logical position updates from→to (still AOD-resident).
  - Store: dest SLM site becomes occupied; reject if dest already occupied (`TransferIntoOccupied`).
- v0 forbids swap rounds (dest must be empty at round start).
- Unit test `transfer_into_occupied_rejected` must construct a store into a site still held by a non-moving atom and expect `TransferIntoOccupied` **without** relying solely on `validate_occupancy` of a single combined layer.

### 5.6 Transfer policy attribution (**B6 — locked**)

| Mode | Params | Transfers per moved atom | Attribution |
| --- | --- | --- | --- |
| **Default (Quon reuse)** | `return_home=false`, `transfers_per_moved_atom=2` | load+store to pair; atom stays; **B8 eviction** between layers when bank would overflow | Quon reuse — **not** Enola |
| **Enola-comparable** | `return_home=true`, `transfers_per_moved_atom=4` | load+store to pair, then load+store home each layer | Comparable to [Enola] Sec. 2/6.1 “4 transfers per gate” |

Module docs and rustdoc **must** state this table. Forbidden phrases for the default: “Enola-aligned transfer count”, “Enola Sec. 2 default”.

---

## 6. Emission details

### 6.1 Grouped moves (critical)

- Exactly **one** `NeutralAtomAction::Move(MovementGroup)` per parallel AOD step, containing **all** legs packed into that step (B7).
- Never emit one `Move` action per atom for the same physical AOD step.
- Never put both legs of a horizontal-pair dual into the same group when src y differs (B7).

### 6.2 Lowering to dialect (test / helper)

```rust
fn atom_moves_to_move_specs(
    moves: &[AtomMove],
    layout: &NeutralAtomLayout,
    aod_meta: &BTreeMap<AtomId, AodTrapRef>,
) -> Result<Vec<MoveSpec>, MovementPlanError>
```

Fill `from_*_um` / `to_*_um` from site positions; attach the AOD row/col from §5.4. Tests build `ActionSpec::Move` / layer and run `verify_aod_legality`.

### 6.3 Relationship to `#107` / `zoned.rs`

- `#107` is merged and uses placeholder AOD indices `(0,0,0)` in places — **do not “fix” zoned emission in this PR** unless a tiny shared-helper extract requires it.
- `#106` must not call `schedule_zoned`. Keep APIs separate: `plan_aod_movement` vs `schedule_zoned`.
- Shared: only geometry/duration helpers.

---

## 7. Collision detection (detail)

| Check | When | Error |
| --- | --- | --- |
| Unique destination sites in a `MovementGroup` | Before emit | `Collision` |
| Unique atom claims in cycle | Before emit | map to `Collision` / `Conflict` |
| Transfer only into empty site (cross-cycle map) | Before emit transfer | `TransferIntoOccupied` |
| Global occupancy map consistency | After each cycle | `Conflict` |
| `ScheduleLayer::validate_occupancy` | After building each layer | `Conflict` (secondary) |
| R1–R3 | Before entangle | `EntanglingGeometry` |

---

## 8. Cost model (detail)

```text
move_us(round)     = movement_duration_us(d_max_um, acceleration_m_s2)  // shared
transfer_us(total) = n_transfers * trap_transfer_us
rearrangement_steps = number of Move(MovementGroup) actions
rearrangement_time_us = sum of Move.duration_us
```

Do **not** use Atomique’s 300 µs flat stage. Document divergence in module docs (architecture_model §5).

---

## 9. Test plan

### 9.1 Unit tests (`quon_na/tests/movement.rs` or `src/movement.rs` `#[cfg(test)]`)

| Test | Asserts | Blocks |
| --- | --- | --- |
| `pair_gap_leq_rb` | Intra-pair distance ≤ \(r_b\) | B1 |
| `pair_pitch_isolates_non_partners` | Inter-pair distance **`>`** min spacing (dialect `≤` reject) | B1/B11 |
| `bank_outside_placement_isolated` | Placement↔bank min distance **`>`** `min_rydberg_spacing_um` | B1/B14 |
| `partner_reachable_under_defaults` | generic_rna defaults: far gate plans | B1 |
| `dual_moves_both_atoms_onto_pair` | Both atoms → pair sites | B1 |
| `unequal_y_partners_pass_aod_verifier` | Unequal src y; all groups pass verifier | B7 |
| `multi_layer_reuse_default_no_exhaustion` | Disjoint atoms → eviction | B8 |
| `multi_layer_same_atom_reuse_no_eviction` | Same atoms → no eviction when B10 skip passes | B8 |
| `partial_overlap_pair_reclaimed` | One-atom overlap → whole pair vacated; plan Ok | B13 |
| `dual_dest_is_existing_site_id` | `to` ∈ layout.sites | B1 |
| `r2_non_partner_too_close_fails` | Illegal park → error | B2 |
| `r3_isolation_spacing_fails` | Spacing `≤ min` (incl. exactly 18.75) → error | B2/B11 |
| `params_include_min_rydberg_spacing` | Field required / validated | B2 |
| `check_geometry_matches_dialect_leq` | Planner ↔ dialect predicate parity | B11 |
| `geometry_scope_includes_idle_atoms` | Idle occupied atoms in R2/R3 scope | B11 |
| `dense_placement_skip_not_ok` | Dense `#104` + skip would violate R2/R3 → no illegal Ok | B10/B11 |
| `skip_requires_geometry_ok` | Skip only when full geometry would pass | B10 |
| `uses_shared_movement_duration_us` | 110 µm / 2750 → ~200 µs via shared fn | B3 |
| `verifier_accepts_emitted_group` | `verify_aod_legality` Ok | B4 |
| `aod_indices_from_site_coords` | Stable row/col assignment | B4 |
| `m3_separation_conflict_in_oracle` | Dest delta `< min_row_col_separation_um` → oracle conflict + verifier Err | B12 |
| `explicit_transfer_layers_around_move` | load → move → store cycle pattern | B5 |
| `transfer_into_occupied_rejected` | Cross-cycle map error | B5 |
| `default_transfer_policy_is_quon_reuse` | default 2 + `return_home=false`; docs/comments | B6 |
| `enola_comparable_return_home_four_transfers` | `return_home` → 4 xfers/atom | B6 |
| `skip_already_adjacent_no_move` | Geometry-legal within \(r_b\) → 0 rearrange; entangle kept | B10/AC |
| `all_adjacent_zero_moves_success` | All geometry-gated-skippable → not `EmptySchedule` | N3/B10 |
| `docs_do_not_claim_enola_duals` | Module docs label both-atom/pair-bank/B7 as Quon; Enola = conflicts + greedy IS idea only | B9 |
| `dual_exclusion_one_atom_moves` | One dual chosen per gate | Quon |
| `sortis_longest_first_rounds` | Long preferred when conflicting | Enola *idea* |
| `order_preservation_conflict` | Crossing not co-scheduled | Enola |
| `same_source_row_conflict` | Same-src-row / different-dst-row conflict | Enola |
| `occupancy_collision_rejected` | Two moves to same site | M5 |
| `sqrt_cost_matches_formula` | architecture_model example | Cost |
| `resource_report_counts_rounds` | report matches move groups | AC |
| `grouped_move_single_action` | One `Move` per round | AC |
| `unsupported_entangle_n_errors` | k>2 → `UnsupportedEntangleArity` | N1 |
| `missing_layout_errors` | `MissingLayout` | API |
| `empty_layers_errors` | `EmptySchedule` | API |
| Integration smoke | `place` → pair bank → entangle → `plan_aod_movement` | E2E |

### 9.2 Property tests (`movement_props.rs`)

| Property | Generator | Invariant |
| --- | --- | --- |
| Occupancy | ER / cubic + place + pair bank + entangle | Every layer `validate_occupancy` + `validate_conflicts` Ok; cross-cycle map consistent |
| Adjacency after moves | Same | Before each entangle, partner distances ≤ \(r_b\) (= `pair_gap_um` when on bank) |
| Isolation after moves | Same | **All occupied** non-partner pairs have distance **`>`** `min_rydberg_spacing_um` (dialect: reject `≤`); **not** “layer atoms only” |
| Verifier groups | Same | Every `MovementGroup` lowers to `verify_aod_legality` Ok (incl. unequal-y + M3 separation) |
| Multi-layer occupancy | ≥2 layers, width ≤ W_max, incl. partial-overlap fixtures | Succeeds under `return_home=false` (B8+B13) |
| No free Manhattan | — | Every multi-atom group is an IS under full conflict oracle (Enola types + M3 sep) |
| Cost monotonicity | Random move sets | `duration_us` nondecreasing in `d_max` |
| Skip count | Mixed distances | `skipped_already_adjacent` ≤ # geometry-legal adjacent pairs |

Bound `n` (e.g. ≤ 16); deterministic seeds.

### 9.3 Dialect tests

Prefer calling shared verify helpers from movement tests (`--no-default-features` pattern). Extend `tests/quantum_na_dialect.rs` only if needed for regression locks under `--features mlir`.

---

## 10. Validation commands

From `.worktrees/issue-106` (on `main`-based branch):

```bash
cargo fmt --all -- --check
cargo clippy -p quon_na --no-default-features --all-targets -- -D warnings
cargo test -p quon_na --no-default-features
# If mlir feature / dialect helper tests:
cargo test -p quon_na --features mlir --test quantum_na_dialect

# Workspace bar before PR (docs/agents/code-quality.md):
cargo clippy --workspace --exclude flux_verify --all-targets -- -D warnings
cargo test --workspace --exclude flux_verify
npx @taskless/cli@latest check $(git diff --name-only main...HEAD)
```

---

## 11. Graphite branch / worktree (**B3 / N2 — locked**)

```bash
cd /Users/arnabghosh/projects/quon
git fetch origin
# Confirm #158/#159 merged (already true as of plan amendment):
gh pr view 158 --json state,mergedAt
gh pr view 159 --json state,mergedAt

# Worktree from current main tip (contains #105 + #107):
git worktree add .worktrees/issue-106 -b issue-106 origin/main
cd .worktrees/issue-106
gt track --parent main --no-interactive   # if needed
# Implement on this branch, then:
gt submit --no-interactive --no-edit
```

**Do not** stack on `issue-105` / open PR #158 — both are merged. Coordinator: re-root agent to `.worktrees/issue-106` before implementation.

PR title suggestion: `feat(quon_na): AOD-constrained flat movement planner (pair-bank + Enola-inspired conflicts)`  
Body: `Fixes #106` (no “Depends on #105” stack dependency). Do **not** title/body-claim “Enola duals” or “Enola sortIS dual selection” (**B9**).

---

## 12. Docs / attribution touchpoints

- `quon_na/src/movement.rs` module docs (**B9** honesty):
  - Cite [Enola] Sec. 5 **only** for: three per-axis conflict types + greedy longest-first maximal-IS *idea* (not KaMIS).
  - Explicitly state: Enola duals are **one-atom** “move either endpoint” candidates; **this planner does not implement those duals**.
  - Label as **Quon**: interaction-pair bank, both-atom orientations, B7 second packing pass, B8/B13 reuse/eviction.
  - Forbidden phrases: “Enola duals”, “implements Enola Sec. 5 sortIS dual selection”, equating issue-comment “move either endpoint” with the shipped algorithm.
  - architecture_model §5 M1–M5 (incl. M3 separation via `min_row_col_separation_um`); §6 R1–R3 with **dialect `≤` predicates** and all-occupied scope.
  - √-law via shared helpers; **not** Atomique 300 µs.
  - **Not** RAP zoned (#107).
  - Transfer policy table (Quon reuse default vs Enola-comparable) — B6.
  - AOD index assignment rule — B4; conflict oracle includes M3 separation — B12.
  - Geometry-gated skip — B10.
- `lib.rs` / `schedule_entry.rs`: one paragraph for #106 pipeline step (Quon flat movement, Enola-inspired conflicts).
- Touch `architecture_model.md` / `literature_notes.md` **only** if wording still implies the flat movement planner is unimplemented — no new literature claims; do not rewrite Enola dual semantics into Quon’s both-atom model.
- Never claim Enola near-optimality beyond “greedy maximal IS heuristic inspired by sortIS.”

---

## 13. Out of scope

- Zoned joint placement-routing, A*, ghost spots as in [RAP] (**#107** — already shipped; do not reimplement)
- Schedule compaction (**#108**)
- windowIS scaling variant
- Heating / atom-loss models ([Atomique] Eqs. 1–2)
- Continuous trajectory simulation
- SMT / OLSQ-DPQA optimal solving
- Changing #105 Misra–Gries or #104 placement heuristics (beyond appending the interaction-pair bank)
- Hard dependency on `backend` crate types inside `quon_na`
- Full `quonc` CLI wiring (optional follow-up)
- Claiming this reproduces [RAP] Table I (#111)
- Site-swap rounds in a single IS (v0)
- Implementing Enola’s literal one-atom duals / “move either endpoint” algorithm (**B9** — cite only)

---

## 14. Risks

| Risk | Mitigation |
| --- | --- |
| Not enough interaction pairs for a dense layer | `min_pairs = W_max`; fail `NoInteractionPair`; test |
| Multi-layer bank exhaustion under reuse | B8 eviction + **B13** half-pair reclaim; multi-layer + `partial_overlap_pair_reclaimed` |
| Horizontal dual breaks M2 | B7 serialize legs into separate MovementGroups |
| R2/R3 vs many parallel pairs | Staggered bank + **B14** ε on `x0` + dialect `≤` check before entangle (**B11**) |
| Default pitch == min → exact 18.75 placement↔bank | **B14:** `x0 += BANK_ISOLATION_EPS_UM`; test asserts `>` min |
| Dense `#104` in-place skip illegal | **B10** geometry-gated skip; `dense_placement_skip_not_ok` |
| Partner-stationary duals vs sparse lattice | **Removed**; both atoms move onto pair (B1 fix) |
| Duplicate √-law vs `zoned.rs` | B3: reuse or extract `geometry.rs` |
| Enola IS ≠ dialect without indices/separation | B4 assignment + **B12** M3 sep + verifier tests |
| `validate_occupancy` false confidence | B5 cross-cycle map + transfer-into-occupied test |
| Mis-attributing 2-xfer default to Enola | B6 docs table + unit test on defaults |
| Mis-attributing Quon duals as Enola | **B9** docs + `docs_do_not_claim_enola_duals` |
| Float / order-preservation edge cases | Prefer grid indices for conflicts; match dialect tolerance |
| Unsatisfiable layers on pathological graphs | Clear error; props use sparse ER/cubic |
| `#107` placeholder AOD indices confuse readers | Keep APIs separate; document #106’s stricter assignment |

---

## 15. Implementation order (for the implementer)

1. Confirm worktree on `origin/main` (not `issue-105`); Graphite parent `main`.
2. Optional: extract `geometry.rs` from `zoned.rs` helpers; wire re-exports.
3. `MovementParams` (+ `min_rydberg_spacing_um`, `pair_gap_um`, `pair_pitch_um`, `min_row_col_separation_um`, `return_home`) + validation.
4. `ensure_interaction_pairs` + B1/B14 geometry (`x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`; tests `pair_gap_leq_rb`, `pair_pitch_isolates_non_partners`, `bank_outside_placement_isolated` with dialect `>`, `partner_reachable_under_defaults`).
5. Cross-cycle occupancy + explicit Transfer emission helpers + B5 tests.
6. Dual generation (Quon both-atom orientations onto free pairs) + **B10** geometry-gated skip.
7. Conflict oracle (three Enola types × axes + **B12** M3 separation + dual exclusion + site conflicts).
8. AOD index assignment (§5.4) + `atom_moves_to_move_specs`.
9. sortIS-style dual selection + **B7** leg packing + load/move/store emission.
10. R1–R3 gate before entangle (**B11** dialect predicates + all-occupied scope) + B2/B11 tests.
11. **B8** inter-layer eviction + **B13** half-pair reclaim; default vs `return_home` (B6).
12. `plan_aod_movement` integration + multi-layer / unequal-y / partial-overlap / dense-skip tests.
13. Verifier-aligned tests (B4/B12) + proptest + **B9** docs/attribution.
14. fmt / clippy / test / Taskless.
15. `gt submit` with parent `main`.

---

## 16. Done definition (checklist for implementer)

- [ ] `plan_aod_movement` + `ensure_interaction_pairs` public API with `thiserror` + `deny_unknown_fields`
- [ ] Interaction-pair bank with `pair_gap_um ≤ r_b`, inter-pair isolation, and **B14** placement↔bank `>` min via `BANK_ISOLATION_EPS_UM` on `x0`; both-atom duals; no virtual destinations; partner-reachable under defaults
- [ ] `bank_outside_placement_isolated` asserts min placement↔bank **`>`** `min_rydberg_spacing_um` (not `≥ pair_pitch_um`)
- [ ] B7: dual legs serialized so unequal-y partners pass `verify_aod_legality`
- [ ] B8: multi-layer default reuse via same-atom reuse + eviction; bank size `W_max`
- [ ] **B13:** half-pair / partial-overlap reclaim; `partial_overlap_pair_reclaimed` Ok
- [ ] `min_rydberg_spacing_um` on params; R1–R3 before entangle with dialect `≤` + all occupied (**B2**/**B11**)
- [ ] **B10:** geometry-gated skip; dense `#104` cannot illegal in-place entangle
- [ ] Reuses `movement_duration_us` / `euclidean_um` — no duplicate (B3)
- [ ] AOD index assignment documented; emitted groups pass `verify_aod_legality` (B4)
- [ ] **B12:** packing oracle includes `min_row_col_separation_um`; `m3_separation_conflict_in_oracle`
- [ ] Explicit Transfer around moves; cross-cycle occupancy; transfer-into-occupied test (B5)
- [ ] Default transfer policy documented as Quon reuse (2); Enola-comparable via `return_home` (4) (B6)
- [ ] **B9:** docs cite Enola only for conflict types + greedy IS idea; Quon pair-bank/both-atom/B7 labeled Quon
- [ ] Grouped `MovementGroup` emission; rounds counted
- [ ] √-law + transfer cost reported; not 300 µs flat
- [ ] Docs explicitly **not** claiming RAP zoned routing
- [ ] Branch based on `main`; `Fixes #106`

---

## 17. Reviewer amendment trace

| Blocker | Resolution in this plan |
| --- | --- |
| **B1** | §5.0 **interaction-pair bank** (single staggered formula + **B14** ε): `pair_gap_um ≤ r_b`, `pair_pitch_um ≥ min_rydberg_spacing_um`, `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`; duals = both atoms onto a free pair; `partner_reachable_under_defaults` test. Explicitly rejects sparse-lattice + partner-stationary parking. |
| **B2** | `min_rydberg_spacing_um` on params; mandatory R1–R3; `r2_non_partner_too_close_fails` |
| **B3** | Stack on `origin/main`; reuse/extract shared duration helpers |
| **B4** | §5.4 AOD grid overlay from site coords; verifier tests |
| **B5** | Explicit load/move/store; cross-cycle map; transfer-into-occupied test |
| **B6** | Default = Quon reuse (2); Enola-comparable = `return_home` (4); no mis-attribution |
| **B7** | Serialize dual legs into separate `MovementGroup`s when co-scheduling would break M2; `unequal_y_partners_pass_aod_verifier` |
| **B8** | Bank size `W_max`; same-atom reuse + evict non-participants between layers; tests `multi_layer_reuse_default_no_exhaustion` + `multi_layer_same_atom_reuse_no_eviction` |
| **B9** (reviewer #1) | Enola attribution honesty: cite Enola only for three conflict types + greedy longest-first IS *idea*; label pair-bank / both-atom duals / B7 as **Quon**; forbid equating issue “move either endpoint” with shipped algorithm; test `docs_do_not_claim_enola_duals` |
| **B10** (reviewer #2) | Geometry-gated skip: skip iff distance ≤ \(r_b\) **and** full R1–R3 would pass without moves; else force bank duals; rewrite `skip_already_adjacent_no_move` / N3; tests `skip_requires_geometry_ok`, `dense_placement_skip_not_ok` |
| **B11** (reviewer #3) | `check_entangling_geometry` matches dialect `≤` predicates; scope = **all occupied** atoms; exact 18.75 fails; tests `check_geometry_matches_dialect_leq`, `geometry_scope_includes_idle_atoms`, `dense_placement_skip_not_ok` |
| **B12** (reviewer #4) | Conflict/packing oracle includes M3 dest separation (`abs(dst_i−dst_j) < min_row_col_separation_um`); test `m3_separation_conflict_in_oracle` (oracle + `verify_aod_legality`) |
| **B13** (reviewer #5) | Partial-overlap / half-pair: vacate **whole** pair to home before dual gen; test `partial_overlap_pair_reclaimed` must `Ok` |
| **B14** (re-review `5d03ba68`) | Bank isolation vs dialect `≤`: `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM` (ε = 0.01 µm); keep `pair_pitch_um ≥ min` (defaults may equal); change `bank_outside_placement_isolated` to assert min placement↔bank **`>`** `min_rydberg_spacing_um`. Rejects prior lock of isolation as `≥ pair_pitch_um` (exact 18.75 unsatisfiable). |

Non-blocking from earlier reviews also incorporated: **N1** `UnsupportedEntangleArity`; **N2** Graphite on `main`; **N3** all-adjacent success **only when geometry-gated** (B10); **N4** sortIS ≠ KaMIS.
