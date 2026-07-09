# Issue #104 — Placement passes (row-major, degree-based, interaction-clustering)

**Branch**: `issue-104-placement`  
**Worktree**: `/Users/arnabghosh/projects/quon-worktrees/104-placement`  
**Parent**: `issue-103-interaction-graph` (PR #147)  
**Blocked by**: #103 (landed in this stack)

## Goal

Map logical qubits from an [`InteractionGraph`](../../quon_na/src/graph.rs) onto a rectangular grid of SLM sites, producing a filled [`NeutralAtomLayout`](../../quon_na/src/layout.rs) on a [`GraphScheduleRequest`](../../quon_na/src/schedule_entry.rs). Three strategies, increasing sophistication; each reports a placement score so later slices can compare.

## Upstream contract (#103)

Already present on this branch:

| Type / API | Role for #104 |
| ---------- | ------------- |
| `InteractionGraph { vertices, edges, … }` | Input; edges carry Atomique `Σ γ^l` weights |
| `LogicalQubitId` | Placement domain keys |
| `GraphScheduleRequest { graph, layers, layout }` | `place` fills `layout: Some(…)` |
| `schedule_from_graph` | Produces `layout: None`; placement is a separate step |
| `cubic_commutation_graph` / `erdos_renyi_commutation_graph` | Benchmark graphs |

## Design

### Module layout (MLIR-free)

New file: `quon_na/src/placement.rs` (no `melior` / dialect deps).

```text
quon_na/src/
  placement.rs   ← NEW
  lib.rs         ← mod + re-exports
```

### Public API

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementStrategy {
    RowMajor,
    DegreeBased,
    InteractionClustering,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementResult {
    pub request: GraphScheduleRequest, // layout: Some(…)
    pub score: f64,                    // lower is better
    pub strategy: PlacementStrategy,
    /// Optional routing-awareness proxy (#104 comment): count of
    /// interacting pairs whose sites share a row or column (cheap
    /// AOD-parallelism hint). Not part of acceptance scoring.
    pub axis_aligned_pairs: u32,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum PlacementError {
    #[error(transparent)]
    InvalidGraph(#[from] GraphError),
    #[error("empty interaction graph: no vertices to place")]
    EmptyGraph,
    #[error("placement produced overlapping site bindings")]
    SiteOverlap,
}

/// Place qubits onto a square-ish grid of SLM sites and fill `req.layout`.
pub fn place(
    req: GraphScheduleRequest,
    strategy: PlacementStrategy,
) -> Result<PlacementResult, PlacementError>;

/// Score an existing layout against the request's interaction graph.
pub fn placement_score(graph: &InteractionGraph, layout: &NeutralAtomLayout) -> Result<f64, PlacementError>;
```

### Grid construction

For `n = |vertices|` qubits:

1. `cols = ceil(sqrt(n))`, `rows = ceil(n / cols)` — compact rectangle.
2. Sites `SiteId(0..rows*cols)` at positions `(col * PITCH_UM, row * PITCH_UM)` with `PITCH_UM = 5.0` (matches typical SLM pitch scale in architecture notes; absolute scale cancels in relative comparisons).
3. Only the first `n` sites receive bindings; extra sites (if `rows*cols > n`) remain unbound.

Identity: `AtomId(q.0) ↔ LogicalQubitId(q.0)` (1:1 for v0; code-block units come later).
Iterate `graph.vertices` as given — do **not** assume ids are a dense `0..n-1` range (ER/cubic happen to be dense; production extracts may not).

Bindings: `TrapBinding::Slm { site }` only (flat-array initial placement; AOD assignment is #106).

### Strategies

#### 1. Row-major (baseline)

Sort vertices by `LogicalQubitId` ascending. Assign to sites in row-major order: site index `i` → `(row = i / cols, col = i % cols)`.

#### 2. Degree-based (Atomique load-balance–inspired)

Degree of qubit `q` = sum of edge weights incident on `q` (weighted degree, not unweighted). Sort qubits by degree **descending**, break ties by `LogicalQubitId` ascending. Place in that order onto sites in a **spiral from the grid center** (Atomique Sec. III-B / Fig. 6 inspiration: high-degree qubits near the center so partners are closer on average).

Spiral order: start at center cell `(rows/2, cols/2)`, walk right → down → left → up with increasing leg length (classic Ulam spiral adapted to rectangle; skip out-of-bounds).

Document as **inspired by** Atomique load-balance mapping, not a reproduction (architecture_model §4; issue comment).

#### 3. Interaction-clustering (MAX-k-Cut–inspired coarse partition)

1. Choose `k = cols` (partition into column-stripes) — coarse spatial buckets.
2. Greedy MAX-k-Cut style: assign each qubit (in degree-descending order) to the part that **maximizes** cut weight of already-placed edges (i.e. prefers putting strongly interacting pairs in *different* parts when that improves the cut — Atomique Alg. 1 maps across arrays).  
   **Adaptation for our score**: on a single flat grid we want *strongly interacting* qubits *spatially close*. So invert the Atomique array-cut objective for site placement: assign each qubit to the part that **minimizes** expected score contribution given already-placed partners (greedy local placement into the best remaining site within the chosen part's column stripe). Practical algorithm:

   **Implemented algorithm (smallest correct):**
   1. Partition vertices into `k = max(2, cols)` clusters via greedy **min-cut / community** heuristic: place qubits degree-descending; assign each to the cluster that maximizes *intra-cluster* weight to already-assigned members (agglomerative clustering on the gate-frequency graph). Empty clusters preferred only when all give equal weight (fill round-robin for balance).
   2. Order clusters by total internal weight descending.
   3. Within each cluster, order qubits by degree descending.
   4. Assign consecutive blocks of sites in **row-major** so each cluster occupies a contiguous rectangular band (row strips of height `ceil(cluster_size / cols)`, packed top-to-bottom).

This keeps strongly interacting qubits in nearby sites → lower `Σ w · √d` than row-major on clustered graphs.

Document as **inspired by** Atomique MAX-k-Cut qubit-array mapper (Alg. 1, 1−1/k approx), adapted from multi-array partitioning to single-grid spatial clustering (issue #104 comment).

### Placement score (acceptance metric)

Per architecture_model §5 / issue comment (movement-cost-shaped, not raw Manhattan):

```text
score(layout) = Σ_{edges (a,b)}  weight(a,b) * sqrt( euclidean_um(site(a), site(b)) )
```

- Euclidean distance in µm between bound site positions.
- Lower is better.
- Empty edge set → score `0.0`.
- Missing binding for a vertex → `PlacementError` (should not happen after `place`).

Also compute `axis_aligned_pairs`: count of edges whose sites share `x` or `y` (AOD row/column parallelism proxy). Reported but **not** used for the “clustering beats row-major” acceptance check.

### How `place` fills `GraphScheduleRequest`

```rust
pub fn place(mut req: GraphScheduleRequest, strategy: PlacementStrategy)
    -> Result<PlacementResult, PlacementError>
{
    req.graph.validate()?;
    if req.graph.vertices.is_empty() {
        return Err(PlacementError::EmptyGraph);
    }
    let layout = match strategy { … };
    assert_no_site_overlap(&layout)?;
    let score = placement_score(&req.graph, &layout)?;
    let axis_aligned_pairs = count_axis_aligned(&req.graph, &layout)?;
    req.layout = Some(layout);
    // layers untouched
    Ok(PlacementResult { request: req, score, strategy, axis_aligned_pairs })
}
```

Invariant: `req.layers` unchanged; only `layout` is written.

### Overlap check

After building bindings: every `SiteId` appears in at most one `AtomBinding`; every vertex has exactly one binding. Fail with `SiteOverlap` / missing-binding error otherwise.

## Tests

| Test | Asserts |
| ---- | ------- |
| `row_major_no_overlap` | triangle / cubic: bijective site map |
| `degree_based_no_overlap` | same |
| `clustering_no_overlap` | same |
| `all_strategies_on_benchmarks` | ER + cubic fixtures; all three strategies; no overlap |
| `clustering_beats_row_major` | Constructed 2-cluster graph (two dense cliques, sparse bridge): clustering score < row-major score |
| `score_nonnegative_finite` | proptest on ER/cubic |
| `place_preserves_graph_and_layers` | graph equal, layers empty |
| `place_rejects_empty` | EmptyGraph |

Files:

- `quon_na/tests/placement.rs` — unit + acceptance
- `quon_na/tests/placement_props.rs` — proptest no-overlap + finite score

## Flux

Optional small kernel (feature-gated like `report.rs`):

```rust
#[cfg_attr(feature = "flux", spec(fn(a: u32, b: u32) -> bool{v: true}))]
fn sites_distinct(a: SiteId, b: SiteId) -> bool { a != b }
```

Prefer a real invariant if easy: e.g. `grid_capacity(rows, cols) >= n`. Keep Flux optional; do not block stable build. Document in PR under Flux section (run `cargo flux -p quon_na --no-default-features --features flux` if a spec is added; else note N/A with rationale).

## Literature attribution (PR + module docs)

| Claim | Source | Our stance |
| ----- | ------ | ---------- |
| Degree / load-balance fill from center | Atomique Sec. III-B Fig. 6 | Inspired |
| MAX-k-Cut array mapper | Atomique Alg. 1 | Inspired; adapted to single-grid clusters |
| Score `Σ w √d` | OLSQ-DPQA / Enola / RAP √-law; architecture_model §5 | Cited; not RAP Eq. (1) group form |
| Not Enola SA placer | Enola Sec. 4 | Explicitly out of scope (#104 comment / literature_notes) |

## Out of scope

- AOD trap assignment / mid-circuit remapping (#106)
- Edge coloring / layers (#105)
- Zoned RAP A* placement (#107)
- Enola simulated annealing
- MLIR dialect ops for placement

## Implementation order

1. `placement.rs` types + grid helpers + score + overlap check
2. Row-major
3. Degree-based + spiral
4. Interaction-clustering
5. Wire `lib.rs` exports
6. Tests
7. Validate + literature check + commit + `gt submit`

## Acceptance mapping

| Criterion | How verified |
| --------- | ------------ |
| Row-major, no overlaps | unit + prop |
| Degree-based, no overlaps | unit + prop |
| Clustering, no overlaps | unit + prop |
| All three on every benchmark graph | `all_strategies_on_benchmarks` |
| Clustering improves score on ≥1 benchmark | `clustering_beats_row_major` |
