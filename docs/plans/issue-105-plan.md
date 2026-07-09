# Issue #105 — Entangling layer scheduler (Misra–Gries)

**Branch**: `issue-105`  
**Worktree**: `/Users/arnabghosh/projects/quon/.worktrees/issue-105`  
**Parent**: `main` (interaction graph #103 + placement #104 already merged)  
**Agent brief**: issue #105 comment (Misra–Gries, graph-only OK)

## Goal

Turn an [`InteractionGraph`](../../quon_na/src/graph.rs) into ordered [`ScheduleLayer`](../../quon_na/src/schedule.rs)s of parallel entangling actions:

- **`CommutationGroup` segments** → Misra–Gries edge-coloring of the 2Q interaction graph (Enola Thm. 1 / ≤ Δ+1), then capacity-split by `max_parallel_entangling_pairs`.
- **`DependencyDag` segments** → ASAP layering by existing `dag_layer` (critical-path optimal), then capacity-split.
- **Graph-only** — `layout` remains optional; atom ids use the established `AtomId(logical.0)` identity (same as placement).

## Upstream contract

| Type / API | Role |
| ---------- | ---- |
| `InteractionGraph` + `SegmentKind` | Input; segments already partition interactions |
| `cubic_commutation_graph` | Δ=3 regression fixture → ≤ 4 colors |
| `GraphScheduleRequest` | Output container; fill `layers`, leave `layout` untouched |
| `ScheduleLayer::validate_conflicts` | Invariant every emitted layer must pass |
| `ResourceReport::from_layers` | Counts `rydberg_stages`; utilization is separate metadata |
| `max_parallel_entangling_pairs` | Capacity from target JSON; passed as `u32` into the scheduler (no hard dep on `backend` crate) |

## Design

### Module layout

```text
quon_na/src/
  entangling_schedule.rs   ← NEW (Misra–Gries + ASAP + capacity + emission)
  schedule_entry.rs        ← doc updates; keep schedule_from_graph as empty-layers stub
  lib.rs                   ← mod + re-exports
```

Follow the `#104 place()` pattern: `schedule_from_graph` stays a validate-only stub (property tests already assert empty layers). New entry point fills layers.

### Public API

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerUtilization {
    pub cycle: u32,
    pub entangling_pairs: u32,
    pub capacity: u32,
    /// `entangling_pairs as f64 / capacity as f64` when capacity > 0; else 0.0
    pub utilization: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntanglingScheduleResult {
    pub request: GraphScheduleRequest, // layers filled; layout unchanged
    pub utilizations: Vec<LayerUtilization>,
    /// Max degree Δ of the largest 2Q commutation subgraph colored (0 if none).
    pub max_degree: u32,
    /// True iff every commutation segment was 2Q-only and used Misra–Gries.
    pub misra_gries_applied: bool,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum EntanglingScheduleError {
    #[error(transparent)]
    InvalidGraph(#[from] GraphError),
    #[error("max_parallel_entangling_pairs must be ≥ 1, got {0}")]
    InvalidCapacity(u32),
    #[error("schedule layer conflict: {0}")]
    Conflict(String),
    #[error("commutation segment contains multi-qubit gate {0:?}; Misra–Gries requires 2Q edges")]
    MultiQubitCommutation(InteractionId),
}

/// Fill `req.layers` with entangling actions from the interaction graph.
///
/// `layout` is ignored (graph-only). Atom identity: `AtomId(LogicalQubitId.0)`.
pub fn schedule_entangling_layers(
    req: GraphScheduleRequest,
    max_parallel_entangling_pairs: u32,
) -> Result<EntanglingScheduleResult, EntanglingScheduleError>;
```

Default entangle duration: `1` µs placeholder (timing comes from target fidelity/timing in later slices; stage *count* is the Enola metric).

### Algorithm

#### Per-segment dispatch

Process `graph.segments` **in order**. Append layers with monotonically increasing `cycle` indices.

**`CommutationGroup`:**

1. Collect interactions; require every interaction has `qubits.len() == 2` (v0). Multi-qubit in a commutation group → `MultiQubitCommutation` error (forces callers to put k>2 gates in `DependencyDag` segments or split). Rationale: Misra–Gries is an *edge*-coloring algorithm; claiming Enola Thm. 1 on a hypergraph would be an attribution slip (#99 / brief).
2. Build simple undirected multigraph-free interaction graph: vertices = qubits appearing in the segment; one edge per interaction (reject parallel duplicate qubit-pairs as separate interactions that share endpoints — they conflict and Misra–Gries on simple graphs needs care: if two interactions share the exact same qubit pair, treat as multi-edge → color as conflict via expanding to a conflict-graph fallback **or** reject. **Decision**: allow at most one interaction per unordered pair in a commutation group; if duplicates, fall back to conflict-graph greedy vertex coloring and set `misra_gries_applied = false` for that segment (document; do not claim Δ+1). Prefer: cubic/ER fixtures have unique pairs — unit-test the duplicate path.
3. Run **Misra–Gries** → map each interaction edge → color in `0..Δ` or `0..Δ` (≤ Δ+1 colors).
4. Bucket interactions by color; for each color class, **capacity-split** into chunks of size ≤ `max_parallel_entangling_pairs` (stable order by `InteractionId`).
5. Emit one `ScheduleLayer` per chunk: `NeutralAtomAction::Entangle2 { atoms: [AtomId(a), AtomId(b)], duration_us: 1 }`.

**`DependencyDag`:**

1. Collect interactions; group by `dag_layer` ascending (already ASAP-assigned by extract / `schedule_dependency_segment`).
2. Within each ASAP bucket, capacity-split (stable by `InteractionId`).
3. Emit `Entangle2` or `EntangleN` based on arity. No edge-coloring. Do **not** claim Enola Thm. 1.

#### Misra–Gries (constructive Vizing)

Implement the standard fan / path-inversion algorithm on a simple graph:

- Maintain per-vertex missing colors and per-edge colors.
- For each uncolored edge `uv`, build a maximal fan at `u` ending at free color, invert paths, rotate fan, color.
- Guarantee: uses at most `Δ+1` colors.
- Cite in module docs: Enola Sec. 3 / Thm. 1; Misra & Gries 1992.

Determinism: iterate edges in sorted `(min_qubit, max_qubit, InteractionId)` order.

#### Capacity split

```text
for class in color_classes_or_asap_buckets:
  for chunk in class.chunks(capacity):
    emit layer(chunk)
```

If `capacity == 0` → `InvalidCapacity`.

#### Utilization

For each emitted layer:

```text
entangling_pairs = count of Entangle2 + EntangleN actions
utilization = entangling_pairs / capacity
```

### Conflict invariant

After building all layers, run `validate_conflicts` on each; map errors to `EntanglingScheduleError::Conflict`.

### Docs / attribution

- Module rustdoc: **Misra–Gries**, not “greedy edge-coloring”.
- Touch `architecture_model.md` / `literature_notes.md` only if wording still says “greedy” without Misra–Gries (architecture_model already says Misra–Gries — verify; update `schedule_entry` comments that say “edge-coloring (#105)” to name Misra–Gries).
- Never claim ≤ Δ+1 / Enola Thm. 1 for DependencyDag or multi-edge fallback paths.

## Tests

| Test | Asserts |
| ---- | ------- |
| `path_of_three_edges_uses_two_colors` | P₃ edge graph Δ=2 → ≤ 3 layers, typically 2 |
| `cubic_n12_at_most_four_layers` | `cubic_commutation_graph(12)`, capacity=340 → ≤ 4 layers; Δ=3 |
| `cubic_n8_validate_conflicts` | every layer `validate_conflicts` Ok |
| `capacity_one_splits_matching` | matching of 3 edges, capacity=1 → 3 layers |
| `dependency_dag_asap_not_edge_color` | chain of 3 dependent CZs → 3 layers (ASAP), not 1 |
| `graph_only_no_layout` | `layout` stays `None` after scheduling |
| `utilization_reported` | full layer at capacity → utilization 1.0 |
| proptest update | cubic/ER: after `schedule_entangling_layers`, layers non-empty when graph has edges; all `validate_conflicts`; commutation-only cubic → layer count ≤ 4 |

Keep existing `schedule_from_graph_*` props asserting **empty** layers (stub unchanged).

## Flux

Add a small pure helper with a Flux spec, e.g. capacity chunk bound:

```rust
/// Number of layers needed to host `n` parallel pairs under capacity `cap`.
#[cfg_attr(feature = "flux", spec(fn(n: u32, cap: u32{v: v > 0}) -> u32{v: v >= 1 || n == 0}))]
fn capacity_layer_count(n: u32, cap: u32) -> u32
```

(Or equivalent provable bound: `ceil(n/cap)`.) Mirror pattern from `report::simultaneous_layer_time` / `qec::ceil_div`.

## Out of scope

- AOD movement (#106), zoned RAP (#107), compaction (#108)
- Placement requirement
- Real gate durations from target timing
- Multi-qubit Misra–Gries / hypergraph coloring
- Changing `schedule_from_graph` to auto-schedule (keeps #103 stub contract)

## Implementation order

1. `capacity_layer_count` + unit tests
2. Misra–Gries core + small graph unit tests
3. `schedule_entangling_layers` wiring (segments, emission, utilization)
4. Cubic / capacity / DAG / graph-only tests + proptest additions
5. Docs/comment attribution pass
6. fmt / clippy / test / Taskless / Flux if specs touched
